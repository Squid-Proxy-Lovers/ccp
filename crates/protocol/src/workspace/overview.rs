// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Aggregate-only admin workspace overview; never carries entry content or names.
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub workspace_id: String,
    pub days: u32,
}

#[derive(Serialize, Deserialize)]
pub struct Overview {
    pub workspace_id: String,
    pub generated_at: i64,
    pub days: u32,
    pub totals: Totals,
    pub activity: Vec<ActivityDay>,
    pub writers: Vec<Writer>,
    pub access: Access,
}

#[derive(Serialize, Deserialize)]
pub struct Totals {
    pub shelves: u64,
    pub books: u64,
    pub entries: u64,
    pub archived_entries: u64,
}

/// One UTC calendar day; `day` is the Unix time of its 00:00 UTC start.
#[derive(Serialize, Deserialize)]
pub struct ActivityDay {
    pub day: i64,
    pub create: u64,
    pub append: u64,
    pub revise: u64,
    pub archive: u64,
    pub restore: u64,
}

/// `client_id` is null for administrative writes, labelled `Admin`.
#[derive(Serialize, Deserialize)]
pub struct Writer {
    pub client_id: Option<String>,
    pub label: String,
    pub writes: u64,
    pub last_write_at: i64,
}

#[derive(Serialize, Deserialize)]
pub struct Access {
    pub active_clients: u64,
    pub credentials_expiring_within_7d: u64,
    pub pending_invitations: u64,
}
