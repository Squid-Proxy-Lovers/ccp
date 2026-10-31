// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::Actor;
use uuid::Uuid;

pub const GUIDANCE_TEXT_BYTES: usize = 8 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuidanceScope {
    Global,
    Workspace(Uuid),
}
impl GuidanceScope {
    pub(super) fn key(self) -> String {
        match self {
            Self::Global => "global".into(),
            Self::Workspace(id) => id.to_string(),
        }
    }
    pub(super) fn workspace(self) -> Option<Uuid> {
        match self {
            Self::Global => None,
            Self::Workspace(id) => Some(id),
        }
    }
}

pub struct GuidanceSave {
    pub operation_id: Uuid,
    pub expected_revision: u64,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuidancePublisher {
    pub actor: Actor,
    pub principal_id: Uuid,
    pub recorded_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuidanceBoard {
    pub scope: GuidanceScope,
    /// Zero denotes no published revision, distinct from an explicit empty save.
    pub revision: u64,
    pub text: String,
    pub publisher: Option<GuidancePublisher>,
}

/// Historical save result, not a claim about the current board or model receipt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuidanceReceipt {
    pub scope: GuidanceScope,
    pub operation_id: Uuid,
    pub revision: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuidancePrecedence {
    GlobalFirst,
}

/// CCP guidance only: host instructions and actual server permissions retain
/// priority. Workspace prose cannot change these structural source labels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuidanceBundle {
    pub precedence: GuidancePrecedence,
    pub global: GuidanceBoard,
    pub workspace: Option<GuidanceBoard>,
}

/// Current authenticated delivery partition, not a token or permission lease.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuidanceContext {
    pub instance_id: Uuid,
    pub client_id: Uuid,
    pub credential_id: Uuid,
    pub workspace_grant_id: Option<Uuid>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuidanceRead {
    pub context: GuidanceContext,
    pub guidance: GuidanceBundle,
}
