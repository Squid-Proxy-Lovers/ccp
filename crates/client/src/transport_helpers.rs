// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use anyhow::{Context, bail};
use protocol::{ClientRequest, ErrorCode, ErrorResponse, ServerResponse};
use reqwest::Url;
use serde::Serialize;
use std::sync::OnceLock;
use std::time::Duration;

use crate::enrollment_structs::StoredEnrollment;

pub(crate) fn normalized_endpoint(endpoint: &str) -> anyhow::Result<String> {
    let url = Url::parse(endpoint).context("invalid HTTP endpoint URL")?;
    if url.scheme() != "http" {
        bail!("server endpoint must use plaintext http://");
    }
    if url.host_str().is_none() {
        bail!("HTTP endpoint missing host");
    }
    if !url.username().is_empty() || url.password().is_some() {
        bail!("server endpoint must not contain credentials");
    }
    if url.query().is_some() || url.fragment().is_some() {
        bail!("server endpoint must not contain a query or fragment");
    }
    Ok(url.as_str().trim_end_matches('/').to_string())
}

// Keep a single pool, bound stalled servers, and do not redirect request bodies or keys.
fn http_client() -> anyhow::Result<&'static reqwest::Client> {
    static CLIENT: OnceLock<Result<reqwest::Client, reqwest::Error>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(60))
                .redirect(reqwest::redirect::Policy::none())
                .build()
        })
        .as_ref()
        .map_err(|error| anyhow::anyhow!("failed to initialize HTTP client: {error}"))
}

pub(crate) async fn server_health(enrollment: &StoredEnrollment) -> anyhow::Result<String> {
    let endpoint = normalized_endpoint(&enrollment.metadata.mtls_endpoint)?;
    let response = http_client()?
        .get(format!("{endpoint}/health"))
        .send()
        .await
        .context("failed to get server health")?;
    if !response.status().is_success() {
        bail!("server rejected health request: HTTP {}", response.status());
    }
    let value: serde_json::Value = response
        .json()
        .await
        .context("failed to decode server health")?;
    Ok(serde_json::to_string(&value)?)
}

#[derive(Serialize)]
struct RequestEnvelope<'a> {
    subscribed_session_ids: Vec<i64>,
    request: &'a ClientRequest,
}

pub(crate) fn select_remote_session<'a>(
    sessions: &'a [protocol::SessionMetadata],
    selector: &str,
) -> anyhow::Result<&'a protocol::SessionMetadata> {
    let mut matches = sessions.iter().filter(|session| {
        session.session_name == selector || session.session_id.to_string() == selector
    });
    let session = matches
        .next()
        .with_context(|| format!("open topic '{selector}' was not found"))?;
    if matches.next().is_some() {
        bail!("remote session selector '{selector}' is ambiguous; choose a unique session ID");
    }
    Ok(session)
}

pub(crate) async fn list_remote_sessions(
    endpoint: &str,
) -> anyhow::Result<Vec<protocol::SessionMetadata>> {
    let endpoint = normalized_endpoint(endpoint)?;
    let response = http_client()?
        .get(format!("{endpoint}/v1/sessions"))
        .send()
        .await
        .context("failed to list remote sessions")?;
    if !response.status().is_success() {
        bail!(
            "server rejected session discovery: HTTP {}",
            response.status()
        );
    }
    response
        .json()
        .await
        .context("failed to decode remote sessions")
}

pub(crate) async fn perform_http_request(
    enrollment: &StoredEnrollment,
    request: &ClientRequest,
) -> anyhow::Result<ServerResponse> {
    let endpoint = normalized_endpoint(&enrollment.metadata.mtls_endpoint)?;
    let client_key = std::env::var("CCP_CLIENT_KEY")
        .unwrap_or_else(|_| "ccp-client-7b6c2f915e4a8d30".to_string());
    let response = http_client()?
        .post(format!("{endpoint}/v1/request"))
        .header("X-CCP-Client-Key", client_key)
        .json(&RequestEnvelope {
            subscribed_session_ids: vec![enrollment.metadata.session_id],
            request,
        })
        .send()
        .await
        .context("failed to send HTTP request")?;
    let status = response.status();
    let decoded: ServerResponse = response
        .json()
        .await
        .with_context(|| format!("failed to decode HTTP response (HTTP {status})"))?;
    if !status.is_success() && !matches!(decoded, ServerResponse::Error(_)) {
        bail!("server rejected request: HTTP {status}");
    }
    Ok(decoded)
}

pub(crate) fn response_to_json_string(response: ServerResponse) -> anyhow::Result<String> {
    match response {
        ServerResponse::EntrySummaries(entries) => {
            serde_json::to_string(&entries).context("failed to serialize entry summaries")
        }
        ServerResponse::ShelfSummaries(entries) => {
            serde_json::to_string(&entries).context("failed to serialize shelf summaries")
        }
        ServerResponse::BookSummaries(entries) => {
            serde_json::to_string(&entries).context("failed to serialize book summaries")
        }
        ServerResponse::SearchContextResults(results) => {
            serde_json::to_string(&results).context("failed to serialize context search results")
        }
        ServerResponse::DeletedEntries(entries) => {
            serde_json::to_string(&entries).context("failed to serialize deleted entries")
        }
        ServerResponse::EntryAdded {
            entry,
            duplicate_warning,
        } => {
            if let Some(warning) = duplicate_warning {
                eprintln!(
                    "warning: similar entry '{}' already exists in {}/{} ({})",
                    warning.existing_name,
                    warning.existing_shelf,
                    warning.existing_book,
                    warning.similarity
                );
            }
            serde_json::to_string(&entry).context("failed to serialize message entry")
        }
        ServerResponse::Entry(entry) | ServerResponse::EntryAtTime(entry) => {
            serde_json::to_string(&entry).context("failed to serialize message entry")
        }
        ServerResponse::AppendResult(result) => {
            serde_json::to_string(&result).context("failed to serialize append result")
        }
        ServerResponse::Deleted(result) => {
            serde_json::to_string(&result).context("failed to serialize delete result")
        }
        ServerResponse::Restored(result) => {
            serde_json::to_string(&result).context("failed to serialize restore result")
        }
        ServerResponse::History(history) => {
            serde_json::to_string(&history).context("failed to serialize history")
        }
        ServerResponse::ExportedBundle(bundle) => {
            serde_json::to_string(&bundle).context("failed to serialize bundle")
        }
        ServerResponse::ImportResult(result) => {
            serde_json::to_string(&result).context("failed to serialize import result")
        }
        ServerResponse::CertRevoked(result) => {
            serde_json::to_string(&result).context("failed to serialize revoke result")
        }
        ServerResponse::Pong => serde_json::to_string(&serde_json::json!({ "status": "ok" }))
            .context("failed to serialize pong"),
        ServerResponse::ShelfAdded(result) => {
            serde_json::to_string(&result).context("failed to serialize shelf added result")
        }
        ServerResponse::BookAdded(result) => {
            serde_json::to_string(&result).context("failed to serialize book added result")
        }
        ServerResponse::HandshakeOk(info) => {
            serde_json::to_string(&info).context("failed to serialize handshake response")
        }
        ServerResponse::ShelfDeleted(result) => {
            serde_json::to_string(&result).context("failed to serialize shelf deleted result")
        }
        ServerResponse::Brief(brief) => {
            serde_json::to_string(&brief).context("failed to serialize session brief")
        }
        ServerResponse::StatusSet(status) => {
            serde_json::to_string(&status).context("failed to serialize agent status")
        }
        ServerResponse::StatusCleared(result) => {
            serde_json::to_string(&result).context("failed to serialize clear status result")
        }
        ServerResponse::TeamStatuses(statuses) => {
            serde_json::to_string(&statuses).context("failed to serialize team statuses")
        }
        ServerResponse::HandshakeRejected(info) => {
            anyhow::bail!(
                "protocol version mismatch: server={}, client={}",
                info.protocol_version,
                protocol::PROTOCOL_VERSION
            )
        }
        ServerResponse::Sessions(sessions) | ServerResponse::Subscribed(sessions) => {
            serde_json::to_string(&sessions).context("failed to serialize sessions")
        }
        ServerResponse::SessionCreated(session) => {
            serde_json::to_string(&session).context("failed to serialize created session")
        }
        ServerResponse::MasterInstructions(instructions) => {
            serde_json::to_string(&instructions).context("failed to serialize master instructions")
        }
        ServerResponse::Error(error) => error_response_to_anyhow(error),
    }
}

pub(crate) fn error_response_to_anyhow<T>(error: ErrorResponse) -> anyhow::Result<T> {
    let label = match error.code {
        ErrorCode::BadRequest => "bad request",
        ErrorCode::Forbidden => "forbidden",
        ErrorCode::NotFound => "not found",
        ErrorCode::Internal => "internal error",
    };
    bail!("{label}: {}", error.message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enrollment_structs::EnrollmentMetadata;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn enrollment(endpoint: &str, access: &str) -> StoredEnrollment {
        StoredEnrollment {
            metadata: serde_json::from_value::<EnrollmentMetadata>(serde_json::json!({
                "session_name": "topic", "session_id": 42, "access": access,
                "client_cn": "plaintext-client", "mtls_endpoint": endpoint,
                "client_cert_expires_at": u64::MAX, "enrolled_at": 1
            }))
            .unwrap(),
            directory: "unused".into(),
        }
    }

    async fn mock_response(
        status: &str,
        body: &str,
        extra_headers: &str,
    ) -> (String, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{extra_headers}\r\n{body}",
            body.len()
        );
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            loop {
                let mut buffer = [0u8; 1024];
                let count = stream.read(&mut buffer).await.unwrap();
                assert!(count > 0, "request must complete before connection closes");
                request.extend_from_slice(&buffer[..count]);
                if let Some(header_end) =
                    request.windows(4).position(|window| window == b"\r\n\r\n")
                {
                    let headers = String::from_utf8_lossy(&request[..header_end]);
                    let content_length = headers
                        .lines()
                        .find_map(|line| {
                            let (key, value) = line.split_once(':')?;
                            key.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if request.len() >= header_end + 4 + content_length {
                        break;
                    }
                }
            }
            stream.write_all(response.as_bytes()).await.unwrap();
            String::from_utf8(request).unwrap()
        });
        (endpoint, task)
    }

    #[test]
    fn normalizes_base_paths_and_rejects_url_components_that_break_routes() {
        assert_eq!(
            normalized_endpoint("HTTP://LOCALHOST:80/prefix/").unwrap(),
            "http://localhost/prefix"
        );
        for endpoint in [
            "https://localhost",
            "http://localhost?x=1",
            "http://localhost/#fragment",
            "http://user:password@localhost",
        ] {
            assert!(normalized_endpoint(endpoint).is_err(), "{endpoint}");
        }
    }

    #[test]
    fn remote_discovery_rejects_name_id_collisions() {
        let session = |name: &str, id| protocol::SessionMetadata {
            session_name: name.into(),
            session_id: id,
            description: String::new(),
            owner: String::new(),
            labels: vec![],
            visibility: "public".into(),
            purpose: String::new(),
        };
        let sessions = vec![session("42", 7), session("other", 42)];
        assert!(
            select_remote_session(&sessions, "42")
                .unwrap_err()
                .to_string()
                .contains("ambiguous")
        );
        assert_eq!(
            select_remote_session(&sessions, "7").unwrap().session_name,
            "42"
        );
    }

    #[tokio::test]
    async fn posts_selected_session_and_preserves_typed_http_error() {
        let response = ServerResponse::Error(ErrorResponse {
            code: ErrorCode::Forbidden,
            message: "read denied".into(),
        });
        let (endpoint, task) = mock_response(
            "403 Forbidden",
            &serde_json::to_string(&response).unwrap(),
            "",
        )
        .await;
        let received = crate::transport::perform_request(
            &enrollment(&endpoint, "read"),
            ClientRequest::List { session_id: 42 },
        )
        .await
        .unwrap();
        assert_eq!(received, response);
        let request = task.await.unwrap();
        assert!(request.starts_with("POST /v1/request HTTP/1.1"));
        let body: serde_json::Value =
            serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(body["subscribed_session_ids"], serde_json::json!([42]));
        assert_eq!(
            body["request"],
            serde_json::json!({"List": {"session_id": 42}})
        );
    }

    #[tokio::test]
    async fn rejects_unrelated_success_and_misleading_http_status() {
        for (status, body) in [
            ("200 OK", "\"Pong\""),
            ("500 Internal Server Error", "{\"EntrySummaries\":[]}"),
        ] {
            let (endpoint, task) = mock_response(status, body, "").await;
            assert!(
                crate::transport::perform_request(
                    &enrollment(&endpoint, "read"),
                    ClientRequest::List { session_id: 42 }
                )
                .await
                .is_err()
            );
            task.await.unwrap();
        }
    }

    #[tokio::test]
    async fn redirects_are_not_followed() {
        let destination = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let headers = format!(
            "Location: http://{}/v1/sessions\r\n",
            destination.local_addr().unwrap()
        );
        let (endpoint, task) = mock_response("307 Temporary Redirect", "[]", &headers).await;
        assert!(
            list_remote_sessions(&endpoint)
                .await
                .unwrap_err()
                .to_string()
                .contains("307")
        );
        task.await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(50), destination.accept())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn read_subscription_cannot_mutate_through_library_transport() {
        // An unreachable endpoint proves failure happens before any network request.
        let error = crate::transport::perform_request(
            &enrollment("http://127.0.0.1:1", "read"),
            ClientRequest::DeleteShelf {
                session_id: 42,
                shelf_name: "team".into(),
            },
        )
        .await
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "saved subscription does not allow writes"
        );
    }

    #[tokio::test]
    async fn health_uses_selected_endpoint_base_path() {
        let (endpoint, task) = mock_response("200 OK", "{\"status\":\"ok\"}", "").await;
        let body = server_health(&enrollment(&format!("{endpoint}/prefix/"), "read"))
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&body).unwrap()["status"],
            "ok"
        );
        assert!(
            task.await
                .unwrap()
                .starts_with("GET /prefix/health HTTP/1.1")
        );
    }

    #[tokio::test]
    async fn typed_add_entry_can_preserve_duplicate_warning_without_changing_cli_payload() {
        let entry: protocol::MessageEntry = serde_json::from_value(serde_json::json!({
            "name": "note", "description": "test", "labels": [], "context": "contents"
        }))
        .unwrap();
        let warning = protocol::DuplicateWarning {
            existing_name: "older".into(),
            existing_shelf: "main".into(),
            existing_book: "default".into(),
            similarity: "high".into(),
        };
        let response = ServerResponse::EntryAdded {
            entry: entry.clone(),
            duplicate_warning: Some(warning.clone()),
        };
        assert_eq!(
            serde_json::from_str::<protocol::MessageEntry>(
                &response_to_json_string(response.clone()).unwrap()
            )
            .unwrap(),
            entry
        );
        let (endpoint, task) =
            mock_response("200 OK", &serde_json::to_string(&response).unwrap(), "").await;
        let session = crate::SessionClient {
            enrollment: enrollment(&endpoint, "read_write"),
        };
        let outcome = session
            .add_entry_with_warning(crate::AddEntryRequest {
                shelf_name: "main".into(),
                book_name: "default".into(),
                entry_name: "note".into(),
                entry_description: "test".into(),
                entry_labels: vec![],
                entry_data: "contents".into(),
            })
            .await
            .unwrap();
        assert_eq!(outcome.entry, entry);
        assert_eq!(outcome.duplicate_warning, Some(warning));
        task.await.unwrap();
    }
}
