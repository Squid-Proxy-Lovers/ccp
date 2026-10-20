// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::{
    dispatch::{decode, id},
    management::principal,
    request::Input,
    response::{self, Error, Reply},
};
use crate::foundation::{
    AccessStore, Actor, AdminAppointment, AdminKeyRequest, AdminKeyScope, IssuedAdminKey,
};
use hyper::Method;
use protocol::workspace as api;

pub(super) fn handles(path: &str) -> bool {
    matches!(
        path,
        "/api/v1/admin/team/appoint"
            | "/api/v1/admin/team/revoke"
            | "/api/v1/admin/team/list"
            | "/api/v1/admin/keys/issue"
            | "/api/v1/admin/keys/revoke"
            | "/api/v1/admin/keys/list"
            | "/api/v1/admin/workspaces/transfer-owner"
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
        "/api/v1/admin/team/appoint" => {
            let body: api::AdminAppointment = decode(&input.body)?;
            let request = AdminAppointment {
                principal: principal(body.principal)?,
                key_expires_at: body.key_expires_at,
            };
            issued(
                store
                    .appoint_workspace_admin_with_key(actor, id(&body.workspace_id)?, &request, now)
                    .map_err(operation)?,
            )
        }
        "/api/v1/admin/team/revoke" => {
            let body: api::AdminTarget = decode(&input.body)?;
            store
                .revoke_workspace_admin(
                    actor,
                    id(&body.workspace_id)?,
                    id(&body.principal_id)?,
                    now,
                )
                .map_err(operation)?;
            complete()
        }
        "/api/v1/admin/workspaces/transfer-owner" => {
            let body: api::OwnerTransfer = decode(&input.body)?;
            store
                .transfer_workspace_owner(
                    actor,
                    id(&body.workspace_id)?,
                    id(&body.new_owner_id)?,
                    now,
                )
                .map_err(operation)?;
            complete()
        }
        "/api/v1/admin/keys/issue" => {
            let body: api::WorkspaceKeyRequest = decode(&input.body)?;
            let request = AdminKeyRequest {
                principal_id: id(&body.principal_id)?,
                scope: AdminKeyScope::Workspace(id(&body.workspace_id)?),
                name: body.name,
                expires_at: body.expires_at,
            };
            issued(
                store
                    .issue_admin_key(actor, &request, now)
                    .map_err(operation)?,
            )
        }
        "/api/v1/admin/keys/revoke" => {
            let body: api::WorkspaceKeyTarget = decode(&input.body)?;
            store
                .revoke_admin_key(
                    actor,
                    AdminKeyScope::Workspace(id(&body.workspace_id)?),
                    id(&body.key_id)?,
                    now,
                )
                .map_err(operation)?;
            complete()
        }
        "/api/v1/admin/team/list" | "/api/v1/admin/keys/list" => page(store, actor, input, now),
        _ => Err(Error::NotFound),
    }
}

fn page(
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
    let limit = usize::try_from(limit).map_err(|_limit| Error::BadRequest)?;
    let workspace = id(&body.workspace_id)?;
    let after = body.after.as_deref().map(id).transpose()?;
    if input.path == "/api/v1/admin/team/list" {
        let page = store
            .workspace_admins_page(actor, workspace, after, limit, now)
            .map_err(operation)?;
        return response::success(api::WorkspaceAdmins {
            admins: page
                .admins
                .into_iter()
                .map(|a| api::WorkspaceAdmin {
                    id: a.id.to_string(),
                    name: a.name,
                    enabled: a.enabled,
                    is_owner: a.is_owner,
                })
                .collect(),
            next_cursor: page.next_cursor.map(|id| id.to_string()),
        });
    }
    let page = store
        .workspace_admin_keys_page(actor, workspace, after, limit, now)
        .map_err(operation)?;
    response::success(api::WorkspaceAdminKeys {
        keys: page
            .keys
            .into_iter()
            .map(|k| api::WorkspaceAdminKey {
                id: k.id.to_string(),
                principal_id: k.principal_id.to_string(),
                name: k.name,
                expires_at: k.expires_at,
                revoked_at: k.revoked_at,
                role_active: k.role_active,
            })
            .collect(),
        next_cursor: page.next_cursor.map(|id| id.to_string()),
    })
}

fn issued(key: IssuedAdminKey) -> Result<Reply, Error> {
    response::success(api::AdminKeySecret {
        key_id: key.id.to_string(),
        principal_id: key.principal_id.to_string(),
        secret: key.secret().to_string(),
    })
}
fn complete() -> Result<Reply, Error> {
    response::success(api::OperationComplete { completed: true })
}
fn operation(error: anyhow::Error) -> Error {
    response::operation(error, Error::Forbidden)
}
