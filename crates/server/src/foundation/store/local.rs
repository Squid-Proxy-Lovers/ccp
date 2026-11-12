// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use rusqlite::OptionalExtension;

use super::keys::insert_key;
use super::*;
use crate::foundation::admin::has_key_role;
use crate::foundation::{
    AdminKeyScope, BootstrappedInstance, IssuedAdminKey, LocalKeyReset, LocalKeyTarget,
};

impl AccessStore<'_> {
    /// Trusted host operator or verified one-use startup capability only.
    /// Local CLI wrappers enforce maintenance access. The setup route must hold
    /// the runtime directory lease and validate the host-issued capability first;
    /// a remote visitor without that proof must never reach this operation.
    pub fn bootstrap_instance(
        &mut self,
        operator_name: &str,
        workspace_name: &str,
        now: i64,
    ) -> anyhow::Result<BootstrappedInstance> {
        let operator_name = validate_name(operator_name)?;
        let workspace_name = validate_name(workspace_name)?;
        if now < 0 {
            bail!("invalid server time");
        }
        self.mutate(|tx| {
            let claimed: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM ccp_setup_reservation WHERE singleton=1 AND claimed_at IS NOT NULL)", [], |row| row.get(0))?;
            if claimed { bail!("setup was already claimed; use saved keys or restore a matching backup"); }
            let principal = bootstrap_principal(tx, operator_name, now)?;
            let workspace = Uuid::new_v4();
            tx.execute(
                "INSERT INTO ccp_workspaces(id, name, owner_admin_id) VALUES (?1, ?2, ?3)",
                params![workspace.to_string(), workspace_name, principal.to_string()],
            )?;
            audit_operator_scope(tx, "create_workspace", Some(workspace), workspace, now)?;
            let instance_key = IssuedAdminKey::generate(principal)?;
            let owner_key = IssuedAdminKey::generate(principal)?;
            insert_key(tx, &instance_key, None, "initial instance admin", None)?;
            insert_key(
                tx,
                &owner_key,
                Some(workspace),
                "initial workspace owner",
                None,
            )?;
            audit_operator_scope(tx, "issue_admin_key", None, instance_key.id, now)?;
            audit_operator_scope(tx, "issue_admin_key", Some(workspace), owner_key.id, now)?;
            let instance = parse_id(tx.query_row(
                "SELECT instance_id FROM ccp_instance WHERE singleton = 1",
                [],
                |row| row.get(0),
            )?)?;
            if tx.execute("UPDATE ccp_setup_reservation SET claimed_at=?1 WHERE singleton=1 AND claimed_at IS NULL", [now])? > 0 {
                audit_operator_scope(tx, "complete_setup", None, instance, now)?;
            }
            Ok(BootstrappedInstance {
                instance_id: instance,
                workspace_id: workspace,
                instance_key,
                owner_key,
            })
        })
    }

    /// Physical/OS operator recovery, not a global-key permission. Replaces only
    /// the selected existing principal's keys in this exact scope. It never
    /// changes ownership, enables a principal or adds a global/admin role.
    pub fn reset_local_admin_key(
        &mut self,
        target: LocalKeyTarget,
        expires_at: Option<i64>,
        now: i64,
    ) -> anyhow::Result<LocalKeyReset> {
        if now < 0 || expires_at.is_some_and(|expiry| expiry <= now) {
            bail!("replacement key expiry must be after the current server time");
        }
        self.mutate(|tx| {
            let (principal, scope) = reset_target(tx, target)?;
            let workspace = scope.workspace();
            if !has_key_role(tx, principal, workspace)? {
                bail!("replacement requires an enabled principal with its current role");
            }
            let keys = tx.prepare("SELECT id FROM ccp_admin_keys WHERE admin_id = ?1 AND workspace_id IS ?2 AND revoked_at IS NULL ORDER BY id")?
                .query_map(params![principal.to_string(), workspace.map(|id| id.to_string())], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?.into_iter().map(parse_id).collect::<anyhow::Result<Vec<_>>>()?;
            let changed = tx.execute("UPDATE ccp_admin_keys SET revoked_at = ?1 WHERE admin_id = ?2 AND workspace_id IS ?3 AND revoked_at IS NULL",
                params![now, principal.to_string(), workspace.map(|id| id.to_string())])?;
            if changed != keys.len() { bail!("key state changed during local replacement"); }
            for key in &keys { audit_operator_scope(tx, "revoke_admin_key", workspace, *key, now)?; }
            let key = IssuedAdminKey::generate(principal)?;
            insert_key(tx, &key, workspace, "local replacement", expires_at)?;
            audit_operator_scope(tx, "issue_admin_key", workspace, key.id, now)?;
            audit_operator_scope(tx, "reset_admin_key", workspace, principal, now)?;
            Ok(LocalKeyReset { key, scope, expires_at, revoked_keys: keys.len() })
        })
    }
}

fn reset_target(tx: &Connection, target: LocalKeyTarget) -> anyhow::Result<(Uuid, AdminKeyScope)> {
    match target {
        LocalKeyTarget::InstancePrincipal(principal) => Ok((principal, AdminKeyScope::Instance)),
        LocalKeyTarget::WorkspaceOwner(workspace) => {
            let principal: Option<String> = tx
                .query_row(
                    "SELECT owner_admin_id FROM ccp_workspaces WHERE id = ?1 AND state IN ('active', 'archived')",
                    [workspace.to_string()],
                    |row| row.get(0),
                )
                .optional()?
                .flatten();
            let principal =
                parse_id(principal.context("workspace has no recoverable current owner")?)?;
            Ok((principal, AdminKeyScope::Workspace(workspace)))
        }
    }
}

fn audit_operator_scope(
    tx: &Connection,
    action: &str,
    workspace: Option<Uuid>,
    target: Uuid,
    now: i64,
) -> anyhow::Result<()> {
    tx.execute("INSERT INTO ccp_access_audit(actor_kind, actor_id, action, workspace_id, target_id, recorded_at)
        VALUES ('local_operator', 'local', ?1, ?2, ?3, ?4)",
        params![action, workspace.map(|id| id.to_string()), target.to_string(), now])?;
    Ok(())
}
