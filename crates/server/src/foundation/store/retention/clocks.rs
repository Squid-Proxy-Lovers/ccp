// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

pub(super) fn configure(
    db: &Connection,
    policy: &EntryRetentionPolicy,
    now: i64,
) -> anyhow::Result<()> {
    let workspace = policy.workspace_id.to_string();
    if let Some(seconds) = policy.retention_seconds {
        let expected = db.query_row(
            "SELECT count(*) FROM ccp_content_entries WHERE workspace_id=?1 AND archived=1",
            [&workspace],
            |r| r.get::<_, usize>(0),
        )?;
        exact_count(db.execute("INSERT INTO ccp_entry_retention_clocks(entry_id,workspace_id,archive_revision,policy_revision,remaining_seconds,active_since) SELECT id,workspace_id,revision,?2,?3,?4 FROM ccp_content_entries WHERE workspace_id=?1 AND archived=1",params![workspace,i64::try_from(policy.revision)?,i64::try_from(seconds)?,now])?,expected)?;
    } else {
        let expected = count(db, policy.workspace_id)?;
        exact_count(
            db.execute(
                "DELETE FROM ccp_entry_retention_clocks WHERE workspace_id=?1",
                [workspace],
            )?,
            expected,
        )?;
    }
    Ok(())
}

fn count(db: &Connection, workspace: Uuid) -> anyhow::Result<usize> {
    Ok(db.query_row(
        "SELECT count(*) FROM ccp_entry_retention_clocks WHERE workspace_id=?1",
        [workspace.to_string()],
        |r| r.get(0),
    )?)
}

pub(in crate::foundation::store) fn entry_changed(
    db: &Connection,
    workspace: Uuid,
    change: &EntryChange,
    now: i64,
) -> anyhow::Result<()> {
    match change {
        EntryChange::Archive { id, .. } => {
            let policy = policy::current(db, workspace)?;
            if let Some(seconds) = policy.retention_seconds {
                require_changed(db.execute("INSERT INTO ccp_entry_retention_clocks(entry_id,workspace_id,archive_revision,policy_revision,remaining_seconds,active_since) SELECT id,workspace_id,revision,?3,?4,?5 FROM ccp_content_entries WHERE workspace_id=?1 AND id=?2 AND archived=1",params![workspace.to_string(),id.to_string(),i64::try_from(policy.revision)?,i64::try_from(seconds)?,now])?)?;
            }
        }
        EntryChange::Restore { id, .. } => {
            let expected = db.query_row("SELECT count(*) FROM ccp_entry_retention_clocks WHERE workspace_id=?1 AND entry_id=?2",params![workspace.to_string(),id.to_string()],|r|r.get::<_,usize>(0))?;
            exact_count(
                db.execute(
                    "DELETE FROM ccp_entry_retention_clocks WHERE workspace_id=?1 AND entry_id=?2",
                    params![workspace.to_string(), id.to_string()],
                )?,
                expected,
            )?;
        }
        _ => {}
    }
    Ok(())
}

pub(in crate::foundation::store) fn workspace_state(
    db: &Connection,
    workspace: Uuid,
    archived: bool,
    now: i64,
) -> anyhow::Result<()> {
    let expected = count(db, workspace)?;
    let changed = if archived {
        db.execute("UPDATE ccp_entry_retention_clocks SET remaining_seconds=max(0,remaining_seconds-max(0,?2-active_since)),active_since=NULL WHERE workspace_id=?1 AND active_since IS NOT NULL",params![workspace.to_string(),now])?
    } else {
        db.execute("UPDATE ccp_entry_retention_clocks SET active_since=?2 WHERE workspace_id=?1 AND active_since IS NULL",params![workspace.to_string(),now])?
    };
    exact_count(changed, expected)
}
