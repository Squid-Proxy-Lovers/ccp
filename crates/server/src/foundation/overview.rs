// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use uuid::Uuid;

/// Longest activity window, in UTC days, that one overview read may cover.
pub const OVERVIEW_MAX_DAYS: u32 = 90;

/// Aggregate workspace metrics from one read snapshot. It carries no entry
/// content, entry names or shelf/book names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceOverview {
    pub workspace_id: Uuid,
    pub generated_at: i64,
    pub days: u32,
    pub totals: ContentTotals,
    /// Exactly `days` UTC days, oldest first, ending with the current day.
    pub activity: Vec<DailyActivity>,
    pub writers: Vec<WriterActivity>,
    pub access: AccessCounts,
}

/// Current rows: unarchived shelves, all books and entries by archive flag.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ContentTotals {
    pub shelves: u64,
    pub books: u64,
    pub entries: u64,
    pub archived_entries: u64,
}

/// Retained content versions recorded during one UTC day, by action.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DailyActivity {
    pub day: i64,
    pub create: u64,
    pub append: u64,
    pub revise: u64,
    pub archive: u64,
    pub restore: u64,
}

/// A writer within the window. Administrative writes have no client.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WriterActivity {
    pub client_id: Option<Uuid>,
    pub client_name: Option<String>,
    pub writes: u64,
    pub last_write_at: i64,
}

/// Current membership counts using the grant and invitation-redemption rules.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AccessCounts {
    pub active_clients: u64,
    pub credentials_expiring_within_7d: u64,
    pub pending_invitations: u64,
}
