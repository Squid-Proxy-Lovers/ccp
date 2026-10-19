// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use uuid::Uuid;

/// An identity already authenticated by the caller's trusted transport layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Actor {
    Admin(Uuid),
    AdminKey(Uuid),
    Credential(Uuid),
}

/// Workspace-level capabilities. Entry ownership is checked by the later
/// content layer in addition to these coarse create/append/write capabilities.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    ReadContext,
    WriteContext,
    SaveWorkspaceBoard,
    AdmitClient,
    RevokeGrant,
    RecoverWorkspaceClient,
    AppointAdmin,
    RevokeAdmin,
    TransferOwner,
    ArchiveWorkspace,
    ReactivateWorkspace,
    PurgeWorkspace,
    ManageArchives,
    CreateWorkspace,
    SaveGlobalBoard,
    DisableClient,
    RecoverGlobalClient,
    ManageInstanceKeys,
    ManageWorkspaceKeys,
}

impl Action {
    pub(super) fn instance_wide(self) -> bool {
        matches!(
            self,
            Self::CreateWorkspace
                | Self::SaveGlobalBoard
                | Self::DisableClient
                | Self::RecoverGlobalClient
                | Self::ManageInstanceKeys
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataRole {
    Read,
    ReadWrite,
}

impl DataRole {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::ReadWrite => "read_write",
        }
    }
}

/// Records a new registration and its first workspace grant. Certificate
/// issuance and invitation redemption are the next phase's external boundary.
#[derive(Clone, Debug)]
pub struct ClientRegistration {
    pub workspace_id: Uuid,
    pub role: DataRole,
    pub expires_at: i64,
}

/// Adds a grant to an existing client proven by a live presented credential.
#[derive(Clone, Copy, Debug)]
pub struct Admission {
    pub workspace_id: Uuid,
    pub presented_credential: Uuid,
    pub role: DataRole,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Enrollment {
    pub client_id: Uuid,
    pub credential_id: Uuid,
    pub grant_id: Uuid,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceSummary {
    pub id: Uuid,
    pub name: String,
    pub archived: bool,
}
