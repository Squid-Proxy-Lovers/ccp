// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use serde::{Deserialize, Serialize, de::DeserializeOwned};

/// Wire protocol version. Bump this when the ClientRequest/ServerResponse
/// enums change in a backward-incompatible way.
pub const PROTOCOL_VERSION: u32 = 1;

// ── Offline transfer ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TransferScope {
    Session,
    Shelf {
        shelf: String,
    },
    Book {
        shelf: String,
        book: String,
    },
    Entries {
        shelf: String,
        book: String,
        entries: Vec<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TransferSelector {
    pub scope: TransferScope,
    pub include_history: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ConflictPolicy {
    Error,
    Overwrite,
    Skip,
    MergeHistory,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TransferBundle {
    pub session: SessionMetadata,
    pub selector: TransferSelector,
    pub exported_at: String,
    pub entries: Vec<BundleEntry>,
    /// SHA-256 hex digest of the JSON-serialised `entries` field.
    pub bundle_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImportBundleResult {
    pub imported_entries: usize,
    pub overwritten_entries: usize,
    pub skipped_entries: usize,
}

fn default_shelf_name() -> String {
    "main".to_string()
}

fn default_book_name() -> String {
    "default".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionMetadata {
    pub session_name: String,
    pub session_id: i64,
    pub description: String,
    pub owner: String,
    pub labels: Vec<String>,
    pub visibility: String,
    pub purpose: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EntrySummary {
    pub name: String,
    pub description: String,
    pub labels: Vec<String>,
    #[serde(default = "default_shelf_name")]
    pub shelf_name: String,
    #[serde(default = "default_book_name")]
    pub book_name: String,
    #[serde(default)]
    pub shelf_description: String,
    #[serde(default)]
    pub book_description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageEntry {
    pub name: String,
    pub description: String,
    pub labels: Vec<String>,
    pub context: String,
    #[serde(default = "default_shelf_name")]
    pub shelf_name: String,
    #[serde(default = "default_book_name")]
    pub book_name: String,
    #[serde(default)]
    pub shelf_description: String,
    #[serde(default)]
    pub book_description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShelfSummary {
    pub shelf_name: String,
    #[serde(default)]
    pub description: String,
    pub book_count: usize,
    pub entry_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BookSummary {
    #[serde(default = "default_shelf_name")]
    pub shelf_name: String,
    pub book_name: String,
    #[serde(default)]
    pub shelf_description: String,
    #[serde(default)]
    pub description: String,
    pub entry_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AddShelfResult {
    pub shelf_name: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeleteShelfResult {
    pub shelf_name: String,
    pub deleted_books: usize,
    pub deleted_entries: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionBrief {
    pub session_name: String,
    pub session_id: i64,
    pub total_entries: usize,
    pub total_shelves: usize,
    pub total_books: usize,
    pub shelves: Vec<ShelfOverview>,
    pub recent_entries: Vec<RecentEntry>,
    pub frequent_labels: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShelfOverview {
    pub shelf_name: String,
    pub description: String,
    pub book_count: usize,
    pub entry_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecentEntry {
    pub name: String,
    pub description: String,
    pub shelf_name: String,
    pub book_name: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DuplicateWarning {
    pub existing_name: String,
    pub existing_shelf: String,
    pub existing_book: String,
    pub similarity: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AddBookResult {
    #[serde(default = "default_shelf_name")]
    pub shelf_name: String,
    pub book_name: String,
    #[serde(default)]
    pub shelf_description: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppendMetadata {
    pub agent_name: Option<String>,
    pub host_name: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppendResult {
    pub operation_id: String,
    pub name: String,
    pub appended_bytes: usize,
    pub updated_context_length: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeleteResult {
    pub entry_key: String,
    pub name: String,
    pub deleted_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RestoreResult {
    pub entry_key: String,
    pub restored_entry: MessageEntry,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RevokeCertResult {
    pub client_common_name: String,
    pub revoked_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SearchContextMatch {
    pub name: String,
    pub description: String,
    pub snippets: Vec<String>,
    #[serde(default = "default_shelf_name")]
    pub shelf_name: String,
    #[serde(default = "default_book_name")]
    pub book_name: String,
    #[serde(default)]
    pub shelf_description: String,
    #[serde(default)]
    pub book_description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeletedEntrySummary {
    pub entry_key: String,
    pub name: String,
    pub description: String,
    pub labels: Vec<String>,
    #[serde(default = "default_shelf_name")]
    pub shelf_name: String,
    #[serde(default = "default_book_name")]
    pub book_name: String,
    #[serde(default)]
    pub shelf_description: String,
    #[serde(default)]
    pub book_description: String,
    pub deleted_at: String,
    pub deleted_by_client_common_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageHistoryEntry {
    pub operation_id: String,
    pub client_common_name: String,
    pub agent_name: Option<String>,
    pub host_name: Option<String>,
    pub reason: Option<String>,
    pub appended_content: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BundleEntry {
    pub name: String,
    pub description: String,
    pub labels: Vec<String>,
    pub context: String,
    #[serde(default = "default_shelf_name")]
    pub shelf_name: String,
    #[serde(default = "default_book_name")]
    pub book_name: String,
    #[serde(default)]
    pub shelf_description: String,
    #[serde(default)]
    pub book_description: String,
    pub history: Vec<MessageHistoryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VersionInfo {
    pub protocol_version: u32,
    pub client_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServerVersionInfo {
    pub protocol_version: u32,
    pub server_version: String,
    pub schema_version: u32,
    pub compatible: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ClientRequest {
    Ping,
    Handshake(VersionInfo),
    List {
        session_id: i64,
    },
    Get {
        session_id: i64,
        name: String,
        shelf_name: Option<String>,
        book_name: Option<String>,
    },
    AddShelf {
        session_id: i64,
        shelf_name: String,
        description: String,
    },
    AddBook {
        session_id: i64,
        shelf_name: String,
        book_name: String,
        description: String,
    },
    AddEntry {
        session_id: i64,
        name: String,
        description: String,
        labels: Vec<String>,
        context: String,
        shelf_name: String,
        book_name: String,
    },
    Append {
        session_id: i64,
        name: String,
        content: String,
        metadata: AppendMetadata,
        shelf_name: Option<String>,
        book_name: Option<String>,
    },
    Delete {
        session_id: i64,
        name: String,
        shelf_name: Option<String>,
        book_name: Option<String>,
    },
    SearchEntries {
        session_id: i64,
        query: String,
    },
    SearchShelves {
        session_id: i64,
        query: String,
    },
    SearchBooks {
        session_id: i64,
        query: String,
    },
    SearchContext {
        session_id: i64,
        query: String,
    },
    SearchDeleted {
        session_id: i64,
        query: String,
    },
    RestoreDeleted {
        session_id: i64,
        entry_key: String,
    },
    GetHistory {
        session_id: i64,
        name: String,
        shelf_name: Option<String>,
        book_name: Option<String>,
    },
    ExportBundle {
        session_id: i64,
        selector: TransferSelector,
    },
    ImportBundle {
        session_id: i64,
        bundle: TransferBundle,
        policy: ConflictPolicy,
    },
    RevokeClientCert {
        session_id: i64,
        client_common_name: String,
    },
    DeleteShelf {
        session_id: i64,
        shelf_name: String,
    },
    BriefMe {
        session_id: i64,
    },
    GetEntryAt {
        session_id: i64,
        name: String,
        shelf_name: Option<String>,
        book_name: Option<String>,
        at_timestamp: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ErrorCode {
    BadRequest,
    Forbidden,
    NotFound,
    Internal,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ErrorResponse {
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ServerResponse {
    Pong,
    HandshakeOk(ServerVersionInfo),
    HandshakeRejected(ServerVersionInfo),
    EntrySummaries(Vec<EntrySummary>),
    ShelfSummaries(Vec<ShelfSummary>),
    BookSummaries(Vec<BookSummary>),
    ShelfAdded(AddShelfResult),
    BookAdded(AddBookResult),
    SearchContextResults(Vec<SearchContextMatch>),
    DeletedEntries(Vec<DeletedEntrySummary>),
    Entry(MessageEntry),
    EntryAdded {
        entry: MessageEntry,
        duplicate_warning: Option<DuplicateWarning>,
    },
    AppendResult(AppendResult),
    Deleted(DeleteResult),
    Restored(RestoreResult),
    History(Vec<MessageHistoryEntry>),
    ExportedBundle(TransferBundle),
    ImportResult(ImportBundleResult),
    CertRevoked(RevokeCertResult),
    Brief(SessionBrief),
    EntryAtTime(MessageEntry),
    ShelfDeleted(DeleteShelfResult),
    Error(ErrorResponse),
}

pub fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, bincode::Error> {
    bincode::serialize(value)
}

pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, bincode::Error> {
    bincode::deserialize(bytes)
}
