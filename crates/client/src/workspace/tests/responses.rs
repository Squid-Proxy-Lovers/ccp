// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

// Only the private response decoder is isolated here; public profile transport
// is always HTTPS and separately exercised against the actual TLS server.
async fn response(bytes: Vec<u8>) -> anyhow::Result<api::Health> {
    decode_response(bytes, false).await
}

async fn decode_response(bytes: Vec<u8>, snapshot: bool) -> anyhow::Result<api::Health> {
    decode_mode(bytes, snapshot, false).await
}

async fn decode_mode(
    bytes: Vec<u8>,
    snapshot: bool,
    operation: bool,
) -> anyhow::Result<api::Health> {
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
    let request = http.get(format!("http://{address}/"));
    let result = if operation {
        network::operation_json(request).await
    } else if snapshot {
        network::snapshot_json(request).await
    } else {
        network::json(request).await
    };
    task.await.unwrap();
    result
}

#[tokio::test]
async fn only_bounded_exact_operation_conflicts_can_finish_a_pending_intent() {
    let body = r#"{"api_version":1,"error":{"code":"content_conflict"}}"#;
    let error = decode_mode(fixed("409 Conflict", body, ""), false, true)
        .await
        .err()
        .unwrap();
    assert!(network::content_conflict(&error));
    assert!(!network::content_conflict(
        &response(fixed("409 Conflict", body, ""))
            .await
            .err()
            .unwrap()
    ));
    for bytes in [
        fixed("403 Forbidden", body, ""),
        fixed("503 Unavailable", body, ""),
        fixed("409 Conflict", body, "Content-Encoding: gzip\r\n"),
        fixed("409 Conflict", &body.replace(":1,", ":99,"), ""),
        fixed(
            "409 Conflict",
            &body.replace("content_conflict", "content_cancelled"),
            "",
        ),
        fixed(
            "409 Conflict",
            r#"{"api_version":1,"error":{"code":"content_conflict","detail":"PRIVATE_BODY"}}"#,
            "",
        ),
        fixed(
            "409 Conflict",
            r#"{"api_version":1,"error":{"code":"content_conflict"},"extra":true}"#,
            "",
        ),
        fixed("409 Conflict", "PRIVATE_BODY", ""),
        fixed("409 Conflict", &format!("{body}{}", " ".repeat(8193)), ""),
        b"HTTP/1.1 409 Conflict\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{}".to_vec(),
    ] {
        let error = decode_mode(bytes, false, true).await.err().unwrap();
        assert!(!network::content_conflict(&error));
        assert!(!error.to_string().contains("PRIVATE_BODY"));
    }
    let mut bytes =
        b"HTTP/1.1 409 Conflict\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n2001\r\n"
            .to_vec();
    bytes.extend(vec![b' '; 8193]);
    bytes.extend(b"\r\n0\r\n\r\n");
    assert!(!network::content_conflict(
        &decode_mode(bytes, false, true).await.err().unwrap()
    ));
}

#[tokio::test]
async fn snapshot_decoder_accepts_large_json_but_keeps_four_mib_stream_and_encoding_guards() {
    let text = "x".repeat(3 * 1024 * 1024);
    let body = serde_json::json!({"api_version":1,"data":{"instance_id":text}}).to_string();
    let result = decode_response(fixed("200 OK", &body, ""), true)
        .await
        .unwrap();
    assert_eq!(result.instance_id, text);
    assert!(response(fixed("200 OK", &body, "")).await.is_err());
    let declared =
        b"HTTP/1.1 200 OK\r\nContent-Length: 4194305\r\nConnection: close\r\n\r\n".to_vec();
    assert!(decode_response(declared, true).await.is_err());
    let mut chunked =
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n400001\r\n"
            .to_vec();
    chunked.extend(vec![b'x'; 4 * 1024 * 1024 + 1]);
    chunked.extend(b"\r\n0\r\n\r\n");
    assert!(decode_response(chunked, true).await.is_err());
    for bytes in [
        fixed("200 OK", "PRIVATE_BODY", ""),
        fixed("403 Forbidden", "PRIVATE_BODY", ""),
        fixed("200 OK", &body, "Content-Encoding: gzip\r\n"),
        fixed(
            "200 OK",
            r#"{"api_version":999,"data":{"instance_id":"PRIVATE_BODY"}}"#,
            "",
        ),
        b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{}".to_vec(),
    ] {
        let error = decode_response(bytes, true)
            .await
            .err()
            .unwrap()
            .to_string();
        assert!(!error.contains("PRIVATE_BODY"));
    }
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
