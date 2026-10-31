// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::keys::insert_key;
use super::*;
use crate::foundation::{AdminAppointment, InitialOwner, IssuedAdminKey};

mod pages;

impl AccessStore<'_> {
    /// Appointment and first key are one operation; labels never claim identity.
    /// Historical keys cannot regain authority when an old principal returns.
    pub fn appoint_workspace_admin_with_key(
        &mut self,
        actor: Actor,
        workspace: Uuid,
        request: &AdminAppointment,
        now: i64,
    ) -> anyhow::Result<IssuedAdminKey> {
        if now < 0 || request.key_expires_at.is_some_and(|expiry| expiry <= now) {
            bail!("admin key expiry must be after the current server time");
        }
        if let InitialOwner::NewPrincipal(name) = &request.principal {
            validate_name(name)?;
        }
        self.mutate(|tx| {
            authorize(tx,actor,Action::AppointAdmin,Some(workspace),now)?;
            let principal=resolve_principal(tx,actor,&request.principal,now)?;
            let present:bool=tx.query_row(
                "SELECT owner_admin_id=?2 OR EXISTS(SELECT 1 FROM ccp_workspace_admins WHERE workspace_id=?1 AND admin_id=?2)
                 FROM ccp_workspaces WHERE id=?1",
                params![workspace.to_string(),principal.to_string()],|r|r.get(0),
            )?;
            if present {bail!("principal already administers this workspace");}
            revoke_principal_keys(tx,actor,workspace,principal,now)?;
            tx.execute("INSERT INTO ccp_workspace_admins(workspace_id,admin_id) VALUES (?1,?2)",
                params![workspace.to_string(),principal.to_string()])?;
            let key=IssuedAdminKey::generate(principal)?;
            insert_key(tx,&key,Some(workspace),"initial workspace admin",request.key_expires_at)?;
            audit(tx,actor,"appoint_admin",Some(workspace),principal,now)?;
            audit(tx,actor,"issue_admin_key",Some(workspace),key.id,now)?;
            Ok(key)
        })
    }

    /// Transfer only to an already provisioned, enabled scoped administrator.
    /// The former owner's scoped keys and redundant role are terminally removed.
    pub fn transfer_workspace_owner(
        &mut self,
        actor: Actor,
        workspace: Uuid,
        new_owner: Uuid,
        now: i64,
    ) -> anyhow::Result<()> {
        self.mutate(|tx| {
            authorize(tx,actor,Action::TransferOwner,Some(workspace),now)?;
            let owner=parse_id(tx.query_row("SELECT owner_admin_id FROM ccp_workspaces WHERE id=?1",
                [workspace.to_string()],|r|r.get(0))?)?;
            if owner==new_owner {bail!("recipient is already the owner");}
            require_enabled_principal(tx,new_owner)?;
            let ready:bool=tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM ccp_workspace_admins a JOIN ccp_admin_keys k ON k.admin_id=a.admin_id
                 AND k.workspace_id=a.workspace_id WHERE a.workspace_id=?1 AND a.admin_id=?2
                 AND k.revoked_at IS NULL AND (k.expires_at IS NULL OR k.expires_at>?3))",
                params![workspace.to_string(),new_owner.to_string(),now],|r|r.get(0),
            )?;
            if !ready {bail!("recipient needs a current workspace admin role and live scoped key");}
            require_changed(tx.execute("UPDATE ccp_workspaces SET owner_admin_id=?1 WHERE id=?2 AND owner_admin_id=?3 AND state='active'",
                params![new_owner.to_string(),workspace.to_string(),owner.to_string()])?)?;
            tx.execute("DELETE FROM ccp_workspace_admins WHERE workspace_id=?1 AND admin_id IN (?2,?3)",
                params![workspace.to_string(),owner.to_string(),new_owner.to_string()])?;
            revoke_principal_keys(tx,actor,workspace,owner,now)?;
            audit(tx,actor,"transfer_owner",Some(workspace),new_owner,now)
        })
    }
}

fn resolve_principal(
    tx: &Connection,
    actor: Actor,
    selection: &InitialOwner,
    now: i64,
) -> anyhow::Result<Uuid> {
    match selection {
        InitialOwner::Existing(id) => {
            require_enabled_principal(tx, *id)?;
            Ok(*id)
        }
        InitialOwner::NewPrincipal(name) => {
            let id = insert_principal(tx, validate_name(name)?)?;
            audit(tx, actor, "create_principal", None, id, now)?;
            Ok(id)
        }
    }
}

pub(super) fn revoke_principal_keys(
    tx: &Connection,
    actor: Actor,
    workspace: Uuid,
    principal: Uuid,
    now: i64,
) -> anyhow::Result<()> {
    let changed=tx.execute("UPDATE ccp_admin_keys SET revoked_at=?1 WHERE workspace_id=?2 AND admin_id=?3 AND revoked_at IS NULL",
        params![now,workspace.to_string(),principal.to_string()])?;
    if changed > 0 {
        audit(
            tx,
            actor,
            "revoke_workspace_principal_keys",
            Some(workspace),
            principal,
            now,
        )?;
    }
    Ok(())
}
