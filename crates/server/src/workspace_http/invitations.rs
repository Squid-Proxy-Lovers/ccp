// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::{
    dispatch::{decode, id, role},
    request::Input,
    response::{self, Error, Reply},
};
use crate::foundation::{
    AccessStore, Actor, IndividualInvitationRequest, InitialInvitationRequest,
    ReadmissionInvitationRequest, RecoveryInvitationRequest, RecoveryScope,
};
mod enrollment;
mod listing;

use hyper::Method;
use protocol::workspace as api;

pub(super) fn handles(path: &str) -> bool {
    matches!(
        path,
        "/api/v1/admin/invitations/list"
            | "/api/v1/admin/invitations/initial"
            | "/api/v1/admin/invitations/individual"
            | "/api/v1/admin/invitations/readmission"
            | "/api/v1/admin/invitations/status"
            | "/api/v1/admin/invitations/revoke"
            | "/api/v1/admin/enrollment/close"
            | "/api/v1/admin/enrollment/status"
            | "/api/v1/admin/recovery/issue"
            | "/api/v1/admin/recovery/status"
            | "/api/v1/admin/recovery/revoke"
    )
}

pub(super) fn dispatch(
    store: &mut AccessStore<'_>,
    actor: Actor,
    input: &Input,
    now: i64,
) -> Result<Reply, Error> {
    if input.method != Method::POST {
        return Err(Error::Method);
    }
    match input.path.as_str() {
        "/api/v1/admin/invitations/list" => listing::list(store, actor, input, now),
        "/api/v1/admin/enrollment/status" => enrollment::get(store, actor, input, now),
        "/api/v1/admin/invitations/initial" => {
            let body: api::InitialInvitation = decode(&input.body)?;
            let issued = store
                .issue_initial_invitation(
                    actor,
                    InitialInvitationRequest {
                        workspace_id: id(&body.workspace_id)?,
                        role: role(body.role),
                        max_clients: body.max_clients,
                        expires_at: body.expires_at,
                    },
                    now,
                )
                .map_err(operation)?;
            response::success(api::InvitationSecret {
                invitation_id: issued.id.to_string(),
                secret: issued.secret().to_string(),
            })
        }
        "/api/v1/admin/invitations/individual" => {
            let body: api::IndividualInvitation = decode(&input.body)?;
            let issued = store
                .issue_individual_invitation(
                    actor,
                    IndividualInvitationRequest {
                        workspace_id: id(&body.workspace_id)?,
                        role: role(body.role),
                        expires_at: body.expires_at,
                    },
                    now,
                )
                .map_err(operation)?;
            response::success(api::InvitationSecret {
                invitation_id: issued.id.to_string(),
                secret: issued.secret().to_string(),
            })
        }
        "/api/v1/admin/invitations/readmission" => {
            let body: api::ReadmissionInvitation = decode(&input.body)?;
            let issued = store
                .issue_readmission_invitation(
                    actor,
                    ReadmissionInvitationRequest {
                        workspace_id: id(&body.workspace_id)?,
                        client_id: id(&body.client_id)?,
                        role: role(body.role),
                        expires_at: body.expires_at,
                    },
                    now,
                )
                .map_err(operation)?;
            response::success(api::InvitationSecret {
                invitation_id: issued.id.to_string(),
                secret: issued.secret().to_string(),
            })
        }
        "/api/v1/admin/enrollment/close" => {
            let body: api::WorkspaceTarget = decode(&input.body)?;
            store
                .close_initial_enrollment(actor, id(&body.workspace_id)?, now)
                .map_err(operation)?;
            complete()
        }
        "/api/v1/admin/invitations/status" | "/api/v1/admin/invitations/revoke" => {
            invitation(store, actor, input, now)
        }
        _ => recovery(store, actor, input, now),
    }
}

fn invitation(
    store: &mut AccessStore<'_>,
    actor: Actor,
    input: &Input,
    now: i64,
) -> Result<Reply, Error> {
    let body: api::InvitationTarget = decode(&input.body)?;
    let workspace = id(&body.workspace_id)?;
    let invitation = id(&body.invitation_id)?;
    if input.path == "/api/v1/admin/invitations/revoke" {
        store
            .revoke_invitation(actor, workspace, invitation, now)
            .map_err(operation)?;
        return complete();
    }
    let status = store
        .invitation_status(actor, workspace, invitation, now)
        .map_err(operation)?;
    response::success(status_view(status))
}

fn status_view(status: crate::foundation::InvitationStatus) -> api::InvitationStatus {
    api::InvitationStatus {
        invitation_id: status.id.to_string(),
        uses: status.uses,
        max_uses: status.max_uses,
        expires_at: status.expires_at,
        revoked: status.revoked,
        expired: status.expired,
        initial_closed: status.initial_closed,
        issuer_authorized: status.issuer_authorized,
        target_client_id: status.target_client_id.map(|id| id.to_string()),
    }
}

fn recovery(
    store: &mut AccessStore<'_>,
    actor: Actor,
    input: &Input,
    now: i64,
) -> Result<Reply, Error> {
    if input.path == "/api/v1/admin/recovery/issue" {
        let body: api::RecoveryInvitation = decode(&input.body)?;
        let scope = match body.scope {
            api::RecoveryScope::Instance {} => RecoveryScope::Instance,
            api::RecoveryScope::Workspace { workspace_id } => {
                RecoveryScope::Workspace(id(&workspace_id)?)
            }
        };
        let issued = store
            .issue_recovery_invitation(
                actor,
                RecoveryInvitationRequest {
                    client_id: id(&body.client_id)?,
                    scope,
                    expires_at: body.expires_at,
                },
                now,
            )
            .map_err(operation)?;
        return response::success(api::InvitationSecret {
            invitation_id: issued.id.to_string(),
            secret: issued.secret().to_string(),
        });
    }
    let body: api::RecoveryTarget = decode(&input.body)?;
    let invitation = id(&body.invitation_id)?;
    if input.path == "/api/v1/admin/recovery/revoke" {
        store
            .revoke_recovery_invitation(actor, invitation, now)
            .map_err(operation)?;
        return complete();
    }
    if input.path != "/api/v1/admin/recovery/status" {
        return Err(Error::NotFound);
    }
    let status = store
        .recovery_invitation_status(actor, invitation, now)
        .map_err(operation)?;
    let scope = match status.scope {
        RecoveryScope::Instance => api::RecoveryScope::Instance {},
        RecoveryScope::Workspace(workspace) => api::RecoveryScope::Workspace {
            workspace_id: workspace.to_string(),
        },
    };
    response::success(api::RecoveryStatus {
        client_id: status.client_id.to_string(),
        scope,
        expires_at: status.expires_at,
        revoked: status.revoked,
        expired: status.expired,
        redeemed: status.redeemed,
        issuer_authorized: status.issuer_authorized,
    })
}

fn complete() -> Result<Reply, Error> {
    response::success(api::OperationComplete { completed: true })
}

fn operation(error: anyhow::Error) -> Error {
    response::operation(error, Error::Forbidden)
}
