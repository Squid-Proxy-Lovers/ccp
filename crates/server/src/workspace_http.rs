// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Direct TLS-only workspace HTTP with current scoped admin keys, ephemeral
//! browser sessions, and verified mTLS client identity. Native mode is separate.
mod browser;
mod content;
mod dispatch;
mod guidance;
mod invitation_link;
mod invitations;
mod management;
mod panel;
mod request;
mod response;
mod retention;
mod retention_worker;
mod session;
mod setup;
mod up;
pub use up::UpConfig;
mod team;
mod tunnel;
mod work_status;
mod workspace_deletion;

use std::future::Future;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, bail};
use serde::Serialize;
use tokio::net::TcpListener;
use tokio::sync::{Semaphore, watch};
use tokio::task::JoinSet;
use tokio::time::timeout;
use tokio_rustls::TlsAcceptor;

use crate::directory_mode::DirectoryLease;
use crate::instance_tls::InstanceTls;

pub struct Config {
    pub data_dir: PathBuf,
    pub https_listen: SocketAddr,
    pub mtls_listen: SocketAddr,
    pub activate_workspace: bool,
}

#[derive(Serialize)]
pub struct Startup {
    pub api_version: u32,
    pub instance_id: String,
    pub https_address: SocketAddr,
    pub mtls_address: SocketAddr,
}

#[derive(Default, Serialize)]
pub struct Report {
    pub failed_connections: usize,
}

pub struct WorkspaceServer {
    https: TcpListener,
    mtls: TcpListener,
    https_tls: TlsAcceptor,
    mtls_tls: TlsAcceptor,
    state: Arc<State>,
    cleanup_budget: Arc<Semaphore>,
    websocket_tunnels: bool,
}

struct State {
    identity: InstanceTls,
    database: PathBuf,
    browser_sessions: browser::Sessions,
    setup: Option<crate::setup_code::SetupCode>,
    mtls_port: u16,
}

#[derive(Clone, Copy)]
enum Surface {
    Https,
    Bootstrap,
    Client,
}

impl WorkspaceServer {
    pub async fn bind(config: Config) -> anyhow::Result<Self> {
        if !config.data_dir.join("ccp.sqlite3").is_file() {
            bail!("run workspace-init and workspace-tls-init first");
        }
        let https = TcpListener::bind(config.https_listen)
            .await
            .context("cannot bind workspace HTTPS listener")?;
        let mtls = TcpListener::bind(config.mtls_listen)
            .await
            .context("cannot bind workspace mTLS listener")?;
        crate::instance_tls::info(&config.data_dir)?;
        let existing = rusqlite::Connection::open_with_flags(
            config.data_dir.join("ccp.sqlite3"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        if !setup::owned(&existing)
            .map_err(|_database| anyhow::anyhow!("cannot inspect workspace initialization"))?
        {
            bail!("setup is pending; use server up to obtain a setup link");
        }
        drop(existing);
        let browser_sessions = browser::Sessions::new()
            .map_err(|_entropy| anyhow::anyhow!("cannot initialize browser sessions"))?;
        let lease = if config.activate_workspace {
            DirectoryLease::activate_workspace(&config.data_dir)?
        } else {
            DirectoryLease::resume_workspace(&config.data_dir)?
        };
        let identity = InstanceTls::load(&lease)?;
        let https_tls = TlsAcceptor::from(identity.https_config()?);
        let mtls_tls = TlsAcceptor::from(identity.client_mtls_config()?);
        let state = Arc::new(State {
            identity,
            database: lease.directory().join("ccp.sqlite3"),
            browser_sessions,
            setup: None,
            mtls_port: mtls.local_addr()?.port(),
        });
        Ok(Self {
            https,
            mtls,
            https_tls,
            mtls_tls,
            state,
            cleanup_budget: Arc::new(Semaphore::new(1)),
            websocket_tunnels: false,
        })
    }

    /// Opt into fixed-path WSS carriers on the existing HTTPS listener.
    pub fn with_websocket_tunnels(mut self, enabled: bool) -> Self {
        self.websocket_tunnels = enabled;
        self
    }

    pub fn startup(&self) -> anyhow::Result<Startup> {
        Ok(Startup {
            api_version: protocol::workspace::API_VERSION,
            instance_id: self.state.identity.trust().instance_id.clone(),
            https_address: self.https.local_addr()?,
            mtls_address: self.mtls.local_addr()?,
        })
    }

    pub async fn run_until(
        self,
        shutdown: impl Future<Output = anyhow::Result<()>>,
    ) -> anyhow::Result<Report> {
        tokio::pin!(shutdown);
        let budget = Arc::new(Semaphore::new(64));
        let (stop, receiver) = watch::channel(false);
        let mut tasks = JoinSet::new();
        tasks.spawn(retention_worker::run(
            self.state.clone(),
            receiver.clone(),
            self.cleanup_budget.clone(),
        ));
        let mut report = Report::default();
        let outcome = loop {
            tokio::select! {
                result = &mut shutdown => break result,
                result = self.https.accept() => {
                    let (socket, _) = result.context("HTTPS accept failed")?;
                    session::spawn(&mut tasks, socket, session::Endpoint { acceptor: self.https_tls.clone(), surface: Surface::Https, tunnels: self.websocket_tunnels }, self.state.clone(), receiver.clone(), budget.clone());
                }
                result = self.mtls.accept() => {
                    let (socket, _) = result.context("mTLS accept failed")?;
                    session::spawn(&mut tasks, socket, session::Endpoint { acceptor: self.mtls_tls.clone(), surface: Surface::Client, tunnels: false }, self.state.clone(), receiver.clone(), budget.clone());
                }
                result = tasks.join_next(), if !tasks.is_empty() => collect_task(result, &mut report)?,
            }
        };
        stop.send(true)
            .context("workspace shutdown channel failed")?;
        let drain = async {
            while let Some(result) = tasks.join_next().await {
                collect_task(Some(result), &mut report)?;
            }
            Ok::<_, anyhow::Error>(())
        };
        match timeout(Duration::from_secs(30), drain).await {
            Ok(result) => result?,
            Err(_drain_timeout) => {
                tasks.abort_all();
                while let Some(result) = tasks.join_next().await {
                    if result.as_ref().is_err_and(|error| error.is_cancelled()) {
                        continue;
                    }
                    collect_task(Some(result), &mut report)?;
                }
            }
        }
        // A cancelled HTTP future cannot cancel spawn_blocking. Wait for every
        // admission slot, including those retained by detached SQL jobs, before
        // reporting a successful stop or releasing the runtime's lease.
        let all_slots = timeout(Duration::from_secs(30), budget.acquire_many_owned(64))
            .await
            .context("workspace database jobs did not drain")?
            .context("workspace admission budget closed")?;
        drop(all_slots);
        let cleanup_slot = timeout(
            Duration::from_secs(30),
            self.cleanup_budget.clone().acquire_owned(),
        )
        .await
        .context("workspace retention job did not drain")?
        .context("workspace retention budget closed")?;
        drop(cleanup_slot);
        outcome?;
        Ok(report)
    }
}

fn collect_task(
    result: Option<Result<anyhow::Result<()>, tokio::task::JoinError>>,
    report: &mut Report,
) -> anyhow::Result<()> {
    match result {
        Some(Ok(Err(_connection_error))) => {
            report.failed_connections = report.failed_connections.saturating_add(1)
        }
        Some(Err(error)) => return Err(error.into()),
        _ => (),
    }
    Ok(())
}
