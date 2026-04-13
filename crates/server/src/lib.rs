// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

pub mod auth_request;
pub mod identity;
pub mod init;
pub mod journal;
pub mod message;
pub mod state;

use std::panic;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use std::net::SocketAddr;

use anyhow::{Context, anyhow, bail};
use protocol::{ClientRequest, ServerResponse, decode, encode};
use rustls::crypto::ring;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};
use rustls::server::WebPkiClientVerifier;
use rustls::{RootCertStore, ServerConfig};
use socket2::{Domain, Socket, Type};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Notify, Semaphore, watch};
use tokio::time::timeout;
use tokio_rustls::{TlsAcceptor, server::TlsStream};
use x509_parser::{extensions::GeneralName, parse_x509_certificate};

use crate::auth_request::try_handle_auth_request;
use crate::identity::{
    ACCESS_URI_PREFIX, ConnectionAuthContext, PeerIdentity, SESSION_URI_PREFIX,
    parse_access_identity_uri, parse_session_identity_uri,
};
use crate::init::{
    auth_listener_addr, ca_cert_path, ensure_active_session_binding, initialize_cpp_server,
    journal_path, mtls_listener_addr, mtls_server_base_url, server_cert_path, server_key_path,
};
use crate::journal::JournalHandle;
use crate::message::handle_message_request;
use crate::state::ServerState;

const MAX_CONCURRENT_CONNECTIONS: usize = 8192;
const AUTH_LISTEN_BACKLOG: i32 = 2048;
const MTLS_LISTEN_BACKLOG: i32 = 2048;
const AUTH_REQUEST_READ_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_AUTH_REQUEST_SIZE: usize = 64 * 1024;
const TLS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const FRAME_READ_TIMEOUT: Duration = Duration::from_secs(30);
const FRAME_WRITE_TIMEOUT: Duration = Duration::from_secs(10);

pub async fn run_server(session_name: &str) -> anyhow::Result<()> {
    let _ = ring::default_provider().install_default();
    let bootstrap = initialize_cpp_server(session_name).await?;
    let journal = Arc::new(JournalHandle::start(journal_path())?);
    let state = Arc::new(ServerState::load_from_storage(Arc::clone(&journal)).await?);
    install_crash_persistence_hook(Arc::clone(&state));
    let shutdown = Arc::new(ShutdownCoordinator::new());

    println!(
        "Initialized session '{}' (id={})",
        bootstrap.session_name, bootstrap.session_id
    );
    println!("Auth redeem endpoint: {}", bootstrap.auth_redeem_url);
    println!("mTLS endpoint: {}", mtls_server_base_url());
    if let Some(token) = &bootstrap.initial_read_token {
        println!("Initial read token (save now): {}", token.token);
    }
    if let Some(token) = &bootstrap.initial_read_write_token {
        println!("Initial read_write token (save now): {}", token.token);
    }

    let auth_listener = {
        let addr: SocketAddr = auth_listener_addr()
            .parse()
            .context("invalid auth listener address")?;
        let domain = if addr.is_ipv4() {
            Domain::IPV4
        } else {
            Domain::IPV6
        };
        let socket = Socket::new(domain, Type::STREAM, None)
            .context("failed to create auth listener socket")?;
        socket
            .set_nonblocking(true)
            .context("failed to set auth listener nonblocking")?;
        socket
            .bind(&addr.into())
            .context("failed to bind auth listener")?;
        socket
            .listen(AUTH_LISTEN_BACKLOG)
            .context("failed to listen on auth socket")?;
        let std_listener: std::net::TcpListener = socket.into();
        TcpListener::from_std(std_listener).context("failed to create tokio auth listener")?
    };
    let mtls_listener = {
        let addr: SocketAddr = mtls_listener_addr()
            .parse()
            .context("invalid mTLS listener address")?;
        let domain = if addr.is_ipv4() {
            Domain::IPV4
        } else {
            Domain::IPV6
        };
        let socket = Socket::new(domain, Type::STREAM, None)
            .context("failed to create mTLS listener socket")?;
        socket
            .set_nonblocking(true)
            .context("failed to set mTLS listener nonblocking")?;
        socket
            .bind(&addr.into())
            .context("failed to bind mTLS listener")?;
        socket
            .listen(MTLS_LISTEN_BACKLOG)
            .context("failed to listen on mTLS socket")?;
        let std_listener: std::net::TcpListener = socket.into();
        TcpListener::from_std(std_listener).context("failed to create tokio mTLS listener")?
    };
    let tls_acceptor = TlsAcceptor::from(build_mtls_server_config(bootstrap.session_id)?);
    let connection_limit = Arc::new(Semaphore::new(MAX_CONCURRENT_CONNECTIONS));

    let auth_task = tokio::spawn({
        let state = Arc::clone(&state);
        let shutdown = Arc::clone(&shutdown);
        let connection_limit = Arc::clone(&connection_limit);
        async move {
            loop {
                let (socket, addr) = auth_listener.accept().await?;
                socket.set_nodelay(true).ok();
                let connection_permit = match timeout(
                    Duration::from_secs(5),
                    Arc::clone(&connection_limit).acquire_owned(),
                )
                .await
                {
                    Ok(Ok(permit)) => permit,
                    _ => {
                        drop(socket);
                        continue;
                    }
                };
                let Some(request_guard) = shutdown.try_track_request() else {
                    drop(socket);
                    continue;
                };
                let state = Arc::clone(&state);
                let shutdown_rx = shutdown.subscribe();

                tokio::spawn(async move {
                    let _connection_permit = connection_permit;
                    let _request_guard = request_guard;
                    if let Err(error) = auth_handler(socket, state, shutdown_rx).await {
                        eprintln!("Auth error from {addr}: {error}");
                    }
                });
            }
            #[allow(unreachable_code)]
            Ok::<(), anyhow::Error>(())
        }
    });

    let mtls_task = tokio::spawn({
        let state = Arc::clone(&state);
        let shutdown = Arc::clone(&shutdown);
        let connection_limit = Arc::clone(&connection_limit);
        async move {
            loop {
                let (socket, addr) = mtls_listener.accept().await?;
                socket.set_nodelay(true).ok();
                let connection_permit = match timeout(
                    Duration::from_secs(5),
                    Arc::clone(&connection_limit).acquire_owned(),
                )
                .await
                {
                    Ok(Ok(permit)) => permit,
                    _ => {
                        drop(socket);
                        continue;
                    }
                };
                let Some(request_guard) = shutdown.try_track_request() else {
                    drop(socket);
                    continue;
                };
                let acceptor = tls_acceptor.clone();
                let state = Arc::clone(&state);
                let shutdown_rx = shutdown.subscribe();
                let expected_session_id = bootstrap.session_id;

                tokio::spawn(async move {
                    let _connection_permit = connection_permit;
                    let _request_guard = request_guard;
                    if let Err(error) = mtls_message_handler(
                        socket,
                        acceptor,
                        state,
                        expected_session_id,
                        shutdown_rx,
                    )
                    .await
                    {
                        eprintln!("mTLS error from {addr}: {error}");
                    }
                });
            }
            #[allow(unreachable_code)]
            Ok::<(), anyhow::Error>(())
        }
    });

    let mut auth_task = auth_task;
    let mut mtls_task = mtls_task;
    let server_result: anyhow::Result<()> = tokio::select! {
        result = &mut auth_task => {
            result.context("auth listener task failed to join")??;
            Ok(())
        }
        result = &mut mtls_task => {
            result.context("mTLS listener task failed to join")??;
            Ok(())
        }
        result = tokio::signal::ctrl_c() => {
            result.context("failed to listen for ctrl-c")?;
            Ok(())
        }
    };

    shutdown.begin_shutdown();
    auth_task.abort();
    mtls_task.abort();
    let _ = auth_task.await;
    let _ = mtls_task.await;
    shutdown.wait_for_idle().await;

    state.mark_sessions_stopped().await?;
    journal.shutdown()?;
    state.persist_snapshot_to_sqlite().await?;
    journal.truncate_blocking()?;

    server_result
}

async fn auth_handler(
    mut socket: TcpStream,
    state: Arc<ServerState>,
    mut shutdown: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    let request = read_full_auth_request(&mut socket, &mut shutdown).await?;
    if try_handle_auth_request(&mut socket, &request, state).await? {
        return Ok(());
    }
    send_plain_404(&mut socket).await
}

async fn read_full_auth_request(
    socket: &mut TcpStream,
    shutdown: &mut watch::Receiver<bool>,
) -> anyhow::Result<String> {
    let mut buf = Vec::with_capacity(4096);
    let mut header_buf = [0u8; 4096];
    loop {
        let n = tokio::select! {
            _ = shutdown.changed() => bail!("shutdown during auth request read"),
            result = timeout(AUTH_REQUEST_READ_TIMEOUT, socket.read(&mut header_buf)) => result,
        }
        .context("auth request read timed out")??;
        if n == 0 {
            bail!("auth connection closed before request complete");
        }
        buf.extend_from_slice(&header_buf[..n]);
        if buf.len() > MAX_AUTH_REQUEST_SIZE {
            bail!("auth request exceeds max size");
        }
        if let Some(header_end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let header_len = header_end + 4;
            let content_length = parse_content_length(&buf[..header_len]).unwrap_or(0);
            let needed = header_len + content_length;
            while buf.len() < needed {
                let to_read = (needed - buf.len()).min(header_buf.len());
                let n = tokio::select! {
                    _ = shutdown.changed() => bail!("shutdown during auth request read"),
                    result = timeout(AUTH_REQUEST_READ_TIMEOUT, socket.read(&mut header_buf[..to_read])) => result,
                }
                .context("auth request read timed out")??;
                if n == 0 {
                    bail!("auth connection closed before body complete");
                }
                buf.extend_from_slice(&header_buf[..n]);
                if buf.len() > MAX_AUTH_REQUEST_SIZE {
                    bail!("auth request exceeds max size");
                }
            }
            return String::from_utf8(buf).context("auth request is not valid UTF-8");
        }
    }
}

fn parse_content_length(headers: &[u8]) -> Option<usize> {
    let headers_str = std::str::from_utf8(headers).ok()?;
    for line in headers_str.lines() {
        let line = line.trim();
        if line.to_lowercase().starts_with("content-length:") {
            let value = line[15..].trim();
            return value.parse().ok();
        }
    }
    None
}

async fn mtls_message_handler(
    socket: TcpStream,
    acceptor: TlsAcceptor,
    state: Arc<ServerState>,
    expected_session_id: i64,
    mut shutdown: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    let mut tls_stream = tokio::select! {
        _ = shutdown.changed() => return Ok(()),
        result = timeout(TLS_HANDSHAKE_TIMEOUT, acceptor.accept(socket)) => result,
    }
    .context("TLS handshake timed out")?
    .context("TLS accept failed")?;
    let peer_identity = extract_peer_identity(&tls_stream)?;
    if peer_identity.session_id != expected_session_id {
        bail!(
            "client certificate session {} does not match active session {}",
            peer_identity.session_id,
            expected_session_id
        );
    }
    let auth_context = ConnectionAuthContext::try_from(&peer_identity)?;

    loop {
        let request = match tokio::select! {
            _ = shutdown.changed() => return Ok(()),
            result = timeout(FRAME_READ_TIMEOUT, read_frame::<ClientRequest, _>(&mut tls_stream)) => result,
        } {
            Ok(Ok(Some(request))) => request,
            Ok(Ok(None)) => return Ok(()),
            Err(_) => {
                let response = ServerResponse::Error(protocol::ErrorResponse {
                    code: protocol::ErrorCode::BadRequest,
                    message: "request timed out".to_string(),
                });
                timeout(FRAME_WRITE_TIMEOUT, write_frame(&mut tls_stream, &response))
                    .await
                    .ok();
                return Ok(());
            }
            Ok(Err(error)) => {
                let response = ServerResponse::Error(protocol::ErrorResponse {
                    code: protocol::ErrorCode::BadRequest,
                    message: error.to_string(),
                });
                timeout(FRAME_WRITE_TIMEOUT, write_frame(&mut tls_stream, &response))
                    .await
                    .context("response write timed out")??;
                return Ok(());
            }
        };

        let response = handle_message_request(&state, &auth_context, request).await;
        timeout(FRAME_WRITE_TIMEOUT, write_frame(&mut tls_stream, &response))
            .await
            .context("response write timed out")??;
    }
}

async fn send_plain_404(socket: &mut (impl AsyncWriteExt + Unpin)) -> anyhow::Result<()> {
    let body = "Unknown route\n";
    let response = format!(
        "HTTP/1.1 404 Not Found\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    socket.write_all(response.as_bytes()).await?;
    Ok(())
}

fn build_mtls_server_config(session_id: i64) -> anyhow::Result<Arc<ServerConfig>> {
    let _ = ensure_active_session_binding(session_id)?;
    let server_cert_path = server_cert_path();
    let server_key_path = server_key_path();
    let ca_cert_path = ca_cert_path();
    let cert_pem = std::fs::read(&server_cert_path).with_context(|| {
        format!(
            "failed to read server cert from {}",
            server_cert_path.display()
        )
    })?;
    let key_pem = std::fs::read(&server_key_path).with_context(|| {
        format!(
            "failed to read server key from {}",
            server_key_path.display()
        )
    })?;
    let ca_pem = std::fs::read(&ca_cert_path)
        .with_context(|| format!("failed to read CA cert from {}", ca_cert_path.display()))?;

    let cert_chain = CertificateDer::pem_slice_iter(&cert_pem)
        .collect::<Result<Vec<_>, _>>()
        .context("failed to parse server certificate chain")?;
    let private_key =
        PrivateKeyDer::from_pem_slice(&key_pem).context("failed to parse server private key")?;

    let mut roots = RootCertStore::empty();
    for cert in CertificateDer::pem_slice_iter(&ca_pem) {
        roots
            .add(cert.context("failed to parse CA certificate")?)
            .context("failed to add CA certificate to root store")?;
    }

    let client_verifier = WebPkiClientVerifier::builder(roots.into())
        .build()
        .context("failed to build client verifier")?;

    let server_config = ServerConfig::builder()
        .with_client_cert_verifier(client_verifier)
        .with_single_cert(cert_chain, private_key)
        .context("failed to build rustls server config")?;

    Ok(Arc::new(server_config))
}

fn extract_peer_identity(stream: &TlsStream<TcpStream>) -> anyhow::Result<PeerIdentity> {
    let (_, connection) = stream.get_ref();
    let peer_cert = connection
        .peer_certificates()
        .and_then(|certs| certs.first())
        .context("missing peer certificate after TLS handshake")?;

    let (_, cert) = parse_x509_certificate(peer_cert.as_ref())
        .map_err(|error| anyhow!("failed to parse peer certificate: {error}"))?;

    let common_name = cert
        .subject()
        .iter_common_name()
        .next()
        .and_then(|cn| cn.as_str().ok())
        .context("peer certificate is missing a UTF-8 common name")?;

    let san = cert
        .subject_alternative_name()
        .map_err(|error| anyhow!("failed to read peer certificate SAN: {error}"))?
        .context("peer certificate is missing a subject alternative name")?;
    let session_uri = san
        .value
        .general_names
        .iter()
        .find_map(|name| match name {
            GeneralName::URI(uri) if uri.starts_with(SESSION_URI_PREFIX) => {
                Some((*uri).to_string())
            }
            _ => None,
        })
        .context("peer certificate is missing a CCP session identity URI")?;
    let session_id = parse_session_identity_uri(&session_uri)?;
    let access_uri = san
        .value
        .general_names
        .iter()
        .find_map(|name| match name {
            GeneralName::URI(uri) if uri.starts_with(ACCESS_URI_PREFIX) => Some((*uri).to_string()),
            _ => None,
        })
        .context("peer certificate is missing a CCP access identity URI")?;
    let access_level = parse_access_identity_uri(&access_uri)?;

    Ok(PeerIdentity {
        common_name: common_name.to_string(),
        session_id,
        access_level,
    })
}

async fn read_frame<T, R>(reader: &mut R) -> anyhow::Result<Option<T>>
where
    T: serde::de::DeserializeOwned,
    R: AsyncRead + Unpin,
{
    let mut header = [0u8; 4];
    match reader.read_exact(&mut header).await {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error).context("failed to read frame header"),
    }

    let frame_len = u32::from_be_bytes(header) as usize;
    if frame_len == 0 {
        bail!("empty frames are not allowed");
    }
    if frame_len > 8 * 1024 * 1024 {
        bail!("frame exceeds maximum size");
    }

    let mut payload = vec![0u8; frame_len];
    reader
        .read_exact(&mut payload)
        .await
        .context("failed to read frame payload")?;
    decode(&payload)
        .context("failed to decode framed payload")
        .map(Some)
}

async fn write_frame<T, W>(writer: &mut W, value: &T) -> anyhow::Result<()>
where
    T: serde::Serialize,
    W: AsyncWrite + Unpin,
{
    let payload = encode(value).context("failed to encode response frame")?;
    let frame_len = u32::try_from(payload.len()).context("encoded frame is too large")?;
    writer
        .write_all(&frame_len.to_be_bytes())
        .await
        .context("failed to write frame header")?;
    writer
        .write_all(&payload)
        .await
        .context("failed to write frame payload")?;
    writer.flush().await.context("failed to flush frame")?;
    Ok(())
}

fn install_crash_persistence_hook(state: Arc<ServerState>) {
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |panic_info| {
        let _ = state.try_persist_snapshot_to_sqlite();
        previous(panic_info);
    }));
}

struct ShutdownCoordinator {
    is_shutting_down: AtomicBool,
    active_requests: AtomicUsize,
    idle_notify: Notify,
    shutdown_tx: watch::Sender<bool>,
}

impl ShutdownCoordinator {
    fn new() -> Self {
        let (shutdown_tx, _) = watch::channel(false);
        Self {
            is_shutting_down: AtomicBool::new(false),
            active_requests: AtomicUsize::new(0),
            idle_notify: Notify::new(),
            shutdown_tx,
        }
    }

    fn try_track_request(self: &Arc<Self>) -> Option<RequestGuard> {
        if self.is_shutting_down.load(Ordering::Acquire) {
            return None;
        }

        self.active_requests.fetch_add(1, Ordering::AcqRel);
        if self.is_shutting_down.load(Ordering::Acquire) {
            self.finish_request();
            return None;
        }

        Some(RequestGuard {
            coordinator: Arc::clone(self),
        })
    }

    fn subscribe(&self) -> watch::Receiver<bool> {
        self.shutdown_tx.subscribe()
    }

    fn begin_shutdown(&self) {
        if self.is_shutting_down.swap(true, Ordering::AcqRel) {
            return;
        }
        let _ = self.shutdown_tx.send(true);
        if self.active_requests.load(Ordering::Acquire) == 0 {
            self.idle_notify.notify_waiters();
        }
    }

    async fn wait_for_idle(&self) {
        while self.active_requests.load(Ordering::Acquire) != 0 {
            self.idle_notify.notified().await;
        }
    }

    fn finish_request(&self) {
        if self.active_requests.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.idle_notify.notify_waiters();
        }
    }
}

struct RequestGuard {
    coordinator: Arc<ShutdownCoordinator>,
}

impl Drop for RequestGuard {
    fn drop(&mut self) {
        self.coordinator.finish_request();
    }
}
