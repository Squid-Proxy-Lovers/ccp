// SPDX-License-Identifier: AGPL-3.0-or-later
use super::{
    ConversionReceipt, ConversionResult, ConvertedWorkspace,
    destination::Stage,
    projection::{self, Projection},
    source::{Ledger, Record, Source, Table},
};
use crate::{
    directory_mode::DirectoryLease,
    foundation::{AccessStore, Actor, AdminKeyRequest, AdminKeyScope, IssuedAdminKey},
};
use anyhow::{Context, bail};
use rusqlite::{Connection, params};
use uuid::Uuid;

pub(super) fn build(
    source: &Source,
    stage: &Stage,
    operator: &str,
) -> anyhow::Result<ConversionResult> {
    crate::init::init_sqlite(&stage.database())?;
    let mut db = Connection::open(stage.database())?;
    db.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON; PRAGMA max_page_count=1048576;")?;
    let page_size: i64 = db.pragma_query_value(None, "page_size", |r| r.get(0))?;
    if page_size != 4096 {
        bail!("unsupported target page size");
    }
    let now = crate::instance_tls::now()?;
    let (instance_id, principal, instance_key) = bootstrap(&mut db, operator, now)?;
    let mut workspaces = Vec::new();
    let mut ledger = Ledger::new();
    source.visit(Table::Sessions, None, &mut ledger, |session, ledger| {
        workspaces.push(workspace(source, &mut db, session, ledger, principal, now)?);
        Ok(())
    })?;
    if ledger.seen != source.expected {
        bail!("incomplete native source accounting");
    }
    verify(&db, &ledger)?;
    db.execute("INSERT INTO ccp_access_audit(actor_kind,actor_id,action,target_id,recorded_at) VALUES('local_operator','local','convert_native_database',?1,?2)",params![source.fingerprint(),now])?;
    db.close()
        .map_err(|(_db, _error)| anyhow::anyhow!("converted database did not close"))?;
    // Persist the data-mode fence; the private control ledger is not copied.
    let lease = DirectoryLease::activate_workspace(stage.directory())?;
    drop(lease);
    standalone(stage, &ledger)?;
    let source_rows = source
        .expected
        .iter()
        .map(|(t, n)| (t.name().to_owned(), *n))
        .collect();
    let entries = source
        .expected
        .get(&Table::Entries)
        .context("missing source count")?
        .checked_add(
            *source
                .expected
                .get(&Table::Archives)
                .context("missing source count")?,
        )
        .context("entry count overflow")?;
    Ok(ConversionResult {
        receipt: ConversionReceipt {
            instance_id: instance_id.to_string(),
            source_sha256: source.fingerprint().to_owned(),
            source_schema: 2,
            source_rows,
            converted_at: now,
            workspace_count: u64::try_from(workspaces.len())?,
            entry_count: entries,
            conversion_performed: true,
            source_database_unchanged_at_verification: false,
        },
        instance_key,
        workspaces,
    })
}
fn standalone(stage: &Stage, ledger: &Ledger) -> anyhow::Result<()> {
    // Activation uses the runtime initializer's WAL mode. Publish only after
    // every frame is checkpointed and the target is back in DELETE mode.
    let db = Connection::open(stage.database())?;
    db.pragma_update(None, "synchronous", "FULL")?;
    let (busy, _frames, _done): (i64, i64, i64) =
        db.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?;
    if busy != 0 {
        bail!("converted target checkpoint is busy");
    }
    let mode: String = db.query_row("PRAGMA journal_mode=DELETE", [], |r| r.get(0))?;
    if mode != "delete" {
        bail!("converted database is not standalone");
    }
    verify(&db, ledger)?;
    db.close()
        .map_err(|(_db, _error)| anyhow::anyhow!("standalone database did not close"))?;
    for name in ["ccp.sqlite3-wal", "ccp.sqlite3-shm", "ccp.sqlite3-journal"] {
        match std::fs::symlink_metadata(stage.directory().join(name)) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            _ => bail!("converted database still has recovery files"),
        }
    }
    Ok(())
}
fn bootstrap(
    db: &mut Connection,
    operator: &str,
    now: i64,
) -> anyhow::Result<(Uuid, Uuid, IssuedAdminKey)> {
    let mut store = AccessStore::new(db)?;
    let principal = store.bootstrap_admin(operator, now)?;
    let key = store.issue_admin_key(
        Actor::Admin(principal),
        &AdminKeyRequest {
            principal_id: principal,
            scope: AdminKeyScope::Instance,
            name: "initial instance admin".into(),
            expires_at: None,
        },
        now,
    )?;
    Ok((store.instance_id()?, principal, key))
}
fn workspace(
    source: &Source,
    db: &mut Connection,
    session: Record,
    ledger: &mut Ledger,
    operator: Uuid,
    now: i64,
) -> anyhow::Result<ConvertedWorkspace> {
    let name = projection::text(&session.row, "name")?.to_owned();
    if !projection::valid_name(&name) {
        bail!("source workspace name cannot be preserved");
    }
    let legacy_id = projection::integer(&session.row, "id")?;
    let mut store = AccessStore::new(db)?;
    let id = store.create_workspace(Actor::Admin(operator), &name, operator, now)?;
    let owner_key = store.issue_admin_key(
        Actor::Admin(operator),
        &AdminKeyRequest {
            principal_id: operator,
            scope: AdminKeyScope::Workspace(id),
            name: "initial imported workspace owner".into(),
            expires_at: None,
        },
        now,
    )?;
    let tx = db.transaction()?;
    tx.execute("INSERT INTO ccp_native_imports(workspace_id,source_sha256,source_schema,legacy_session_id,recorded_by,recorded_at) VALUES(?1,?2,2,?3,?4,?5)",params![id.to_string(),source.fingerprint(),legacy_id,operator.to_string(),now])?;
    let mut ordinal = 0_i64;
    workspace_row(&tx, id, &session, &mut ordinal)?;
    let mut projection = Projection::new(&tx, id, operator, now);
    for table in [
        Table::Shelves,
        Table::Books,
        Table::Certificates,
        Table::Tokens,
        Table::Transfers,
    ] {
        source.visit(table, Some(&legacy_id), ledger, |record, _ledger| {
            match table {
                Table::Shelves => projection.physical_shelf(&record.row)?,
                Table::Books => projection.physical_book(&record.row)?,
                _ => {}
            }
            workspace_row(&tx, id, &record, &mut ordinal)
        })?;
    }
    for table in [Table::Entries, Table::Archives] {
        source.visit(table, Some(&legacy_id), ledger, |record, ledger| {
            let entry = projection.entry(&record.row, table == Table::Archives)?;
            let mut ordinal = 0_i64;
            entry_row(&tx, id, entry, &record, &mut ordinal)?;
            let history = if table == Table::Entries {
                Table::Appends
            } else {
                Table::ArchivedAppends
            };
            source.visit(
                history,
                Some(record.key.sql()),
                ledger,
                |append, _ledger| entry_row(&tx, id, entry, &append, &mut ordinal),
            )
        })?;
    }
    tx.execute(
        "UPDATE ccp_native_imports SET sealed=1 WHERE workspace_id=?1",
        [id.to_string()],
    )?;
    tx.execute("INSERT INTO ccp_access_audit(actor_kind,actor_id,action,workspace_id,target_id,recorded_at) VALUES('admin',?1,'convert_native_workspace',?2,?2,?3)",params![operator.to_string(),id.to_string(),now])?;
    tx.commit()?;
    Ok(ConvertedWorkspace {
        id,
        name,
        owner_key,
    })
}
fn workspace_row(
    db: &Connection,
    workspace: Uuid,
    record: &Record,
    ordinal: &mut i64,
) -> anyhow::Result<()> {
    *ordinal = ordinal.checked_add(1).context("source ordinal overflow")?;
    db.execute("INSERT INTO ccp_native_workspace_rows(workspace_id,ordinal,source_table,source_key,row_json) VALUES(?1,?2,?3,?4,?5)",params![workspace.to_string(),*ordinal,record.table.name(),record.key.label(),record.json])?;
    Ok(())
}
fn entry_row(
    db: &Connection,
    workspace: Uuid,
    entry: Uuid,
    record: &Record,
    ordinal: &mut i64,
) -> anyhow::Result<()> {
    *ordinal = ordinal.checked_add(1).context("source ordinal overflow")?;
    db.execute("INSERT INTO ccp_native_entry_rows(workspace_id,entry_id,ordinal,source_table,source_key,row_json) VALUES(?1,?2,?3,?4,?5,?6)",params![workspace.to_string(),entry.to_string(),*ordinal,record.table.name(),record.key.label(),record.json])?;
    Ok(())
}
fn verify(db: &Connection, ledger: &Ledger) -> anyhow::Result<()> {
    let integrity: String = db.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    if integrity != "ok"
        || db
            .prepare("PRAGMA foreign_key_check")?
            .query([])?
            .next()?
            .is_some()
    {
        bail!("converted database integrity failure");
    }
    for table in Table::ALL {
        let native: u64 =
            db.query_row(&format!("SELECT count(*) FROM {}", table.name()), [], |r| {
                r.get(0)
            })?;
        let raw:u64=db.query_row("SELECT (SELECT count(*) FROM ccp_native_entry_rows WHERE source_table=?1)+(SELECT count(*) FROM ccp_native_workspace_rows WHERE source_table=?1)",[table.name()],|r|r.get(0))?;
        if native != 0 || ledger.seen.get(&table) != Some(&raw) {
            bail!("target source accounting failure");
        }
    }
    for table in [
        "ccp_clients",
        "ccp_credentials",
        "ccp_grants",
        "ccp_credential_certificates",
    ] {
        let count: u64 =
            db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))?;
        if count != 0 {
            bail!("conversion invented client authority");
        }
    }
    let unsealed: u64 = db.query_row(
        "SELECT count(*) FROM ccp_native_imports WHERE sealed!=1",
        [],
        |r| r.get(0),
    )?;
    let linked: u64 = db.query_row(
        "SELECT count(*) FROM ccp_workspaces WHERE legacy_session_id IS NOT NULL",
        [],
        |r| r.get(0),
    )?;
    if unsealed != 0 || linked != 0 {
        bail!("incomplete workspace adoption");
    }
    Ok(())
}
