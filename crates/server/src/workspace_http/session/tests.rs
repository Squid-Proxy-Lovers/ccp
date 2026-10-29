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
