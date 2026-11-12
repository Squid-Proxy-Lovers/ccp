use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_tracks_stalled_inner_handshake_and_releases_instance_lease() {
    let f = Fixture::with_tunnels(true).await;
    let _pending = upgrade(&f, ENROLL_PATH).await;
    tokio::time::timeout(Duration::from_secs(3), f.finish())
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sixty_four_live_tunnels_share_budget_and_a_released_slot_recovers() {
    let f = Fixture::with_tunnels(true).await;
    let mut connections = Vec::new();
    for _ in 0..64 {
        let mut wire = connect(&f, None).await;
        assert_eq!(send(&mut wire, "/api/v1/health", None, "").await.0, 200);
        connections.push(wire);
    }
    let socket = TcpStream::connect(f.startup.https_address).await.unwrap();
    let refused = TlsConnector::from(Arc::new(tls_config(
        Some(&f.trust.ca_certificate_pem),
        None,
    )))
    .connect(ServerName::try_from("localhost").unwrap(), socket)
    .await
    .unwrap_err();
    assert!(matches!(
        refused.kind(),
        std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::ConnectionReset
    ));
    drop(connections.pop().unwrap());
    // EOF can precede permit release. Bounded retry waits for that real lifecycle event.
    let recovered = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let socket = TcpStream::connect(f.startup.https_address).await.unwrap();
            let result = TlsConnector::from(Arc::new(tls_config(
                Some(&f.trust.ca_certificate_pem),
                None,
            )))
            .connect(ServerName::try_from("localhost").unwrap(), socket)
            .await;
            if let Ok(tls) = result {
                break tls;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    drop(recovered);
    tokio::time::timeout(Duration::from_secs(3), f.finish())
        .await
        .unwrap();
    drop(connections);
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_client_certificate_produces_specific_inner_tls_alert() {
    let f = Fixture::with_tunnels(true).await;
    let ws = upgrade(&f, CLIENT_PATH).await;
    let mut tls = TlsConnector::from(Arc::new(tls_config(
        Some(&f.trust.ca_certificate_pem),
        None,
    )))
    .connect(
        ServerName::try_from("localhost").unwrap(),
        ByteStream::new(ws),
    )
    .await
    .unwrap();
    let error = tokio::time::timeout(Duration::from_secs(2), tls.read(&mut [0; 1]))
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    assert!(matches!(
        error
            .get_ref()
            .and_then(|e| e.downcast_ref::<rustls::Error>()),
        Some(rustls::Error::AlertReceived(
            rustls::AlertDescription::CertificateRequired
        ))
    ));
    assert!(error.to_string().contains("CertificateRequired"));
    drop(tls);
    f.finish().await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn idle_inner_handshake_hits_its_own_deadline_and_returns_permit() {
    use futures_util::StreamExt;
    let f = Fixture::with_tunnels(true).await;
    let mut ws = upgrade(&f, ENROLL_PATH).await;
    let start = std::time::Instant::now();
    let closed = tokio::time::timeout(Duration::from_secs(7), ws.next())
        .await
        .unwrap();
    assert!(closed.unwrap().is_err());
    assert!(start.elapsed() >= Duration::from_secs(4));
    drop(ws);
    let report = f.finish().await;
    assert!(report.failed_connections >= 1);
}
