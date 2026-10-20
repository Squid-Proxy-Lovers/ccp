// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Versioned JSON workspace API, separate from the native binary wire protocol.
use serde::{Deserialize, Serialize};

pub const API_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
pub struct Success<T> {
    pub api_version: u32,
    pub data: T,
}

#[derive(Serialize, Deserialize)]
pub struct Failure {
    pub api_version: u32,
    pub error: ApiError,
}

#[derive(Serialize, Deserialize)]
pub struct ApiError {
    pub code: String,
}

#[derive(Serialize, Deserialize)]
pub struct Health {
    pub instance_id: String,
}

#[derive(Serialize, Deserialize)]
pub struct Workspace {
    pub id: String,
    pub name: String,
    pub archived: bool,
}

#[derive(Serialize, Deserialize)]
pub struct Workspaces {
    pub workspaces: Vec<Workspace>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Read,
    ReadWrite,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InitialInvitation {
    pub workspace_id: String,
    pub role: Role,
    pub max_clients: Option<u32>,
    pub expires_at: Option<i64>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndividualInvitation {
    pub workspace_id: String,
    pub role: Role,
    pub expires_at: i64,
}

// Secret-bearing wire objects deliberately have no Debug or Clone. Issuance
// adapters must explicitly project domain secrets into the response.
#[derive(Serialize, Deserialize)]
pub struct InvitationSecret {
    pub invitation_id: String,
    pub secret: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Enrollment {
    pub invitation: String,
    pub request_id: String,
    pub csr_pem: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Renewal {
    pub request_id: String,
    pub csr_pem: String,
}

#[derive(Serialize, Deserialize)]
pub struct CertificateReceipt {
    pub instance_id: String,
    pub client_id: String,
    pub credential_id: String,
    pub certificate_pem: String,
    pub expires_at: i64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceTarget {
    pub workspace_id: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvitationTarget {
    pub workspace_id: String,
    pub invitation_id: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadmissionInvitation {
    pub workspace_id: String,
    pub client_id: String,
    pub role: Role,
    pub expires_at: i64,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RecoveryScope {
    Instance {},
    Workspace { workspace_id: String },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryInvitation {
    pub client_id: String,
    pub scope: RecoveryScope,
    pub expires_at: i64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryTarget {
    pub invitation_id: String,
}

#[derive(Serialize, Deserialize)]
pub struct OperationComplete {
    pub completed: bool,
}

#[derive(Serialize, Deserialize)]
pub struct InvitationStatus {
    pub invitation_id: String,
    pub uses: u64,
    pub max_uses: Option<u64>,
    pub expires_at: Option<i64>,
    pub revoked: bool,
    pub expired: bool,
    pub initial_closed: bool,
    pub issuer_authorized: bool,
    pub target_client_id: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct RecoveryStatus {
    pub client_id: String,
    pub scope: RecoveryScope,
    pub expires_at: i64,
    pub revoked: bool,
    pub expired: bool,
    pub redeemed: bool,
    pub issuer_authorized: bool,
}
