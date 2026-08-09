// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use anyhow::{Context, bail};
use protocol::{ClientRequest, ErrorCode, ErrorResponse, ServerResponse};
use reqwest::Url;
use serde::Serialize;

use crate::enrollment_structs::StoredEnrollment;

fn normalized_endpoint(endpoint: &str) -> anyhow::Result<String> {
    let url = Url::parse(endpoint).context("invalid HTTP endpoint URL")?;
    if url.scheme() != "http" {
        bail!("server endpoint must use plaintext http://");
    }
    if url.host_str().is_none() {
        bail!("HTTP endpoint missing host");
    }
    Ok(endpoint.trim_end_matches('/').to_string())
}

#[derive(Serialize)]
struct RequestEnvelope {
    subscribed_session_ids: Vec<i64>,
    request: ClientRequest,
}

pub(crate) async fn list_remote_sessions(
    endpoint: &str,
) -> anyhow::Result<Vec<protocol::SessionMetadata>> {
    let endpoint = normalized_endpoint(endpoint)?;
    reqwest::Client::new()
        .get(format!("{endpoint}/v1/sessions"))
        .send()
        .await
        .context("failed to list remote sessions")?
        .error_for_status()
        .context("server rejected session discovery")?
        .json()
        .await
        .context("failed to decode remote sessions")
}

pub(crate) async fn perform_http_request(
    enrollment: &StoredEnrollment,
    request: ClientRequest,
) -> anyhow::Result<ServerResponse> {
    let endpoint = normalized_endpoint(&enrollment.metadata.mtls_endpoint)?;
    let client_key = std::env::var("CCP_CLIENT_KEY")
        .unwrap_or_else(|_| "ccp-client-7b6c2f915e4a8d30".to_string());
    reqwest::Client::new()
        .post(format!("{endpoint}/v1/request"))
        .header("X-CCP-Client-Key", client_key)
        .json(&RequestEnvelope {
            subscribed_session_ids: vec![enrollment.metadata.session_id],
            request,
        })
        .send()
        .await
        .context("failed to send HTTP request")?
        .json()
        .await
        .context("failed to decode HTTP response")
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
        ServerResponse::Entry(entry)
        | ServerResponse::EntryAdded { entry, .. }
        | ServerResponse::EntryAtTime(entry) => {
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

pub(crate) fn error_response_to_anyhow(error: ErrorResponse) -> anyhow::Result<String> {
    let label = match error.code {
        ErrorCode::BadRequest => "bad request",
        ErrorCode::Forbidden => "forbidden",
        ErrorCode::NotFound => "not found",
        ErrorCode::Internal => "internal error",
    };
    bail!("{label}: {}", error.message)
}
