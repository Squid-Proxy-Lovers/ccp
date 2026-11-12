// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;

#[test]
fn imported_archives_and_whole_workspaces_purge_all_raw_payload_without_remapping() {
    let f = Fixture::new();
    let result = convert(&f.options()).unwrap();
    let workspace = result.workspaces[0].id;
    let path = f.destination.join("ccp.sqlite3");
    let mut db = Connection::open(&path).unwrap();
    let (active,archive):(String,String)=db.query_row("SELECT (SELECT id FROM ccp_content_entries WHERE archived=0),(SELECT id FROM ccp_content_entries WHERE archived=1)",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    let mut store = AccessStore::new(&mut db).unwrap();
    let now = result.receipt.converted_at;
    let actor = store
        .authenticate_admin_key(result.workspaces[0].owner_key.secret(), now)
        .unwrap();
    store
        .purge_entry(
            actor,
            workspace,
            &EntryPurge {
                operation_id: Uuid::new_v4(),
                entry_id: Uuid::parse_str(&archive).unwrap(),
                expected_revision: 1,
                confirm_purge: true,
            },
            now,
        )
        .unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM ccp_native_entry_rows", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        2
    );
    let mut store = AccessStore::new(&mut db).unwrap();
    store
        .mutate_entry(
            actor,
            workspace,
            &EntryMutation {
                operation_id: Uuid::new_v4(),
                agent: None,
                change: EntryChange::Append {
                    id: Uuid::parse_str(&active).unwrap(),
                    expected_revision: 1,
                    text: "NEW_APPEND".into(),
                },
            },
            now,
        )
        .unwrap();
    assert_eq!(
        store
            .get_entry(actor, workspace, Uuid::parse_str(&active).unwrap(), now)
            .unwrap()
            .content
            .text,
        "SOURCE_BODYAPPENDNEW_APPEND"
    );
    delete_workspace(&mut store, actor, workspace, now);
    for table in [
        "ccp_native_entry_rows",
        "ccp_native_workspace_rows",
        "ccp_native_imports",
        "ccp_content_entries",
        "ccp_content_versions",
    ] {
        assert_eq!(
            db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                .get::<_, u64>(0))
                .unwrap(),
            0
        );
    }
    drop(db);
    crate::init::init_sqlite(&path).unwrap();
    let db = Connection::open(path).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM ccp_workspaces", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("SELECT state FROM ccp_workspaces", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "purged"
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM ccp_server_admins", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn opaque_extra_columns_and_missing_physical_parents_are_preserved_without_sql_execution() {
    let f = Fixture::new();
    let db = f.db();
    db.execute_batch(
        "ALTER TABLE sessions ADD COLUMN opaque BLOB;
        UPDATE sessions SET opaque=x'00ff';
        ALTER TABLE auth_tokens ADD COLUMN opaque_text TEXT;
        UPDATE auth_tokens SET opaque_text=CAST(x'ff' AS TEXT);
        ALTER TABLE message_packs ADD COLUMN \"unusual\"\";DROP TABLE sessions;--\" TEXT;
        DELETE FROM books; DELETE FROM shelves;",
    )
    .unwrap();
    drop(db);
    let result = convert(&f.options()).unwrap();
    let db = Connection::open(f.destination.join("ccp.sqlite3")).unwrap();
    let raw: String = db
        .query_row(
            "SELECT row_json FROM ccp_native_workspace_rows WHERE source_table='sessions'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<NativeRow>(&raw)
            .unwrap()
            .values
            .last(),
        Some(&NativeSqlValue::Blob(vec![0, 255]))
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM ccp_content_books", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM ccp_content_shelves", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM ccp_admin_principals", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("SELECT name FROM ccp_admin_principals", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "new operator"
    );
    assert_eq!(result.receipt.source_rows["books"], 0);
    assert_eq!(result.receipt.source_rows["shelves"], 0);
    let raw: String = db
        .query_row(
            "SELECT row_json FROM ccp_native_workspace_rows WHERE source_table='auth_tokens'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<NativeRow>(&raw)
            .unwrap()
            .values
            .last(),
        Some(&NativeSqlValue::TextBytes(vec![255]))
    );
}

fn delete_workspace(
    store: &mut AccessStore<'_>,
    actor: crate::foundation::Actor,
    workspace: Uuid,
    now: i64,
) {
    store.set_archived(actor, workspace, true, now).unwrap();
    let preview = store
        .workspace_deletion_preview(actor, workspace, now)
        .unwrap();
    store
        .begin_workspace_deletion(
            actor,
            &WorkspaceDeletionRequest {
                workspace_id: workspace,
                operation_id: Uuid::new_v4(),
                expected_revision: preview.lifecycle_revision,
                confirm_workspace_id: workspace,
            },
            now,
        )
        .unwrap();
    let mut completed = false;
    for _ in 0..100 {
        if store
            .advance_workspace_deletion(workspace, 1, now)
            .unwrap()
            .completed_at
            .is_some()
        {
            completed = true;
            break;
        }
    }
    assert!(completed);
}
