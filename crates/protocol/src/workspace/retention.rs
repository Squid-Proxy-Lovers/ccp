// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Additive admin retention DTOs; targets and operation IDs never grant authority.
use serde::{Deserialize, Deserializer, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Get {
    pub workspace_id: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Change {
    pub workspace_id: String,
    pub operation_id: String,
    pub expected_revision: u64,
    #[serde(deserialize_with = "required_seconds")]
    pub retention_seconds: Option<u64>,
    pub confirm_timed_purge: bool,
}

fn required_seconds<'de, D: Deserializer<'de>>(value: D) -> Result<Option<u64>, D::Error> {
    Option::<u64>::deserialize(value)
}

#[derive(Serialize, Deserialize)]
pub struct Policy {
    pub workspace_id: String,
    pub revision: u64,
    pub retention_seconds: Option<u64>,
    pub changed_at: Option<i64>,
}

#[derive(Serialize, Deserialize)]
pub struct Receipt {
    pub operation_id: String,
    pub policy: Policy,
}
