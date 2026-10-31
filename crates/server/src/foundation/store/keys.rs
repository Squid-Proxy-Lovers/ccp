// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use rusqlite::OptionalExtension;

use super::*;
use crate::foundation::admin::{has_key_role, key_hash, live_admin_key, validate_secret};
use crate::foundation::{AdminKeyIdentity, AdminKeyRequest, AdminKeyScope, IssuedAdminKey};

impl AccessStore<'_> {
    /// Only a server-issued session may use this trusted key-ID lookup.
    pub(crate) fn browser_admin_key(
        &mut self,
        key: Uuid,
        now: i64,
    ) -> anyhow::Result<Option<AdminKeyIdentity>> {
        let tx = self.connection.transaction()?;
        let Some((principal, workspace)) = live_admin_key(&tx, key, now)? else {
            tx.commit()?;
            return Ok(None);
        };
        let (principal_name,key_name) = tx.query_row("SELECT p.name,k.name FROM ccp_admin_keys k JOIN ccp_admin_principals p ON p.id=k.admin_id WHERE k.id=?1",[key.to_string()],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?;
        let is_owner = if let Some(workspace) = workspace {
            tx.query_row(
                "SELECT owner_admin_id IS ?2 FROM ccp_workspaces WHERE id=?1",
                params![workspace.to_string(), principal.to_string()],
                |r| r.get(0),
            )?
        } else {
            false
        };
        let result = AdminKeyIdentity {
            key_id: key,
            principal_id: principal,
            principal_name,
            key_name,
            scope: workspace.map_or(AdminKeyScope::Instance, AdminKeyScope::Workspace),
            is_owner,
        };
        tx.commit()?;
        Ok(Some(result))
    }
    /// Local operator only, with the same OS boundary as bootstrap_admin. The
    /// principal, initial key verifier and audit records commit together.
    pub fn bootstrap_admin_key(&mut self, name: &str, now: i64) -> anyhow::Result<IssuedAdminKey> {
        let name = validate_name(name)?;
        self.mutate(|transaction| {
            let principal = bootstrap_principal(transaction, name, now)?;
            let issued = IssuedAdminKey::generate(principal)?;
            insert_key(transaction, &issued, None, "initial operator", None)?;
            audit_local_operator(transaction, "issue_admin_key", issued.id, now)?;
            Ok(issued)
        })
    }

    pub fn issue_admin_key(
        &mut self,
        actor: Actor,
        request: &AdminKeyRequest,
        now: i64,
    ) -> anyhow::Result<IssuedAdminKey> {
        let name = validate_name(&request.name)?;
        if now < 0 || request.expires_at.is_some_and(|expiry| expiry <= now) {
            bail!("admin key expiry must be after the current server time");
        }
        self.mutate(|transaction| {
            let workspace = request.scope.workspace();
            authorize(
                transaction,
                actor,
                key_action(request.scope),
                workspace,
                now,
            )?;
            if !has_key_role(transaction, request.principal_id, workspace)? {
                bail!("key recipient has no current administrative role in this scope");
            }
            let issued = IssuedAdminKey::generate(request.principal_id)?;
            insert_key(transaction, &issued, workspace, name, request.expires_at)?;
            audit(
                transaction,
                actor,
                "issue_admin_key",
                workspace,
                issued.id,
                now,
            )?;
            Ok(issued)
        })
    }

    /// A successful secret check does not lease authority. Every protected
    /// operation must retain this AdminKey actor so current expiry/revocation,
    /// principal role and key scope are checked in that operation's transaction.
    pub fn authenticate_admin_key(&mut self, secret: &str, now: i64) -> anyhow::Result<Actor> {
        validate_secret(secret)?;
        let transaction = self.connection.transaction()?;
        let id: Option<String> = transaction
            .query_row(
                "SELECT id FROM ccp_admin_keys WHERE token_hash = ?1",
                [key_hash(secret)],
                |row| row.get(0),
            )
            .optional()?;
        let key = id.map(parse_id).transpose()?;
        let Some(key) = key else {
            bail!("admin key is not valid");
        };
        if live_admin_key(&transaction, key, now)?.is_none() {
            bail!("admin key is not valid");
        }
        transaction.commit()?;
        Ok(Actor::AdminKey(key))
    }

    /// Scope is explicit, so a foreign or unknown key ID has the same result
    /// after the caller is authorized for the requested scope.
    pub fn revoke_admin_key(
        &mut self,
        actor: Actor,
        scope: AdminKeyScope,
        key: Uuid,
        now: i64,
    ) -> anyhow::Result<()> {
        self.mutate(|transaction| {
            let workspace = scope.workspace();
            authorize(transaction, actor, key_action(scope), workspace, now)?;
            require_changed(transaction.execute(
                "UPDATE ccp_admin_keys SET revoked_at = ?1 WHERE id = ?2
                 AND workspace_id IS ?3 AND revoked_at IS NULL",
                params![now, key.to_string(), workspace.map(|id| id.to_string())],
            )?)?;
            audit(transaction, actor, "revoke_admin_key", workspace, key, now)
        })
    }
}

fn key_action(scope: AdminKeyScope) -> Action {
    match scope {
        AdminKeyScope::Instance => Action::ManageInstanceKeys,
        AdminKeyScope::Workspace(_) => Action::ManageWorkspaceKeys,
    }
}

pub(super) fn insert_key(
    connection: &Connection,
    issued: &IssuedAdminKey,
    workspace: Option<Uuid>,
    name: &str,
    expires_at: Option<i64>,
) -> anyhow::Result<()> {
    connection.execute(
        "INSERT INTO ccp_admin_keys(id, admin_id, workspace_id, name, token_hash, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            issued.id.to_string(),
            issued.principal_id.to_string(),
            workspace.map(|id| id.to_string()),
            name,
            key_hash(issued.secret()),
            expires_at
        ],
    )?;
    Ok(())
}
