// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Direct TLS-only workspace HTTP runtime. There is no proxy/header identity,
//! browser cookie authentication or native transport fallback.
mod content;
mod dispatch;
mod guidance;
mod invitations;
mod management;
mod request;
mod response;
mod session;
mod team;

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
}

struct State {
    identity: InstanceTls,
    database: PathBuf,
}

#[derive(Clone, Copy)]
enum Surface {
    Https,
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
        });
        Ok(Self {
            https,
            mtls,
            https_tls,
            mtls_tls,
            state,
        })
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
        let mut report = Report::default();
        let outcome = loop {
            tokio::select! {
                result = &mut shutdown => break result,
                result = self.https.accept() => {
                    let (socket, _) = result.context("HTTPS accept failed")?;
                    session::spawn(&mut tasks, socket, self.https_tls.clone(), self.state.clone(), Surface::Https, receiver.clone(), budget.clone());
                }
                result = self.mtls.accept() => {
                    let (socket, _) = result.context("mTLS accept failed")?;
                    session::spawn(&mut tasks, socket, self.mtls_tls.clone(), self.state.clone(), Surface::Client, receiver.clone(), budget.clone());
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
