// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

#[test]
fn concurrent_writers_accept_one_cas_and_reopen_keeps_stale_messages_and_cleared_fences() {
    let path = std::env::temp_dir().join(format!("ccp-status-{}.sqlite3", Uuid::new_v4()));
    crate::init::init_sqlite(&path).unwrap();
    let mut f = fixture_with_connection(Connection::open(&path).unwrap());
    let a = enroll(&mut f, DataRole::ReadWrite);
    let id = Uuid::new_v4();
    apply(&mut f, a.credential_id, &post(id, 0, "initial"), NOW).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let mut threads = Vec::new();
    for text in ["one", "two"] {
        let path = path.clone();
        let barrier = barrier.clone();
        let workspace = f.workspace;
        let credential = a.credential_id;
        threads.push(std::thread::spawn(move || {
            let mut db = Connection::open(path).unwrap();
            db.busy_timeout(std::time::Duration::from_secs(3)).unwrap();
            let mut store = AccessStore::new(&mut db).unwrap();
            barrier.wait();
            store.mutate_work_status(
                Actor::Credential(credential),
                workspace,
                &post(id, 1, text),
                NOW + 1,
            )
        }));
    }
    barrier.wait();
    let results = threads
        .into_iter()
        .map(|t| t.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    for result in results {
        if result.is_err() {
            error(result, ContentError::Conflict);
        }
    }
    assert_eq!(read(&mut f, id, NOW + 1).revision, 2);
    let clear_request = clear(id, 2);
    let receipt = apply(&mut f, a.credential_id, &clear_request, NOW + 2).unwrap();
    drop(f.connection);
    crate::init::init_sqlite(&path).unwrap();
    f.connection = Connection::open(&path).unwrap();
    assert_eq!(read(&mut f, id, NOW + 300).message, None);
    assert_eq!(
        apply(&mut f, a.credential_id, &clear_request, NOW + 300).unwrap(),
        receipt
    );
    apply(
        &mut f,
        a.credential_id,
        &post(id, 3, "latest survives restart"),
        NOW + 301,
    )
    .unwrap();
    drop(f.connection);
    crate::init::init_sqlite(&path).unwrap();
    f.connection = Connection::open(&path).unwrap();
    let value = read(&mut f, id, NOW + 601);
    assert!(value.stale);
    assert_eq!(value.message.as_deref(), Some("latest survives restart"));
    assert_eq!(value.client_id, a.client_id);
    drop(f.connection);
    fs::remove_file(path).unwrap();
}

#[test]
fn schema22_upgrade_preserves_rows_creates_empty_status_and_rolls_back_failed_schema_record() {
    let path = std::env::temp_dir().join(format!("ccp-status-upgrade-{}.sqlite3", Uuid::new_v4()));
    crate::init::init_sqlite(&path).unwrap();
    let mut f = fixture_with_connection(Connection::open(&path).unwrap());
    let a = enroll(&mut f, DataRole::ReadWrite);
    f.connection.execute_batch("DROP TABLE ccp_work_status_operations; DROP TABLE ccp_work_status; DELETE FROM schema_version WHERE version>22; INSERT INTO schema_version(version) VALUES(22);").unwrap();
    let client_count = rows(&f.connection, "ccp_clients");
    let audit_count = rows(&f.connection, "ccp_access_audit");
    f.connection.execute_batch("CREATE TRIGGER status_migration_fault BEFORE INSERT ON schema_version WHEN NEW.version=23 BEGIN SELECT RAISE(ABORT,'schema failure'); END;").unwrap();
    drop(f.connection);
    assert!(crate::init::init_sqlite(&path).is_err());
    f.connection = Connection::open(&path).unwrap();
    let statuses:i64=f.connection.query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name LIKE 'ccp_work_status%'",[],|r|r.get(0)).unwrap();
    assert_eq!(statuses, 0);
    assert_eq!(rows(&f.connection, "ccp_clients"), client_count);
    assert_eq!(rows(&f.connection, "ccp_access_audit"), audit_count);
    f.connection
        .execute_batch("DROP TRIGGER status_migration_fault")
        .unwrap();
    drop(f.connection);
    crate::init::init_sqlite(&path).unwrap();
    f.connection = Connection::open(&path).unwrap();
    assert_eq!(rows(&f.connection, "ccp_clients"), client_count);
    assert_eq!(rows(&f.connection, "ccp_access_audit"), audit_count);
    assert_eq!(rows(&f.connection, "ccp_work_status"), 0);
    assert_eq!(rows(&f.connection, "ccp_work_status_operations"), 0);
    assert!(page(&mut f, None, 32, NOW).items.is_empty());
    apply(
        &mut f,
        a.credential_id,
        &post(Uuid::new_v4(), 0, "explicit first status"),
        NOW,
    )
    .unwrap();
    drop(f.connection);
    fs::remove_file(path).unwrap();
}
