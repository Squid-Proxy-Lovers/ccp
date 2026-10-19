// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use anyhow::{Context, bail};
use rusqlite::{Connection, Transaction, TransactionBehavior, params};
use uuid::Uuid;

mod invitations;
mod keys;
mod redemption;
mod renewal;

use super::access::{authenticated_client, permitted};
use super::{
    Action, Actor, Admission, ClientRegistration, DataRole, Enrollment, WorkspaceSummary,
    authorize, parse_id, validate_name,
};

/// A transaction boundary for the new persistent access records. This is an
/// internal server API, not proof that a supplied actor/credential was presented
/// by a remote peer. Transport authentication is integrated in phase 2.
pub struct AccessStore<'a> {
    connection: &'a mut Connection,
}

impl<'a> AccessStore<'a> {
    pub fn new(connection: &'a mut Connection) -> anyhow::Result<Self> {
        connection.pragma_update(None, "foreign_keys", true)?;
        let enforced: bool =
            connection.pragma_query_value(None, "foreign_keys", |row| row.get(0))?;
        if !enforced {
            bail!("foundation operations require enforced foreign keys");
        }
        Ok(Self { connection })
    }

    pub fn instance_id(&self) -> anyhow::Result<Uuid> {
        parse_id(self.connection.query_row(
            "SELECT instance_id FROM ccp_instance WHERE singleton = 1",
            [],
            |row| row.get(0),
        )?)
    }

    fn mutate<T>(
        &mut self,
        operation: impl FnOnce(&Transaction<'_>) -> anyhow::Result<T>,
    ) -> anyhow::Result<T> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let result = operation(&transaction)?;
        transaction
            .commit()
            .context("failed to commit access mutation")?;
        Ok(result)
    }

    /// Initial local-operator provisioning only. A future CLI must enforce OS /
    /// data-directory access; this method must never be a first-web-visitor API.
    /// Existing global-admin records, even disabled ones, prevent re-bootstrap.
    pub fn bootstrap_admin(&mut self, name: &str, now: i64) -> anyhow::Result<Uuid> {
        let name = validate_name(name)?;
        self.mutate(|transaction| bootstrap_principal(transaction, name, now))
    }

    /// Creates an unprivileged principal for initial-owner appointment. Merely
    /// naming a principal never grants a role or matches an existing person.
    pub fn create_principal(&mut self, actor: Actor, name: &str, now: i64) -> anyhow::Result<Uuid> {
        let name = validate_name(name)?;
        self.mutate(|transaction| {
            authorize(transaction, actor, Action::CreateWorkspace, None, now)?;
            let id = insert_principal(transaction, name)?;
            audit(transaction, actor, "create_principal", None, id, now)?;
            Ok(id)
        })
    }

    pub fn create_workspace(
        &mut self,
        actor: Actor,
        name: &str,
        owner: Uuid,
        now: i64,
    ) -> anyhow::Result<Uuid> {
        let name = validate_name(name)?;
        self.mutate(|transaction| {
            authorize(transaction, actor, Action::CreateWorkspace, None, now)?;
            require_enabled_principal(transaction, owner)?;
            let id = Uuid::new_v4();
            transaction.execute(
                "INSERT INTO ccp_workspaces(id, name, owner_admin_id) VALUES (?1, ?2, ?3)",
                params![id.to_string(), name, owner.to_string()],
            )?;
            audit(transaction, actor, "create_workspace", Some(id), owner, now)?;
            Ok(id)
        })
    }

    /// Assigns only a missing first owner. It cannot transfer an owned workspace
    /// or promote the legacy sessions.owner display string into authority.
    pub fn assign_initial_owner(
        &mut self,
        actor: Actor,
        workspace: Uuid,
        owner: Uuid,
        now: i64,
    ) -> anyhow::Result<()> {
        self.mutate(|transaction| {
            authorize(transaction, actor, Action::CreateWorkspace, None, now)?;
            require_enabled_principal(transaction, owner)?;
            let changed = transaction.execute(
                "UPDATE ccp_workspaces SET owner_admin_id = ?1
                 WHERE id = ?2 AND owner_admin_id IS NULL AND state != 'purged'",
                params![owner.to_string(), workspace.to_string()],
            )?;
            require_changed(changed)?;
            audit(
                transaction,
                actor,
                "appoint_initial_owner",
                Some(workspace),
                owner,
                now,
            )
        })
    }

    pub fn appoint_workspace_admin(
        &mut self,
        actor: Actor,
        workspace: Uuid,
        name: &str,
        now: i64,
    ) -> anyhow::Result<Uuid> {
        let name = validate_name(name)?;
        self.mutate(|transaction| {
            authorize(
                transaction,
                actor,
                Action::AppointAdmin,
                Some(workspace),
                now,
            )?;
            let id = insert_principal(transaction, name)?;
            transaction.execute(
                "INSERT INTO ccp_workspace_admins(workspace_id, admin_id) VALUES (?1, ?2)",
                params![workspace.to_string(), id.to_string()],
            )?;
            audit(
                transaction,
                actor,
                "appoint_admin",
                Some(workspace),
                id,
                now,
            )?;
            Ok(id)
        })
    }

    pub fn revoke_workspace_admin(
        &mut self,
        actor: Actor,
        workspace: Uuid,
        admin: Uuid,
        now: i64,
    ) -> anyhow::Result<()> {
        self.mutate(|transaction| {
            authorize(
                transaction,
                actor,
                Action::RevokeAdmin,
                Some(workspace),
                now,
            )?;
            require_changed(transaction.execute(
                "DELETE FROM ccp_workspace_admins WHERE workspace_id = ?1 AND admin_id = ?2",
                params![workspace.to_string(), admin.to_string()],
            )?)?;
            audit(
                transaction,
                actor,
                "revoke_admin",
                Some(workspace),
                admin,
                now,
            )
        })
    }

    pub fn register_client(
        &mut self,
        actor: Actor,
        request: &ClientRegistration,
        now: i64,
    ) -> anyhow::Result<Enrollment> {
        if request.expires_at <= now || now < 0 {
            bail!("new credential must expire after the current server time");
        }
        self.mutate(|transaction| {
            authorize(
                transaction,
                actor,
                Action::AdmitClient,
                Some(request.workspace_id),
                now,
            )?;
            let client_id = Uuid::new_v4();
            let credential_id = Uuid::new_v4();
            transaction.execute(
                "INSERT INTO ccp_clients(id, name) VALUES (?1, ?2)",
                params![
                    client_id.to_string(),
                    format!("client-{}", client_id.simple())
                ],
            )?;
            transaction.execute(
                "INSERT INTO ccp_credentials(id, client_id, expires_at) VALUES (?1, ?2, ?3)",
                params![
                    credential_id.to_string(),
                    client_id.to_string(),
                    request.expires_at
                ],
            )?;
            let grant_id = grant_and_bind(
                transaction,
                request.workspace_id,
                client_id,
                credential_id,
                request.role,
            )?;
            audit(
                transaction,
                actor,
                "register_client",
                Some(request.workspace_id),
                client_id,
                now,
            )?;
            Ok(Enrollment {
                client_id,
                credential_id,
                grant_id,
            })
        })
    }

    /// The credential must already have been authenticated by the transport.
    /// This transaction also rechecks its expiry/revocation/disabled state.
    pub fn admit_existing_client(
        &mut self,
        actor: Actor,
        request: Admission,
        now: i64,
    ) -> anyhow::Result<Uuid> {
        self.mutate(|transaction| {
            authorize(
                transaction,
                actor,
                Action::AdmitClient,
                Some(request.workspace_id),
                now,
            )?;
            let client = authenticated_client(transaction, request.presented_credential, now)?;
            let grant = grant_and_bind(
                transaction,
                request.workspace_id,
                client,
                request.presented_credential,
                request.role,
            )?;
            audit(
                transaction,
                actor,
                "admit_client",
                Some(request.workspace_id),
                grant,
                now,
            )?;
            Ok(grant)
        })
    }

    pub fn revoke_grant(&mut self, actor: Actor, grant: Uuid, now: i64) -> anyhow::Result<()> {
        self.mutate(|transaction| {
            let workspace = parse_id(transaction.query_row(
                "SELECT workspace_id FROM ccp_grants WHERE id = ?1",
                [grant.to_string()],
                |row| row.get(0),
            )?)?;
            authorize(
                transaction,
                actor,
                Action::RevokeGrant,
                Some(workspace),
                now,
            )?;
            require_changed(transaction.execute(
                "UPDATE ccp_grants SET revoked_at = ?1 WHERE id = ?2 AND revoked_at IS NULL",
                params![now, grant.to_string()],
            )?)?;
            audit(
                transaction,
                actor,
                "revoke_grant",
                Some(workspace),
                grant,
                now,
            )
        })
    }

    pub fn disable_client(&mut self, actor: Actor, client: Uuid, now: i64) -> anyhow::Result<()> {
        self.mutate(|transaction| {
            authorize(transaction, actor, Action::DisableClient, None, now)?;
            require_changed(transaction.execute(
                "UPDATE ccp_clients SET disabled = 1 WHERE id = ?1",
                [client.to_string()],
            )?)?;
            audit(transaction, actor, "disable_client", None, client, now)
        })
    }

    pub fn set_archived(
        &mut self,
        actor: Actor,
        workspace: Uuid,
        archived: bool,
        now: i64,
    ) -> anyhow::Result<()> {
        self.mutate(|transaction| {
            let (action, state) = if archived {
                (Action::ArchiveWorkspace, "archived")
            } else {
                (Action::ReactivateWorkspace, "active")
            };
            authorize(transaction, actor, action, Some(workspace), now)?;
            transaction.execute(
                "UPDATE ccp_workspaces SET state = ?1 WHERE id = ?2",
                params![state, workspace.to_string()],
            )?;
            audit(transaction, actor, state, Some(workspace), workspace, now)
        })
    }

    /// Discovery follows current effective access, not caller selection or only
    /// identity-wide grants. SQL/storage failures propagate instead of hiding.
    pub fn visible_workspaces(
        &mut self,
        actor: Actor,
        now: i64,
    ) -> anyhow::Result<Vec<WorkspaceSummary>> {
        let transaction = self.connection.transaction()?;
        validate_actor(&transaction, actor, now)?;
        let workspaces = transaction
            .prepare("SELECT id, name, state FROM ccp_workspaces ORDER BY name")?
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut visible = Vec::new();
        for (id, name, state) in workspaces {
            let id = parse_id(id)?;
            if permitted(&transaction, actor, Action::ReadContext, Some(id), now)? {
                visible.push(WorkspaceSummary {
                    id,
                    name,
                    archived: state == "archived",
                });
            }
        }
        transaction.commit()?;
        Ok(visible)
    }
}

fn validate_actor(connection: &Connection, actor: Actor, now: i64) -> anyhow::Result<()> {
    if now < 0 {
        bail!("invalid server time");
    }
    match actor {
        Actor::Credential(id) => {
            authenticated_client(connection, id, now)?;
        }
        Actor::Admin(id) => require_enabled_principal(connection, id)?,
        Actor::AdminKey(id) => {
            if super::admin::live_admin_key(connection, id, now)?.is_none() {
                bail!("admin key is not valid");
            }
        }
    }
    Ok(())
}

fn require_enabled_principal(connection: &Connection, id: Uuid) -> anyhow::Result<()> {
    let enabled: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM ccp_admin_principals WHERE id = ?1 AND enabled = 1)",
        [id.to_string()],
        |row| row.get(0),
    )?;
    if !enabled {
        bail!("administrator principal is unknown or disabled");
    }
    Ok(())
}

fn insert_principal(connection: &Connection, name: &str) -> anyhow::Result<Uuid> {
    let id = Uuid::new_v4();
    connection.execute(
        "INSERT INTO ccp_admin_principals(id, name) VALUES (?1, ?2)",
        params![id.to_string(), name],
    )?;
    Ok(id)
}

fn grant_and_bind(
    connection: &Connection,
    workspace: Uuid,
    client: Uuid,
    credential: Uuid,
    role: DataRole,
) -> anyhow::Result<Uuid> {
    let grant = Uuid::new_v4();
    connection.execute(
        "INSERT INTO ccp_grants(id, workspace_id, client_id, role) VALUES (?1, ?2, ?3, ?4)",
        params![
            grant.to_string(),
            workspace.to_string(),
            client.to_string(),
            role.as_str()
        ],
    )?;
    connection.execute(
        "INSERT INTO ccp_credential_grants(credential_id, grant_id, client_id) VALUES (?1, ?2, ?3)",
        params![
            credential.to_string(),
            grant.to_string(),
            client.to_string()
        ],
    )?;
    Ok(grant)
}

fn require_changed(changed: usize) -> anyhow::Result<()> {
    if changed != 1 {
        bail!("target does not exist or state changed");
    }
    Ok(())
}

fn audit(
    connection: &Connection,
    actor: Actor,
    action: &str,
    workspace: Option<Uuid>,
    target: Uuid,
    now: i64,
) -> anyhow::Result<()> {
    let (kind, id, key) = match actor {
        Actor::Admin(id) => ("admin", id, None),
        Actor::Credential(id) => ("credential", id, None),
        Actor::AdminKey(key) => {
            let principal = parse_id(connection.query_row(
                "SELECT admin_id FROM ccp_admin_keys WHERE id = ?1",
                [key.to_string()],
                |row| row.get(0),
            )?)?;
            ("admin", principal, Some(key))
        }
    };
    connection.execute(
        "INSERT INTO ccp_access_audit
         (actor_kind, actor_id, action, workspace_id, target_id, recorded_at, admin_key_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            kind,
            id.to_string(),
            action,
            workspace.map(|id| id.to_string()),
            target.to_string(),
            now,
            key.map(|id| id.to_string())
        ],
    )?;
    Ok(())
}

fn bootstrap_principal(connection: &Connection, name: &str, now: i64) -> anyhow::Result<Uuid> {
    let count: u64 = connection.query_row("SELECT COUNT(*) FROM ccp_server_admins", [], |row| {
        row.get(0)
    })?;
    if count != 0 {
        bail!("instance already has administrative bootstrap records");
    }
    let id = insert_principal(connection, name)?;
    connection.execute(
        "INSERT INTO ccp_server_admins(admin_id) VALUES (?1)",
        [id.to_string()],
    )?;
    audit_local_operator(connection, "bootstrap_admin", id, now)?;
    Ok(id)
}

fn audit_local_operator(
    connection: &Connection,
    action: &str,
    target: Uuid,
    now: i64,
) -> anyhow::Result<()> {
    connection.execute(
        "INSERT INTO ccp_access_audit(actor_kind, actor_id, action, target_id, recorded_at)
         VALUES ('local_operator', 'local', ?1, ?2, ?3)",
        params![action, target.to_string(), now],
    )?;
    Ok(())
}
