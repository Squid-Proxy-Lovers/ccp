// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

pub(super) fn validate(request: &EntryRetentionChange) -> anyhow::Result<()> {
    if request.operation_id.is_nil()
        || request.expected_revision >= i64::MAX.unsigned_abs()
        // Always reserve a revision for disabling an enabled destructive policy.
        || (request.retention_seconds.is_some()
            && request.expected_revision >= i64::MAX.unsigned_abs() - 1)
        || request.retention_seconds.is_some() != request.confirm_timed_purge
        || request
            .retention_seconds
            .is_some_and(|v| v == 0 || v > i64::MAX.unsigned_abs())
    {
        return Err(ContentError::InvalidInput.into());
    }
    Ok(())
}

pub(super) fn current(db: &Connection, workspace: Uuid) -> anyhow::Result<EntryRetentionPolicy> {
    let saved = db.query_row(
        "SELECT p.revision,p.retention_seconds,p.changed_at FROM ccp_entry_retention_heads h JOIN ccp_entry_retention_versions p ON p.workspace_id=h.workspace_id AND p.revision=h.revision WHERE h.workspace_id=?1",
        [workspace.to_string()],
        |r| Ok((r.get::<_,u64>(0)?,r.get::<_,Option<u64>>(1)?,r.get::<_,i64>(2)?)),
    ).optional()?;
    Ok(match saved {
        Some((revision, retention_seconds, changed_at)) => EntryRetentionPolicy {
            workspace_id: workspace,
            revision,
            retention_seconds,
            changed_at: Some(changed_at),
        },
        None => EntryRetentionPolicy {
            workspace_id: workspace,
            revision: 0,
            retention_seconds: None,
            changed_at: None,
        },
    })
}

fn parts(actor: Actor) -> anyhow::Result<(&'static str, Uuid)> {
    match actor {
        Actor::Admin(id) => Ok(("admin", id)),
        Actor::AdminKey(id) => Ok(("key", id)),
        Actor::Credential(_) => Err(ContentError::Forbidden.into()),
    }
}

pub(super) fn replay(
    db: &Connection,
    actor: Actor,
    workspace: Uuid,
    request: &EntryRetentionChange,
) -> anyhow::Result<Option<EntryRetentionPolicy>> {
    let row = db.query_row("SELECT revision,retention_seconds,changed_at,actor_kind,actor_id FROM ccp_entry_retention_versions WHERE workspace_id=?1 AND operation_id=?2",
        params![workspace.to_string(),request.operation_id.to_string()],
        |r| Ok((r.get::<_,u64>(0)?,r.get::<_,Option<u64>>(1)?,r.get::<_,i64>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?)),
    ).optional()?;
    let Some((revision, seconds, changed_at, kind, id)) = row else {
        return Ok(None);
    };
    let current = parts(actor)?;
    if kind != current.0
        || id != current.1.to_string()
        || revision != request.expected_revision + 1
        || seconds != request.retention_seconds
    {
        return Err(ContentError::Conflict.into());
    }
    Ok(Some(EntryRetentionPolicy {
        workspace_id: workspace,
        revision,
        retention_seconds: seconds,
        changed_at: Some(changed_at),
    }))
}

pub(super) fn record(
    db: &Connection,
    actor: Actor,
    workspace: Uuid,
    request: &EntryRetentionChange,
    now: i64,
) -> anyhow::Result<EntryRetentionPolicy> {
    let (kind, id) = parts(actor)?;
    let principal = match actor {
        Actor::Admin(id) => id,
        Actor::AdminKey(id) => parse_id(db.query_row(
            "SELECT admin_id FROM ccp_admin_keys WHERE id=?1",
            [id.to_string()],
            |r| r.get(0),
        )?)?,
        Actor::Credential(_) => return Err(ContentError::Forbidden.into()),
    };
    let revision = request.expected_revision + 1;
    require_changed(db.execute("INSERT INTO ccp_entry_retention_versions(workspace_id,revision,operation_id,retention_seconds,actor_kind,actor_id,principal_id,changed_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
        params![workspace.to_string(),i64::try_from(revision)?,request.operation_id.to_string(),request.retention_seconds.map(i64::try_from).transpose()?,kind,id.to_string(),principal.to_string(),now])?)?;
    require_changed(db.execute("INSERT INTO ccp_entry_retention_heads(workspace_id,revision) VALUES(?1,?2) ON CONFLICT(workspace_id) DO UPDATE SET revision=excluded.revision WHERE ccp_entry_retention_heads.revision=?3",params![workspace.to_string(),i64::try_from(revision)?,i64::try_from(request.expected_revision)?])?)?;
    Ok(EntryRetentionPolicy {
        workspace_id: workspace,
        revision,
        retention_seconds: request.retention_seconds,
        changed_at: Some(now),
    })
}
