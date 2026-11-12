// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use crate::foundation::{
    ContentError, WorkspaceDeletion, WorkspaceDeletionPreview, WorkspaceDeletionRequest,
};
use rusqlite::OptionalExtension;

mod cleanup;

struct Job {
    public: WorkspaceDeletion,
    expected_revision: u64,
    actor: Actor,
    principal: Uuid,
}

impl AccessStore<'_> {
    pub fn workspace_deletion_preview(
        &mut self,
        actor: Actor,
        workspace: Uuid,
        now: i64,
    ) -> anyhow::Result<WorkspaceDeletionPreview> {
        let tx = self.connection.transaction()?;
        content::content_scope(&tx, actor, workspace, Action::PurgeWorkspace, now)?;
        let (name, revision) = tx.query_row("SELECT w.name,l.revision FROM ccp_workspaces w JOIN ccp_workspace_lifecycle l ON l.workspace_id=w.id WHERE w.id=?1",
            [workspace.to_string()], |r| Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?)))?;
        let result = WorkspaceDeletionPreview {
            workspace_id: workspace,
            name,
            lifecycle_revision: u64::try_from(revision)?,
        };
        tx.commit()?;
        Ok(result)
    }

    pub fn begin_workspace_deletion(
        &mut self,
        actor: Actor,
        request: &WorkspaceDeletionRequest,
        now: i64,
    ) -> anyhow::Result<WorkspaceDeletion> {
        if request.operation_id.is_nil()
            || request.confirm_workspace_id != request.workspace_id
            || request.expected_revision == 0
            || request.expected_revision >= u64::try_from(i64::MAX)?
        {
            return Err(ContentError::InvalidInput.into());
        }
        self.mutate(|tx| {
            if let Some(old) = job(tx, request.workspace_id)? {
                readable(tx, actor, &old, now)?;
                if old.public.operation_id != request.operation_id || old.actor != actor
                    || old.expected_revision != request.expected_revision {
                    return Err(ContentError::Conflict.into());
                }
                return Ok(old.public);
            }
            content::content_scope(tx, actor, request.workspace_id, Action::PurgeWorkspace, now)?;
            let revision: i64 = tx.query_row("SELECT revision FROM ccp_workspace_lifecycle WHERE workspace_id=?1", [request.workspace_id.to_string()], |r| r.get(0))?;
            if u64::try_from(revision)? != request.expected_revision { return Err(ContentError::Conflict.into()); }
            let principal = principal(tx, actor)?;
            let (kind, actor_id) = match actor { Actor::Admin(id) => ("admin",id), Actor::AdminKey(id) => ("key",id), Actor::Credential(_) => return Err(ContentError::Forbidden.into()) };
            require_changed(tx.execute("INSERT INTO ccp_workspace_deletions(workspace_id,operation_id,actor_kind,actor_id,principal_id,expected_revision,started_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",
                params![request.workspace_id.to_string(),request.operation_id.to_string(),kind,actor_id.to_string(),principal.to_string(),revision,now]).map_err(content_conflict)?)?;
            require_changed(tx.execute("UPDATE ccp_workspaces SET state='purged',name='',owner_admin_id=NULL WHERE id=?1 AND state='archived'", [request.workspace_id.to_string()])?)?;
            audit(tx, actor, "begin_workspace_deletion", None, request.workspace_id, now)?;
            Ok(job(tx, request.workspace_id)?.context("missing committed workspace deletion")?.public)
        })
    }

    pub fn workspace_deletion_status(
        &mut self,
        actor: Actor,
        workspace: Uuid,
        operation: Uuid,
        now: i64,
    ) -> anyhow::Result<WorkspaceDeletion> {
        let tx = self.connection.transaction()?;
        let Some(value) = job(&tx, workspace)? else {
            if !permitted(&tx, actor, Action::InspectWorkspaceDeletion, None, now)?
                && !permitted(&tx, actor, Action::ReadContext, Some(workspace), now)?
            {
                return Err(ContentError::Forbidden.into());
            }
            return Err(ContentError::NotFound.into());
        };
        readable(&tx, actor, &value, now)?;
        if value.public.operation_id != operation {
            return Err(ContentError::NotFound.into());
        }
        tx.commit()?;
        Ok(value.public)
    }

    /// Trusted scheduling IDs only; cleanup rechecks the immutable confirmed job.
    pub fn pending_workspace_deletions(
        &self,
        after: Option<Uuid>,
        limit: usize,
    ) -> anyhow::Result<Vec<Uuid>> {
        if limit == 0 || limit > 128 {
            return Err(ContentError::InvalidInput.into());
        }
        self.connection.prepare("SELECT workspace_id FROM ccp_workspace_deletions WHERE completed_at IS NULL AND (?1 IS NULL OR workspace_id>?1) ORDER BY workspace_id LIMIT ?2")?
            .query_map(params![after.map(|id|id.to_string()),i64::try_from(limit)?], |r| r.get::<_,String>(0))?
            .map(|r| parse_id(r?)).collect()
    }

    /// Internal continuation of an explicit committed deletion; not new admin authority.
    pub fn advance_workspace_deletion(
        &mut self,
        workspace: Uuid,
        limit: usize,
        now: i64,
    ) -> anyhow::Result<WorkspaceDeletion> {
        if limit == 0 || limit > 128 {
            return Err(ContentError::InvalidInput.into());
        }
        self.mutate(|tx| {
            let value = job(tx, workspace)?.ok_or(ContentError::NotFound)?;
            if now < value.public.started_at { return Err(ContentError::InvalidInput.into()); }
            if value.public.completed_at.is_some() { return Ok(value.public); }
            if cleanup::retire(tx, workspace, limit, now)? || cleanup::payload(tx, workspace, limit)? {
                return Ok(value.public);
            }
            require_changed(tx.execute("UPDATE ccp_workspace_deletions SET completed_at=?1 WHERE workspace_id=?2 AND completed_at IS NULL", params![now,workspace.to_string()])?)?;
            audit(tx, value.actor, "complete_workspace_deletion", None, workspace, now)?;
            Ok(job(tx, workspace)?.context("missing completed workspace deletion")?.public)
        })
    }
}

fn job(db: &Connection, workspace: Uuid) -> anyhow::Result<Option<Job>> {
    let row = db.query_row("SELECT d.operation_id,d.actor_kind,d.actor_id,d.principal_id,d.expected_revision,d.started_at,d.completed_at,w.state,l.revision FROM ccp_workspace_deletions d JOIN ccp_workspaces w ON w.id=d.workspace_id JOIN ccp_workspace_lifecycle l ON l.workspace_id=w.id WHERE d.workspace_id=?1",
        [workspace.to_string()], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,i64>(4)?,r.get::<_,i64>(5)?,r.get::<_,Option<i64>>(6)?,r.get::<_,String>(7)?,r.get::<_,i64>(8)?))).optional()?;
    let Some((operation, kind, id, principal, revision, started, completed, state, current)) = row
    else {
        return Ok(None);
    };
    if state != "purged" || current.checked_sub(1) != Some(revision) {
        bail!("invalid workspace deletion fence");
    }
    let actor = match kind.as_str() {
        "admin" => Actor::Admin(parse_id(id)?),
        "key" => Actor::AdminKey(parse_id(id)?),
        _ => bail!("invalid deletion actor"),
    };
    Ok(Some(Job {
        public: WorkspaceDeletion {
            workspace_id: workspace,
            operation_id: parse_id(operation)?,
            lifecycle_revision: u64::try_from(current)?,
            started_at: started,
            completed_at: completed,
        },
        expected_revision: u64::try_from(revision)?,
        actor,
        principal: parse_id(principal)?,
    }))
}

fn principal(db: &Connection, actor: Actor) -> anyhow::Result<Uuid> {
    match actor {
        Actor::Admin(id) => Ok(id),
        Actor::AdminKey(id) => parse_id(db.query_row(
            "SELECT admin_id FROM ccp_admin_keys WHERE id=?1",
            [id.to_string()],
            |r| r.get(0),
        )?),
        Actor::Credential(_) => Err(ContentError::Forbidden.into()),
    }
}

fn readable(db: &Connection, actor: Actor, value: &Job, now: i64) -> anyhow::Result<()> {
    if now < 0 {
        return Err(ContentError::Forbidden.into());
    }
    if permitted(db, actor, Action::InspectWorkspaceDeletion, None, now)? {
        return Ok(());
    }
    if actor == value.actor {
        let owned = match actor {
            Actor::Admin(id) if id == value.principal => db.query_row(
                "SELECT EXISTS(SELECT 1 FROM ccp_admin_principals WHERE id=?1 AND enabled=1)",
                [id.to_string()],
                |r| r.get::<_, bool>(0),
            )?,
            // The terminal fence already removes the key's workspace role.
            // Never bypass normal key authentication to expose a receipt.
            Actor::AdminKey(_) => false,
            Actor::Admin(_) | Actor::Credential(_) => false,
        };
        if owned {
            return Ok(());
        }
    }
    Err(ContentError::Forbidden.into())
}
