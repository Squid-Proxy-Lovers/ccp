// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

mod commands;
mod enrollment;
mod enrollment_structs;
mod storage;
mod transport;
mod transport_helpers;
use std::path::Path;

use anyhow::{Context, bail};
pub use protocol::{
    AddBookResult, AddShelfResult, AppendMetadata, AppendResult, BookSummary, BundleEntry,
    ClientRequest, ConflictPolicy, DeleteResult, DeletedEntrySummary, EntrySummary, ErrorCode,
    ErrorResponse, ImportBundleResult, MessageEntry, MessageHistoryEntry, RestoreResult,
    RevokeCertResult, SearchContextMatch, ServerResponse, ShelfSummary, TransferBundle,
    TransferScope, TransferSelector,
};

pub use enrollment_structs::{EnrollmentMetadata, SessionSummary, StoredEnrollment};

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct EntryLocation {
    pub shelf_name: Option<String>,
    pub book_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddShelfRequest {
    pub shelf_name: String,
    pub shelf_description: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddBookRequest {
    pub shelf_name: String,
    pub book_name: String,
    pub book_description: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddEntryRequest {
    pub shelf_name: String,
    pub book_name: String,
    pub entry_name: String,
    pub entry_description: String,
    pub entry_labels: Vec<String>,
    pub entry_data: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendEntryRequest {
    pub name: String,
    pub content: String,
    pub metadata: AppendMetadata,
    pub shelf_name: Option<String>,
    pub book_name: Option<String>,
}

#[derive(Debug, Default, Clone)]
pub struct CcpClient;

#[derive(Debug, Clone)]
pub struct SessionClient {
    enrollment: StoredEnrollment,
}

impl CcpClient {
    pub fn new() -> Self {
        Self
    }

    pub async fn enroll(&self, redeem_url: &str, token: &str) -> anyhow::Result<StoredEnrollment> {
        enrollment::enroll_and_save(redeem_url, token).await
    }

    pub fn sessions(&self) -> anyhow::Result<Vec<SessionSummary>> {
        let enrollments = storage::load_enrollments()?;
        Ok(storage::summarize_sessions(&enrollments))
    }

    pub fn delete_session(&self, session_selector: &str) -> anyhow::Result<usize> {
        storage::delete_session_enrollments(session_selector)
    }

    pub fn session(&self, session_selector: &str) -> anyhow::Result<SessionClient> {
        self.select_session(session_selector, false)
    }

    pub fn writable_session(&self, session_selector: &str) -> anyhow::Result<SessionClient> {
        self.select_session(session_selector, true)
    }

    fn select_session(
        &self,
        session_selector: &str,
        require_write: bool,
    ) -> anyhow::Result<SessionClient> {
        let enrollment = storage::select_enrollment(session_selector, require_write)?;
        Ok(SessionClient { enrollment })
    }
}

impl SessionClient {
    pub fn enrollment(&self) -> &StoredEnrollment {
        &self.enrollment
    }

    pub async fn list_entries(&self) -> anyhow::Result<Vec<EntrySummary>> {
        match perform_session_request(
            &self.enrollment,
            ClientRequest::List {
                session_id: self.enrollment.metadata.session_id,
            },
        )
        .await?
        {
            ServerResponse::EntrySummaries(entries) => Ok(entries),
            other => unexpected_response("list", other),
        }
    }

    pub async fn get_entry(
        &self,
        name: &str,
        location: EntryLocation,
    ) -> anyhow::Result<MessageEntry> {
        match perform_session_request(
            &self.enrollment,
            ClientRequest::Get {
                session_id: self.enrollment.metadata.session_id,
                name: name.to_string(),
                shelf_name: location.shelf_name,
                book_name: location.book_name,
            },
        )
        .await?
        {
            ServerResponse::Entry(entry) => Ok(entry),
            other => unexpected_response("get", other),
        }
    }

    pub async fn search_entries(&self, query: &str) -> anyhow::Result<Vec<EntrySummary>> {
        match perform_session_request(
            &self.enrollment,
            ClientRequest::SearchEntries {
                session_id: self.enrollment.metadata.session_id,
                query: query.to_string(),
            },
        )
        .await?
        {
            ServerResponse::EntrySummaries(entries) => Ok(entries),
            other => unexpected_response("search_entries", other),
        }
    }

    pub async fn search_shelves(&self, query: &str) -> anyhow::Result<Vec<ShelfSummary>> {
        match perform_session_request(
            &self.enrollment,
            ClientRequest::SearchShelves {
                session_id: self.enrollment.metadata.session_id,
                query: query.to_string(),
            },
        )
        .await?
        {
            ServerResponse::ShelfSummaries(entries) => Ok(entries),
            other => unexpected_response("search_shelves", other),
        }
    }

    pub async fn search_books(&self, query: &str) -> anyhow::Result<Vec<BookSummary>> {
        match perform_session_request(
            &self.enrollment,
            ClientRequest::SearchBooks {
                session_id: self.enrollment.metadata.session_id,
                query: query.to_string(),
            },
        )
        .await?
        {
            ServerResponse::BookSummaries(entries) => Ok(entries),
            other => unexpected_response("search_books", other),
        }
    }

    pub async fn search_context(&self, query: &str) -> anyhow::Result<Vec<SearchContextMatch>> {
        match perform_session_request(
            &self.enrollment,
            ClientRequest::SearchContext {
                session_id: self.enrollment.metadata.session_id,
                query: query.to_string(),
            },
        )
        .await?
        {
            ServerResponse::SearchContextResults(entries) => Ok(entries),
            other => unexpected_response("search_context", other),
        }
    }

    pub async fn search_deleted_entries(
        &self,
        query: &str,
    ) -> anyhow::Result<Vec<DeletedEntrySummary>> {
        match perform_session_request(
            &self.enrollment,
            ClientRequest::SearchDeleted {
                session_id: self.enrollment.metadata.session_id,
                query: query.to_string(),
            },
        )
        .await?
        {
            ServerResponse::DeletedEntries(entries) => Ok(entries),
            other => unexpected_response("search_deleted", other),
        }
    }

    pub async fn add_shelf(&self, request: AddShelfRequest) -> anyhow::Result<AddShelfResult> {
        match perform_session_request(
            &self.enrollment,
            ClientRequest::AddShelf {
                session_id: self.enrollment.metadata.session_id,
                shelf_name: request.shelf_name,
                description: request.shelf_description,
            },
        )
        .await?
        {
            ServerResponse::ShelfAdded(result) => Ok(result),
            other => unexpected_response("add_shelf", other),
        }
    }

    pub async fn add_book(&self, request: AddBookRequest) -> anyhow::Result<AddBookResult> {
        match perform_session_request(
            &self.enrollment,
            ClientRequest::AddBook {
                session_id: self.enrollment.metadata.session_id,
                shelf_name: request.shelf_name,
                book_name: request.book_name,
                description: request.book_description,
            },
        )
        .await?
        {
            ServerResponse::BookAdded(result) => Ok(result),
            other => unexpected_response("add_book", other),
        }
    }

    pub async fn add_entry(&self, request: AddEntryRequest) -> anyhow::Result<MessageEntry> {
        match perform_session_request(
            &self.enrollment,
            ClientRequest::AddEntry {
                session_id: self.enrollment.metadata.session_id,
                name: request.entry_name,
                description: request.entry_description,
                labels: request.entry_labels,
                context: request.entry_data,
                shelf_name: request.shelf_name,
                book_name: request.book_name,
            },
        )
        .await?
        {
            ServerResponse::EntryAdded { entry, .. } => Ok(entry),
            other => unexpected_response("add_entry", other),
        }
    }

    pub async fn append_entry(&self, request: AppendEntryRequest) -> anyhow::Result<AppendResult> {
        match perform_session_request(
            &self.enrollment,
            ClientRequest::Append {
                session_id: self.enrollment.metadata.session_id,
                name: request.name,
                content: request.content,
                metadata: request.metadata,
                shelf_name: request.shelf_name,
                book_name: request.book_name,
            },
        )
        .await?
        {
            ServerResponse::AppendResult(result) => Ok(result),
            other => unexpected_response("append", other),
        }
    }

    pub async fn delete_entry(
        &self,
        name: &str,
        location: EntryLocation,
    ) -> anyhow::Result<DeleteResult> {
        match perform_session_request(
            &self.enrollment,
            ClientRequest::Delete {
                session_id: self.enrollment.metadata.session_id,
                name: name.to_string(),
                shelf_name: location.shelf_name,
                book_name: location.book_name,
            },
        )
        .await?
        {
            ServerResponse::Deleted(result) => Ok(result),
            other => unexpected_response("delete", other),
        }
    }

    pub async fn restore_deleted_entry(&self, entry_key: &str) -> anyhow::Result<RestoreResult> {
        match perform_session_request(
            &self.enrollment,
            ClientRequest::RestoreDeleted {
                session_id: self.enrollment.metadata.session_id,
                entry_key: entry_key.to_string(),
            },
        )
        .await?
        {
            ServerResponse::Restored(result) => Ok(result),
            other => unexpected_response("restore_deleted", other),
        }
    }

    pub async fn get_history(
        &self,
        name: &str,
        location: EntryLocation,
    ) -> anyhow::Result<Vec<MessageHistoryEntry>> {
        match perform_session_request(
            &self.enrollment,
            ClientRequest::GetHistory {
                session_id: self.enrollment.metadata.session_id,
                name: name.to_string(),
                shelf_name: location.shelf_name,
                book_name: location.book_name,
            },
        )
        .await?
        {
            ServerResponse::History(history) => Ok(history),
            other => unexpected_response("get_history", other),
        }
    }

    pub async fn export_bundle(
        &self,
        selector: TransferSelector,
    ) -> anyhow::Result<TransferBundle> {
        match perform_session_request(
            &self.enrollment,
            ClientRequest::ExportBundle {
                session_id: self.enrollment.metadata.session_id,
                selector,
            },
        )
        .await?
        {
            ServerResponse::ExportedBundle(bundle) => Ok(bundle),
            other => unexpected_response("export_bundle", other),
        }
    }

    pub async fn import_bundle(
        &self,
        bundle: TransferBundle,
        policy: ConflictPolicy,
    ) -> anyhow::Result<ImportBundleResult> {
        match perform_session_request(
            &self.enrollment,
            ClientRequest::ImportBundle {
                session_id: self.enrollment.metadata.session_id,
                bundle,
                policy,
            },
        )
        .await?
        {
            ServerResponse::ImportResult(result) => Ok(result),
            other => unexpected_response("import_bundle", other),
        }
    }

    pub async fn import_bundle_from_path(
        &self,
        bundle_path: impl AsRef<Path>,
        policy: ConflictPolicy,
    ) -> anyhow::Result<ImportBundleResult> {
        let bundle_path = bundle_path.as_ref();
        let bundle = serde_json::from_slice::<TransferBundle>(
            &std::fs::read(bundle_path)
                .with_context(|| format!("failed to read {}", bundle_path.display()))?,
        )
        .with_context(|| format!("failed to parse {}", bundle_path.display()))?;
        self.import_bundle(bundle, policy).await
    }

    pub async fn revoke_client_cert(
        &self,
        client_common_name: &str,
    ) -> anyhow::Result<RevokeCertResult> {
        match perform_session_request(
            &self.enrollment,
            ClientRequest::RevokeClientCert {
                session_id: self.enrollment.metadata.session_id,
                client_common_name: client_common_name.to_string(),
            },
        )
        .await?
        {
            ServerResponse::CertRevoked(result) => Ok(result),
            other => unexpected_response("revoke_client_cert", other),
        }
    }
}

pub async fn run_cli() -> anyhow::Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    commands::run().await
}

async fn perform_session_request(
    enrollment: &StoredEnrollment,
    request: ClientRequest,
) -> anyhow::Result<ServerResponse> {
    match transport::perform_request(enrollment, request).await? {
        ServerResponse::Error(error) => error_response_to_anyhow(error),
        response => Ok(response),
    }
}

fn unexpected_response<T>(operation: &str, response: ServerResponse) -> anyhow::Result<T> {
    bail!("unexpected server response for {operation}: {response:?}")
}

fn error_response_to_anyhow(error: ErrorResponse) -> anyhow::Result<ServerResponse> {
    let label = match error.code {
        ErrorCode::BadRequest => "bad request",
        ErrorCode::Forbidden => "forbidden",
        ErrorCode::NotFound => "not found",
        ErrorCode::Internal => "internal error",
    };
    bail!("{label}: {}", error.message)
}
