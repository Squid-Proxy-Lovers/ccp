// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Additive guidance DTOs. Scope and publisher metadata never grant authority.
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Scope {
    Global {},
    Workspace { workspace_id: String },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Get {
    pub workspace_id: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Save {
    pub scope: Scope,
    pub operation_id: String,
    pub expected_revision: u64,
    pub text: String,
}

#[derive(Serialize, Deserialize)]
pub struct Receipt {
    pub scope: Scope,
    pub operation_id: String,
    pub revision: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublisherKind {
    Admin,
    Key,
}

#[derive(Serialize, Deserialize)]
pub struct Publisher {
    pub actor_kind: PublisherKind,
    pub actor_id: String,
    pub principal_id: String,
    pub recorded_at: i64,
}

#[derive(Serialize, Deserialize)]
pub struct Board {
    pub scope: Scope,
    pub revision: u64,
    pub text: String,
    pub publisher: Option<Publisher>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Precedence {
    GlobalFirst,
}

#[derive(Serialize, Deserialize)]
pub struct Bundle {
    pub precedence: Precedence,
    pub global: Board,
    pub workspace: Option<Board>,
}

/// Current response context; IDs are not access tokens or permission leases.
#[derive(Serialize, Deserialize)]
pub struct Context {
    pub instance_id: String,
    pub client_id: String,
    pub credential_id: String,
    pub workspace_grant_id: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct Delivery {
    pub format: u32,
    pub context: Context,
    pub guidance: Bundle,
}
