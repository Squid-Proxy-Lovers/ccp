// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::InitialOwner;
use uuid::Uuid;

/// Uses the same explicit existing-ID/new-label selection as first ownership.
pub type PrincipalSelection = InitialOwner;

#[derive(Clone, Debug)]
pub struct AdminAppointment {
    pub principal: PrincipalSelection,
    pub key_expires_at: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdminSummary {
    pub id: Uuid,
    pub name: String,
    pub enabled: bool,
    pub is_owner: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdminPage {
    pub admins: Vec<AdminSummary>,
    pub next_cursor: Option<Uuid>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdminKeySummary {
    pub id: Uuid,
    pub principal_id: Uuid,
    pub name: String,
    pub expires_at: Option<i64>,
    pub revoked_at: Option<i64>,
    pub role_active: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdminKeyPage {
    pub keys: Vec<AdminKeySummary>,
    pub next_cursor: Option<Uuid>,
}
