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
use axum::extract::{Path, Query, State};
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
    base_url: String,
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

#[derive(Debug, Deserialize)]
struct InstructionUpdate {
    content: String,
}

#[derive(Debug, Deserialize)]
struct ActivityQuery {
    session: Option<String>,
    limit: Option<usize>,
}

#[derive(Debug, Serialize)]
struct AdminOverview {
    sessions: Vec<protocol::SessionStats>,
    global_master: protocol::InstructionRecord,
}

pub async fn run_server(session_name: &str) -> anyhow::Result<()> {
    run_plain_server(Some(session_name)).await
}

pub async fn run_plain_server(initial_session: Option<&str>) -> anyhow::Result<()> {
    run_plain_server_with_shutdown(initial_session, shutdown_signal()).await
}

pub async fn run_plain_server_with_shutdown(
    initial_session: Option<&str>,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> anyhow::Result<()> {
    let base_url = configured_base_url()?;
    let client_key = configured_key("CCP_CLIENT_KEY", DEFAULT_CLIENT_KEY)?;
    let admin_key = configured_key("CCP_ADMIN_KEY", DEFAULT_ADMIN_KEY)?;
    let listener = tokio::net::TcpListener::bind(http_listener_addr())
        .await
        .context("failed to bind HTTP listener")?;
    let initial_id = initialize_plain_server(initial_session)?;
    let journal = match JournalHandle::start(journal_path()) {
        Ok(journal) => Arc::new(journal),
        Err(error) => {
            stop_bootstrap_session(initial_id);
            return Err(error);
        }
    };

    let ccp = match ServerState::load_from_storage(Arc::clone(&journal)).await {
        Ok(state) => Arc::new(state),
        Err(error) => {
            let _ = journal.shutdown();
            stop_bootstrap_session(initial_id);
            return Err(error);
        }
    };
    let state = AppState {
        ccp: Arc::clone(&ccp),
        client_key,
        admin_key,
        download_dir: std::env::var_os("CCP_DOWNLOAD_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("downloads")),
        base_url: base_url.clone(),
    };

    if let (Some(name), Some(id)) = (initial_session, initial_id) {
        println!("Initialized session '{name}' (id={id})");
    }
    println!("HTTP endpoint: {base_url}");

    let app = Router::new()
        .route("/health", get(health))
        .route("/v1/sessions", get(list_open_sessions))
        .route("/v1/subscribe", post(subscribe))
        .route("/v1/request", post(request))
        .route("/v1/admin/sessions", post(admin_create_session))
        .route("/v1/admin/overview", get(admin_overview))
        .route("/v1/admin/activity", get(admin_activity))
        .route(
            "/v1/admin/master",
            get(admin_get_global_master).put(admin_set_global_master),
        )
        .route("/v1/admin/sessions/{session}", delete(admin_delete_session))
        .route(
            "/v1/admin/sessions/{session}/stats",
            get(admin_session_stats),
        )
        .route(
            "/v1/admin/sessions/{session}/master",
            get(admin_get_session_master).put(admin_set_session_master),
        )
        .route("/admin", get(admin_dashboard))
        .route("/setup-client.sh", get(setup_client_script))
        .route("/setup-client.ps1", get(setup_client_powershell))
        .route("/ccp-manage", get(management_script))
        .route("/ccp-manage.ps1", get(management_powershell))
        .route("/ccp-update", get(update_script))
        .route("/ccp-update.ps1", get(update_powershell))
        .route("/downloads/{artifact}", get(download_artifact))
        .with_state(state);

    let serve_result = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await
        .context("HTTP server failed");
    let persist_result = async {
        ccp.mark_sessions_stopped().await?;
        ccp.checkpoint().await
    }
    .await;
    // Always drain/stop the writer, including failed snapshot/server exits.
    let shutdown_result = journal.shutdown();
    serve_result?;
    persist_result?;
    shutdown_result?;
    Ok(())
}

fn stop_bootstrap_session(session_id: Option<i64>) {
    if let Some(session_id) = session_id
        && let Ok(connection) = init::open_sqlite_connection()
    {
        let _ = connection.execute(
            "UPDATE sessions SET is_active=0, last_stopped_at=CURRENT_TIMESTAMP WHERE id=?1",
            [session_id],
        );
    }
}

fn configured_base_url() -> anyhow::Result<String> {
    let url = url::Url::parse(&http_server_base_url())
        .context("CCP_HTTP_BASE_URL must be an absolute HTTP(S) URL")?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        anyhow::bail!(
            "CCP_HTTP_BASE_URL must be an HTTP(S) URL without credentials, query, or fragment"
        );
    }
    Ok(url.as_str().trim_end_matches('/').to_string())
}

fn configured_key(name: &str, default: &str) -> anyhow::Result<String> {
    let value = match std::env::var(name) {
        Ok(value) => value,
        Err(std::env::VarError::NotPresent) => default.to_string(),
        Err(error) => return Err(error).with_context(|| format!("{name} must be valid UTF-8")),
    };
    if value.is_empty()
        || value.trim() != value
        || !axum::http::HeaderValue::from_str(&value).is_ok_and(|header| header.to_str().is_ok())
    {
        anyhow::bail!(
            "{name} must be a nonempty valid HTTP header value without surrounding whitespace"
        );
    }
    Ok(value)
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut terminate) => tokio::select! {
                _ = tokio::signal::ctrl_c() => {},
                _ = terminate.recv() => {},
            },
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

fn render_hosted_script(script: &str, base_url: &str, powershell: bool) -> String {
    // The placeholders are inside quoted literals. Preserve those literals even
    // when a configured URL contains characters meaningful to the shell.
    let escaped = if powershell {
        base_url
            .replace('`', "``")
            .replace('"', "`\"")
            .replace('$', "`$")
    } else {
        base_url
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('$', "\\$")
            .replace('`', "\\`")
    };
    script.replace("http://127.0.0.1:1338", &escaped)
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

async fn admin_overview(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !admin_authorized(&state, &headers) {
        return error(StatusCode::UNAUTHORIZED, "invalid admin key");
    }
    match state.ccp.global_master_instructions() {
        Ok(global_master) => Json(AdminOverview {
            sessions: state.ccp.all_session_stats().await,
            global_master,
        })
        .into_response(),
        Err(error_value) => error(StatusCode::INTERNAL_SERVER_ERROR, error_value.to_string()),
    }
}

async fn admin_activity(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ActivityQuery>,
) -> Response {
    if !admin_authorized(&state, &headers) {
        return error(StatusCode::UNAUTHORIZED, "invalid admin key");
    }
    match state
        .ccp
        .recent_activity(query.session.as_deref(), query.limit.unwrap_or(100))
        .await
    {
        Ok(activity) => Json(activity).into_response(),
        Err(error_value) => error(StatusCode::NOT_FOUND, error_value.to_string()),
    }
}

async fn admin_get_global_master(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !admin_authorized(&state, &headers) {
        return error(StatusCode::UNAUTHORIZED, "invalid admin key");
    }
    match state.ccp.global_master_instructions() {
        Ok(record) => Json(record).into_response(),
        Err(error_value) => error(StatusCode::INTERNAL_SERVER_ERROR, error_value.to_string()),
    }
}

async fn admin_set_global_master(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(update): Json<InstructionUpdate>,
) -> Response {
    if !admin_authorized(&state, &headers) {
        return error(StatusCode::UNAUTHORIZED, "invalid admin key");
    }
    match state.ccp.set_global_master_instructions(&update.content) {
        Ok(record) => Json(record).into_response(),
        Err(error_value) => error(StatusCode::INTERNAL_SERVER_ERROR, error_value.to_string()),
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

async fn admin_get_session_master(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session): Path<String>,
) -> Response {
    if !admin_authorized(&state, &headers) {
        return error(StatusCode::UNAUTHORIZED, "invalid admin key");
    }
    match state.ccp.session_stats(&session).await {
        Ok(stats) => match state
            .ccp
            .master_instructions(stats.session.session_id)
            .await
        {
            Ok(instructions) => Json(instructions.session).into_response(),
            Err(error_value) => error(StatusCode::INTERNAL_SERVER_ERROR, error_value.to_string()),
        },
        Err(error_value) => error(StatusCode::NOT_FOUND, error_value.to_string()),
    }
}

async fn admin_set_session_master(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session): Path<String>,
    Json(update): Json<InstructionUpdate>,
) -> Response {
    if !admin_authorized(&state, &headers) {
        return error(StatusCode::UNAUTHORIZED, "invalid admin key");
    }
    match state
        .ccp
        .set_session_master_instructions(&session, &update.content)
        .await
    {
        Ok(record) => Json(record).into_response(),
        Err(error_value) => error(StatusCode::NOT_FOUND, error_value.to_string()),
    }
}

async fn admin_dashboard() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        include_str!("admin.html"),
    )
}

async fn setup_client_script(State(state): State<AppState>) -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/x-shellscript; charset=utf-8")],
        render_hosted_script(
            include_str!("../../../scripts/setup-client.sh"),
            &state.base_url,
            false,
        ),
    )
}

async fn setup_client_powershell(State(state): State<AppState>) -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        render_hosted_script(
            include_str!("../../../scripts/setup-client.ps1"),
            &state.base_url,
            true,
        ),
    )
}

async fn management_script(State(state): State<AppState>) -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/x-shellscript; charset=utf-8")],
        render_hosted_script(
            include_str!("../../../scripts/ccp-manage"),
            &state.base_url,
            false,
        ),
    )
}

async fn management_powershell(State(state): State<AppState>) -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        render_hosted_script(
            include_str!("../../../scripts/ccp-manage.ps1"),
            &state.base_url,
            true,
        ),
    )
}

async fn update_script(State(state): State<AppState>) -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/x-shellscript; charset=utf-8")],
        render_hosted_script(
            include_str!("../../../scripts/ccp-update"),
            &state.base_url,
            false,
        ),
    )
}

async fn update_powershell(State(state): State<AppState>) -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        render_hosted_script(
            include_str!("../../../scripts/ccp-update.ps1"),
            &state.base_url,
            true,
        ),
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
        | ClientRequest::GetMasterInstructions { session_id }
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
        | ClientRequest::GetEntryAt { session_id, .. }
        | ClientRequest::SetStatus { session_id, .. }
        | ClientRequest::ClearStatus { session_id, .. }
        | ClientRequest::ListTeamStatus { session_id, .. }
        | ClientRequest::SearchTeamStatus { session_id, .. } => Some(*session_id),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosted_defaults_preserve_quoted_shell_and_powershell_literals() {
        let url = "http://127.0.0.1:1338/path$segment`quoted\"";
        let shell =
            render_hosted_script("DEFAULT_SERVER_URL=\"http://127.0.0.1:1338\"", url, false);
        assert_eq!(
            shell,
            "DEFAULT_SERVER_URL=\"http://127.0.0.1:1338/path\\$segment\\`quoted\\\"\""
        );
        let powershell = render_hosted_script("$Default = \"http://127.0.0.1:1338\"", url, true);
        assert_eq!(
            powershell,
            "$Default = \"http://127.0.0.1:1338/path`$segment``quoted`\"\""
        );
    }
}
