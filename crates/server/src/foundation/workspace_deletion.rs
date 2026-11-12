// SPDX-License-Identifier: AGPL-3.0-or-later

use rusqlite::Transaction;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceDeletionPreview {
    pub workspace_id: Uuid,
    pub name: String,
    pub lifecycle_revision: u64,
}

pub struct WorkspaceDeletionRequest {
    pub workspace_id: Uuid,
    pub operation_id: Uuid,
    pub expected_revision: u64,
    pub confirm_workspace_id: Uuid,
}

/// A durable irreversible start receipt; completion is a separate observed fact.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceDeletion {
    pub workspace_id: Uuid,
    pub operation_id: Uuid,
    pub lifecycle_revision: u64,
    pub started_at: i64,
    pub completed_at: Option<i64>,
}

// Order respects payload FKs. A row count bound never requires loading bodies.
pub(super) const PAYLOAD: &[(&str, &str)] = &[
    ("ccp_native_entry_rows", "workspace_id"),
    ("ccp_native_workspace_rows", "workspace_id"),
    ("ccp_native_imports", "workspace_id"),
    ("ccp_entry_retention_clocks", "workspace_id"),
    ("ccp_entry_retention_purges", "workspace_id"),
    ("ccp_entry_purges", "workspace_id"),
    ("ccp_entry_retention_heads", "workspace_id"),
    ("ccp_entry_retention_versions", "workspace_id"),
    ("ccp_guidance_heads", "board_key"),
    ("ccp_guidance_operations", "workspace_id"),
    ("ccp_guidance_versions", "workspace_id"),
    ("ccp_work_status_operations", "workspace_id"),
    ("ccp_work_status", "workspace_id"),
    ("ccp_shelf_operations", "workspace_id"),
    ("ccp_content_cancellations", "workspace_id"),
    ("ccp_content_operations", "workspace_id"),
    ("ccp_content_versions", "workspace_id"),
    ("ccp_content_entries", "workspace_id"),
    ("ccp_shelf_lifecycle", "workspace_id"),
    ("ccp_content_books", "workspace_id"),
    ("ccp_content_shelves", "workspace_id"),
    ("ccp_enrollment_events", "workspace_id"),
    ("ccp_access_audit", "workspace_id"),
];

const RETAIN: &[(&str, &str)] = &[
    ("ccp_retain_native_import", "ccp_native_imports"),
    (
        "ccp_retain_native_workspace_row",
        "ccp_native_workspace_rows",
    ),
    ("ccp_retain_content_operation", "ccp_content_operations"),
    ("ccp_retain_entry_purge", "ccp_entry_purges"),
    ("ccp_retain_guidance_operation", "ccp_guidance_operations"),
    (
        "ccp_retain_entry_retention_version",
        "ccp_entry_retention_versions",
    ),
    (
        "ccp_retain_entry_retention_purge",
        "ccp_entry_retention_purges",
    ),
    ("ccp_retain_work_status_tombstone", "ccp_work_status"),
    (
        "ccp_retain_work_status_operation",
        "ccp_work_status_operations",
    ),
];

pub(super) fn migrate(tx: &Transaction<'_>) -> anyhow::Result<()> {
    // Installed atomically with the schema marker. Avoid rewriting all retained
    // guards on every concurrent session startup once this migration is current.
    let installed: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='trigger' AND name='ccp_no_incomplete_workspace_deletion') AND EXISTS(SELECT 1 FROM sqlite_master WHERE type='trigger' AND name='ccp_no_deleted_ccp_native_entry_rows')", [], |r| r.get(0))?;
    if installed {
        return Ok(());
    }
    // Schema25 adds payload tables. Rebuild the old completion predicate once,
    // in the same schema transaction, before accepting a completion claim.
    tx.execute_batch("DROP TRIGGER IF EXISTS ccp_no_incomplete_workspace_deletion;")?;
    tx.execute_batch(include_str!("workspace_deletion.sql"))?;
    for (guard, table) in RETAIN {
        // Identifiers are compile-time schema constants, never request data.
        tx.execute_batch(&format!("DROP TRIGGER IF EXISTS {guard};
            CREATE TRIGGER {guard} BEFORE DELETE ON {table}
            WHEN NOT EXISTS(SELECT 1 FROM ccp_workspace_deletions d JOIN ccp_workspaces w
                ON w.id=d.workspace_id WHERE d.workspace_id=OLD.workspace_id AND w.state='purged')
            BEGIN SELECT RAISE(ABORT,'history must be retained outside confirmed workspace deletion'); END;"))?;
    }
    for (table, scope) in PAYLOAD {
        tx.execute_batch(&format!(
            "CREATE INDEX IF NOT EXISTS ccp_deletion_{table}_scope ON {table}({scope});
            CREATE TRIGGER IF NOT EXISTS ccp_no_deleted_{table} BEFORE INSERT ON {table}
            WHEN EXISTS(SELECT 1 FROM ccp_workspaces WHERE id=NEW.{scope} AND state='purged')
            BEGIN SELECT RAISE(ABORT,'deleted workspace cannot accept content'); END;
            CREATE TRIGGER IF NOT EXISTS ccp_no_deleted_update_{table} BEFORE UPDATE OF {scope} ON {table}
            WHEN EXISTS(SELECT 1 FROM ccp_workspaces WHERE id=NEW.{scope} AND state='purged')
            BEGIN SELECT RAISE(ABORT,'deleted workspace cannot receive content'); END;"
        ))?;
    }
    for table in [
        "ccp_admin_keys",
        "ccp_grants",
        "ccp_invitations",
        "ccp_recovery_invitations",
    ] {
        let pending = if table == "ccp_admin_keys" {
            "revoked_at IS NULL OR name!=''"
        } else {
            "revoked_at IS NULL"
        };
        tx.execute_batch(&format!(
            "CREATE INDEX IF NOT EXISTS ccp_deletion_{table}_pending ON {table}(workspace_id) WHERE {pending};"
        ))?;
    }
    let remaining = PAYLOAD
        .iter()
        .map(|(table, scope)| {
            format!("EXISTS(SELECT 1 FROM {table} WHERE {scope}=NEW.workspace_id)")
        })
        .collect::<Vec<_>>()
        .join(" OR ");
    tx.execute_batch(&format!(
        "CREATE TRIGGER IF NOT EXISTS ccp_no_incomplete_workspace_deletion
        BEFORE UPDATE OF completed_at ON ccp_workspace_deletions
        WHEN NEW.completed_at IS NOT NULL AND ({remaining})
        BEGIN SELECT RAISE(ABORT,'workspace cleanup is incomplete'); END;"
    ))?;
    Ok(())
}
