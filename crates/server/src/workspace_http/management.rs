// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::{
    dispatch::{decode, id},
    request::Input,
    response::{self, Error, Reply},
};
use crate::foundation::{
    AccessStore, Actor, DataRole, InitialOwner, OwnerProvisioning, ProvisionedWorkspace,
};
use hyper::Method;
use protocol::workspace as api;

pub(super) fn handles(path: &str) -> bool {
    matches!(
        path,
        "/api/v1/admin/workspaces/create"
            | "/api/v1/admin/workspaces/first-owner"
            | "/api/v1/admin/workspaces/inspect"
            | "/api/v1/admin/workspaces/grants"
            | "/api/v1/admin/workspaces/archive"
            | "/api/v1/admin/workspaces/reactivate"
            | "/api/v1/admin/grants/revoke"
            | "/api/v1/admin/clients/disable"
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
        "/api/v1/admin/workspaces/create" => {
            let body: api::CreateWorkspace = decode(&input.body)?;
            let request = provisioning(body.owner, body.key_expires_at)?;
            provisioned(
                store
                    .create_workspace_with_owner_key(actor, &body.name, &request, now)
                    .map_err(operation)?,
            )
        }
        "/api/v1/admin/workspaces/first-owner" => {
            let body: api::FirstOwner = decode(&input.body)?;
            let request = provisioning(body.owner, body.key_expires_at)?;
            provisioned(
                store
                    .assign_initial_owner_with_key(actor, id(&body.workspace_id)?, &request, now)
                    .map_err(operation)?,
            )
        }
        "/api/v1/admin/workspaces/inspect" => {
            let body: api::WorkspaceTarget = decode(&input.body)?;
            let info = store
                .workspace_administration(actor, id(&body.workspace_id)?, now)
                .map_err(operation)?;
            response::success(api::WorkspaceAdministration {
                workspace: api::Workspace {
                    id: info.workspace.id.to_string(),
                    name: info.workspace.name,
                    archived: info.workspace.archived,
                },
                owner_id: info.owner_id.map(|id| id.to_string()),
            })
        }
        "/api/v1/admin/workspaces/grants" => grants(store, actor, input, now),
        "/api/v1/admin/workspaces/archive" | "/api/v1/admin/workspaces/reactivate" => {
            let body: api::WorkspaceTarget = decode(&input.body)?;
            store
                .set_archived(
                    actor,
                    id(&body.workspace_id)?,
                    input.path == "/api/v1/admin/workspaces/archive",
                    now,
                )
                .map_err(operation)?;
            complete()
        }
        "/api/v1/admin/grants/revoke" => {
            let body: api::GrantTarget = decode(&input.body)?;
            store
                .revoke_grant(actor, id(&body.grant_id)?, now)
                .map_err(operation)?;
            complete()
        }
        "/api/v1/admin/clients/disable" => {
            let body: api::ClientTarget = decode(&input.body)?;
            store
                .disable_client(actor, id(&body.client_id)?, now)
                .map_err(operation)?;
            complete()
        }
        _ => Err(Error::NotFound),
    }
}

fn provisioning(
    owner: api::InitialOwner,
    key_expires_at: Option<i64>,
) -> Result<OwnerProvisioning, Error> {
    Ok(OwnerProvisioning {
        owner: match owner {
            api::InitialOwner::Existing { principal_id } => {
                InitialOwner::Existing(id(&principal_id)?)
            }
            api::InitialOwner::NewPrincipal { name } => InitialOwner::NewPrincipal(name),
        },
        key_expires_at,
    })
}

fn provisioned(result: ProvisionedWorkspace) -> Result<Reply, Error> {
    response::success(api::ProvisionedWorkspace {
        workspace_id: result.workspace_id.to_string(),
        owner_id: result.owner_key.principal_id.to_string(),
        owner_key_id: result.owner_key.id.to_string(),
        owner_key: result.owner_key.secret().to_string(),
    })
}

fn grants(
    store: &mut AccessStore<'_>,
    actor: Actor,
    input: &Input,
    now: i64,
) -> Result<Reply, Error> {
    let body: api::GrantPage = decode(&input.body)?;
    let limit = body.limit.unwrap_or(100);
    if limit == 0 || limit > 500 {
        return Err(Error::BadRequest);
    }
    let page = store
        .workspace_grants_page(
            actor,
            id(&body.workspace_id)?,
            body.after.as_deref().map(id).transpose()?,
            usize::try_from(limit).map_err(|_limit| Error::BadRequest)?,
            now,
        )
        .map_err(operation)?;
    response::success(api::Grants {
        grants: page
            .grants
            .into_iter()
            .map(|g| api::Grant {
                id: g.id.to_string(),
                client_id: g.client_id.to_string(),
                client_name: g.client_name,
                role: match g.role {
                    DataRole::Read => api::Role::Read,
                    DataRole::ReadWrite => api::Role::ReadWrite,
                },
                revoked_at: g.revoked_at,
                client_disabled: g.client_disabled,
            })
            .collect(),
        next_cursor: page.next_cursor.map(|id| id.to_string()),
    })
}

fn complete() -> Result<Reply, Error> {
    response::success(api::OperationComplete { completed: true })
}
fn operation(error: anyhow::Error) -> Error {
    response::operation(error, Error::Forbidden)
}
