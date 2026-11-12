// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

pub(super) fn due(
    db: &Connection,
    workspace: Uuid,
    limit: usize,
    now: i64,
) -> anyhow::Result<Vec<TimedEntryPurge>> {
    let legacy = db
        .query_row(
            "SELECT legacy_session_id IS NOT NULL FROM ccp_workspaces WHERE id=?1",
            [workspace.to_string()],
            |r| r.get::<_, bool>(0),
        )
        .optional()?
        .ok_or(ContentError::NotFound)?;
    if legacy {
        return Err(ContentError::LegacyConversionRequired.into());
    }
    let rows = db.prepare("SELECT c.entry_id,c.archive_revision,c.policy_revision FROM ccp_entry_retention_clocks c JOIN ccp_content_entries e ON e.id=c.entry_id AND e.workspace_id=c.workspace_id JOIN ccp_workspaces w ON w.id=c.workspace_id JOIN ccp_entry_retention_heads h ON h.workspace_id=c.workspace_id JOIN ccp_entry_retention_versions p ON p.workspace_id=h.workspace_id AND p.revision=h.revision WHERE c.workspace_id=?1 AND w.state='active' AND p.retention_seconds IS NOT NULL AND h.revision=c.policy_revision AND e.archived=1 AND e.revision=c.archive_revision AND c.active_since IS NOT NULL AND ?2>=c.active_since AND ?2-c.active_since>=c.remaining_seconds ORDER BY c.active_since,c.entry_id LIMIT ?3")?
        .query_map(params![workspace.to_string(),now,i64::try_from(limit)?],|r|Ok((r.get::<_,String>(0)?,r.get::<_,u64>(1)?,r.get::<_,u64>(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(|(id, archive_revision, policy_revision)| {
            let receipt = TimedEntryPurge {
                workspace_id: workspace,
                entry_id: parse_id(id)?,
                archive_revision,
                policy_revision,
                purged_at: now,
            };
            remove(db, &receipt)?;
            Ok(receipt)
        })
        .collect()
}

fn remove(db: &Connection, proof: &TimedEntryPurge) -> anyhow::Result<()> {
    require_changed(db.execute("INSERT INTO ccp_entry_retention_purges(entry_id,workspace_id,archive_revision,policy_revision,purged_at) VALUES(?1,?2,?3,?4,?5)",params![proof.entry_id.to_string(),proof.workspace_id.to_string(),i64::try_from(proof.archive_revision)?,i64::try_from(proof.policy_revision)?,proof.purged_at])?)?;
    db.execute(
        "DELETE FROM ccp_content_versions WHERE workspace_id=?1 AND entry_id=?2",
        params![proof.workspace_id.to_string(), proof.entry_id.to_string()],
    )?;
    require_changed(db.execute("DELETE FROM ccp_content_entries WHERE workspace_id=?1 AND id=?2 AND revision=?3 AND archived=1",params![proof.workspace_id.to_string(),proof.entry_id.to_string(),i64::try_from(proof.archive_revision)?])?)?;
    Ok(())
}
