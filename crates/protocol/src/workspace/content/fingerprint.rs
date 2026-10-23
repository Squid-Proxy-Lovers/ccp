// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::{EntryChange, EntryContent};
use serde::Serialize;

// Field order is part of entry-mutation-1. Serialize directly, without a Value
// object map whose ordering can change when preserve_order is enabled.
#[derive(Serialize)]
struct Content<'a> {
    description: &'a str,
    labels: &'a [String],
    text: &'a str,
}

impl<'a> From<&'a EntryContent> for Content<'a> {
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
    Create(&'a str, &'a str, Content<'a>),
    Append(&'a str, u64, &'a str),
    Revise(&'a str, u64, Content<'a>),
    Lifecycle(&'a str, u64),
}

/// Existing entry-mutation-1 digest input, excluding row scope and operation ID.
/// Callers must normalize target UUIDs before calling; this codec grants no
/// authority and performs no input validation. Hash the returned UTF-8 bytes.
pub fn fingerprint_bytes(
    change: &EntryChange,
    agent: Option<&str>,
) -> Result<Vec<u8>, serde_json::Error> {
    let (action, payload) = match change {
        EntryChange::Create {
            book_id,
            name,
            content,
        } => ("create", Payload::Create(book_id, name, content.into())),
        EntryChange::Append {
            entry_id,
            expected_revision,
            text,
        } => (
            "append",
            Payload::Append(entry_id, *expected_revision, text),
        ),
        EntryChange::Revise {
            entry_id,
            expected_revision,
            content,
        } => (
            "revise",
            Payload::Revise(entry_id, *expected_revision, content.into()),
        ),
        EntryChange::Archive {
            entry_id,
            expected_revision,
        } => ("archive", Payload::Lifecycle(entry_id, *expected_revision)),
        EntryChange::Restore {
            entry_id,
            expected_revision,
        } => ("restore", Payload::Lifecycle(entry_id, *expected_revision)),
    };
    serde_json::to_vec(&("entry-mutation-1", action, agent, payload))
}

#[cfg(test)]
mod tests;
