// SPDX-License-Identifier: AGPL-3.0-or-later

//! Explicit archived-workspace deletion; a start receipt does not imply cleanup finished.
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Start {
    pub workspace_id: String,
    pub operation_id: String,
    pub expected_revision: u64,
    pub confirm_workspace_id: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Status {
    pub workspace_id: String,
    pub operation_id: String,
}

#[derive(Serialize, Deserialize)]
pub struct Preview {
    pub workspace_id: String,
    pub name: String,
    pub lifecycle_revision: u64,
}

#[derive(Serialize, Deserialize)]
pub struct Receipt {
    pub workspace_id: String,
    pub operation_id: String,
    pub lifecycle_revision: u64,
    pub started_at: i64,
    pub completed_at: Option<i64>,
}
