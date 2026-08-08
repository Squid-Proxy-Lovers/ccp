// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

pub mod identity;
pub mod init;
pub mod journal;
pub mod message;
pub mod state;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use protocol::{ClientRequest, ErrorCode, ErrorResponse, ServerResponse, SessionMetadata};
use serde::{Deserialize, Serialize};

use crate::identity::ConnectionAuthContext;
use crate::init::{
    http_listener_addr, http_server_base_url, initialize_plain_server, journal_path,
};
use crate::journal::JournalHandle;
use crate::message::handle_message_request;
use crate::state::ServerState;

pub const DEFAULT_CLIENT_KEY: &str = "ccp-client-7b6c2f915e4a8d30";
pub const DEFAULT_ADMIN_KEY: &str = "ccp-admin-f1a847d36c509e2b";
const CLIENT_KEY_HEADER: &str = "x-ccp-client-key";
const ADMIN_KEY_HEADER: &str = "x-ccp-admin-key";

#[derive(Clone)]
struct AppState {
    ccp: Arc<ServerState>,
    client_key: String,
    admin_key: String,
    download_dir: PathBuf,
}

#[derive(Debug, Deserialize, Serialize)]
struct RequestEnvelope {
    subscribed_session_ids: Vec<i64>,
    request: ClientRequest,
}

#[derive(Debug, Deserialize)]
struct SessionSelector {
    session: String,
}

#[derive(Debug, Deserialize)]
struct CreateSessionBody {
    session_name: String,
}

pub async fn run_server(session_name: &str) -> anyhow::Result<()> {
    run_plain_server(Some(session_name)).await
}

pub async fn run_plain_server(initial_session: Option<&str>) -> anyhow::Result<()> {
    let initial_id = initialize_plain_server(initial_session)?;
    let journal = Arc::new(JournalHandle::start(journal_path())?);
    let ccp = Arc::new(ServerState::load_from_storage(Arc::clone(&journal)).await?);
    let state = AppState {
        ccp: Arc::clone(&ccp),
        client_key: std::env::var("CCP_CLIENT_KEY")
            .unwrap_or_else(|_| DEFAULT_CLIENT_KEY.to_string()),
        admin_key: std::env::var("CCP_ADMIN_KEY").unwrap_or_else(|_| DEFAULT_ADMIN_KEY.to_string()),
        download_dir: std::env::var_os("CCP_DOWNLOAD_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("downloads")),
    };

    if let (Some(name), Some(id)) = (initial_session, initial_id) {
        println!("Initialized session '{name}' (id={id})");
    }
    println!("HTTP endpoint: {}", http_server_base_url());

    let app = Router::new()
        .route("/health", get(health))
        .route("/v1/sessions", get(list_open_sessions))
        .route("/v1/subscribe", post(subscribe))
        .route("/v1/request", post(request))
        .route("/v1/admin/sessions", post(admin_create_session))
        .route("/v1/admin/sessions/{session}", delete(admin_delete_session))
        .route(
            "/v1/admin/sessions/{session}/stats",
            get(admin_session_stats),
        )
        .route("/setup-client.sh", get(setup_client_script))
        .route("/setup-client.ps1", get(setup_client_powershell))
        .route("/ccp-manage", get(management_script))
        .route("/ccp-manage.ps1", get(management_powershell))
        .route("/downloads/{artifact}", get(download_artifact))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(http_listener_addr())
        .await
        .context("failed to bind HTTP listener")?;
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .context("HTTP server failed")?;

    ccp.mark_sessions_stopped().await?;
    journal.shutdown()?;
    ccp.persist_snapshot_to_sqlite().await?;
    journal.truncate_blocking()?;
    Ok(())
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({"status": "ok"}))
}

async fn list_open_sessions(State(state): State<AppState>) -> Json<Vec<SessionMetadata>> {
    Json(
        state
            .ccp
            .list_sessions()
            .await
            .into_iter()
            .filter(|session| session.visibility == "public")
            .collect(),
    )
}

async fn subscribe(
    State(state): State<AppState>,
    Json(selector): Json<SessionSelector>,
) -> Response {
    match resolve_open_session(&state.ccp, &selector.session).await {
        Some(session) => (StatusCode::OK, Json(session)).into_response(),
        None => error(StatusCode::NOT_FOUND, "open session not found"),
    }
}

async fn request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(envelope): Json<RequestEnvelope>,
) -> Response {
    let session_id = request_session_id(&envelope.request);
    if let Some(session_id) = session_id {
        if !envelope.subscribed_session_ids.contains(&session_id) {
            return protocol_error(
                StatusCode::FORBIDDEN,
                ErrorCode::Forbidden,
                format!("not subscribed to session {session_id}"),
            );
        }
        let sessions = state.ccp.list_sessions().await;
        let Some(session) = sessions
            .iter()
            .find(|session| session.session_id == session_id)
        else {
            return protocol_error(
                StatusCode::NOT_FOUND,
                ErrorCode::NotFound,
                "session not found".to_string(),
            );
        };
        let key_matches =
            header_value(&headers, CLIENT_KEY_HEADER).is_some_and(|key| key == state.client_key);
        if session.visibility != "public" && !key_matches {
            return protocol_error(
                StatusCode::UNAUTHORIZED,
                ErrorCode::Forbidden,
                "invalid client key".to_string(),
            );
        }
    }

    let context = ConnectionAuthContext {
        common_name: "http-client".to_string(),
        session_id: session_id.unwrap_or(0),
        can_write: true,
        can_revoke_others: false,
    };
    Json(handle_message_request(&state.ccp, &context, envelope.request).await).into_response()
}

async fn admin_create_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CreateSessionBody>,
) -> Response {
    if !admin_authorized(&state, &headers) {
        return error(StatusCode::UNAUTHORIZED, "invalid admin key");
    }
    match state.ccp.create_session(&body.session_name).await {
        Ok(session) => (StatusCode::CREATED, Json(session)).into_response(),
        Err(error_value) => error(StatusCode::BAD_REQUEST, error_value.to_string()),
    }
}

async fn admin_delete_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session): Path<String>,
) -> Response {
    if !admin_authorized(&state, &headers) {
        return error(StatusCode::UNAUTHORIZED, "invalid admin key");
    }
    match state.ccp.delete_session(&session).await {
        Ok(metadata) => (StatusCode::OK, Json(metadata)).into_response(),
        Err(error_value) => error(StatusCode::NOT_FOUND, error_value.to_string()),
    }
}

async fn admin_session_stats(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session): Path<String>,
) -> Response {
    if !admin_authorized(&state, &headers) {
        return error(StatusCode::UNAUTHORIZED, "invalid admin key");
    }
    match state.ccp.session_stats(&session).await {
        Ok(stats) => (StatusCode::OK, Json(stats)).into_response(),
        Err(error_value) => error(StatusCode::NOT_FOUND, error_value.to_string()),
    }
}

async fn setup_client_script() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/x-shellscript; charset=utf-8")],
        include_str!("../../../scripts/setup-client.sh"),
    )
}

async fn setup_client_powershell() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        include_str!("../../../scripts/setup-client.ps1"),
    )
}

async fn management_script() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/x-shellscript; charset=utf-8")],
        include_str!("../../../scripts/ccp-manage"),
    )
}

async fn management_powershell() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        include_str!("../../../scripts/ccp-manage.ps1"),
    )
}

async fn download_artifact(
    State(state): State<AppState>,
    Path(artifact): Path<String>,
) -> Response {
    if !artifact
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_'))
    {
        return error(StatusCode::BAD_REQUEST, "invalid artifact name");
    }
    let path = state.download_dir.join(&artifact);
    match tokio::fs::read(&path).await {
        Ok(bytes) => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .header(
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{artifact}\""),
            )
            .body(Body::from(bytes))
            .expect("valid artifact response"),
        Err(_) => error(StatusCode::NOT_FOUND, "artifact not found"),
    }
}

async fn resolve_open_session(ccp: &ServerState, selector: &str) -> Option<SessionMetadata> {
    ccp.list_sessions().await.into_iter().find(|session| {
        session.visibility == "public"
            && (session.session_name == selector || session.session_id.to_string() == selector)
    })
}

fn admin_authorized(state: &AppState, headers: &HeaderMap) -> bool {
    header_value(headers, ADMIN_KEY_HEADER).is_some_and(|key| key == state.admin_key)
}

fn header_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name)?.to_str().ok()
}

fn request_session_id(request: &ClientRequest) -> Option<i64> {
    match request {
        ClientRequest::List { session_id }
        | ClientRequest::Get { session_id, .. }
        | ClientRequest::AddShelf { session_id, .. }
        | ClientRequest::AddBook { session_id, .. }
        | ClientRequest::AddEntry { session_id, .. }
        | ClientRequest::Append { session_id, .. }
        | ClientRequest::Delete { session_id, .. }
        | ClientRequest::SearchEntries { session_id, .. }
        | ClientRequest::SearchShelves { session_id, .. }
        | ClientRequest::SearchBooks { session_id, .. }
        | ClientRequest::SearchContext { session_id, .. }
        | ClientRequest::SearchDeleted { session_id, .. }
        | ClientRequest::RestoreDeleted { session_id, .. }
        | ClientRequest::GetHistory { session_id, .. }
        | ClientRequest::ExportBundle { session_id, .. }
        | ClientRequest::ImportBundle { session_id, .. }
        | ClientRequest::RevokeClientCert { session_id, .. }
        | ClientRequest::DeleteShelf { session_id, .. }
        | ClientRequest::BriefMe { session_id }
        | ClientRequest::GetEntryAt { session_id, .. } => Some(*session_id),
        ClientRequest::Ping
        | ClientRequest::Handshake(_)
        | ClientRequest::ListSessions
        | ClientRequest::CreateSession { .. }
        | ClientRequest::Subscribe { .. } => None,
    }
}

fn protocol_error(status: StatusCode, code: ErrorCode, message: String) -> Response {
    (
        status,
        Json(ServerResponse::Error(ErrorResponse { code, message })),
    )
        .into_response()
}

fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(serde_json::json!({"error": message.into()}))).into_response()
}
