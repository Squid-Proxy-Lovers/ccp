// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use serde::{Deserialize, Serialize};

/// Explicit scoped-admin intent; IDs select targets and never authenticate.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntryPurge {
    pub workspace_id: String,
    pub entry_id: String,
    pub operation_id: String,
    pub expected_revision: u64,
    pub confirm_purge: bool,
}

/// Historical body-free result, without a forensic erasure guarantee.
#[derive(Serialize, Deserialize)]
pub struct EntryPurgeReceipt {
    pub workspace_id: String,
    pub entry_id: String,
    pub operation_id: String,
    pub revision: u64,
    pub purged_at: i64,
}
