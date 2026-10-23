// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use rusqlite::Transaction;

/// Schema 18 receipts reference version rows. Preserve their exact body-free
/// proof independently, so future payload purge cannot erase retry authority.
/// All DDL, data copy and trigger replacement share initialization's transaction.
pub(super) fn migrate(tx: &Transaction<'_>) -> anyhow::Result<()> {
    let parents = tx
        .prepare("PRAGMA foreign_key_list(ccp_content_operations)")?
        .query_map([], |row| row.get::<_, String>(2))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if !parents.iter().any(|table| table == "ccp_content_versions") {
        return Ok(());
    }
    tx.execute_batch(
        "DROP TRIGGER IF EXISTS ccp_immutable_content_operation;
         DROP TRIGGER IF EXISTS ccp_retain_content_operation;
         DROP TRIGGER IF EXISTS ccp_validate_content_operation;
         DROP TRIGGER IF EXISTS ccp_cancellation_excludes_own_commit;
         DROP TRIGGER IF EXISTS ccp_operation_excludes_own_cancellation;
         ALTER TABLE ccp_content_operations RENAME TO ccp_content_operations_v18;",
    )?;
    tx.execute_batch(include_str!("entry.sql"))?;
    tx.execute_batch(
        "INSERT INTO ccp_content_operations
         (workspace_id,operation_id,actor_kind,actor_id,request_hash,entry_id,revision)
         SELECT workspace_id,operation_id,actor_kind,actor_id,request_hash,entry_id,revision
         FROM ccp_content_operations_v18;
         DROP TABLE ccp_content_operations_v18;",
    )?;
    // The caller restores cancellation/commit exclusion triggers immediately
    // afterward, before this transaction can commit or serve any request.
    Ok(())
}
