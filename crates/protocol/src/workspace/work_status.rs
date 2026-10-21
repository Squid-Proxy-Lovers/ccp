// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Get {
    pub workspace_id: String,
    pub status_id: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct List {
    pub workspace_id: String,
    pub after: Option<String>,
    pub limit: Option<u16>,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Change {
    Set {
        agent: Option<String>,
        message: String,
    },
    Clear {},
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mutation {
    pub workspace_id: String,
    pub status_id: String,
    pub operation_id: String,
    pub expected_revision: u64,
    pub change: Change,
}
#[derive(Serialize, Deserialize)]
pub struct Status {
    pub workspace_id: String,
    pub status_id: String,
    pub client_id: String,
    pub client_name: String,
    pub client_disabled: bool,
    pub revision: u64,
    pub agent: Option<String>,
    pub message: Option<String>,
    pub updated_at: i64,
    pub stale: bool,
}
#[derive(Serialize, Deserialize)]
pub struct Page {
    pub items: Vec<Status>,
    pub next_cursor: Option<String>,
    pub observed_at: i64,
    pub stale_after_seconds: i64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receipt {
    pub workspace_id: String,
    pub status_id: String,
    pub operation_id: String,
    pub client_id: String,
    pub revision: u64,
    pub cleared: bool,
    pub updated_at: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_status_changes_reject_unknown_fields_for_both_actions() {
        for value in [
            serde_json::json!({"action":"clear","message":"unexpected"}),
            serde_json::json!({"action":"clear","client_id":"claimed"}),
            serde_json::json!({"action":"set","message":"valid","actor":"claimed"}),
        ] {
            assert!(serde_json::from_value::<Change>(value).is_err());
        }
        assert!(serde_json::from_value::<Change>(serde_json::json!({"action":"clear"})).is_ok());
        assert!(
            serde_json::from_value::<Change>(
                serde_json::json!({"action":"set","agent":null,"message":"valid"})
            )
            .is_ok()
        );
    }
}
