// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::IssuedAdminKey;
use uuid::Uuid;

/// Explicit identity selection. A new display name never claims an existing
/// principal, legacy owner label, workspace or global administrative role.
#[derive(Clone, Debug)]
pub enum InitialOwner {
    Existing(Uuid),
    NewPrincipal(String),
}

#[derive(Clone, Debug)]
pub struct OwnerProvisioning {
    pub owner: InitialOwner,
    pub key_expires_at: Option<i64>,
}

/// One-time secret delivery only at creation/first appointment. No implicit
/// Debug/Serialize/Clone; adapters must explicitly disclose the owner key once.
pub struct ProvisionedWorkspace {
    pub workspace_id: Uuid,
    pub owner_key: IssuedAdminKey,
}
