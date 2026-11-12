// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;

#[test]
fn refuses_dirty_unsupported_existing_workspace_and_incompatible_values_intact() {
    for sql in [
        "UPDATE schema_version SET version=3",
        "UPDATE schema_version SET version=1",
        "CREATE TABLE ccp_existing_state(value TEXT)",
        "CREATE VIEW custom_view AS SELECT 1",
        "UPDATE message_packs SET labels='a,,a'",
        "UPDATE message_packs SET name=' padded '",
        "UPDATE message_packs SET context=CAST(x'ff' AS TEXT)",
        "UPDATE message_packs SET context=printf('%0300000d',1)",
    ] {
        let f = Fixture::new();
        f.db().execute_batch(sql).unwrap();
        let before = f.files();
        assert!(convert(&f.options()).is_err(), "{sql}");
        assert_eq!(f.files(), before);
        assert!(!f.destination.exists());
        assert_eq!(fs::read_dir(&f.root).unwrap().count(), 1);
    }
    let f = Fixture::new();
    fs::write(f.source.join("runtime-journal.jsonl"), b"SOURCE_PENDING").unwrap();
    let before = f.files();
    error(convert(&f.options()), ConversionError::SourceRefused);
    assert_eq!(f.files(), before);
    assert!(!f.destination.exists());
}

#[test]
fn refuses_virtual_native_tables_before_querying_or_copying_them() {
    let f = Fixture::new();
    let db = f.db();
    db.execute_batch("DROP TABLE auth_tokens; CREATE VIRTUAL TABLE auth_tokens USING fts5(id,session_id,token_nonce,token_value,token_hash,token_prefix,access_level,created_at,last_used_at,expires_at)").unwrap();
    drop(db);
    let before = f.files();
    error(convert(&f.options()), ConversionError::SourceRefused);
    assert_eq!(f.files(), before);
    assert!(!f.destination.exists());
}

#[test]
fn requires_explicit_offline_operator_and_matching_snapshot_fingerprint() {
    let f = Fixture::new();
    let mut options = f.options();
    options.confirm_offline = false;
    error(convert(&options), ConversionError::InvalidInput);
    options.confirm_offline = true;
    options.operator = "".into();
    error(convert(&options), ConversionError::InvalidInput);
    options.operator = "operator".into();
    options.expected_source_sha256 = "0".repeat(64);
    error(convert(&options), ConversionError::SourceRefused);
    assert!(!f.destination.exists());
}

#[test]
fn resource_limits_and_source_sidecars_refuse_without_partial_output() {
    for sql in [
        "WITH RECURSIVE n(i) AS(SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<256) INSERT INTO sessions(id,name) SELECT 1000+i,'session-'||i FROM n",
        "WITH RECURSIVE n(i) AS(SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<100000) INSERT INTO message_history(message_pack_id,client_common_name,appended_content) SELECT 42,'client','' FROM n",
        "UPDATE message_history SET appended_content=zeroblob(2097153)",
    ] {
        let f = Fixture::new();
        f.db().execute_batch(sql).unwrap();
        let before = f.files();
        assert!(convert(&f.options()).is_err());
        assert!(!f.destination.exists());
        assert_eq!(f.files(), before);
        assert_eq!(fs::read_dir(&f.root).unwrap().count(), 1);
    }
    for sidecar in ["ccp.sqlite3-wal", "ccp.sqlite3-shm", "ccp.sqlite3-journal"] {
        let f = Fixture::new();
        fs::write(f.source.join(sidecar), b"").unwrap();
        let before = f.files();
        error(convert(&f.options()), ConversionError::SourceRefused);
        assert_eq!(f.files(), before);
        assert!(!f.destination.exists());
    }
}

#[test]
fn multiple_sessions_and_an_empty_database_create_only_explicit_fresh_roles() {
    let f = Fixture::new();
    f.db().execute("INSERT INTO sessions(id,name,owner,visibility,is_active) VALUES(-1,'empty session','SOURCE_OWNER','public',0)",[]).unwrap();
    let result = convert(&f.options()).unwrap();
    assert_eq!(result.workspaces.len(), 2);
    assert_eq!(result.workspaces[0].name, "empty session");
    let mut db = Connection::open(f.destination.join("ccp.sqlite3")).unwrap();
    let mut store = AccessStore::new(&mut db).unwrap();
    for workspace in &result.workspaces {
        let actor = store
            .authenticate_admin_key(workspace.owner_key.secret(), result.receipt.converted_at)
            .unwrap();
        assert!(
            store
                .shelves(actor, workspace.id, None, 100, result.receipt.converted_at)
                .is_ok()
        );
    }
    assert_eq!(
        db.query_row("SELECT count(*) FROM ccp_admin_principals", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM ccp_clients", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        0
    );
    assert_eq!(db.query_row("SELECT count(*) FROM ccp_workspaces WHERE state='active' AND legacy_session_id IS NULL",[],|r|r.get::<_,u64>(0)).unwrap(),2);
    drop(db);
    let empty = Fixture::new();
    empty.db().execute("DELETE FROM sessions", []).unwrap();
    let result = convert(&empty.options()).unwrap();
    assert!(result.workspaces.is_empty());
    assert_eq!(result.receipt.entry_count, 0);
}
