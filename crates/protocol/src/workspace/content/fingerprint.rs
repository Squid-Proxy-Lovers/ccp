// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::{EntryChange, EntryContent};
use serde::Serialize;

// Field order is part of entry-mutation-1. Serialize directly, without a Value
// object map whose ordering can change when preserve_order is enabled.
#[derive(Serialize)]
pub struct BorrowedEntryContent<'a> {
    pub description: &'a str,
    pub labels: &'a [String],
    pub text: &'a str,
}

impl<'a> From<&'a EntryContent> for BorrowedEntryContent<'a> {
    fn from(value: &'a EntryContent) -> Self {
        Self {
            description: &value.description,
            labels: &value.labels,
            text: &value.text,
        }
    }
}

#[derive(Serialize)]
#[serde(untagged)]
enum Payload<'a> {
    Create(&'a str, &'a str, BorrowedEntryContent<'a>),
    Append(&'a str, u64, &'a str),
    Revise(&'a str, u64, BorrowedEntryContent<'a>),
    Lifecycle(&'a str, u64),
}

/// Borrowed digest input. This is not a wire DTO or an authorization decision.
pub enum BorrowedEntryChange<'a> {
    Create {
        book_id: &'a str,
        name: &'a str,
        content: BorrowedEntryContent<'a>,
    },
    Append {
        entry_id: &'a str,
        expected_revision: u64,
        text: &'a str,
    },
    Revise {
        entry_id: &'a str,
        expected_revision: u64,
        content: BorrowedEntryContent<'a>,
    },
    Archive {
        entry_id: &'a str,
        expected_revision: u64,
    },
    Restore {
        entry_id: &'a str,
        expected_revision: u64,
    },
}

/// Existing entry-mutation-1 digest input, excluding row scope and operation ID.
/// Callers normalize UUIDs and validate input before hashing these UTF-8 bytes.
pub fn fingerprint_bytes(
    change: &EntryChange,
    agent: Option<&str>,
) -> Result<Vec<u8>, serde_json::Error> {
    let borrowed = match change {
        EntryChange::Create {
            book_id,
            name,
            content,
        } => BorrowedEntryChange::Create {
            book_id,
            name,
            content: content.into(),
        },
        EntryChange::Append {
            entry_id,
            expected_revision,
            text,
        } => BorrowedEntryChange::Append {
            entry_id,
            expected_revision: *expected_revision,
            text,
        },
        EntryChange::Revise {
            entry_id,
            expected_revision,
            content,
        } => BorrowedEntryChange::Revise {
            entry_id,
            expected_revision: *expected_revision,
            content: content.into(),
        },
        EntryChange::Archive {
            entry_id,
            expected_revision,
        } => BorrowedEntryChange::Archive {
            entry_id,
            expected_revision: *expected_revision,
        },
        EntryChange::Restore {
            entry_id,
            expected_revision,
        } => BorrowedEntryChange::Restore {
            entry_id,
            expected_revision: *expected_revision,
        },
    };
    borrowed_fingerprint_bytes(borrowed, agent)
}

/// The same canonical bytes using borrowed payloads; label order is preserved.
pub fn borrowed_fingerprint_bytes(
    change: BorrowedEntryChange<'_>,
    agent: Option<&str>,
) -> Result<Vec<u8>, serde_json::Error> {
    let (action, payload) = match change {
        BorrowedEntryChange::Create {
            book_id,
            name,
            content,
        } => ("create", Payload::Create(book_id, name, content)),
        BorrowedEntryChange::Append {
            entry_id,
            expected_revision,
            text,
        } => ("append", Payload::Append(entry_id, expected_revision, text)),
        BorrowedEntryChange::Revise {
            entry_id,
            expected_revision,
            content,
        } => (
            "revise",
            Payload::Revise(entry_id, expected_revision, content),
        ),
        BorrowedEntryChange::Archive {
            entry_id,
            expected_revision,
        } => ("archive", Payload::Lifecycle(entry_id, expected_revision)),
        BorrowedEntryChange::Restore {
            entry_id,
            expected_revision,
        } => ("restore", Payload::Lifecycle(entry_id, expected_revision)),
    };
    serde_json::to_vec(&("entry-mutation-1", action, agent, payload))
}

#[cfg(test)]
mod tests;
