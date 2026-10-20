// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

#[test]
fn paused_clocks_survive_file_reopen_and_stale_writers_cannot_repurge_or_reset_policy() {
    let path = std::env::temp_dir().join(format!("ccp-retention-{}.sqlite3", Uuid::new_v4()));
    crate::init::init_sqlite(&path).unwrap();
    let mut f = fixture_with_connection(Connection::open(&path).unwrap());
    let e = entry(&mut f, "entry");
    let request = change(0, Some(100));
    configure(&mut f, &request, NOW);
    archive(&mut f, e.entry_id, 1, NOW);
    lifecycle(&mut f, true, NOW + 25);
    drop(f.connection);
    crate::init::init_sqlite(&path).unwrap();
    f.connection = Connection::open(&path).unwrap();
    assert_eq!(clock(&f, e.entry_id), (2, 75, None));
    assert!(cleanup(&mut f, NOW + 10000).is_empty());
    lifecycle(&mut f, false, NOW + 10000);
    let mut second = Connection::open(&path).unwrap();
    let mut stale = AccessStore::new(&mut second).unwrap();
    assert_eq!(
        stale
            .change_entry_retention(Actor::Admin(f.owner), f.workspace, &request, NOW + 10000)
            .unwrap()
            .revision,
        1
    );
    assert_eq!(clock(&f, e.entry_id), (2, 75, Some(NOW + 10000)));
    assert!(cleanup(&mut f, NOW + 10074).is_empty());
    assert_eq!(cleanup(&mut f, NOW + 10075).len(), 1);
    assert!(
        stale
            .purge_due_entry_archives(f.workspace, 32, NOW + 10075)
            .unwrap()
            .is_empty()
    );
    drop(second);
    drop(f.connection);
    fs::remove_file(path).unwrap();
}

#[test]
fn manual_purge_clock_cascade_and_timed_proof_birth_guards_preserve_scope() {
    let mut f = fixture();
    let e = entry(&mut f, "entry");
    archive(&mut f, e.entry_id, 1, NOW);
    configure(&mut f, &change(0, Some(100)), NOW);
    assert!(
        f.connection
            .execute(
                "INSERT INTO ccp_entry_retention_purges VALUES(?1,?2,2,1,?3)",
                params![e.entry_id.to_string(), f.workspace.to_string(), NOW + 99]
            )
            .is_err()
    );
    lifecycle(&mut f, true, NOW + 100);
    assert!(
        f.connection
            .execute(
                "INSERT INTO ccp_entry_retention_purges VALUES(?1,?2,2,1,?3)",
                params![e.entry_id.to_string(), f.workspace.to_string(), NOW + 1000]
            )
            .is_err()
    );
    let request = EntryPurge {
        operation_id: Uuid::new_v4(),
        entry_id: e.entry_id,
        expected_revision: 2,
        confirm_purge: true,
    };
    AccessStore::new(&mut f.connection)
        .unwrap()
        .purge_entry(Actor::Admin(f.owner), f.workspace, &request, NOW + 1000)
        .unwrap();
    assert_eq!(rows(&f.connection, "ccp_entry_retention_clocks"), 0);
    assert_eq!(rows(&f.connection, "ccp_entry_purges"), 1);
    assert_eq!(rows(&f.connection, "ccp_entry_retention_purges"), 0);
    assert!(cleanup(&mut f, NOW + 1000).is_empty());
}

#[test]
fn retention_refuses_unconverted_native_workspaces_and_does_not_create_timers_on_upgrade() {
    let mut f = fixture();
    f.connection
        .execute("INSERT INTO sessions(id,name) VALUES(88,'native')", [])
        .unwrap();
    f.connection
        .execute(
            "UPDATE ccp_workspaces SET legacy_session_id=88 WHERE id=?1",
            [f.workspace.to_string()],
        )
        .unwrap();
    error(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .change_entry_retention(Actor::Admin(f.owner), f.workspace, &change(0, Some(1)), NOW),
        ContentError::LegacyConversionRequired,
    );
    error(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .purge_due_entry_archives(f.workspace, 1, NOW),
        ContentError::LegacyConversionRequired,
    );
    assert_eq!(rows(&f.connection, "ccp_entry_retention_versions"), 0);
    assert_eq!(rows(&f.connection, "ccp_entry_retention_clocks"), 0);
}

#[test]
fn failed_schema_record_rolls_back_all_retention_tables_before_preserving_retry() {
    let path =
        std::env::temp_dir().join(format!("ccp-retention-upgrade-{}.sqlite3", Uuid::new_v4()));
    crate::init::init_sqlite(&path).unwrap();
    let mut f = fixture_with_connection(Connection::open(&path).unwrap());
    let e = entry(&mut f, "archive");
    archive(&mut f, e.entry_id, 1, NOW);
    f.connection.execute_batch("DROP TRIGGER ccp_no_timed_purged_entry_recreation; DROP TABLE ccp_entry_retention_clocks; DROP TABLE ccp_entry_retention_purges; DROP TABLE ccp_entry_retention_heads; DROP TABLE ccp_entry_retention_versions; DELETE FROM schema_version WHERE version>21; INSERT OR IGNORE INTO schema_version(version) VALUES(21);").unwrap();
    f.connection.execute_batch(&format!("CREATE TRIGGER retention_upgrade_fault BEFORE INSERT ON schema_version WHEN NEW.version={} BEGIN SELECT RAISE(ABORT,'schema fault'); END;",crate::init::SCHEMA_VERSION)).unwrap();
    drop(f.connection);
    assert!(crate::init::init_sqlite(&path).is_err());
    f.connection = Connection::open(&path).unwrap();
    assert_eq!(
        f.connection
            .query_row("SELECT MAX(version) FROM schema_version", [], |r| r
                .get::<_, u32>(0))
            .unwrap(),
        21
    );
    assert_eq!(
        f.connection
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name LIKE 'ccp_entry_retention_%'",
                [],
                |r| r.get::<_, u32>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(rows(&f.connection, "ccp_content_entries"), 1);
    assert_eq!(rows(&f.connection, "ccp_content_versions"), 2);
    f.connection
        .execute_batch("DROP TRIGGER retention_upgrade_fault")
        .unwrap();
    drop(f.connection);
    crate::init::init_sqlite(&path).unwrap();
    f.connection = Connection::open(&path).unwrap();
    assert_eq!(rows(&f.connection, "ccp_entry_retention_versions"), 0);
    assert_eq!(rows(&f.connection, "ccp_entry_retention_clocks"), 0);
    assert_eq!(rows(&f.connection, "ccp_content_versions"), 2);
    assert_eq!(rows(&f.connection, "ccp_content_operations"), 2);
    assert_eq!(
        f.connection
            .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| r
                .get::<_, u32>(
                0
            ))
            .unwrap(),
        0
    );
    drop(f.connection);
    fs::remove_file(path).unwrap();
}
