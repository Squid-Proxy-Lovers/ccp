// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use ccp_transport::{ENROLL_PATH, SUBPROTOCOL, websocket::tungstenite::client::IntoClientRequest};
use rustls::{
    ClientConfig, RootCertStore,
    pki_types::{CertificateDer, ServerName, pem::PemObject},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    time::advance,
};
use tokio_rustls::TlsConnector;
use uuid::Uuid;

async fn health<S: AsyncRead + AsyncWrite + Unpin>(stream: &mut S) {
    stream
        .write_all(b"GET /api/v1/health HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .await
        .unwrap();
    stream.flush().await.unwrap();
    let mut header = Vec::new();
    while !header.ends_with(b"\r\n\r\n") {
        let mut byte = [0];
        stream.read_exact(&mut byte).await.unwrap();
        header.push(byte[0]);
        assert!(header.len() < 8192);
    }
    let header = String::from_utf8(header).unwrap();
    assert!(header.starts_with("HTTP/1.1 200"));
    let length: usize = header
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length: ")
                .map(str::to_owned)
        })
        .unwrap()
        .parse()
        .unwrap();
    let mut body = vec![0; length];
    stream.read_exact(&mut body).await.unwrap();
    assert!(
        serde_json::from_slice::<serde_json::Value>(&body).unwrap()["data"]["instance_id"]
            .is_string()
    );
}
#[tokio::test(start_paused = true)]
async fn one_deadline_covers_outer_tls_upgrade_and_inner_http_without_renewing() {
    let directory = std::env::temp_dir().join(format!("ccp-carrier-deadline-{}", Uuid::new_v4()));
    crate::local_admin::bootstrap(&directory, "owner", "first").unwrap();
    let trust = crate::instance_tls::initialize(&directory, &["localhost".into()]).unwrap();
    let lease = DirectoryLease::activate_workspace(&directory).unwrap();
    let identity = InstanceTls::load(&lease).unwrap();
    let acceptor = TlsAcceptor::from(identity.https_config().unwrap());
    let state = Arc::new(State {
        identity,
        database: directory.join("ccp.sqlite3"),
        browser_sessions: browser::Sessions::new().unwrap(),
        setup: None,
        mtls_port: 8444,
    });
    let (a, b) = tokio::io::duplex(65536);
    let (stop, receiver) = watch::channel(false);
    let budget = Arc::new(Semaphore::new(1));
    let permit = Arc::new(budget.clone().try_acquire_owned().unwrap());
    let started = Instant::now();
    let task = tokio::spawn(run(
        a,
        Endpoint {
            acceptor,
            surface: Surface::Https,
            tunnels: true,
        },
        state,
        receiver,
        permit,
        started + Duration::from_secs(120),
    ));
    advance(Duration::from_secs(3)).await;
    let mut roots = RootCertStore::empty();
    roots
        .add(CertificateDer::from_pem_slice(trust.ca_certificate_pem.as_bytes()).unwrap())
        .unwrap();
    let config =
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth();
    let connector = TlsConnector::from(Arc::new(config));
    let outer = connector
        .connect(ServerName::try_from("localhost").unwrap(), b)
        .await
        .unwrap();
    advance(Duration::from_secs(8)).await;
    let mut request = format!("wss://localhost{ENROLL_PATH}")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("sec-websocket-protocol", SUBPROTOCOL.parse().unwrap());
    let (ws, _) = ccp_transport::websocket::client_async_with_config(
        request,
        outer,
        Some(websocket_config()),
    )
    .await
    .unwrap();
    advance(Duration::from_secs(3)).await;
    let mut inner = connector
        .connect(
            ServerName::try_from("localhost").unwrap(),
            ByteStream::new(ws),
        )
        .await
        .unwrap();
    health(&mut inner).await;
    while Instant::now() - started < Duration::from_secs(118) {
        advance(Duration::from_secs(4)).await;
        health(&mut inner).await;
    }
    assert!(!task.is_finished());
    assert_eq!(budget.available_permits(), 0);
    advance(Duration::from_secs(3)).await;
    let error = task.await.unwrap().unwrap_err();
    assert!(error.to_string().contains("deadline exceeded"));
    assert_eq!(budget.available_permits(), 1);
    drop(inner);
    drop(connector);
    drop(stop);
    drop(lease);
    let released = DirectoryLease::maintenance(&directory).unwrap();
    drop(released);
    std::fs::remove_dir_all(directory).unwrap();
}
#[test]
fn admission_slots_default_and_clamp() {
    assert_eq!(parse_admission_slots(None), 64);
    assert_eq!(parse_admission_slots(Some("")), 64);
    assert_eq!(parse_admission_slots(Some("many")), 64);
    assert_eq!(parse_admission_slots(Some("-5")), 64);
    assert_eq!(parse_admission_slots(Some("0")), 16);
    assert_eq!(parse_admission_slots(Some("8")), 16);
    assert_eq!(parse_admission_slots(Some("256")), 256);
    assert_eq!(parse_admission_slots(Some(" 512 ")), 512);
    assert_eq!(parse_admission_slots(Some("4096")), 4096);
    assert_eq!(parse_admission_slots(Some("100000")), 4096);
    assert_eq!(parse_admission_slots(Some("99999999999999999999999")), 64);
}
struct Fixture {
    directory: std::path::PathBuf,
    lease: DirectoryLease,
    ca: String,
    acceptor: TlsAcceptor,
    state: Arc<State>,
}
fn fixture(name: &str) -> Fixture {
    let directory = std::env::temp_dir().join(format!("ccp-{name}-{}", Uuid::new_v4()));
    crate::local_admin::bootstrap(&directory, "owner", "first").unwrap();
    let trust = crate::instance_tls::initialize(&directory, &["localhost".into()]).unwrap();
    let lease = DirectoryLease::activate_workspace(&directory).unwrap();
    let identity = InstanceTls::load(&lease).unwrap();
    let acceptor = TlsAcceptor::from(identity.https_config().unwrap());
    let state = Arc::new(State {
        identity,
        database: directory.join("ccp.sqlite3"),
        browser_sessions: browser::Sessions::new().unwrap(),
        setup: None,
        mtls_port: 8444,
    });
    Fixture {
        directory,
        lease,
        ca: trust.ca_certificate_pem,
        acceptor,
        state,
    }
}
fn finish(fixture: Fixture) {
    let Fixture {
        directory,
        lease,
        acceptor,
        state,
        ..
    } = fixture;
    drop(acceptor);
    drop(state);
    drop(lease);
    let released = DirectoryLease::maintenance(&directory).unwrap();
    drop(released);
    std::fs::remove_dir_all(directory).unwrap();
}
fn https(fixture: &Fixture) -> Endpoint {
    Endpoint {
        acceptor: fixture.acceptor.clone(),
        surface: Surface::Https,
        tunnels: false,
    }
}
#[tokio::test(start_paused = true)]
async fn busy_slots_queue_a_connection_until_one_frees() {
    let fixture = fixture("admission-queue");
    let (a, b) = tokio::io::duplex(65536);
    let (stop, receiver) = watch::channel(false);
    let budget = Arc::new(Semaphore::new(1));
    let held = budget.clone().try_acquire_owned().unwrap();
    let task = tokio::spawn(queued(
        a,
        https(&fixture),
        fixture.state.clone(),
        receiver,
        budget.clone(),
    ));
    advance(Duration::from_secs(4)).await;
    assert!(!task.is_finished());
    drop(held);
    let mut roots = RootCertStore::empty();
    roots
        .add(CertificateDer::from_pem_slice(fixture.ca.as_bytes()).unwrap())
        .unwrap();
    let config =
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth();
    let mut client = TlsConnector::from(Arc::new(config))
        .connect(ServerName::try_from("localhost").unwrap(), b)
        .await
        .unwrap();
    health(&mut client).await;
    assert_eq!(budget.available_permits(), 0);
    client.shutdown().await.unwrap();
    let mut rest = Vec::new();
    client.read_to_end(&mut rest).await.unwrap();
    task.await.unwrap().unwrap();
    drop(client);
    assert_eq!(budget.available_permits(), 1);
    drop(stop);
    finish(fixture);
}
#[tokio::test(start_paused = true)]
async fn queued_connection_closes_after_the_admission_wait() {
    let fixture = fixture("admission-wait");
    let (a, mut b) = tokio::io::duplex(65536);
    let (stop, receiver) = watch::channel(false);
    let budget = Arc::new(Semaphore::new(1));
    let held = budget.clone().try_acquire_owned().unwrap();
    let task = tokio::spawn(queued(
        a,
        https(&fixture),
        fixture.state.clone(),
        receiver,
        budget.clone(),
    ));
    advance(ADMISSION_WAIT + Duration::from_secs(1)).await;
    task.await.unwrap().unwrap();
    assert_eq!(b.read(&mut [0; 1]).await.unwrap(), 0);
    assert_eq!(budget.available_permits(), 0);
    drop(held);
    drop(stop);
    finish(fixture);
}
#[tokio::test(start_paused = true)]
async fn shutdown_releases_a_queued_connection() {
    let fixture = fixture("admission-stop");
    let (a, b) = tokio::io::duplex(65536);
    let (stop, receiver) = watch::channel(false);
    let budget = Arc::new(Semaphore::new(1));
    let held = budget.clone().try_acquire_owned().unwrap();
    let task = tokio::spawn(queued(
        a,
        https(&fixture),
        fixture.state.clone(),
        receiver,
        budget.clone(),
    ));
    advance(Duration::from_secs(1)).await;
    stop.send(true).unwrap();
    task.await.unwrap().unwrap();
    drop(held);
    assert_eq!(budget.available_permits(), 1);
    drop(b);
    finish(fixture);
}
