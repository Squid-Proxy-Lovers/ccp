// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use crate::foundation::workspace_deletion::PAYLOAD;

pub(super) fn retire(
    tx: &Transaction<'_>,
    workspace: Uuid,
    limit: usize,
    now: i64,
) -> anyhow::Result<bool> {
    for table in [
        "ccp_admin_keys",
        "ccp_grants",
        "ccp_invitations",
        "ccp_recovery_invitations",
    ] {
        let condition = if table == "ccp_admin_keys" {
            "revoked_at IS NULL OR name!=''"
        } else {
            "revoked_at IS NULL"
        };
        let label = if table == "ccp_admin_keys" {
            ",name=''"
        } else {
            ""
        };
        // Table/column fragments are fixed schema constants; values are bound.
        let changed = tx.execute(&format!("UPDATE {table} SET revoked_at=coalesce(revoked_at,?1){label} WHERE rowid IN(SELECT rowid FROM {table} WHERE workspace_id=?2 AND ({condition}) LIMIT ?3)"), params![now,workspace.to_string(),i64::try_from(limit)?])?;
        if changed > 0 {
            return Ok(true);
        }
    }
    let changed=tx.execute("DELETE FROM ccp_workspace_admins WHERE rowid IN(SELECT rowid FROM ccp_workspace_admins WHERE workspace_id=?1 LIMIT ?2)", params![workspace.to_string(),i64::try_from(limit)?])?;
    if changed > 0 {
        return Ok(true);
    }
    Ok(tx.execute("UPDATE ccp_initial_enrollment SET closed_at=?1 WHERE workspace_id=?2 AND closed_at IS NULL", params![now,workspace.to_string()])?>0)
}

pub(super) fn payload(tx: &Transaction<'_>, workspace: Uuid, limit: usize) -> anyhow::Result<bool> {
    for (table, scope) in PAYLOAD {
        let changed=tx.execute(&format!("DELETE FROM {table} WHERE rowid IN(SELECT rowid FROM {table} WHERE {scope}=?1 LIMIT ?2)"), params![workspace.to_string(),i64::try_from(limit)?])?;
        if changed > 0 {
            return Ok(true);
        }
    }
    Ok(false)
}
