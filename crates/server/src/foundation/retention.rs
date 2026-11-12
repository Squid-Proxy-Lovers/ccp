// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use uuid::Uuid;

/// Explicit scoped configuration. A changed duration requires disable then enable.
pub struct EntryRetentionChange {
    pub operation_id: Uuid,
    pub expected_revision: u64,
    pub retention_seconds: Option<u64>,
    pub confirm_timed_purge: bool,
}

/// A policy snapshot or historical operation result, not a current authority lease.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryRetentionPolicy {
    pub workspace_id: Uuid,
    pub revision: u64,
    pub retention_seconds: Option<u64>,
    pub changed_at: Option<i64>,
}

/// Body-free automatic purge audit linked to the immutable configuring policy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimedEntryPurge {
    pub workspace_id: Uuid,
    pub entry_id: Uuid,
    pub archive_revision: u64,
    pub policy_revision: u64,
    pub purged_at: i64,
}
