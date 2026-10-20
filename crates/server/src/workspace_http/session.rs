// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::response::{Error, Reply};
use super::*;
use crate::instance_tls::CapturedPeer;
use anyhow::Context as ErrorContext;
use hyper::{Request, body::Incoming, server::conn::http1, service::service_fn};
use hyper_util::rt::{TokioIo, TokioTimer};
use std::convert::Infallible;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::net::TcpStream;
use tokio::sync::OwnedSemaphorePermit;

pub(super) struct Context {
    pub state: Arc<State>,
    pub surface: Surface,
    pub peer: Option<Arc<CapturedPeer>>,
    // Retained by blocking jobs too: disconnecting must not free the admission
    // slot while SQL work continues, nor allow that work to outlive the lease.
    _budget: Arc<OwnedSemaphorePermit>,
}

pub(super) fn spawn(
    tasks: &mut JoinSet<anyhow::Result<()>>,
    socket: TcpStream,
    acceptor: TlsAcceptor,
    state: Arc<State>,
    surface: Surface,
    stop: watch::Receiver<bool>,
    budget: Arc<Semaphore>,
) {
    if let Ok(permit) = budget.try_acquire_owned() {
        tasks.spawn(serve(
            socket,
            acceptor,
            state,
            surface,
            stop,
            Arc::new(permit),
        ));
    }
}

async fn serve(
    socket: TcpStream,
    acceptor: TlsAcceptor,
    state: Arc<State>,
    surface: Surface,
    mut stop: watch::Receiver<bool>,
    budget: Arc<OwnedSemaphorePermit>,
) -> anyhow::Result<()> {
    let tls = tokio::select! {
        _ = stop.changed() => return Ok(()),
        result = timeout(Duration::from_secs(5),acceptor.accept(socket)) => result.context("TLS handshake timed out")??,
    };
    let peer = match surface {
        Surface::Https => None,
        Surface::Client => Some(Arc::new(state.identity.capture_peer(tls.get_ref().1)?)),
    };
    let context = Arc::new(Context {
        state,
        surface,
        peer,
        _budget: budget,
    });
    let count = AtomicUsize::new(0);
    let service = service_fn(move |request| {
        let context = context.clone();
        let index = count.fetch_add(1, Ordering::Relaxed);
        async move {
            let reply = if index >= 100 {
                Error::Unavailable.reply()
            } else {
                handle(context, request).await
            };
            Ok::<_, Infallible>(if index >= 99 {
                response::close(reply)
            } else {
                reply
            })
        }
    });
    let mut builder = http1::Builder::new();
    builder
        .timer(TokioTimer::new())
        .header_read_timeout(Duration::from_secs(10))
        .max_headers(32)
        .max_header_size(8192)
        .max_buf_size(8192)
        .keep_alive(true);
    let connection = builder.serve_connection(TokioIo::new(tls), service);
    tokio::select! {
        _ = stop.changed() => Ok(()),
        result = timeout(Duration::from_secs(120),connection) => { result.context("HTTP connection deadline exceeded")??; Ok(()) },
    }
}

async fn handle(context: Arc<Context>, request: Request<Incoming>) -> Reply {
    let input = match request::read(request).await {
        Ok(input) => input,
        Err(error) => return response::close(error.reply()),
    };
    match tokio::task::spawn_blocking(move || dispatch::dispatch(&context, input)).await {
        Ok(Ok(reply)) => reply,
        Ok(Err(error)) => error.reply(),
        Err(_task_error) => response::close(Error::Unavailable.reply()),
    }
}
