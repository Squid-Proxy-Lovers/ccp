// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use uuid::Uuid;

pub const WORK_STATUS_MESSAGE_BYTES: usize = 4096;
pub const WORK_STATUS_AGENT_BYTES: usize = 128;
pub const WORK_STATUS_STALE_SECONDS: i64 = 300;
pub const WORK_STATUS_LIVE_LIMIT: usize = 64;

pub enum WorkStatusChange {
    Set {
        agent: Option<String>,
        message: String,
    },
    Clear,
}

pub struct WorkStatusMutation {
    pub operation_id: Uuid,
    pub status_id: Uuid,
    pub expected_revision: u64,
    pub change: WorkStatusChange,
}

/// Latest explicit message, not proof of contact, liveness or completed work.
/// A cleared tombstone retains ownership/revision but contains no old text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkStatus {
    pub workspace_id: Uuid,
    pub status_id: Uuid,
    pub client_id: Uuid,
    pub client_name: String,
    pub client_disabled: bool,
    pub revision: u64,
    pub agent: Option<String>,
    pub message: Option<String>,
    pub updated_at: i64,
    pub stale: bool,
}

/// Body-free historical result; read current state separately.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkStatusReceipt {
    pub workspace_id: Uuid,
    pub status_id: Uuid,
    pub operation_id: Uuid,
    pub client_id: Uuid,
    pub revision: u64,
    pub cleared: bool,
    pub updated_at: i64,
}

#[derive(Debug, PartialEq, Eq)]
pub struct WorkStatusPage {
    pub items: Vec<WorkStatus>,
    pub next_cursor: Option<Uuid>,
    pub observed_at: i64,
    pub stale_after_seconds: i64,
}
