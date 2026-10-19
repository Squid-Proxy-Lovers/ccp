// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use uuid::Uuid;

use super::DataRole;

#[derive(Clone, Copy, Debug)]
pub struct InitialInvitationRequest {
    pub workspace_id: Uuid,
    pub role: DataRole,
    pub max_clients: Option<u32>,
    pub expires_at: Option<i64>,
}

#[derive(Clone, Copy, Debug)]
pub struct IndividualInvitationRequest {
    pub workspace_id: Uuid,
    pub role: DataRole,
    pub expires_at: i64,
}

/// Issuance-only secret. No implicit Debug/Serialize disclosure is available.
pub struct IssuedInvitation {
    pub id: Uuid,
    secret: String,
}

impl IssuedInvitation {
    pub fn secret(&self) -> &str {
        &self.secret
    }

    pub(super) fn generate() -> anyhow::Result<Self> {
        let mut random = [0_u8; 32];
        getrandom::getrandom(&mut random)
            .map_err(|error| anyhow::anyhow!("failed to generate invitation: {error}"))?;
        let hex: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        Ok(Self {
            id: Uuid::new_v4(),
            secret: format!("ccp_join_{hex}"),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvitationStatus {
    pub id: Uuid,
    pub uses: u64,
    pub max_uses: Option<u64>,
    pub expires_at: Option<i64>,
    pub revoked: bool,
    pub initial_closed: bool,
    pub issuer_authorized: bool,
}
