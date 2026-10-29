// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;
use crate::setup_code::SetupCode;
use rusqlite::Connection;

pub struct UpConfig {
    pub data_dir: PathBuf,
    pub names: Vec<String>,
    pub https_listen: SocketAddr,
    pub mtls_listen: SocketAddr,
}
impl WorkspaceServer {
    /// Bind before first-boot filesystem changes; never create a provisional admin.
    /// Returned plaintext capability belongs only in host-controlled startup output.
    pub async fn up(config: UpConfig) -> anyhow::Result<(Self, Option<String>)> {
        let https = TcpListener::bind(config.https_listen)
            .await
            .context("cannot bind workspace HTTPS listener")?;
        let mtls = TcpListener::bind(config.mtls_listen)
            .await
            .context("cannot bind workspace mTLS listener")?;
        if config.names.iter().any(|name| {
            name.parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_unspecified())
        }) {
            bail!("TLS names must not be unspecified listener addresses");
        }
        crate::setup_reservation::prepare(&config.data_dir)?;
        let lease = DirectoryLease::maintenance(&config.data_dir)?;
        crate::setup_reservation::validate_leased(&lease)?;
        crate::instance_tls::initialize_setup_leased(&lease, &config.names)?;
        let lease = lease.promote_workspace()?;
        let identity = InstanceTls::load(&lease)?;
        let database = lease.directory().join("ccp.sqlite3");
        // Real current state under the exclusive runtime lease, not the immutable
        // preflight observation (which may not include a pending WAL recovery).
        let admins: u64 = Connection::open(&database)?.query_row(
            "SELECT EXISTS(SELECT 1 FROM ccp_server_admins) OR EXISTS(SELECT 1 FROM ccp_setup_reservation WHERE claimed_at IS NOT NULL)",
            [],
            |row| row.get(0),
        )?;
        let (setup, code) = if admins == 0 {
            let (gate, code) = SetupCode::issue()
                .map_err(|_entropy| anyhow::anyhow!("cannot generate setup code"))?;
            (Some(gate), Some(code))
        } else {
            (None, None)
        };
        let https_tls = TlsAcceptor::from(identity.https_config()?);
        let mtls_tls = TlsAcceptor::from(identity.client_mtls_config()?);
        let browser_sessions = browser::Sessions::new()
            .map_err(|_entropy| anyhow::anyhow!("cannot initialize browser sessions"))?;
        Ok((
            Self {
                https,
                mtls,
                https_tls,
                mtls_tls,
                state: Arc::new(State {
                    identity,
                    database,
                    browser_sessions,
                    setup,
                }),
                cleanup_budget: Arc::new(Semaphore::new(1)),
                websocket_tunnels: false,
            },
            code,
        ))
    }
    pub fn panel_origin(&self) -> anyhow::Result<String> {
        let name = self
            .state
            .identity
            .trust()
            .names
            .iter()
            .find(|name| name.as_str() == "localhost")
            .or_else(|| {
                self.state.identity.trust().names.iter().find(|name| {
                    name.parse::<std::net::IpAddr>()
                        .map_or(true, |ip| !ip.is_unspecified())
                })
            })
            .context("a usable advertised TLS name is required")?;
        let name = if name.parse::<std::net::Ipv6Addr>().is_ok() {
            format!("[{name}]")
        } else {
            name.clone()
        };
        Ok(format!(
            "https://{name}:{}",
            self.https.local_addr()?.port()
        ))
    }
    /// Public trust material for local operators and test harnesses only.
    pub fn trust_info(&self) -> &crate::instance_tls::TrustInfo {
        self.state.identity.trust()
    }
    pub fn ca_fingerprint(&self) -> &str {
        &self.state.identity.trust().ca_sha256
    }
}
