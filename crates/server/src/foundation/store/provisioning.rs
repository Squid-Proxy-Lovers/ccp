// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::keys::insert_key;
use super::*;
use crate::foundation::{InitialOwner, IssuedAdminKey, OwnerProvisioning, ProvisionedWorkspace};

impl AccessStore<'_> {
    /// Current instance creation authority may deliver the first owner's key as
    /// part of creation only. It gains no ongoing workspace-key powers.
    pub fn create_workspace_with_owner_key(
        &mut self,
        actor: Actor,
        name: &str,
        request: &OwnerProvisioning,
        now: i64,
    ) -> anyhow::Result<ProvisionedWorkspace> {
        let name = validate_name(name)?;
        validate_request(request, now)?;
        self.mutate(|tx| {
            authorize(tx, actor, Action::CreateWorkspace, None, now)?;
            let owner = resolve_owner(tx, actor, &request.owner, now)?;
            let workspace = Uuid::new_v4();
            tx.execute(
                "INSERT INTO ccp_workspaces(id, name, owner_admin_id) VALUES (?1, ?2, ?3)",
                params![workspace.to_string(), name, owner.to_string()],
            )?;
            audit(tx, actor, "create_workspace", Some(workspace), owner, now)?;
            finish_key(tx, actor, workspace, owner, request.key_expires_at, now)
        })
    }

    /// The owner-NULL transition is the one-time delivery boundary. This cannot
    /// transfer ownership or reissue a key after a committed/lost response.
    pub fn assign_initial_owner_with_key(
        &mut self,
        actor: Actor,
        workspace: Uuid,
        request: &OwnerProvisioning,
        now: i64,
    ) -> anyhow::Result<ProvisionedWorkspace> {
        validate_request(request, now)?;
        self.mutate(|tx| {
            authorize(tx, actor, Action::CreateWorkspace, None, now)?;
            let owner = resolve_owner(tx, actor, &request.owner, now)?;
            require_changed(tx.execute("UPDATE ccp_workspaces SET owner_admin_id = ?1 WHERE id = ?2 AND owner_admin_id IS NULL AND state != 'purged'", params![owner.to_string(), workspace.to_string()])?)?;
            audit(tx, actor, "appoint_initial_owner", Some(workspace), owner, now)?;
            finish_key(tx, actor, workspace, owner, request.key_expires_at, now)
        })
    }
}

fn validate_request(request: &OwnerProvisioning, now: i64) -> anyhow::Result<()> {
    if now < 0 || request.key_expires_at.is_some_and(|expiry| expiry <= now) {
        bail!("initial owner key expiry must be after the current server time");
    }
    if let InitialOwner::NewPrincipal(name) = &request.owner {
        validate_name(name)?;
    }
    Ok(())
}

fn resolve_owner(
    connection: &Connection,
    actor: Actor,
    owner: &InitialOwner,
    now: i64,
) -> anyhow::Result<Uuid> {
    match owner {
        InitialOwner::Existing(id) => {
            require_enabled_principal(connection, *id)?;
            Ok(*id)
        }
        InitialOwner::NewPrincipal(name) => {
            let id = insert_principal(connection, validate_name(name)?)?;
            audit(connection, actor, "create_principal", None, id, now)?;
            Ok(id)
        }
    }
}

fn finish_key(
    tx: &Connection,
    actor: Actor,
    workspace: Uuid,
    owner: Uuid,
    expires_at: Option<i64>,
    now: i64,
) -> anyhow::Result<ProvisionedWorkspace> {
    let key = IssuedAdminKey::generate(owner)?;
    insert_key(
        tx,
        &key,
        Some(workspace),
        "initial workspace owner",
        expires_at,
    )?;
    audit(
        tx,
        actor,
        "issue_initial_owner_key",
        Some(workspace),
        key.id,
        now,
    )?;
    Ok(ProvisionedWorkspace {
        workspace_id: workspace,
        owner_key: key,
    })
}
