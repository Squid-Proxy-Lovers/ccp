// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

//! HTTPS browser session DTOs. Session cookies, never these labels, authenticate.
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Login {
    pub key: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Logout {}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Scope {
    Instance {},
    Workspace {
        workspace_id: String,
        role: WorkspaceRole,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceRole {
    Owner,
    Admin,
}

#[derive(Serialize, Deserialize)]
pub struct Identity {
    pub key_id: String,
    pub principal_id: String,
    pub principal_name: String,
    pub key_name: String,
    pub scope: Scope,
}

#[derive(Serialize, Deserialize)]
pub struct Session {
    pub instance_id: String,
    pub expires_at: i64,
    pub csrf_token: String,
    pub identity: Identity,
}

#[derive(Serialize, Deserialize)]
pub struct LoggedOut {
    pub signed_out: bool,
}
