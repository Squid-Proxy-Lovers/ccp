// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Additive JSON content DTOs. IDs are targets, never authentication claims.
use serde::{Deserialize, Serialize};

mod fingerprint;
mod shelves;
pub use fingerprint::fingerprint_bytes;
pub use shelves::{ArchivedShelf, ShelfState, ShelfTransition, ShelfTransitionReceipt};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateShelf {
    pub workspace_id: String,
    pub name: String,
    pub description: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateBook {
    pub workspace_id: String,
    pub shelf_id: String,
    pub name: String,
    pub description: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeTarget {
    pub workspace_id: String,
    pub node_id: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShelfPage {
    pub workspace_id: String,
    pub after: Option<String>,
    pub limit: Option<u16>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BookPage {
    pub workspace_id: String,
    pub shelf_id: String,
    pub after: Option<String>,
    pub limit: Option<u16>,
}

#[derive(Serialize, Deserialize)]
pub struct Shelf {
    pub id: String,
    pub workspace_id: String,
    pub name: String,
    pub description: String,
    pub created_at: i64,
}

#[derive(Serialize, Deserialize)]
pub struct Book {
    pub id: String,
    pub workspace_id: String,
    pub shelf_id: String,
    pub name: String,
    pub description: String,
    pub created_at: i64,
}

#[derive(Serialize, Deserialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntryContent {
    pub text: String,
    pub description: String,
    pub labels: Vec<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum EntryChange {
    Create {
        book_id: String,
        name: String,
        content: EntryContent,
    },
    Append {
        entry_id: String,
        expected_revision: u64,
        text: String,
    },
    Revise {
        entry_id: String,
        expected_revision: u64,
        content: EntryContent,
    },
    Archive {
        entry_id: String,
        expected_revision: u64,
    },
    Restore {
        entry_id: String,
        expected_revision: u64,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntryMutation {
    pub workspace_id: String,
    pub operation_id: String,
    pub agent: Option<String>,
    pub change: EntryChange,
}

#[derive(Serialize, Deserialize)]
pub struct EntryReceipt {
    pub entry_id: String,
    pub revision: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntryTarget {
    pub workspace_id: String,
    pub entry_id: String,
    /// None reads the current version. Entry metadata always describes the
    /// current head; the response's separate revision describes the snapshot.
    pub revision: Option<u64>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntryPage {
    pub workspace_id: String,
    pub book_id: String,
    pub archived: bool,
    pub after: Option<String>,
    pub limit: Option<u16>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryPage {
    pub workspace_id: String,
    pub entry_id: String,
    pub after: Option<u64>,
    pub limit: Option<u16>,
}

#[derive(Serialize, Deserialize)]
pub struct EntrySummary {
    pub id: String,
    pub workspace_id: String,
    pub book_id: String,
    pub name: String,
    pub owner_client_id: Option<String>,
    pub revision: u64,
    pub archived: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum ContentActor {
    Admin(String),
    AdminKey(String),
    Credential(String),
}

#[derive(Serialize, Deserialize)]
pub struct EntryRevision {
    pub revision: u64,
    pub action: String,
    pub actor: ContentActor,
    pub client_id: Option<String>,
    pub admin_principal_id: Option<String>,
    pub agent: Option<String>,
    pub recorded_at: i64,
}

#[derive(Serialize, Deserialize)]
pub struct EntrySnapshot {
    pub entry: EntrySummary,
    pub revision: EntryRevision,
    pub content: EntryContent,
    pub appended_text: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct History {
    pub items: Vec<EntryRevision>,
    pub next_cursor: Option<u64>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolveEntryOperation {
    pub workspace_id: String,
    pub operation_id: String,
    pub credential_id: String,
    pub request_hash: String,
}

#[derive(Serialize, Deserialize)]
pub struct EntryOperationResolution {
    pub workspace_id: String,
    pub operation_id: String,
    pub credential_id: String,
    pub request_hash: String,
    pub outcome: EntryOperationOutcome,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum EntryOperationOutcome {
    Committed { entry_id: String, revision: u64 },
    Cancelled,
}
