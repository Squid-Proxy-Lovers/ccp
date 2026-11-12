// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;

#[test]
fn source_payload_obeys_current_grants_archive_shelf_workspace_and_legacy_gates() {
    let mut f = fixture();
    let e = entry(&mut f, "imported");
    proof(&f, f.workspace);
    record(&f, f.workspace, e, 1, "PRIVATE");
    seal(&f, f.workspace);
    let owner = Actor::Admin(f.owner);
    let global = Actor::Admin(f.global);
    let admin = Actor::Admin(f.admin);
    let other = Actor::Admin(f.other_owner);
    let reader = enroll(&mut f, DataRole::Read);
    let client = Actor::Credential(reader.credential_id);
    let r = request(e, None, 100);
    for actor in [owner, global, admin, client] {
        assert!(read(&mut f, actor, &r).unwrap().is_some());
    }
    error(read(&mut f, other, &r), ContentError::Forbidden);
    let foreign = f.other_workspace;
    error(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .native_entry_history(other, foreign, &r, NOW),
        ContentError::NotFound,
    );
    f.connection
        .execute(
            "UPDATE ccp_grants SET revoked_at=?1 WHERE id=?2",
            params![NOW, reader.grant_id.to_string()],
        )
        .unwrap();
    error(read(&mut f, client, &r), ContentError::Forbidden);
    apply(
        &mut f,
        owner,
        &command(EntryChange::Archive {
            id: e,
            expected_revision: 1,
        }),
    );
    for actor in [owner, global, admin] {
        error(read(&mut f, actor, &r), ContentError::NotFound);
    }
    apply(
        &mut f,
        owner,
        &command(EntryChange::Restore {
            id: e,
            expected_revision: 2,
        }),
    );
    assert!(read(&mut f, owner, &r).unwrap().is_some());
    let shelf: String = f.connection.query_row("SELECT s.id FROM ccp_content_shelves s JOIN ccp_content_books b ON b.shelf_id=s.id JOIN ccp_content_entries e ON e.book_id=b.id WHERE e.id=?1",[e.to_string()],|r|r.get(0)).unwrap();
    let shelf_id = Uuid::parse_str(&shelf).unwrap();
    AccessStore::new(&mut f.connection)
        .unwrap()
        .transition_shelf(
            owner,
            f.workspace,
            &ShelfTransition {
                operation_id: Uuid::new_v4(),
                shelf_id,
                expected_revision: 1,
                archived: true,
            },
            NOW,
        )
        .unwrap();
    error(read(&mut f, owner, &r), ContentError::NotFound);
    AccessStore::new(&mut f.connection)
        .unwrap()
        .transition_shelf(
            owner,
            f.workspace,
            &ShelfTransition {
                operation_id: Uuid::new_v4(),
                shelf_id,
                expected_revision: 2,
                archived: false,
            },
            NOW,
        )
        .unwrap();
    AccessStore::new(&mut f.connection)
        .unwrap()
        .set_archived(owner, f.workspace, true, NOW)
        .unwrap();
    assert!(read(&mut f, global, &r).unwrap().is_some());
    f.connection
        .execute("INSERT INTO sessions(id,name) VALUES(99,'unconverted')", [])
        .unwrap();
    f.connection
        .execute(
            "UPDATE ccp_workspaces SET legacy_session_id=99 WHERE id=?1",
            [f.workspace.to_string()],
        )
        .unwrap();
    error(
        read(&mut f, owner, &r),
        ContentError::LegacyConversionRequired,
    );
}

#[test]
fn row_and_page_bounds_do_not_return_partial_or_malformed_source_payload() {
    let mut f = fixture();
    let e = entry(&mut f, "imported");
    let owner = Actor::Admin(f.owner);
    proof(&f, f.workspace);
    for ordinal in 1..=4 {
        record(&f, f.workspace, e, ordinal, &"x".repeat(1_500_000));
    }
    seal(&f, f.workspace);
    let first = read(&mut f, owner, &request(e, None, 100))
        .unwrap()
        .unwrap();
    assert_eq!(first.items.len(), 2);
    assert_eq!(first.next_cursor, Some(2));
    let second = read(&mut f, owner, &request(e, first.next_cursor, 100))
        .unwrap()
        .unwrap();
    assert_eq!(second.items.len(), 2);
    assert_eq!(second.next_cursor, None);
    for r in [
        request(e, None, 0),
        request(e, None, 101),
        request(e, Some(u64::MAX), 1),
    ] {
        error(read(&mut f, owner, &r), ContentError::InvalidInput);
    }
    let ws = f.workspace;
    let mut bad = fixture();
    let e = entry(&mut bad, "malformed");
    proof(&bad, bad.workspace);
    bad.connection.execute("INSERT INTO ccp_native_entry_rows(workspace_id,entry_id,ordinal,source_table,source_key,row_json) VALUES(?1,?2,1,'message_packs','42',?3)",params![bad.workspace.to_string(),e.to_string(),"PRIVATE_SECRET_INVALID_JSON"]).unwrap();
    seal(&bad, bad.workspace);
    let owner = Actor::Admin(bad.owner);
    let failure = read(&mut bad, owner, &request(e, None, 1))
        .unwrap_err()
        .to_string();
    assert_eq!(failure, "stored native provenance is invalid");
    assert!(!failure.contains("PRIVATE"));
    assert_ne!(ws, bad.workspace);
}

#[test]
fn provenance_cannot_be_rewritten_replaced_retargeted_or_extended_after_sealing() {
    let mut f = fixture();
    let e = entry(&mut f, "imported");
    proof(&f, f.workspace);
    record(&f, f.workspace, e, 1, "PRIVATE");
    f.connection.execute("INSERT INTO ccp_native_workspace_rows(workspace_id,ordinal,source_table,source_key,row_json) VALUES(?1,1,'auth_tokens','7',?2)",params![f.workspace.to_string(),raw("PRIVATE_TOKEN")]).unwrap();
    for sql in [
        "UPDATE ccp_native_entry_rows SET row_json='{}' WHERE workspace_id=?1",
        "DELETE FROM ccp_native_entry_rows WHERE workspace_id=?1",
        "UPDATE ccp_native_workspace_rows SET source_key='changed' WHERE workspace_id=?1",
        "DELETE FROM ccp_native_workspace_rows WHERE workspace_id=?1",
        "INSERT OR REPLACE INTO ccp_native_entry_rows SELECT * FROM ccp_native_entry_rows WHERE workspace_id=?1",
        "INSERT OR REPLACE INTO ccp_native_workspace_rows SELECT * FROM ccp_native_workspace_rows WHERE workspace_id=?1",
        "INSERT OR REPLACE INTO ccp_native_imports SELECT * FROM ccp_native_imports WHERE workspace_id=?1",
    ] {
        assert!(
            f.connection
                .execute(sql, [f.workspace.to_string()])
                .is_err(),
            "{sql}"
        );
    }
    let foreign = entry(&mut f, "sibling");
    assert!(f.connection.execute("INSERT INTO ccp_native_entry_rows(workspace_id,entry_id,ordinal,source_table,source_key,row_json) VALUES(?1,?2,2,'message_history','foreign','{}')",params![f.other_workspace.to_string(),foreign.to_string()]).is_err());
    seal(&f, f.workspace);
    for sql in [
        "UPDATE ccp_native_imports SET sealed=0 WHERE workspace_id=?1",
        "UPDATE ccp_native_imports SET recorded_by='fake' WHERE workspace_id=?1",
        "DELETE FROM ccp_native_imports WHERE workspace_id=?1",
        "INSERT INTO ccp_native_entry_rows SELECT workspace_id,entry_id,2,source_table,'new',row_json FROM ccp_native_entry_rows WHERE workspace_id=?1",
        "INSERT INTO ccp_native_workspace_rows SELECT workspace_id,2,source_table,'new',row_json FROM ccp_native_workspace_rows WHERE workspace_id=?1",
    ] {
        assert!(
            f.connection
                .execute(sql, [f.workspace.to_string()])
                .is_err(),
            "{sql}"
        );
    }
    assert_eq!(rows(&f.connection, "ccp_native_entry_rows"), 1);
    assert_eq!(rows(&f.connection, "ccp_native_workspace_rows"), 1);
}

#[test]
fn malformed_row_shapes_and_oversized_corrupt_storage_fail_without_payload_echo() {
    let oversized = raw(&"PRIVATE".repeat(650_000));
    for json in [
        "{\"columns\":[],\"values\":[]}".to_owned(),
        "{\"columns\":[\"context\"],\"values\":[]}".to_owned(),
        "{\"columns\":[\"context\",\"context\"],\"values\":[{\"type\":\"null\"},{\"type\":\"null\"}]}".to_owned(),
        oversized,
    ] {
        let mut f = fixture();
        let e = entry(&mut f,"corrupt");
        proof(&f,f.workspace);
        // Deliberately corrupt fixture storage only; normal schema limits remain enforced.
        f.connection.pragma_update(None,"ignore_check_constraints",true).unwrap();
        f.connection.execute("INSERT INTO ccp_native_entry_rows(workspace_id,entry_id,ordinal,source_table,source_key,row_json) VALUES(?1,?2,1,'message_packs','42',?3)",params![f.workspace.to_string(),e.to_string(),json]).unwrap();
        f.connection.pragma_update(None,"ignore_check_constraints",false).unwrap();
        seal(&f,f.workspace);
        let owner = Actor::Admin(f.owner);
        let failure = read(&mut f,owner,&request(e,None,1)).unwrap_err();
        assert!(!failure.to_string().contains("PRIVATE"));
        assert!(failure.to_string()=="stored native provenance is invalid" || failure.downcast_ref::<ContentError>()==Some(&ContentError::InvalidInput));
    }
}

#[test]
fn storage_bounds_refuse_large_rows_keys_and_wrong_table_classes() {
    let mut f = fixture();
    let e = entry(&mut f, "imported");
    proof(&f, f.workspace);
    for (key, json, table) in [
        ("x".repeat(8193), raw("body"), "message_packs"),
        (
            "42".to_owned(),
            "x".repeat(2 * 1024 * 1024 + 1),
            "message_packs",
        ),
        ("42".to_owned(), raw("body"), "auth_tokens"),
    ] {
        assert!(f.connection.execute("INSERT INTO ccp_native_entry_rows(workspace_id,entry_id,ordinal,source_table,source_key,row_json) VALUES(?1,?2,1,?3,?4,?5)",params![f.workspace.to_string(),e.to_string(),table,key,json]).is_err());
    }
    assert_eq!(rows(&f.connection, "ccp_native_entry_rows"), 0);
    assert!(f.connection.execute("INSERT INTO ccp_native_workspace_rows(workspace_id,ordinal,source_table,source_key,row_json) VALUES(?1,1,'message_packs','42','{}')",[f.workspace.to_string()]).is_err());
    assert_eq!(rows(&f.connection, "ccp_native_workspace_rows"), 0);
}
