use super::*;
fn request(path: &str) -> String {
    let generated = format!("wss://localhost{path}")
        .into_client_request()
        .unwrap();
    let nonce = generated.headers()["sec-websocket-key"].to_str().unwrap();
    format!(
        "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: {nonce}\r\nSec-WebSocket-Protocol: {SUBPROTOCOL}\r\n\r\n"
    )
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn upgrade_gates_refuse_specific_invalid_requests_and_disabled_mode() {
    let f = Fixture::with_tunnels(true).await;
    let valid = request(ENROLL_PATH);
    let nonce = valid
        .lines()
        .find_map(|line| line.strip_prefix("Sec-WebSocket-Key: "))
        .unwrap();
    let variants = [
        (request("/ccp/tunnel/v1/unknown"), 404),
        (request(&format!("{ENROLL_PATH}?target=localhost:22")), 400),
        (valid.replacen("GET ", "POST ", 1), 400),
        (valid.replacen("HTTP/1.1", "HTTP/1.0", 1), 400),
        (valid.replace("13\r\n", "12\r\n"), 400),
        (valid.replace(nonce, "invalid"), 400),
        (valid.replace(SUBPROTOCOL, "other"), 400),
        (
            valid.replace("Connection: Upgrade", "Connection: keep-alive"),
            400,
        ),
        (valid.replace("Upgrade: websocket", "Upgrade: other"), 400),
        (valid.replace("Host: localhost", "Host: bad@host"), 400),
        (
            valid.replace("Host: localhost", "Host: localhost\r\nHost: other"),
            400,
        ),
        (
            valid.replace("\r\n\r\n", "\r\nOrigin: https://localhost\r\n\r\n"),
            400,
        ),
        (
            valid.replace(
                "\r\n\r\n",
                "\r\nSec-WebSocket-Extensions: permessage-deflate\r\n\r\n",
            ),
            400,
        ),
        (
            valid.replace("\r\n\r\n", "\r\nContent-Length: 1\r\n\r\nx"),
            400,
        ),
        (
            valid.replace(
                "\r\n\r\n",
                "\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n",
            ),
            400,
        ),
        (
            valid.replace("\r\n\r\n", "\r\nSec-WebSocket-Protocol: other\r\n\r\n"),
            400,
        ),
    ];
    for (input, wanted) in variants {
        let mut wire = Wire::connect(&f, None).await;
        wire.send(&input).await;
        let (code, body) = wire.response().await;
        assert_eq!(code, wanted);
        assert_eq!(body["api_version"], 1);
    }
    let mut wire = Wire::connect(&f, None).await;
    assert_eq!(send(&mut wire, "/api/v1/health", None, "").await.0, 200);
    wire.send(&valid).await;
    assert_eq!(wire.response().await.0, 400);
    drop(wire);
    f.finish().await;
    let f = Fixture::new().await;
    let mut wire = Wire::connect(&f, None).await;
    wire.send(&valid).await;
    let (code, body) = wire.response().await;
    assert_eq!(code, 404);
    assert_eq!(body["error"]["code"], "not_found");
    drop(wire);
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hyper_preserves_first_masked_tls_clienthello_buffered_with_upgrade() {
    let f = Fixture::with_tunnels(true).await;
    // Generate a real inner ClientHello before the HTTP upgrade, using an in-memory pipe.
    let (tls_side, mut capture) = tokio::io::duplex(65536);
    let connector = TlsConnector::from(Arc::new(tls_config(
        Some(&f.trust.ca_certificate_pem),
        None,
    )));
    let task = tokio::spawn(async move {
        connector
            .connect(ServerName::try_from("localhost").unwrap(), tls_side)
            .await
    });
    let mut hello = [0; 16384];
    let size = capture.read(&mut hello).await.unwrap();
    assert!(size > 0);
    let mut outer = Wire::connect(&f, None).await;
    // Use the maintained parser to serialize one correctly masked binary frame.
    let (frame_side, mut framed) = tokio::io::duplex(65536);
    let mut writer =
        WebSocketStream::from_raw_socket(frame_side, Role::Client, Some(websocket_config())).await;
    use ccp_transport::websocket::tungstenite::Message;
    use futures_util::SinkExt;
    writer
        .send(Message::Binary(hello[..size].to_vec().into()))
        .await
        .unwrap();
    let mut bytes = vec![0; 65536];
    let n = framed.read(&mut bytes).await.unwrap();
    let mut input = request(ENROLL_PATH).into_bytes();
    input.extend_from_slice(&bytes[..n]);
    outer.stream.write_all(&input).await.unwrap();
    outer.stream.flush().await.unwrap();
    // Parse just the 101; then reuse any buffered first server frame.
    let (code, body) = outer.response().await;
    assert_eq!(code, 101);
    assert!(body.is_null());
    let socket = WebSocketStream::from_partially_read(
        outer.stream,
        outer.buffer,
        Role::Client,
        Some(websocket_config()),
    )
    .await;
    let mut stream = ByteStream::new(socket);
    // Relay the remainder of the fixture TLS handshake and HTTP in the test only.
    let relay =
        tokio::spawn(async move { tokio::io::copy_bidirectional(&mut capture, &mut stream).await });
    let tls = tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let mut inner = Wire {
        stream: tls,
        buffer: Vec::new(),
    };
    let (code, health) = send(&mut inner, "/api/v1/health", None, "").await;
    assert_eq!(code, 200);
    assert_eq!(health["data"]["instance_id"], f.trust.instance_id);
    drop(inner);
    relay.abort();
    assert!(relay.await.unwrap_err().is_cancelled());
    drop(writer);
    f.finish().await;
}
