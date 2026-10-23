// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryScope {
    Instance,
    Workspace(Uuid),
}

impl RecoveryScope {
    pub(super) fn workspace(self) -> Option<Uuid> {
        match self {
            Self::Instance => None,
            Self::Workspace(workspace) => Some(workspace),
        }
    }

    pub(super) fn action(self) -> super::Action {
        match self {
            Self::Instance => super::Action::RecoverGlobalClient,
            Self::Workspace(_) => super::Action::RecoverWorkspaceClient,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct RecoveryInvitationRequest {
    pub client_id: Uuid,
    pub scope: RecoveryScope,
    pub expires_at: i64,
}

/// A recovery secret returned only at issuance, never implicitly formatted.
pub struct IssuedRecoveryInvitation {
    pub id: Uuid,
    secret: String,
}

impl IssuedRecoveryInvitation {
    pub fn secret(&self) -> &str {
        &self.secret
    }

    pub(super) fn generate() -> anyhow::Result<Self> {
        let mut random = [0_u8; 32];
        getrandom::getrandom(&mut random)
            .map_err(|error| anyhow::anyhow!("failed to generate recovery invitation: {error}"))?;
        let hex: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        Ok(Self {
            id: Uuid::new_v4(),
            secret: format!("ccp_recover_{hex}"),
        })
    }
}

/// Target identity and scope are captured by the server, not redemption claims.
#[derive(Clone, Debug)]
pub struct RecoveryRedemptionRequest {
    pub request_id: Uuid,
    pub csr_pem: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryReceipt {
    pub client_id: Uuid,
    pub credential_id: Uuid,
    pub grant_ids: Vec<Uuid>,
    pub certificate_pem: String,
    pub expires_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryInvitationStatus {
    pub client_id: Uuid,
    pub scope: RecoveryScope,
    pub expires_at: i64,
    pub revoked: bool,
    pub expired: bool,
    pub redeemed: bool,
    pub issuer_authorized: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct RecoveryPageRequest {
    pub client_id: Uuid,
    pub scope: RecoveryScope,
    pub after: Option<Uuid>,
    pub limit: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryInvitationSummary {
    pub id: Uuid,
    pub status: RecoveryInvitationStatus,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryInvitationPage {
    pub invitations: Vec<RecoveryInvitationSummary>,
    pub next_cursor: Option<Uuid>,
}
