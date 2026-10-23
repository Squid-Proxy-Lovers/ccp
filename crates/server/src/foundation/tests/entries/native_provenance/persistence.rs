// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;

#[test]
fn reopen_preserves_sealed_provenance_and_reinstalls_expanded_deletion_guards_once() {
    let mut f = fixture();
    let e = entry(&mut f, "imported");
    proof(&f, f.workspace);
    record(&f, f.workspace, e, 1, "PRIVATE");
    seal(&f, f.workspace);
    let owner = Actor::Admin(f.owner);
    let before = read(&mut f, owner, &request(e, None, 100)).unwrap();
    let path =
        std::env::temp_dir().join(format!("ccp-native-provenance-{}.sqlite3", Uuid::new_v4()));
    f.connection
        .execute("VACUUM INTO ?1", [path.to_str().unwrap()])
        .unwrap();
    f.connection = Connection::open(&path).unwrap();
    let tx = f.connection.transaction().unwrap();
    migrate(&tx).unwrap();
    tx.commit().unwrap();
    assert_eq!(read(&mut f, owner, &request(e, None, 100)).unwrap(), before);
    let version: i64 = f
        .connection
        .pragma_query_value(None, "schema_version", |r| r.get(0))
        .unwrap();
    let tx = f.connection.transaction().unwrap();
    migrate(&tx).unwrap();
    tx.commit().unwrap();
    assert_eq!(
        f.connection
            .pragma_query_value(None, "schema_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        version
    );
    drop(f);
    fs::remove_file(path).unwrap();
}

#[test]
fn upgraded_completion_guard_includes_new_payload_and_terminal_namespace_stays_closed() {
    use crate::foundation::workspace_deletion::PAYLOAD;
    let mut f = fixture();
    proof(&f, f.workspace);
    let token = serde_json::to_string(&NativeRow {
        columns: vec!["token_value".into()],
        values: vec![NativeSqlValue::Text("PRIVATE_NATIVE_TOKEN".into())],
    })
    .unwrap();
    f.connection.execute("INSERT INTO ccp_native_workspace_rows(workspace_id,ordinal,source_table,source_key,row_json) VALUES(?1,1,'auth_tokens','7',?2)",params![f.workspace.to_string(),token]).unwrap();
    seal(&f, f.workspace);
    // Simulate the installed24 completion predicate. New25 table setup alone
    // must not cause the migrator to reuse the old predicate.
    f.connection.execute_batch("DROP TRIGGER ccp_no_incomplete_workspace_deletion;
        DROP TRIGGER ccp_no_deleted_ccp_native_entry_rows;
        CREATE TRIGGER ccp_no_incomplete_workspace_deletion BEFORE UPDATE OF completed_at ON ccp_workspace_deletions
        WHEN NEW.completed_at IS NOT NULL AND EXISTS(SELECT 1 FROM ccp_content_entries WHERE workspace_id=NEW.workspace_id)
        BEGIN SELECT RAISE(ABORT,'old completion predicate'); END;").unwrap();
    let tx = f.connection.transaction().unwrap();
    migrate(&tx).unwrap();
    tx.commit().unwrap();
    let owner = Actor::Admin(f.owner);
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    store.set_archived(owner, f.workspace, true, NOW).unwrap();
    let preview = store
        .workspace_deletion_preview(owner, f.workspace, NOW)
        .unwrap();
    store
        .begin_workspace_deletion(
            owner,
            &WorkspaceDeletionRequest {
                workspace_id: f.workspace,
                operation_id: Uuid::new_v4(),
                expected_revision: preview.lifecycle_revision,
                confirm_workspace_id: f.workspace,
            },
            NOW,
        )
        .unwrap();
    for (table, scope) in PAYLOAD {
        if !table.starts_with("ccp_native_") {
            f.connection
                .execute(
                    &format!("DELETE FROM {table} WHERE {scope}=?1"),
                    [f.workspace.to_string()],
                )
                .unwrap();
        }
    }
    assert!(
        f.connection
            .execute("UPDATE ccp_workspace_deletions SET completed_at=?1", [NOW])
            .is_err()
    );
    assert_eq!(rows(&f.connection, "ccp_native_workspace_rows"), 1);
    let mut completed = false;
    for _ in 0..20 {
        if AccessStore::new(&mut f.connection)
            .unwrap()
            .advance_workspace_deletion(f.workspace, 1, NOW)
            .unwrap()
            .completed_at
            .is_some()
        {
            completed = true;
            break;
        }
    }
    assert!(completed);
    assert_eq!(rows(&f.connection, "ccp_native_workspace_rows"), 0);
    assert_eq!(rows(&f.connection, "ccp_native_imports"), 0);
    assert!(f.connection.execute("INSERT INTO ccp_native_imports(workspace_id,source_sha256,source_schema,legacy_session_id,recorded_by,recorded_at) VALUES(?1,?2,2,7,?3,?4)",params![f.workspace.to_string(),"b".repeat(64),f.global.to_string(),NOW]).is_err());
    assert_eq!(rows(&f.connection, "ccp_workspaces"), 2);
}
