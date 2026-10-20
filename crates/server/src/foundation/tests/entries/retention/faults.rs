// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

#[test]
fn retention_policy_and_backfill_abort_or_ignore_roll_back_all_rows_and_audit() {
    let mut f = fixture();
    let e = entry(&mut f, "entry");
    archive(&mut f, e.entry_id, 1, NOW);
    let audit = rows(&f.connection, "ccp_access_audit");
    let request = change(0, Some(100));
    for table in [
        "ccp_entry_retention_versions",
        "ccp_entry_retention_heads",
        "ccp_entry_retention_clocks",
        "ccp_access_audit",
    ] {
        for failure in ["ABORT,'fault'", "IGNORE"] {
            f.connection.execute_batch(&format!("CREATE TRIGGER retention_fault BEFORE INSERT ON {table} BEGIN SELECT RAISE({failure}); END;")).unwrap();
            assert!(
                AccessStore::new(&mut f.connection)
                    .unwrap()
                    .change_entry_retention(Actor::Admin(f.owner), f.workspace, &request, NOW)
                    .is_err()
            );
            assert_eq!(rows(&f.connection, "ccp_entry_retention_versions"), 0);
            assert_eq!(rows(&f.connection, "ccp_entry_retention_heads"), 0);
            assert_eq!(rows(&f.connection, "ccp_entry_retention_clocks"), 0);
            assert_eq!(rows(&f.connection, "ccp_access_audit"), audit);
            assert_eq!(rows(&f.connection, "ccp_content_versions"), 2);
            f.connection
                .execute_batch("DROP TRIGGER retention_fault")
                .unwrap();
        }
    }
    configure(&mut f, &request, NOW);
    for statement in [
        "BEFORE UPDATE ON ccp_entry_retention_heads",
        "BEFORE DELETE ON ccp_entry_retention_clocks",
    ] {
        f.connection
            .execute_batch(&format!(
                "CREATE TRIGGER retention_fault {statement} BEGIN SELECT RAISE(IGNORE); END;"
            ))
            .unwrap();
        assert!(
            AccessStore::new(&mut f.connection)
                .unwrap()
                .change_entry_retention(Actor::Admin(f.owner), f.workspace, &change(1, None), NOW)
                .is_err()
        );
        assert_eq!(rows(&f.connection, "ccp_entry_retention_versions"), 1);
        assert_eq!(clock(&f, e.entry_id), (2, 100, Some(NOW)));
        f.connection
            .execute_batch("DROP TRIGGER retention_fault")
            .unwrap();
    }
}

#[test]
fn entry_clock_faults_roll_back_archive_or_restore_head_versions_and_receipts() {
    let mut f = fixture();
    let owner = Actor::Admin(f.owner);
    let e = entry(&mut f, "entry");
    configure(&mut f, &change(0, Some(100)), NOW);
    f.connection.execute_batch("CREATE TRIGGER retention_fault BEFORE INSERT ON ccp_entry_retention_clocks BEGIN SELECT RAISE(IGNORE); END;").unwrap();
    let request = command(EntryChange::Archive {
        id: e.entry_id,
        expected_revision: 1,
    });
    assert!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .mutate_entry(owner, f.workspace, &request, NOW)
            .is_err()
    );
    assert_eq!(get(&mut f, e.entry_id).entry.revision, 1);
    assert_eq!(rows(&f.connection, "ccp_content_versions"), 1);
    assert_eq!(rows(&f.connection, "ccp_content_operations"), 1);
    f.connection
        .execute_batch("DROP TRIGGER retention_fault")
        .unwrap();
    apply(&mut f, owner, &request);
    f.connection.execute_batch("CREATE TRIGGER retention_fault BEFORE DELETE ON ccp_entry_retention_clocks BEGIN SELECT RAISE(IGNORE); END;").unwrap();
    let request = command(EntryChange::Restore {
        id: e.entry_id,
        expected_revision: 2,
    });
    assert!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .mutate_entry(owner, f.workspace, &request, NOW)
            .is_err()
    );
    assert_eq!(clock(&f, e.entry_id), (2, 100, Some(NOW)));
    assert_eq!(rows(&f.connection, "ccp_content_versions"), 2);
    assert_eq!(rows(&f.connection, "ccp_content_operations"), 2);
}

#[test]
fn workspace_clock_and_state_faults_cannot_commit_partial_pause_or_resume() {
    let mut f = fixture();
    let e = entry(&mut f, "entry");
    configure(&mut f, &change(0, Some(100)), NOW);
    archive(&mut f, e.entry_id, 1, NOW);
    for table in [
        "ccp_entry_retention_clocks",
        "ccp_workspaces",
        "ccp_access_audit",
    ] {
        let verb = if table == "ccp_access_audit" {
            "INSERT"
        } else {
            "UPDATE"
        };
        f.connection.execute_batch(&format!("CREATE TRIGGER retention_fault BEFORE {verb} ON {table} BEGIN SELECT RAISE(IGNORE); END;")).unwrap();
        assert!(
            AccessStore::new(&mut f.connection)
                .unwrap()
                .set_archived(Actor::Admin(f.owner), f.workspace, true, NOW + 30)
                .is_err()
        );
        assert_eq!(clock(&f, e.entry_id), (2, 100, Some(NOW)));
        let state: String = f
            .connection
            .query_row(
                "SELECT state FROM ccp_workspaces WHERE id=?1",
                [f.workspace.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(state, "active");
        f.connection
            .execute_batch("DROP TRIGGER retention_fault")
            .unwrap();
    }
    lifecycle(&mut f, true, NOW + 30);
    f.connection.execute_batch("CREATE TRIGGER retention_fault BEFORE UPDATE ON ccp_entry_retention_clocks BEGIN SELECT RAISE(IGNORE); END;").unwrap();
    assert!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .set_archived(Actor::Admin(f.owner), f.workspace, false, NOW + 100)
            .is_err()
    );
    assert_eq!(clock(&f, e.entry_id), (2, 70, None));
}

#[test]
fn timed_purge_faults_roll_back_proof_versions_entry_and_clock() {
    let mut f = fixture();
    let e = entry(&mut f, "entry");
    archive(&mut f, e.entry_id, 1, NOW);
    configure(&mut f, &change(0, Some(1)), NOW);
    for (verb, table) in [
        ("INSERT", "ccp_entry_retention_purges"),
        ("DELETE", "ccp_content_versions"),
        ("DELETE", "ccp_content_entries"),
        ("DELETE", "ccp_entry_retention_clocks"),
    ] {
        for fault in ["ABORT,'fault'", "IGNORE"] {
            f.connection.execute_batch(&format!("CREATE TRIGGER retention_fault BEFORE {verb} ON {table} BEGIN SELECT RAISE({fault}); END;")).unwrap();
            assert!(
                AccessStore::new(&mut f.connection)
                    .unwrap()
                    .purge_due_entry_archives(f.workspace, 1, NOW + 1)
                    .is_err()
            );
            assert_eq!(rows(&f.connection, "ccp_entry_retention_purges"), 0);
            assert_eq!(rows(&f.connection, "ccp_content_entries"), 1);
            assert_eq!(rows(&f.connection, "ccp_content_versions"), 2);
            assert_eq!(clock(&f, e.entry_id), (2, 1, Some(NOW)));
            f.connection
                .execute_batch("DROP TRIGGER retention_fault")
                .unwrap();
        }
    }
    assert_eq!(cleanup(&mut f, NOW + 1).len(), 1);
}
