// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::path::Path;

use anyhow::{Context, bail};
use protocol::{
    AppendMetadata, ClientRequest, ConflictPolicy, ServerResponse, TransferBundle, TransferSelector,
};

use crate::enrollment_structs::StoredEnrollment;
use crate::transport_helpers::{connect_mtls, error_response_to_anyhow, response_to_json_string};

pub(crate) async fn perform_request(
    enrollment: &StoredEnrollment,
    request: ClientRequest,
) -> anyhow::Result<ServerResponse> {
    let mut connection = connect_mtls(enrollment).await?;
    connection.request(request).await
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
        ServerResponse::Error(error) => {
            let _ = error_response_to_anyhow(error)?;
            unreachable!("error_response_to_anyhow always returns Err");
        }
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

pub(crate) async fn perform_revoke_cert(
    enrollment: &StoredEnrollment,
    client_common_name: &str,
) -> anyhow::Result<String> {
    response_to_json_string(
        perform_request(
            enrollment,
            ClientRequest::RevokeClientCert {
                session_id: enrollment.metadata.session_id,
                client_common_name: client_common_name.to_string(),
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
