// SPDX-License-Identifier: AGPL-3.0-or-later
use super::{InspectionError as Error, NativeCounts, Result, WorkspaceCounts};
use rusqlite::Connection;

// Only fixed native schema identifiers are interpolated. No source strings are SQL.
const TABLES: &[(&str, &[&str])] = &[
    (
        "sessions",
        &[
            "id",
            "name",
            "description",
            "owner",
            "labels",
            "visibility",
            "purpose",
            "is_active",
            "last_started_at",
            "last_stopped_at",
            "created_at",
        ],
    ),
    (
        "shelves",
        &[
            "id",
            "session_id",
            "shelf_name",
            "description",
            "created_at",
            "updated_at",
        ],
    ),
    (
        "books",
        &[
            "id",
            "session_id",
            "shelf_name",
            "book_name",
            "description",
            "created_at",
            "updated_at",
        ],
    ),
    (
        "message_packs",
        &[
            "id",
            "session_id",
            "name",
            "description",
            "labels",
            "context",
            "shelf_name",
            "book_name",
            "created_at",
            "updated_at",
        ],
    ),
    (
        "deleted_message_packs",
        &[
            "entry_key",
            "session_id",
            "name",
            "description",
            "labels",
            "context",
            "shelf_name",
            "book_name",
            "shelf_description",
            "book_description",
            "created_at",
            "updated_at",
            "deleted_at",
            "deleted_by_client_common_name",
        ],
    ),
    (
        "message_history",
        &[
            "id",
            "message_pack_id",
            "operation_id",
            "client_common_name",
            "agent_name",
            "host_name",
            "reason",
            "appended_content",
            "created_at",
        ],
    ),
    (
        "deleted_message_history",
        &[
            "id",
            "entry_key",
            "operation_id",
            "client_common_name",
            "agent_name",
            "host_name",
            "reason",
            "appended_content",
            "created_at",
        ],
    ),
    (
        "issued_client_certs",
        &[
            "id",
            "session_id",
            "common_name",
            "access_level",
            "cert_pem",
            "created_at",
            "expires_at",
            "revoked_at",
        ],
    ),
    (
        "auth_tokens",
        &[
            "id",
            "session_id",
            "token_nonce",
            "token_value",
            "token_hash",
            "token_prefix",
            "access_level",
            "created_at",
            "last_used_at",
            "expires_at",
        ],
    ),
    (
        "transfer_log",
        &[
            "id",
            "session_id",
            "direction",
            "scope_json",
            "bundle_sha256",
            "policy",
            "outcome",
            "entry_count",
            "created_at",
            "created_by",
        ],
    ),
];

pub(crate) fn inventory(
    db: &Connection,
) -> Result<(u32, &'static str, NativeCounts, WorkspaceCounts)> {
    let schema = schema(db)?;
    layout(db)?;
    let check: String = db
        .query_row("PRAGMA quick_check(1)", [], |r| r.get(0))
        .map_err(|_error| Error::InvalidDatabase)?;
    if check != "ok" {
        return Err(Error::InvalidDatabase);
    }
    let mut query = db
        .prepare("PRAGMA foreign_key_check")
        .map_err(|_error| Error::InvalidDatabase)?;
    if query
        .query([])
        .map_err(|_error| Error::InvalidDatabase)?
        .next()
        .map_err(|_error| Error::InvalidDatabase)?
        .is_some()
    {
        return Err(Error::InvalidDatabase);
    }
    let native = NativeCounts {
        sessions: count(db, "sessions")?,
        shelves: count(db, "shelves")?,
        books: count(db, "books")?,
        entries: count(db, "message_packs")?,
        archives: count(db, "deleted_message_packs")?,
        append_records: count(db, "message_history")?,
        archived_append_records: count(db, "deleted_message_history")?,
        certificates: count(db, "issued_client_certs")?,
        tokens: count(db, "auth_tokens")?,
        transfer_records: count(db, "transfer_log")?,
    };
    let workspace = WorkspaceCounts {
        workspaces: optional_count(db, "ccp_workspaces")?,
        principals: optional_count(db, "ccp_admin_principals")?,
        keys: optional_count(db, "ccp_admin_keys")?,
        clients: optional_count(db, "ccp_clients")?,
        grants: optional_count(db, "ccp_grants")?,
        entries: optional_count(db, "ccp_content_entries")?,
    };
    let mode = if exists(db, "ccp_runtime_mode")? {
        let mode: String = db
            .query_row(
                "SELECT mode FROM ccp_runtime_mode WHERE singleton=1",
                [],
                |r| r.get(0),
            )
            .map_err(|_error| Error::InvalidDatabase)?;
        match mode.as_str() {
            "native" => "native",
            "workspace" => "workspace",
            _ => return Err(Error::InvalidDatabase),
        }
    } else {
        "native"
    };
    Ok((schema, mode, native, workspace))
}

fn schema(db: &Connection) -> Result<u32> {
    if !exists(db, "schema_version")? {
        return Ok(0);
    }
    let version: u32 = db
        .query_row(
            "SELECT coalesce(max(version),0) FROM schema_version",
            [],
            |r| r.get(0),
        )
        .map_err(|_error| Error::InvalidDatabase)?;
    if version > crate::init::SCHEMA_VERSION || matches!(version, 3 | 4) {
        return Err(Error::UnsupportedSchema);
    }
    Ok(version)
}
fn layout(db: &Connection) -> Result<()> {
    for (table, required) in TABLES {
        if !exists(db, table)? {
            return Err(Error::InvalidDatabase);
        }
        let mut statement = db
            .prepare(&format!("PRAGMA table_info({table})"))
            .map_err(|_error| Error::InvalidDatabase)?;
        let columns = statement
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|_error| Error::InvalidDatabase)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|_error| Error::InvalidDatabase)?;
        if required
            .iter()
            .any(|name| !columns.iter().any(|column| column == name))
        {
            return Err(Error::InvalidDatabase);
        }
    }
    Ok(())
}
fn exists(db: &Connection, table: &str) -> Result<bool> {
    let mut statement = db
        .prepare("SELECT type FROM sqlite_schema WHERE name=?1 COLLATE NOCASE LIMIT 2")
        .map_err(|_error| Error::InvalidDatabase)?;
    let kinds = statement
        .query_map([table], |row| row.get::<_, String>(0))
        .map_err(|_error| Error::InvalidDatabase)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_error| Error::InvalidDatabase)?;
    match kinds.as_slice() {
        [] => Ok(false),
        [kind] if kind == "table" => Ok(true),
        _ => Err(Error::InvalidDatabase),
    }
}
fn count(db: &Connection, table: &str) -> Result<u64> {
    db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .map_err(|_error| Error::InvalidDatabase)
}
fn optional_count(db: &Connection, table: &str) -> Result<u64> {
    if exists(db, table)? {
        count(db, table)
    } else {
        Ok(0)
    }
}
