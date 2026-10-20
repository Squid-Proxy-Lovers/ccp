// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

// Only the private response decoder is isolated here; public profile transport
// is always HTTPS and separately exercised against the actual TLS server.
async fn response(bytes: Vec<u8>) -> anyhow::Result<api::Health> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = [0; 4096];
        assert!(stream.read(&mut request).await.unwrap() > 0);
        if let Err(error) = stream.write_all(&bytes).await {
            assert!(matches!(
                error.kind(),
                std::io::ErrorKind::BrokenPipe
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::ConnectionAborted
            ));
        }
    });
    let http = reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .unwrap();
    let result = network::json(http.get(format!("http://{address}/"))).await;
    task.await.unwrap();
    result
}
fn fixed(status: &str, body: &str, extra: &str) -> Vec<u8> {
    format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n{body}",body.len()).into_bytes()
}

#[tokio::test]
async fn decoder_rejects_api_versions_malformed_json_and_encoded_responses_without_echo() {
    for bytes in [
        fixed(
            "200 OK",
            r#"{"api_version":999,"data":{"instance_id":"secret-body"}}"#,
            "",
        ),
        fixed("200 OK", "SENTINEL_PRIVATE_VALUE", ""),
        fixed("403 Forbidden", "SENTINEL_PRIVATE_VALUE", ""),
        fixed(
            "200 OK",
            r#"{"api_version":1,"data":{"instance_id":"secret-body"}}"#,
            "Content-Encoding: gzip\r\n",
        ),
    ] {
        let error = response(bytes).await.err().unwrap().to_string();
        assert!(!error.contains("SENTINEL_PRIVATE_VALUE"));
        assert!(!error.contains("secret-body"));
    }
    let parsed = response(fixed(
        "200 OK",
        r#"{"api_version":1,"data":{"instance_id":"public-id"}}"#,
        "",
    ))
    .await
    .unwrap();
    assert_eq!(parsed.instance_id, "public-id");
}

#[tokio::test]
async fn decoder_bounds_declared_and_chunked_bodies_and_refuses_truncation() {
    let declared =
        b"HTTP/1.1 200 OK\r\nContent-Length: 1048577\r\nConnection: close\r\n\r\n".to_vec();
    assert!(response(declared).await.is_err());
    let mut chunked =
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n100001\r\n"
            .to_vec();
    chunked.extend(vec![b'x'; 1024 * 1024 + 1]);
    chunked.extend(b"\r\n0\r\n\r\n");
    assert!(response(chunked).await.is_err());
    let truncated =
        b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{}".to_vec();
    assert!(response(truncated).await.is_err());
}
