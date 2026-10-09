// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::path::Path;

use anyhow::{Context, bail};
use protocol::{
    AppendMetadata, ClientRequest, ConflictPolicy, ServerResponse, TransferBundle, TransferSelector,
};

use crate::enrollment_structs::StoredEnrollment;
use crate::transport_helpers::{
    error_response_to_anyhow, perform_http_request, response_to_json_string,
};

pub(crate) async fn perform_request(
    enrollment: &StoredEnrollment,
    request: ClientRequest,
) -> anyhow::Result<ServerResponse> {
    validate_enrollment_access(enrollment, &request)?;
    let response = perform_http_request(enrollment, &request).await?;
    validate_response(&request, &response)?;
    Ok(response)
}

fn validate_response(request: &ClientRequest, response: &ServerResponse) -> anyhow::Result<()> {
    if matches!(response, ServerResponse::Error(_)) {
        return Ok(());
    }
    let expected = matches!(
        (request, response),
        (ClientRequest::Ping, ServerResponse::Pong)
            | (
                ClientRequest::Handshake(_),
                ServerResponse::HandshakeOk(_) | ServerResponse::HandshakeRejected(_)
            )
            | (ClientRequest::ListSessions, ServerResponse::Sessions(_))
            | (
                ClientRequest::CreateSession { .. },
                ServerResponse::SessionCreated(_)
            )
            | (
                ClientRequest::Subscribe { .. },
                ServerResponse::Subscribed(_)
            )
            | (
                ClientRequest::GetMasterInstructions { .. },
                ServerResponse::MasterInstructions(_)
            )
            | (
                ClientRequest::List { .. } | ClientRequest::SearchEntries { .. },
                ServerResponse::EntrySummaries(_)
            )
            | (ClientRequest::Get { .. }, ServerResponse::Entry(_))
            | (
                ClientRequest::AddShelf { .. },
                ServerResponse::ShelfAdded(_)
            )
            | (ClientRequest::AddBook { .. }, ServerResponse::BookAdded(_))
            | (
                ClientRequest::AddEntry { .. },
                ServerResponse::EntryAdded { .. }
            )
            | (
                ClientRequest::Append { .. },
                ServerResponse::AppendResult(_)
            )
            | (ClientRequest::Delete { .. }, ServerResponse::Deleted(_))
            | (
                ClientRequest::SearchShelves { .. },
                ServerResponse::ShelfSummaries(_)
            )
            | (
                ClientRequest::SearchBooks { .. },
                ServerResponse::BookSummaries(_)
            )
            | (
                ClientRequest::SearchContext { .. },
                ServerResponse::SearchContextResults(_)
            )
            | (
                ClientRequest::SearchDeleted { .. },
                ServerResponse::DeletedEntries(_)
            )
            | (
                ClientRequest::RestoreDeleted { .. },
                ServerResponse::Restored(_)
            )
            | (ClientRequest::GetHistory { .. }, ServerResponse::History(_))
            | (
                ClientRequest::ExportBundle { .. },
                ServerResponse::ExportedBundle(_)
            )
            | (
                ClientRequest::ImportBundle { .. },
                ServerResponse::ImportResult(_)
            )
            | (
                ClientRequest::RevokeClientCert { .. },
                ServerResponse::CertRevoked(_)
            )
            | (
                ClientRequest::DeleteShelf { .. },
                ServerResponse::ShelfDeleted(_)
            )
            | (ClientRequest::BriefMe { .. }, ServerResponse::Brief(_))
            | (
                ClientRequest::GetEntryAt { .. },
                ServerResponse::EntryAtTime(_)
            )
            | (
                ClientRequest::SetStatus { .. },
                ServerResponse::StatusSet(_)
            )
            | (
                ClientRequest::ClearStatus { .. },
                ServerResponse::StatusCleared(_)
            )
            | (
                ClientRequest::ListTeamStatus { .. } | ClientRequest::SearchTeamStatus { .. },
                ServerResponse::TeamStatuses(_)
            )
    );
    if !expected {
        bail!("unexpected server response type for requested operation");
    }
    Ok(())
}

fn validate_enrollment_access(
    enrollment: &StoredEnrollment,
    request: &ClientRequest,
) -> anyhow::Result<()> {
    let writable = matches!(enrollment.metadata.access.as_str(), "read_write" | "admin");
    let mutates = matches!(
        request,
        ClientRequest::AddShelf { .. }
            | ClientRequest::AddBook { .. }
            | ClientRequest::AddEntry { .. }
            | ClientRequest::Append { .. }
            | ClientRequest::Delete { .. }
            | ClientRequest::RestoreDeleted { .. }
            | ClientRequest::ImportBundle { .. }
            | ClientRequest::RevokeClientCert { .. }
            | ClientRequest::DeleteShelf { .. }
            | ClientRequest::SetStatus { .. }
            | ClientRequest::ClearStatus { .. }
            | ClientRequest::CreateSession { .. }
    );
    if mutates && !writable {
        bail!("saved subscription does not allow writes");
    }
    if !writable && enrollment.metadata.access != "read" {
        bail!("saved subscription has unsupported access level");
    }
    Ok(())
}

pub(crate) async fn perform_get(
    enrollment: &StoredEnrollment,
    action: &str,
    name: Option<&str>,
    shelf_name: Option<&str>,
    book_name: Option<&str>,
) -> anyhow::Result<String> {
    let request = match action {
        "list" => ClientRequest::List {
            session_id: enrollment.metadata.session_id,
        },
        "get" => ClientRequest::Get {
            session_id: enrollment.metadata.session_id,
            name: name.unwrap_or_default().to_string(),
            shelf_name: shelf_name.map(|value| value.to_string()),
            book_name: book_name.map(|value| value.to_string()),
        },
        "get_history" => ClientRequest::GetHistory {
            session_id: enrollment.metadata.session_id,
            name: name.unwrap_or_default().to_string(),
            shelf_name: shelf_name.map(|value| value.to_string()),
            book_name: book_name.map(|value| value.to_string()),
        },
        _ => bail!("unsupported action: {action}"),
    };

    response_to_json_string(perform_request(enrollment, request).await?)
}

pub(crate) async fn perform_search(
    enrollment: &StoredEnrollment,
    action: &str,
    query: &str,
) -> anyhow::Result<String> {
    let request = match action {
        "search_entries" => ClientRequest::SearchEntries {
            session_id: enrollment.metadata.session_id,
            query: query.to_string(),
        },
        "search_shelves" => ClientRequest::SearchShelves {
            session_id: enrollment.metadata.session_id,
            query: query.to_string(),
        },
        "search_books" => ClientRequest::SearchBooks {
            session_id: enrollment.metadata.session_id,
            query: query.to_string(),
        },
        "search_context" => ClientRequest::SearchContext {
            session_id: enrollment.metadata.session_id,
            query: query.to_string(),
        },
        "search_deleted" => ClientRequest::SearchDeleted {
            session_id: enrollment.metadata.session_id,
            query: query.to_string(),
        },
        _ => bail!("unsupported search action: {action}"),
    };
    response_to_json_string(perform_request(enrollment, request).await?)
}

pub(crate) async fn perform_add_shelf(
    enrollment: &StoredEnrollment,
    shelf_name: &str,
    description: &str,
) -> anyhow::Result<String> {
    response_to_json_string(
        perform_request(
            enrollment,
            ClientRequest::AddShelf {
                session_id: enrollment.metadata.session_id,
                shelf_name: shelf_name.to_string(),
                description: description.to_string(),
            },
        )
        .await?,
    )
}

pub(crate) async fn perform_add_book(
    enrollment: &StoredEnrollment,
    shelf_name: &str,
    book_name: &str,
    description: &str,
) -> anyhow::Result<String> {
    response_to_json_string(
        perform_request(
            enrollment,
            ClientRequest::AddBook {
                session_id: enrollment.metadata.session_id,
                shelf_name: shelf_name.to_string(),
                book_name: book_name.to_string(),
                description: description.to_string(),
            },
        )
        .await?,
    )
}

pub(crate) async fn perform_add_entry(
    enrollment: &StoredEnrollment,
    entry_name: &str,
    description: &str,
    labels: &[String],
    context: &str,
    shelf_name: &str,
    book_name: &str,
) -> anyhow::Result<String> {
    response_to_json_string(
        perform_request(
            enrollment,
            ClientRequest::AddEntry {
                session_id: enrollment.metadata.session_id,
                name: entry_name.to_string(),
                description: description.to_string(),
                labels: labels.to_vec(),
                context: context.to_string(),
                shelf_name: shelf_name.to_string(),
                book_name: book_name.to_string(),
            },
        )
        .await?,
    )
}

pub(crate) async fn perform_append(
    enrollment: &StoredEnrollment,
    entry_name: &str,
    content: &str,
    metadata: AppendMetadata,
    shelf_name: Option<&str>,
    book_name: Option<&str>,
) -> anyhow::Result<String> {
    response_to_json_string(
        perform_request(
            enrollment,
            ClientRequest::Append {
                session_id: enrollment.metadata.session_id,
                name: entry_name.to_string(),
                content: content.to_string(),
                metadata,
                shelf_name: shelf_name.map(|value| value.to_string()),
                book_name: book_name.map(|value| value.to_string()),
            },
        )
        .await?,
    )
}

pub(crate) async fn perform_delete(
    enrollment: &StoredEnrollment,
    entry_name: &str,
    shelf_name: Option<&str>,
    book_name: Option<&str>,
) -> anyhow::Result<String> {
    response_to_json_string(
        perform_request(
            enrollment,
            ClientRequest::Delete {
                session_id: enrollment.metadata.session_id,
                name: entry_name.to_string(),
                shelf_name: shelf_name.map(|value| value.to_string()),
                book_name: book_name.map(|value| value.to_string()),
            },
        )
        .await?,
    )
}

pub(crate) async fn perform_delete_shelf(
    enrollment: &StoredEnrollment,
    shelf_name: &str,
) -> anyhow::Result<String> {
    response_to_json_string(
        perform_request(
            enrollment,
            ClientRequest::DeleteShelf {
                session_id: enrollment.metadata.session_id,
                shelf_name: shelf_name.to_string(),
            },
        )
        .await?,
    )
}

pub(crate) async fn perform_restore(
    enrollment: &StoredEnrollment,
    entry_key: &str,
) -> anyhow::Result<String> {
    response_to_json_string(
        perform_request(
            enrollment,
            ClientRequest::RestoreDeleted {
                session_id: enrollment.metadata.session_id,
                entry_key: entry_key.to_string(),
            },
        )
        .await?,
    )
}

pub(crate) async fn perform_export(
    enrollment: &StoredEnrollment,
    selector: TransferSelector,
) -> anyhow::Result<TransferBundle> {
    match perform_request(
        enrollment,
        ClientRequest::ExportBundle {
            session_id: enrollment.metadata.session_id,
            selector,
        },
    )
    .await?
    {
        ServerResponse::ExportedBundle(bundle) => Ok(bundle),
        ServerResponse::Error(error) => error_response_to_anyhow(error),
        other => bail!("unexpected server response for export: {other:?}"),
    }
}

pub(crate) async fn perform_import(
    enrollment: &StoredEnrollment,
    bundle_path: &Path,
    policy: ConflictPolicy,
) -> anyhow::Result<String> {
    let bytes = std::fs::read(bundle_path)
        .with_context(|| format!("failed to read {}", bundle_path.display()))?;

    let bundle: TransferBundle = serde_json::from_slice(&bytes)
        .with_context(|| format!("failed to parse bundle from {}", bundle_path.display()))?;

    response_to_json_string(
        perform_request(
            enrollment,
            ClientRequest::ImportBundle {
                session_id: enrollment.metadata.session_id,
                bundle,
                policy,
            },
        )
        .await?,
    )
}

pub(crate) async fn perform_brief_me(enrollment: &StoredEnrollment) -> anyhow::Result<String> {
    response_to_json_string(
        perform_request(
            enrollment,
            ClientRequest::BriefMe {
                session_id: enrollment.metadata.session_id,
            },
        )
        .await?,
    )
}

pub(crate) async fn perform_get_entry_at(
    enrollment: &StoredEnrollment,
    entry_name: &str,
    shelf_name: Option<&str>,
    book_name: Option<&str>,
    at_timestamp: &str,
) -> anyhow::Result<String> {
    response_to_json_string(
        perform_request(
            enrollment,
            ClientRequest::GetEntryAt {
                session_id: enrollment.metadata.session_id,
                name: entry_name.to_string(),
                shelf_name: shelf_name.map(String::from),
                book_name: book_name.map(String::from),
                at_timestamp: at_timestamp.to_string(),
            },
        )
        .await?,
    )
}

pub(crate) async fn perform_set_status(
    enrollment: &StoredEnrollment,
    team: &str,
    agent_name: &str,
    status: &str,
) -> anyhow::Result<String> {
    response_to_json_string(
        perform_request(
            enrollment,
            ClientRequest::SetStatus {
                session_id: enrollment.metadata.session_id,
                team: team.to_string(),
                agent_name: agent_name.to_string(),
                status: status.to_string(),
            },
        )
        .await?,
    )
}

pub(crate) async fn perform_clear_status(
    enrollment: &StoredEnrollment,
    team: &str,
    agent_name: &str,
) -> anyhow::Result<String> {
    response_to_json_string(
        perform_request(
            enrollment,
            ClientRequest::ClearStatus {
                session_id: enrollment.metadata.session_id,
                team: team.to_string(),
                agent_name: agent_name.to_string(),
            },
        )
        .await?,
    )
}

pub(crate) async fn perform_list_team_status(
    enrollment: &StoredEnrollment,
    team: &str,
) -> anyhow::Result<String> {
    response_to_json_string(
        perform_request(
            enrollment,
            ClientRequest::ListTeamStatus {
                session_id: enrollment.metadata.session_id,
                team: team.to_string(),
            },
        )
        .await?,
    )
}

pub(crate) async fn perform_search_team_status(
    enrollment: &StoredEnrollment,
    team: &str,
    query: &str,
) -> anyhow::Result<String> {
    response_to_json_string(
        perform_request(
            enrollment,
            ClientRequest::SearchTeamStatus {
                session_id: enrollment.metadata.session_id,
                team: team.to_string(),
                query: query.to_string(),
            },
        )
        .await?,
    )
}
