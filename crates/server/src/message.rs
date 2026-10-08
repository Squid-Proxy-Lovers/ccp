// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use protocol::{
    ClientRequest, ErrorCode, ErrorResponse, PROTOCOL_VERSION, ServerResponse, ServerVersionInfo,
};

use crate::identity::ConnectionAuthContext;
use crate::init::SCHEMA_VERSION;
use crate::state::ServerState;

const MAX_APPEND_CONTENT_BYTES: usize = 4 * 1024 * 1024;

fn server_version_string() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

fn build_version_info(compatible: bool) -> ServerVersionInfo {
    ServerVersionInfo {
        protocol_version: PROTOCOL_VERSION,
        server_version: server_version_string(),
        schema_version: SCHEMA_VERSION,
        compatible,
    }
}

pub async fn handle_message_request(
    state: &ServerState,
    auth_context: &ConnectionAuthContext,
    request: ClientRequest,
) -> ServerResponse {
    match request {
        ClientRequest::Ping => match state.ensure_ping_access(auth_context).await {
            Ok(()) => ServerResponse::Pong,
            Err(_) => map_error(CcpError::Forbidden),
        },
        ClientRequest::Handshake(info) => {
            let compatible = info.protocol_version == PROTOCOL_VERSION;
            if compatible {
                ServerResponse::HandshakeOk(build_version_info(true))
            } else {
                ServerResponse::HandshakeRejected(build_version_info(false))
            }
        }
        ClientRequest::List { session_id } => {
            match state.list_entries(session_id, auth_context).await {
                Ok(entries) => ServerResponse::EntrySummaries(entries),
                Err(error) => map_error(error),
            }
        }
        ClientRequest::SearchEntries { session_id, query } => {
            if query.trim().is_empty() {
                return bad_request("query is required for search_entries");
            }
            match state.search_entries(session_id, auth_context, &query).await {
                Ok(entries) => ServerResponse::EntrySummaries(entries),
                Err(error) => map_error(error),
            }
        }
        ClientRequest::SearchShelves { session_id, query } => {
            if query.trim().is_empty() {
                return bad_request("query is required for search_shelves");
            }
            match state.search_shelves(session_id, auth_context, &query).await {
                Ok(entries) => ServerResponse::ShelfSummaries(entries),
                Err(error) => map_error(error),
            }
        }
        ClientRequest::SearchBooks { session_id, query } => {
            if query.trim().is_empty() {
                return bad_request("query is required for search_books");
            }
            match state.search_books(session_id, auth_context, &query).await {
                Ok(entries) => ServerResponse::BookSummaries(entries),
                Err(error) => map_error(error),
            }
        }
        ClientRequest::SearchContext { session_id, query } => {
            if query.trim().is_empty() {
                return bad_request("query is required for search_context");
            }
            match state.search_context(session_id, auth_context, &query).await {
                Ok(results) => ServerResponse::SearchContextResults(results),
                Err(error) => map_error(error),
            }
        }
        ClientRequest::SearchDeleted { session_id, query } => {
            if query.trim().is_empty() {
                return bad_request("query is required for search_deleted");
            }
            match state
                .search_deleted_entries(session_id, auth_context, &query)
                .await
            {
                Ok(results) => ServerResponse::DeletedEntries(results),
                Err(error) => map_error(error),
            }
        }
        ClientRequest::RestoreDeleted {
            session_id,
            entry_key,
        } => {
            if entry_key.trim().is_empty() {
                return bad_request("entry_key is required for restore_deleted");
            }
            match state
                .restore_deleted_entry(session_id, &entry_key, auth_context)
                .await
            {
                Ok(result) => ServerResponse::Restored(result),
                Err(error) => map_error(error),
            }
        }
        ClientRequest::Get {
            session_id,
            name,
            shelf_name,
            book_name,
        } => {
            if name.trim().is_empty() {
                return bad_request("name is required for get");
            }
            match state
                .get_entry(
                    session_id,
                    &name,
                    shelf_name.as_deref(),
                    book_name.as_deref(),
                    auth_context,
                )
                .await
            {
                Ok(Some(entry)) => ServerResponse::Entry(entry),
                Ok(None) => not_found("entry not found"),
                Err(error) => map_error(error),
            }
        }
        ClientRequest::AddShelf {
            session_id,
            shelf_name,
            description,
        } => {
            if shelf_name.trim().is_empty() {
                return bad_request("shelf_name is required for add_shelf");
            }
            match state
                .add_shelf(session_id, &shelf_name, &description, auth_context)
                .await
            {
                Ok(result) => ServerResponse::ShelfAdded(result),
                Err(error) => map_error(error),
            }
        }
        ClientRequest::AddBook {
            session_id,
            shelf_name,
            book_name,
            description,
        } => {
            if shelf_name.trim().is_empty() {
                return bad_request("shelf_name is required for add_book");
            }
            if book_name.trim().is_empty() {
                return bad_request("book_name is required for add_book");
            }
            match state
                .add_book(
                    session_id,
                    &shelf_name,
                    &book_name,
                    &description,
                    auth_context,
                )
                .await
            {
                Ok(result) => ServerResponse::BookAdded(result),
                Err(error) => map_error(error),
            }
        }
        ClientRequest::AddEntry {
            session_id,
            name,
            description,
            labels,
            context,
            shelf_name,
            book_name,
        } => {
            if name.trim().is_empty() {
                return bad_request("name is required for add_entry");
            }
            if shelf_name.trim().is_empty() {
                return bad_request("shelf_name is required for add_entry");
            }
            if book_name.trim().is_empty() {
                return bad_request("book_name is required for add_entry");
            }
            match state
                .add_entry(
                    session_id,
                    &name,
                    &description,
                    &labels,
                    &context,
                    &shelf_name,
                    &book_name,
                    auth_context,
                )
                .await
            {
                Ok((entry, duplicate_warning)) => ServerResponse::EntryAdded {
                    entry,
                    duplicate_warning,
                },
                Err(error) => map_error(error),
            }
        }
        ClientRequest::Append {
            session_id,
            name,
            content,
            metadata,
            shelf_name,
            book_name,
        } => {
            if name.trim().is_empty() {
                return bad_request("name is required for append");
            }
            if content.trim().is_empty() {
                return bad_request("append body cannot be empty");
            }
            if content.len() > MAX_APPEND_CONTENT_BYTES {
                return bad_request("append content exceeds maximum size (4 MB)");
            }
            match state
                .append_to_entry(
                    session_id,
                    &name,
                    shelf_name.as_deref(),
                    book_name.as_deref(),
                    auth_context,
                    &content,
                    metadata,
                )
                .await
            {
                Ok(result) => ServerResponse::AppendResult(result),
                Err(error) => map_error(error),
            }
        }
        ClientRequest::Delete {
            session_id,
            name,
            shelf_name,
            book_name,
        } => {
            if name.trim().is_empty() {
                return bad_request("name is required for delete");
            }
            match state
                .delete_entry(
                    session_id,
                    &name,
                    shelf_name.as_deref(),
                    book_name.as_deref(),
                    auth_context,
                )
                .await
            {
                Ok(result) => ServerResponse::Deleted(result),
                Err(error) => map_error(error),
            }
        }
        ClientRequest::GetHistory {
            session_id,
            name,
            shelf_name,
            book_name,
        } => {
            if name.trim().is_empty() {
                return bad_request("name is required for get_history");
            }
            match state
                .get_history(
                    session_id,
                    &name,
                    shelf_name.as_deref(),
                    book_name.as_deref(),
                    auth_context,
                )
                .await
            {
                Ok(history) => ServerResponse::History(history),
                Err(error) => map_error(error),
            }
        }
        ClientRequest::ExportBundle {
            session_id,
            selector,
        } => match state
            .export_bundle(session_id, &selector, auth_context)
            .await
        {
            Ok(bundle) => ServerResponse::ExportedBundle(bundle),
            Err(error) => map_error(error),
        },
        ClientRequest::ImportBundle {
            session_id,
            bundle,
            policy,
        } => {
            match state
                .import_bundle(session_id, &bundle, &policy, auth_context)
                .await
            {
                Ok(result) => ServerResponse::ImportResult(result),
                Err(error) => map_error(error),
            }
        }
        ClientRequest::RevokeClientCert {
            session_id,
            client_common_name: target_client_common_name,
        } => {
            if target_client_common_name.trim().is_empty() {
                return bad_request("client_common_name is required for revoke_client_cert");
            }
            match state
                .revoke_client_cert(session_id, auth_context, &target_client_common_name)
                .await
            {
                Ok(result) => ServerResponse::CertRevoked(result),
                Err(error) => map_error(error),
            }
        }
        ClientRequest::DeleteShelf {
            session_id,
            shelf_name,
        } => {
            if shelf_name.trim().is_empty() {
                return bad_request("shelf_name is required for delete_shelf");
            }
            match state
                .delete_shelf(session_id, &shelf_name, auth_context)
                .await
            {
                Ok(result) => ServerResponse::ShelfDeleted(result),
                Err(error) => map_error(error),
            }
        }
        ClientRequest::BriefMe { session_id } => {
            match state.brief_me(session_id, auth_context).await {
                Ok(brief) => ServerResponse::Brief(brief),
                Err(error) => map_error(error),
            }
        }
        ClientRequest::GetEntryAt {
            session_id,
            name,
            shelf_name,
            book_name,
            at_timestamp,
        } => {
            if name.trim().is_empty() {
                return bad_request("name is required for get_entry_at");
            }
            match state
                .get_entry_at(
                    session_id,
                    &name,
                    shelf_name.as_deref(),
                    book_name.as_deref(),
                    &at_timestamp,
                    auth_context,
                )
                .await
            {
                Ok(Some(entry)) => ServerResponse::EntryAtTime(entry),
                Ok(None) => not_found("entry not found"),
                Err(error) => map_error(error),
            }
        }
    }
}

fn bad_request(message: &str) -> ServerResponse {
    ServerResponse::Error(ErrorResponse {
        code: ErrorCode::BadRequest,
        message: message.to_string(),
    })
}

fn not_found(message: &str) -> ServerResponse {
    ServerResponse::Error(ErrorResponse {
        code: ErrorCode::NotFound,
        message: message.to_string(),
    })
}

pub(crate) enum CcpError {
    Forbidden,
    NotFound,
    BadRequest(String),
    Internal,
}

impl From<anyhow::Error> for CcpError {
    fn from(error: anyhow::Error) -> Self {
        let root = error.root_cause().to_string();
        if root.contains("does not have")
            || root.contains("not active")
            || root.contains("authorize this session")
        {
            CcpError::Forbidden
        } else if root.contains("not found") || root.contains("unknown session") {
            CcpError::NotFound
        } else if root.contains("required")
            || root.contains("positive")
            || root.contains("already exists")
            || root.contains("must be")
            || root.contains("invalid")
        {
            CcpError::BadRequest(error.to_string())
        } else {
            CcpError::Internal
        }
    }
}

fn map_error(error: impl Into<CcpError>) -> ServerResponse {
    let (code, message) = match error.into() {
        CcpError::Forbidden => (ErrorCode::Forbidden, "access denied".to_string()),
        CcpError::NotFound => (ErrorCode::NotFound, "resource not found".to_string()),
        CcpError::BadRequest(msg) => (ErrorCode::BadRequest, msg),
        CcpError::Internal => (ErrorCode::Internal, "internal server error".to_string()),
    };
    ServerResponse::Error(ErrorResponse { code, message })
}
