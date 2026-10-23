// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::Actor;

/// Stable content failures. Unexpected database failures retain their internal
/// cause; a future transport must sanitize those rather than expose SQL errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContentError {
    Forbidden,
    InvalidInput,
    NotFound,
    Conflict,
    OperationCancelled,
    LegacyConversionRequired,
}

impl std::fmt::Display for ContentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Forbidden => "content action is not authorized",
            Self::InvalidInput => "invalid content input",
            Self::NotFound => "content is unavailable in this workspace",
            Self::Conflict => "content revision, name or operation conflicts",
            Self::OperationCancelled => "this client operation has been cancelled",
            Self::LegacyConversionRequired => {
                "legacy workspace content requires a verified conversion"
            }
        })
    }
}

impl std::error::Error for ContentError {}

pub struct EntryOperationTarget {
    pub operation_id: Uuid,
    pub credential_id: Uuid,
    pub request_hash: [u8; 32],
}

#[derive(Debug, PartialEq, Eq)]
pub enum EntryOperationOutcome {
    Committed(EntryReceipt),
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryContent {
    pub text: String,
    pub description: String,
    pub labels: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EntryChange {
    Create {
        book_id: Uuid,
        name: String,
        content: EntryContent,
    },
    Append {
        id: Uuid,
        expected_revision: u64,
        text: String,
    },
    Revise {
        id: Uuid,
        expected_revision: u64,
        content: EntryContent,
    },
    Archive {
        id: Uuid,
        expected_revision: u64,
    },
    Restore {
        id: Uuid,
        expected_revision: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryMutation {
    pub operation_id: Uuid,
    /// Attribution only, never an identity or permission selector.
    pub agent: Option<String>,
    pub change: EntryChange,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryReceipt {
    pub entry_id: Uuid,
    pub revision: u64,
}

/// Safe archive-list metadata: deliberately contains no body or old snapshots.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntrySummary {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub book_id: Uuid,
    pub name: String,
    pub owner_client_id: Option<Uuid>,
    pub revision: u64,
    pub archived: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryRevision {
    pub revision: u64,
    pub action: String,
    pub actor: Actor,
    pub client_id: Option<Uuid>,
    pub admin_principal_id: Option<Uuid>,
    pub agent: Option<String>,
    pub recorded_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntrySnapshot {
    pub entry: EntrySummary,
    pub revision: EntryRevision,
    pub content: EntryContent,
    /// Original exact contribution, retained even after later owner revisions.
    pub appended_text: Option<String>,
}

pub struct EntryPageRequest {
    pub book_id: Uuid,
    pub archived: bool,
    pub after: Option<Uuid>,
    pub limit: usize,
}

pub struct RevisionPageRequest {
    pub entry_id: Uuid,
    pub after: u64,
    pub limit: usize,
}

pub struct RevisionPage {
    pub items: Vec<EntryRevision>,
    pub next_cursor: Option<u64>,
}

/// Explicit permanent deletion intent for an individually archived entry.
pub struct EntryPurge {
    pub operation_id: Uuid,
    pub entry_id: Uuid,
    pub expected_revision: u64,
    pub confirm_purge: bool,
}

/// Body-free historical proof; not a claim about forensic media erasure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryPurgeReceipt {
    pub operation_id: Uuid,
    pub entry_id: Uuid,
    pub revision: u64,
    pub purged_at: i64,
}
