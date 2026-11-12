// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;

#[test]
fn individual_purge_cascades_only_its_provenance_and_a_fault_rolls_everything_back() {
    let mut f = fixture();
    let e = entry(&mut f, "imported");
    let sibling = entry(&mut f, "sibling");
    proof(&f, f.workspace);
    record(&f, f.workspace, e, 1, "PRIVATE_PURGE");
    record(&f, f.workspace, sibling, 1, "PRIVATE_KEEP");
    seal(&f, f.workspace);
    let owner = Actor::Admin(f.owner);
    apply(
        &mut f,
        owner,
        &command(EntryChange::Archive {
            id: e,
            expected_revision: 1,
        }),
    );
    let intent = EntryPurge {
        operation_id: Uuid::new_v4(),
        entry_id: e,
        expected_revision: 2,
        confirm_purge: true,
    };
    f.connection.execute_batch("CREATE TRIGGER native_purge_fault AFTER DELETE ON ccp_native_entry_rows BEGIN SELECT RAISE(ABORT,'fault'); END;").unwrap();
    assert!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .purge_entry(owner, f.workspace, &intent, NOW)
            .is_err()
    );
    assert_eq!(rows(&f.connection, "ccp_native_entry_rows"), 2);
    assert_eq!(rows(&f.connection, "ccp_content_entries"), 2);
    assert_eq!(rows(&f.connection, "ccp_entry_purges"), 0);
    f.connection
        .execute_batch("DROP TRIGGER native_purge_fault")
        .unwrap();
    AccessStore::new(&mut f.connection)
        .unwrap()
        .purge_entry(owner, f.workspace, &intent, NOW)
        .unwrap();
    assert_eq!(rows(&f.connection, "ccp_native_entry_rows"), 1);
    assert_eq!(
        read(&mut f, owner, &request(sibling, None, 1))
            .unwrap()
            .unwrap()
            .items[0]
            .row
            .values[0],
        NativeSqlValue::Text("PRIVATE_KEEP".into())
    );
    error(
        read(&mut f, owner, &request(e, None, 1)),
        ContentError::NotFound,
    );
    AccessStore::new(&mut f.connection)
        .unwrap()
        .purge_entry(owner, f.workspace, &intent, NOW)
        .unwrap();
    assert_eq!(rows(&f.connection, "ccp_native_entry_rows"), 1);
}

#[test]
fn timed_purge_removes_native_rows_and_archived_workspace_still_pauses_it() {
    let mut f = fixture();
    let e = entry(&mut f, "imported");
    proof(&f, f.workspace);
    record(&f, f.workspace, e, 1, "PRIVATE");
    seal(&f, f.workspace);
    let owner = Actor::Admin(f.owner);
    apply(
        &mut f,
        owner,
        &command(EntryChange::Archive {
            id: e,
            expected_revision: 1,
        }),
    );
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    store
        .change_entry_retention(
            owner,
            f.workspace,
            &EntryRetentionChange {
                operation_id: Uuid::new_v4(),
                expected_revision: 0,
                retention_seconds: Some(100),
                confirm_timed_purge: true,
            },
            NOW,
        )
        .unwrap();
    store
        .set_archived(owner, f.workspace, true, NOW + 50)
        .unwrap();
    assert!(
        store
            .purge_due_entry_archives(f.workspace, 32, NOW + 1000)
            .unwrap()
            .is_empty()
    );
    store
        .set_archived(owner, f.workspace, false, NOW + 1000)
        .unwrap();
    assert_eq!(
        store
            .purge_due_entry_archives(f.workspace, 32, NOW + 1050)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(rows(&f.connection, "ccp_native_entry_rows"), 0);
    assert_eq!(rows(&f.connection, "ccp_native_imports"), 1);
    assert_eq!(rows(&f.connection, "ccp_entry_retention_purges"), 1);
}
