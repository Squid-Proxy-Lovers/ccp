// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use serde::{Deserialize, Serialize};

/// Admin-only request; these IDs identify targets, never the acting identity.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShelfTransition {
    pub workspace_id: String,
    pub shelf_id: String,
    pub operation_id: String,
    pub expected_revision: u64,
    pub archived: bool,
}

#[derive(Serialize, Deserialize)]
pub struct ShelfState {
    pub shelf_id: String,
    pub workspace_id: String,
    pub revision: u64,
    pub archived: bool,
}

/// A historical result; fetch shelf state separately for current visibility.
#[derive(Serialize, Deserialize)]
pub struct ShelfTransitionReceipt {
    pub operation_id: String,
    pub result: ShelfState,
}

#[derive(Serialize, Deserialize)]
pub struct ArchivedShelf {
    pub id: String,
    pub workspace_id: String,
    pub name: String,
    pub revision: u64,
    pub created_at: i64,
}
