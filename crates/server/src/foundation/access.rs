// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use anyhow::bail;
use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

use super::{Action, Actor, parse_id};

/// Checks current persisted authority, including live credentials/bindings and
/// workspace lifecycle. `now` must come from the trusted server clock.
/// A missing workspace is never interpreted as the instance scope.
/// Call within the same transaction as the protected read or mutation when
/// composing this primitive; it does not retain an authorization lease.
pub fn authorize(
    connection: &Connection,
    actor: Actor,
    action: Action,
    workspace: Option<Uuid>,
    now: i64,
) -> anyhow::Result<()> {
    if !permitted(connection, actor, action, workspace, now)? {
        bail!("action is not authorized for this identity and scope");
    }
    Ok(())
}

pub(super) fn permitted(
    connection: &Connection,
    actor: Actor,
    action: Action,
    workspace: Option<Uuid>,
    now: i64,
) -> anyhow::Result<bool> {
    if now < 0 || action.instance_wide() != workspace.is_none() {
        bail!("invalid authorization target or server time");
    }
    let allowed = match actor {
        Actor::Admin(admin) => admin_allowed(connection, admin, action, workspace)?,
        Actor::Credential(credential) => {
            client_allowed(connection, credential, action, workspace, now)?
        }
    };
    Ok(allowed)
}

fn admin_allowed(
    connection: &Connection,
    admin: Uuid,
    action: Action,
    workspace: Option<Uuid>,
) -> anyhow::Result<bool> {
    let enabled = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM ccp_admin_principals WHERE id = ?1 AND enabled = 1)",
        [admin.to_string()],
        |row| row.get::<_, bool>(0),
    )?;
    if !enabled {
        return Ok(false);
    }
    let global: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM ccp_server_admins WHERE admin_id = ?1)",
        [admin.to_string()],
        |row| row.get(0),
    )?;
    let Some(workspace) = workspace else {
        return Ok(global && action.instance_wide());
    };
    let Some((state, owner)) = workspace_state(connection, workspace)? else {
        return Ok(false);
    };
    if state == "purged" {
        return Ok(false);
    }
    let owner = owner.as_deref() == Some(admin.to_string().as_str());
    let scoped: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM ccp_workspace_admins
         WHERE workspace_id = ?1 AND admin_id = ?2)",
        params![workspace.to_string(), admin.to_string()],
        |row| row.get(0),
    )?;
    Ok(admin_capability(action, &state, global, owner, scoped))
}

fn admin_capability(action: Action, state: &str, global: bool, owner: bool, scoped: bool) -> bool {
    match action {
        Action::ReadContext => global || owner || scoped,
        Action::RevokeGrant | Action::RecoverWorkspaceClient => owner || scoped,
        Action::RevokeAdmin => owner,
        Action::ArchiveWorkspace => state == "active" && (owner || global),
        Action::ReactivateWorkspace | Action::PurgeWorkspace => {
            state == "archived" && (owner || global)
        }
        Action::AppointAdmin | Action::TransferOwner => state == "active" && owner,
        Action::WriteContext
        | Action::SaveWorkspaceBoard
        | Action::AdmitClient
        | Action::ManageArchives => state == "active" && (owner || scoped),
        _ => false,
    }
}

fn workspace_state(
    connection: &Connection,
    workspace: Uuid,
) -> anyhow::Result<Option<(String, Option<String>)>> {
    Ok(connection
        .query_row(
            "SELECT state, owner_admin_id FROM ccp_workspaces WHERE id = ?1",
            [workspace.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?)
}

pub(super) fn authenticated_client(
    connection: &Connection,
    credential: Uuid,
    now: i64,
) -> anyhow::Result<Uuid> {
    let client: Option<String> = connection
        .query_row(
            "SELECT c.client_id FROM ccp_credentials c JOIN ccp_clients p ON p.id = c.client_id
             WHERE c.id = ?1 AND c.revoked_at IS NULL AND c.expires_at > ?2 AND p.disabled = 0",
            params![credential.to_string(), now],
            |row| row.get(0),
        )
        .optional()?;
    match client {
        Some(client) => parse_id(client),
        None => bail!("credential is unknown, expired, revoked, or disabled"),
    }
}

fn client_allowed(
    connection: &Connection,
    credential: Uuid,
    action: Action,
    workspace: Option<Uuid>,
    now: i64,
) -> anyhow::Result<bool> {
    let Some(workspace) = workspace else {
        return Ok(false);
    };
    // A failed identity lookup is a denied action, not a way to infer grants.
    let live = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM ccp_credentials c JOIN ccp_clients p ON p.id = c.client_id
         WHERE c.id = ?1 AND c.revoked_at IS NULL AND c.expires_at > ?2 AND p.disabled = 0)",
        params![credential.to_string(), now],
        |row| row.get::<_, bool>(0),
    )?;
    if !live {
        return Ok(false);
    }
    let grant: Option<(String, String)> = connection
        .query_row(
            "SELECT g.role, w.state FROM ccp_credential_grants b
             JOIN ccp_grants g ON g.id = b.grant_id AND g.client_id = b.client_id
             JOIN ccp_workspaces w ON w.id = g.workspace_id
             WHERE b.credential_id = ?1 AND g.workspace_id = ?2 AND g.revoked_at IS NULL",
            params![credential.to_string(), workspace.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    Ok(match grant {
        Some((role, state)) if state != "purged" => match action {
            Action::ReadContext => true,
            Action::WriteContext => state == "active" && role == "read_write",
            _ => false,
        },
        _ => false,
    })
}
