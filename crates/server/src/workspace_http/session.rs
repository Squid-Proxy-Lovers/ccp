// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::response::{Error, Reply};
use super::*;
use crate::instance_tls::CapturedPeer;
use anyhow::Context as ErrorContext;
use ccp_transport::{
    ByteStream,
    websocket::{WebSocketStream, tungstenite::protocol::Role},
    websocket_config,
};
use hyper::{Request, body::Incoming, server::conn::http1, service::service_fn};
use hyper_util::rt::{TokioIo, TokioTimer};
use std::{
    convert::Infallible,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
use tokio::{
    io::{AsyncRead, AsyncWrite, AsyncWriteExt},
    net::TcpStream,
    sync::OwnedSemaphorePermit,
    time::{Instant, timeout_at},
};

pub(super) struct Context {
    pub state: Arc<State>,
    pub surface: Surface,
    pub peer: Option<Arc<CapturedPeer>>,
    // Blocking jobs retain the same slot and lease even after a disconnect.
    budget: Arc<OwnedSemaphorePermit>,
}
pub(super) struct Endpoint {
    pub acceptor: TlsAcceptor,
    pub surface: Surface,
    pub tunnels: bool,
}
pub(super) fn spawn(
    tasks: &mut JoinSet<anyhow::Result<()>>,
    socket: TcpStream,
    endpoint: Endpoint,
    state: Arc<State>,
    stop: watch::Receiver<bool>,
    budget: Arc<Semaphore>,
) {
    if let Ok(permit) = budget.try_acquire_owned() {
        let deadline = Instant::now() + Duration::from_secs(120);
        tasks.spawn(run(
            socket,
            endpoint,
            state,
            stop,
            Arc::new(permit),
            deadline,
        ));
    }
}
async fn run<S: AsyncRead + AsyncWrite + Unpin + Send + 'static>(
    socket: S,
    endpoint: Endpoint,
    state: Arc<State>,
    mut stop: watch::Receiver<bool>,
    budget: Arc<OwnedSemaphorePermit>,
    deadline: Instant,
) -> anyhow::Result<()> {
    tokio::select! {
        _=stop.changed()=>Ok(()),
        result=timeout_at(deadline,serve(socket,endpoint,state,budget))=>result.context("HTTP connection deadline exceeded")?,
    }
}
fn context(
    state: Arc<State>,
    surface: Surface,
    peer: &rustls::ServerConnection,
    budget: Arc<OwnedSemaphorePermit>,
) -> anyhow::Result<Arc<Context>> {
    let peer = match surface {
        Surface::Https | Surface::Bootstrap => None,
        Surface::Client => Some(Arc::new(state.identity.capture_peer(peer)?)),
    };
    Ok(Arc::new(Context {
        state,
        surface,
        peer,
        budget,
    }))
}
async fn serve<S: AsyncRead + AsyncWrite + Unpin + Send + 'static>(
    socket: S,
    endpoint: Endpoint,
    state: Arc<State>,
    budget: Arc<OwnedSemaphorePermit>,
) -> anyhow::Result<()> {
    let tls = timeout(Duration::from_secs(5), endpoint.acceptor.accept(socket))
        .await
        .context("TLS handshake timed out")??;
    let context = context(state, endpoint.surface, tls.get_ref().1, budget)?;
    let slot = endpoint.tunnels.then(|| Arc::new(Mutex::new(None)));
    if let Some(pending) = serve_http(tls, context.clone(), slot).await? {
        serve_inner(pending, context).await?;
    }
    Ok(())
}
async fn serve_inner(pending: tunnel::Pending, outer: Arc<Context>) -> anyhow::Result<()> {
    let acceptor = match pending.surface {
        Surface::Bootstrap => outer.state.identity.https_config()?,
        Surface::Client => outer.state.identity.client_mtls_config()?,
        Surface::Https => anyhow::bail!("invalid inner carrier surface"),
    };
    // Upgraded replays any first-frame bytes already buffered by Hyper. Do not downcast it.
    let upgraded = timeout(Duration::from_secs(5), pending.upgrade)
        .await
        .context("HTTP upgrade timed out")??;
    let websocket = WebSocketStream::from_raw_socket(
        TokioIo::new(upgraded),
        Role::Server,
        Some(websocket_config()),
    )
    .await;
    let tls = timeout(
        Duration::from_secs(5),
        accept_inner(TlsAcceptor::from(acceptor), ByteStream::new(websocket)),
    )
    .await
    .context("inner TLS handshake timed out")??;
    let context = context(
        outer.state.clone(),
        pending.surface,
        tls.get_ref().1,
        outer.budget.clone(),
    )?;
    serve_http(tls, context, None).await?;
    Ok(())
}
async fn accept_inner<S: AsyncRead + AsyncWrite + Unpin>(
    acceptor: TlsAcceptor,
    stream: S,
) -> anyhow::Result<tokio_rustls::server::TlsStream<S>> {
    match acceptor.accept(stream).into_fallible().await {
        Ok(tls) => Ok(tls),
        Err((error, mut stream)) => {
            // Rustls's last-gasp alert write may be buffered by the WS sink.
            // Flush it within the caller's handshake/connection deadline.
            stream
                .flush()
                .await
                .context("cannot flush inner TLS alert")?;
            Err(error.into())
        }
    }
}
async fn serve_http<S: AsyncRead + AsyncWrite + Unpin + Send + 'static>(
    socket: S,
    context: Arc<Context>,
    slot: Option<tunnel::Slot>,
) -> anyhow::Result<Option<tunnel::Pending>> {
    let count = AtomicUsize::new(0);
    let request_slot = slot.clone();
    let service = service_fn(move |request: Request<Incoming>| {
        let context = context.clone();
        let slot = request_slot.clone();
        let index = count.fetch_add(1, Ordering::Relaxed);
        async move {
            let reply = if index >= 100 {
                Error::Unavailable.reply()
            } else if tunnel::handles(request.uri().path()) && slot.is_some() {
                match slot.filter(|_| index == 0) {
                    Some(slot) => tunnel::accept(request, &slot)
                        .unwrap_or_else(|error| response::close(error.reply())),
                    None => response::close(Error::BadRequest.reply()),
                }
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
    builder
        .serve_connection(TokioIo::new(socket), service)
        .with_upgrades()
        .await?;
    // Only now is OnUpgrade ready. The calling JoinSet task retains the slot/lease.
    slot.map(|slot| {
        slot.lock()
            .map(|mut pending| pending.take())
            .map_err(|_poisoned| anyhow::anyhow!("upgrade slot poisoned"))
    })
    .transpose()
    .map(Option::flatten)
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

#[cfg(test)]
mod tests;
