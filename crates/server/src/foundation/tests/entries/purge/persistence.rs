// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

#[test]
fn purge_faults_roll_back_proof_payload_history_and_audit_together() {
    let mut f = fixture();
    let owner = Actor::Admin(f.owner);
    let (_, request) = archived(&mut f, owner);
    let before = rows(&f.connection, "ccp_access_audit");
    for sql in [
        "CREATE TRIGGER purge_fault BEFORE INSERT ON ccp_entry_purges BEGIN SELECT RAISE(ABORT,'proof fault'); END;",
        "CREATE TRIGGER purge_fault BEFORE INSERT ON ccp_entry_purges BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER purge_fault BEFORE DELETE ON ccp_content_versions BEGIN SELECT RAISE(ABORT,'version fault'); END;",
        "CREATE TRIGGER purge_fault BEFORE DELETE ON ccp_content_entries BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER purge_fault BEFORE INSERT ON ccp_access_audit WHEN NEW.action='purge_entry' BEGIN SELECT RAISE(ABORT,'audit fault'); END;",
        "CREATE TRIGGER purge_fault BEFORE INSERT ON ccp_access_audit WHEN NEW.action='purge_entry' BEGIN SELECT RAISE(IGNORE); END;",
    ] {
        f.connection.execute_batch(sql).unwrap();
        assert!(
            AccessStore::new(&mut f.connection)
                .unwrap()
                .purge_entry(owner, f.workspace, &request, NOW)
                .is_err()
        );
        assert_eq!(rows(&f.connection, "ccp_entry_purges"), 0);
        assert_eq!(rows(&f.connection, "ccp_content_entries"), 1);
        assert_eq!(rows(&f.connection, "ccp_content_versions"), 2);
        assert_eq!(rows(&f.connection, "ccp_access_audit"), before);
        f.connection
            .execute_batch("DROP TRIGGER purge_fault")
            .unwrap();
    }
    purge(&mut f, owner, &request);
    assert_eq!(rows(&f.connection, "ccp_entry_purges"), 1);
}

#[test]
fn permanent_purge_proof_is_immutable_and_purged_uuid_cannot_be_recreated_in_any_workspace() {
    let mut f = fixture();
    let owner = Actor::Admin(f.owner);
    let (_, request) = archived(&mut f, owner);
    assert!(f.connection.execute("INSERT INTO ccp_entry_purges(workspace_id,operation_id,actor_kind,actor_id,principal_id,entry_id,revision,purged_at) VALUES(?1,?2,'admin',?3,?3,?4,1,1000)",params![f.workspace.to_string(),Uuid::new_v4().to_string(),f.owner.to_string(),request.entry_id.to_string()]).is_err());
    let book: String = f
        .connection
        .query_row(
            "SELECT book_id FROM ccp_content_entries WHERE id=?1",
            [request.entry_id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    purge(&mut f, owner, &request);
    for sql in [
        "UPDATE ccp_entry_purges SET revision=revision",
        "DELETE FROM ccp_entry_purges",
    ] {
        assert!(f.connection.execute(sql, []).is_err());
    }
    for workspace in [f.workspace, f.other_workspace] {
        assert!(f.connection.execute("INSERT INTO ccp_content_entries(id,workspace_id,book_id,name,revision,archived,created_at,updated_at) VALUES(?1,?2,?3,'resurrect',1,0,1000,1000)",params![request.entry_id.to_string(),workspace.to_string(),book]).unwrap_err().to_string().contains("purged entry identity"));
    }
    let columns = f
        .connection
        .prepare("PRAGMA table_info(ccp_entry_purges)")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(1))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(
        columns,
        vec![
            "workspace_id",
            "operation_id",
            "actor_kind",
            "actor_id",
            "principal_id",
            "entry_id",
            "revision",
            "purged_at"
        ]
    );
}

#[test]
fn purge_survives_reopen_and_an_independent_stale_writer_cannot_delete_another_entry() {
    let path = std::env::temp_dir().join(format!("ccp-purge-{}.sqlite3", Uuid::new_v4()));
    crate::init::init_sqlite(&path).unwrap();
    let mut f = fixture_with_connection(Connection::open(&path).unwrap());
    let owner = Actor::Admin(f.owner);
    let (_, request) = archived(&mut f, owner);
    let mut second = Connection::open(&path).unwrap();
    let result = purge(&mut f, owner, &request);
    error(
        AccessStore::new(&mut second).unwrap().purge_entry(
            owner,
            f.workspace,
            &intent(request.entry_id, 2),
            NOW,
        ),
        ContentError::NotFound,
    );
    drop(second);
    drop(f.connection);
    crate::init::init_sqlite(&path).unwrap();
    f.connection = Connection::open(&path).unwrap();
    assert_eq!(purge(&mut f, owner, &request), result);
    assert_eq!(rows(&f.connection, "ccp_content_entries"), 0);
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
fn failed_schema_record_rolls_back_purge_table_and_tombstone_guard_before_retry() {
    let path = std::env::temp_dir().join(format!("ccp-purge-upgrade-{}.sqlite3", Uuid::new_v4()));
    crate::init::init_sqlite(&path).unwrap();
    let mut f = fixture_with_connection(Connection::open(&path).unwrap());
    let owner = Actor::Admin(f.owner);
    let (_, request) = archived(&mut f, owner);
    f.connection.execute_batch("DROP TRIGGER IF EXISTS ccp_no_incomplete_workspace_deletion; DROP TRIGGER ccp_no_purged_entry_recreation; DROP TABLE ccp_entry_purges; DELETE FROM schema_version WHERE version>20; INSERT OR IGNORE INTO schema_version(version) VALUES(20);").unwrap();
    f.connection.execute_batch(&format!("CREATE TRIGGER purge_upgrade_fault BEFORE INSERT ON schema_version WHEN NEW.version={} BEGIN SELECT RAISE(ABORT,'schema fault'); END;",crate::init::SCHEMA_VERSION)).unwrap();
    drop(f.connection);
    assert!(crate::init::init_sqlite(&path).is_err());
    f.connection = Connection::open(&path).unwrap();
    assert_eq!(
        f.connection
            .query_row("SELECT MAX(version) FROM schema_version", [], |r| r
                .get::<_, u32>(0))
            .unwrap(),
        20
    );
    assert_eq!(rows(&f.connection, "ccp_content_versions"), 2);
    assert_eq!(f.connection.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name IN ('ccp_entry_purges','ccp_no_purged_entry_recreation')",[],|r|r.get::<_,u32>(0)).unwrap(),0);
    f.connection
        .execute_batch("DROP TRIGGER purge_upgrade_fault")
        .unwrap();
    drop(f.connection);
    crate::init::init_sqlite(&path).unwrap();
    f.connection = Connection::open(&path).unwrap();
    purge(&mut f, owner, &request);
    drop(f.connection);
    fs::remove_file(path).unwrap();
}
