// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

type Row = (String, String, String, String, Vec<u8>, String, u64);

fn ledger(db: &Connection) -> Vec<Row> {
    db.prepare("SELECT workspace_id,operation_id,actor_kind,actor_id,request_hash,entry_id,revision FROM ccp_content_operations ORDER BY workspace_id,operation_id").unwrap()
        .query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).unwrap()
        .collect::<rusqlite::Result<Vec<_>>>().unwrap()
}
fn parents(db: &Connection) -> Vec<String> {
    db.prepare("PRAGMA foreign_key_list(ccp_content_operations)")
        .unwrap()
        .query_map([], |r| r.get(2))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}
fn schema18(f: &mut Fixture) {
    // Genuine old layout, including its version-row FK, not just an old marker.
    f.connection.execute_batch("DROP TRIGGER IF EXISTS ccp_no_incomplete_workspace_deletion; DROP TRIGGER IF EXISTS ccp_immutable_content_operation;
        DROP TRIGGER IF EXISTS ccp_retain_content_operation;
        DROP TRIGGER ccp_cancellation_excludes_own_commit;
        DROP TRIGGER ccp_operation_excludes_own_cancellation;
        ALTER TABLE ccp_content_operations RENAME TO operation_fixture_saved;
        CREATE TABLE ccp_content_operations (
            workspace_id TEXT NOT NULL,operation_id TEXT NOT NULL,actor_kind TEXT NOT NULL,
            actor_id TEXT NOT NULL,request_hash BLOB NOT NULL CHECK(length(request_hash)=32),
            entry_id TEXT NOT NULL,revision INTEGER NOT NULL,
            PRIMARY KEY(workspace_id,operation_id),
            FOREIGN KEY(entry_id,revision,workspace_id) REFERENCES ccp_content_versions(entry_id,revision,workspace_id) ON DELETE RESTRICT);
        INSERT INTO ccp_content_operations SELECT * FROM operation_fixture_saved;
        DROP TABLE operation_fixture_saved;
        DELETE FROM schema_version WHERE version>18;
        INSERT OR IGNORE INTO schema_version(version) VALUES(18);").unwrap();
    f.connection
        .execute_batch(include_str!("../../../operation_resolution.sql"))
        .unwrap();
    assert_eq!(parents(&f.connection), vec!["ccp_content_versions"; 3]);
}

#[test]
fn schema18_rebuild_preserves_proof_payload_and_cancellation_then_reopen_does_not_recopy() {
    let path = std::env::temp_dir().join(format!("ccp-ledger-upgrade-{}.sqlite3", Uuid::new_v4()));
    crate::init::init_sqlite(&path).unwrap();
    let mut f = fixture_with_connection(Connection::open(&path).unwrap());
    let writer = enroll(&mut f, DataRole::ReadWrite);
    let book = book(&mut f);
    let actor = Actor::Credential(writer.credential_id);
    let request = create(book, "entry", "original payload");
    let created = apply(&mut f, actor, &request);
    let before = get(&mut f, created.entry_id);
    let rows = ledger(&f.connection);
    let missing = EntryOperationTarget {
        operation_id: Uuid::new_v4(),
        credential_id: writer.credential_id,
        request_hash: [1; 32],
    };
    AccessStore::new(&mut f.connection)
        .unwrap()
        .resolve_entry_operation(actor, f.workspace, &missing, NOW)
        .unwrap();
    schema18(&mut f);
    drop(f.connection);
    crate::init::init_sqlite(&path).unwrap();
    f.connection = Connection::open(&path).unwrap();
    assert_eq!(parents(&f.connection), vec!["ccp_workspaces"]);
    assert_eq!(ledger(&f.connection), rows);
    assert_eq!(get(&mut f, created.entry_id), before);
    assert_eq!(apply(&mut f, actor, &request), created);
    assert_eq!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .resolve_entry_operation(actor, f.workspace, &missing, NOW)
            .unwrap(),
        EntryOperationOutcome::Cancelled
    );
    f.connection.execute_batch("CREATE TRIGGER forbid_ledger_recopy BEFORE INSERT ON ccp_content_operations BEGIN SELECT RAISE(ABORT,'reopen recopied receipts'); END;").unwrap();
    drop(f.connection);
    crate::init::init_sqlite(&path).unwrap();
    f.connection = Connection::open(&path).unwrap();
    assert_eq!(ledger(&f.connection), rows);
    assert_eq!(get(&mut f, created.entry_id), before);
    assert_eq!(
        f.connection
            .query_row("SELECT MAX(version) FROM schema_version", [], |r| r
                .get::<_, u32>(0))
            .unwrap(),
        crate::init::SCHEMA_VERSION
    );
    assert!(
        f.connection
            .prepare("PRAGMA foreign_key_check")
            .unwrap()
            .query([])
            .unwrap()
            .next()
            .unwrap()
            .is_none()
    );
    drop(f.connection);
    fs::remove_file(path).unwrap();
}

#[test]
fn failed_schema_record_rolls_back_table_rebuild_and_guards_before_a_successful_retry() {
    let path = std::env::temp_dir().join(format!("ccp-ledger-rollback-{}.sqlite3", Uuid::new_v4()));
    crate::init::init_sqlite(&path).unwrap();
    let mut f = fixture_with_connection(Connection::open(&path).unwrap());
    let book = book(&mut f);
    let actor = Actor::Admin(f.admin);
    let request = create(book, "entry", "keep on failure");
    let entry = apply(&mut f, actor, &request).entry_id;
    let before = get(&mut f, entry);
    let rows = ledger(&f.connection);
    schema18(&mut f);
    f.connection.execute_batch(&format!("CREATE TRIGGER fail_schema_record BEFORE INSERT ON schema_version WHEN NEW.version={} BEGIN SELECT RAISE(ABORT,'fixture schema fault'); END;", crate::init::SCHEMA_VERSION)).unwrap();
    drop(f.connection);
    assert!(crate::init::init_sqlite(&path).is_err());
    f.connection = Connection::open(&path).unwrap();
    assert_eq!(parents(&f.connection), vec!["ccp_content_versions"; 3]);
    assert_eq!(ledger(&f.connection), rows);
    assert_eq!(get(&mut f, entry), before);
    assert_eq!(
        f.connection
            .query_row("SELECT MAX(version) FROM schema_version", [], |r| r
                .get::<_, u32>(0))
            .unwrap(),
        18
    );
    assert_eq!(f.connection.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='trigger' AND name IN ('ccp_immutable_content_operation','ccp_retain_content_operation','ccp_validate_content_operation')",[],|r|r.get::<_,u32>(0)).unwrap(),0);
    assert_eq!(f.connection.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='trigger' AND name IN ('ccp_cancellation_excludes_own_commit','ccp_operation_excludes_own_cancellation')",[],|r|r.get::<_,u32>(0)).unwrap(),2);
    f.connection
        .execute_batch("DROP TRIGGER fail_schema_record")
        .unwrap();
    drop(f.connection);
    crate::init::init_sqlite(&path).unwrap();
    f.connection = Connection::open(&path).unwrap();
    assert_eq!(parents(&f.connection), vec!["ccp_workspaces"]);
    assert_eq!(ledger(&f.connection), rows);
    assert_eq!(get(&mut f, entry), before);
    drop(f.connection);
    fs::remove_file(path).unwrap();
}

#[test]
fn receipt_rebuild_also_works_inside_an_enforced_foreign_key_transaction() {
    let mut f = fixture();
    let book = book(&mut f);
    let actor = Actor::Admin(f.admin);
    let request = create(book, "entry", "body");
    let receipt = apply(&mut f, actor, &request);
    let rows = ledger(&f.connection);
    schema18(&mut f);
    assert!(
        f.connection
            .pragma_query_value(None, "foreign_keys", |r| r.get::<_, bool>(0))
            .unwrap()
    );
    let tx = f.connection.transaction().unwrap();
    crate::foundation::migrate(&tx).unwrap();
    tx.commit().unwrap();
    assert_eq!(ledger(&f.connection), rows);
    assert_eq!(parents(&f.connection), vec!["ccp_workspaces"]);
    assert_eq!(apply(&mut f, actor, &request), receipt);
}
