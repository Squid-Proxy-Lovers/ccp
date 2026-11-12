// SPDX-License-Identifier: AGPL-3.0-or-later
//! Schema27 permits explicitly confirmed non-expiring, single-use invitations.
//! The private initialization transaction already disables FK enforcement before
//! BEGIN. Parent identity/column values, extra indices and triggers are preserved.
use anyhow::{Context, bail};
use rusqlite::{OptionalExtension, Transaction};
const TABLE: &str = include_str!("invitation_table.sql");
pub(super) fn migrate(tx: &Transaction<'_>) -> anyhow::Result<()> {
    let sql: Option<String> = tx
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type='table' AND name='ccp_invitations'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    let Some(sql) = sql else {
        tx.execute_batch(TABLE)?;
        return Ok(());
    };
    let body = body(&sql)?;
    if body == self::body(TABLE)? {
        return Ok(());
    }
    let old = TABLE.replacen(
        "max_uses = 1))",
        "max_uses = 1 AND expires_at IS NOT NULL))",
        1,
    );
    if body != self::body(&old)? {
        bail!("unsupported invitation table definition; inspect or restore a matching backup");
    }
    let foreign_keys: bool = tx.pragma_query_value(None, "foreign_keys", |row| row.get(0))?;
    if foreign_keys {
        bail!("invitation migration requires the private initialization transaction");
    }
    let columns = tx
        .prepare("PRAGMA table_info(ccp_invitations)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if columns
        != [
            "id",
            "workspace_id",
            "kind",
            "role",
            "token_hash",
            "issuer_kind",
            "issuer_id",
            "max_uses",
            "uses",
            "expires_at",
            "revoked_at",
        ]
    {
        bail!("unsupported invitation columns; existing data was not replaced");
    }
    let extras=tx.prepare("SELECT sql FROM sqlite_schema WHERE tbl_name='ccp_invitations' AND type IN ('index','trigger') AND sql IS NOT NULL ORDER BY type,name")?
        .query_map([],|row|row.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let create = TABLE.replacen(
        "CREATE TABLE IF NOT EXISTS ccp_invitations",
        "CREATE TABLE ccp_invitations_upgrade",
        1,
    );
    tx.execute_batch(&create)?;
    tx.execute_batch("INSERT INTO ccp_invitations_upgrade(id,workspace_id,kind,role,token_hash,issuer_kind,issuer_id,max_uses,uses,expires_at,revoked_at)
        SELECT id,workspace_id,kind,role,token_hash,issuer_kind,issuer_id,max_uses,uses,expires_at,revoked_at FROM ccp_invitations;
        DROP TABLE ccp_invitations;
        ALTER TABLE ccp_invitations_upgrade RENAME TO ccp_invitations;")?;
    for sql in extras {
        tx.execute_batch(&sql)?;
    }
    let violations: u64 = tx.query_row(
        "SELECT COUNT(*) FROM pragma_foreign_key_check('ccp_invitations')",
        [],
        |row| row.get(0),
    )?;
    if violations != 0 {
        bail!("invitation migration failed integrity checks; no migration was committed");
    }
    Ok(())
}
fn body(sql: &str) -> anyhow::Result<String> {
    let (_, body) = sql
        .split_once('(')
        .context("invalid invitation table definition")?;
    // Compare known producer SQL exactly. Folding quoted literals or deleting
    // their whitespace could mistake a different CHECK constraint for ours.
    Ok(body.trim().trim_end_matches(';').trim_end().to_owned())
}
