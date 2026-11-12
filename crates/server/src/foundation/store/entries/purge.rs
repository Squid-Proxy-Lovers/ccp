// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use crate::foundation::{EntryPurge, EntryPurgeReceipt};

impl AccessStore<'_> {
    /// Current scoped admins may explicitly purge an individually archived entry
    /// in an active or archived workspace. This removes live database rows only.
    pub fn purge_entry(
        &mut self,
        actor: Actor,
        workspace: Uuid,
        request: &EntryPurge,
        now: i64,
    ) -> anyhow::Result<EntryPurgeReceipt> {
        if request.operation_id.is_nil() || request.entry_id.is_nil() || !request.confirm_purge {
            return Err(ContentError::InvalidInput.into());
        }
        revision_number(request.expected_revision)?;
        self.mutate(|tx| {
            content::content_scope(tx, actor, workspace, Action::PurgeEntry, now)?;
            if let Some(receipt) = replay(tx, actor, workspace, request)? {
                return Ok(receipt);
            }
            let (revision, archived) = tx.query_row(
                "SELECT revision,archived FROM ccp_content_entries WHERE workspace_id=?1 AND id=?2",
                params![workspace.to_string(),request.entry_id.to_string()],
                |r| Ok((r.get::<_,u64>(0)?,r.get::<_,bool>(1)?)),
            ).optional()?.ok_or(ContentError::NotFound)?;
            if !archived || revision != request.expected_revision {
                return Err(ContentError::Conflict.into());
            }
            record(tx, actor, workspace, request, now)?;
            tx.execute("DELETE FROM ccp_content_versions WHERE workspace_id=?1 AND entry_id=?2",
                params![workspace.to_string(),request.entry_id.to_string()])?;
            require_changed(tx.execute("DELETE FROM ccp_content_entries WHERE workspace_id=?1 AND id=?2 AND revision=?3 AND archived=1",
                params![workspace.to_string(),request.entry_id.to_string(),i64::try_from(revision)?])?)?;
            audit(tx,actor,"purge_entry",Some(workspace),request.entry_id,now)?;
            Ok(receipt(request, now))
        })
    }
}

fn receipt(request: &EntryPurge, purged_at: i64) -> EntryPurgeReceipt {
    EntryPurgeReceipt {
        operation_id: request.operation_id,
        entry_id: request.entry_id,
        revision: request.expected_revision,
        purged_at,
    }
}
fn replay(
    db: &Connection,
    actor: Actor,
    workspace: Uuid,
    request: &EntryPurge,
) -> anyhow::Result<Option<EntryPurgeReceipt>> {
    let row=db.query_row("SELECT actor_kind,actor_id,entry_id,revision,purged_at FROM ccp_entry_purges WHERE workspace_id=?1 AND operation_id=?2",params![workspace.to_string(),request.operation_id.to_string()],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,u64>(3)?,r.get::<_,i64>(4)?))).optional()?;
    let Some((kind, id, entry, revision, time)) = row else {
        return Ok(None);
    };
    let current = actor_parts(actor);
    if kind != current.0
        || id != current.1.to_string()
        || entry != request.entry_id.to_string()
        || revision != request.expected_revision
    {
        return Err(ContentError::Conflict.into());
    }
    Ok(Some(receipt(request, time)))
}
fn record(
    db: &Connection,
    actor: Actor,
    workspace: Uuid,
    request: &EntryPurge,
    now: i64,
) -> anyhow::Result<()> {
    let (kind, id) = actor_parts(actor);
    let principal = match actor {
        Actor::Admin(id) => id,
        Actor::AdminKey(key) => parse_id(db.query_row(
            "SELECT admin_id FROM ccp_admin_keys WHERE id=?1",
            [key.to_string()],
            |r| r.get(0),
        )?)?,
        Actor::Credential(_) => return Err(ContentError::Forbidden.into()),
    };
    require_changed(db.execute("INSERT INTO ccp_entry_purges(workspace_id,operation_id,actor_kind,actor_id,principal_id,entry_id,revision,purged_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![workspace.to_string(),request.operation_id.to_string(),kind,id.to_string(),principal.to_string(),request.entry_id.to_string(),i64::try_from(request.expected_revision)?,now])?)
}
