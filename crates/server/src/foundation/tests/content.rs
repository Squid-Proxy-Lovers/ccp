// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

fn draft(name: &str) -> NodeDraft {
    NodeDraft {
        name: name.to_owned(),
        description: "exact\nmetadata".to_owned(),
    }
}

#[test]
fn current_writers_create_hierarchy_and_readers_only_read_within_their_workspace() {
    let mut f = fixture();
    let writer = enroll(&mut f, DataRole::ReadWrite);
    let reader = enroll(&mut f, DataRole::Read);
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    let actor = Actor::Credential(writer.credential_id);
    let shelf = store
        .create_shelf(actor, f.workspace, &draft(" notes "), NOW)
        .unwrap();
    assert_eq!(shelf.name, "notes");
    assert_eq!(shelf.description, "exact\nmetadata");
    let book = store
        .create_book(actor, f.workspace, shelf.id, &draft("records"), NOW)
        .unwrap();
    let reader = Actor::Credential(reader.credential_id);
    assert_eq!(
        store.get_shelf(reader, f.workspace, shelf.id, NOW).unwrap(),
        shelf
    );
    assert_eq!(
        store.get_book(reader, f.workspace, book.id, NOW).unwrap(),
        book
    );
    assert!(
        store
            .create_shelf(reader, f.workspace, &draft("denied"), NOW)
            .is_err()
    );
    assert!(
        store
            .create_book(reader, f.workspace, shelf.id, &draft("denied"), NOW)
            .is_err()
    );
    assert!(
        store
            .get_shelf(actor, f.other_workspace, shelf.id, NOW)
            .is_err()
    );
    assert!(
        store
            .get_book(actor, f.other_workspace, book.id, NOW)
            .is_err()
    );
    assert!(
        store
            .create_book(
                Actor::Admin(f.other_owner),
                f.other_workspace,
                shelf.id,
                &draft("foreign"),
                NOW
            )
            .is_err()
    );
    assert_eq!(rows(&f.connection, "ccp_content_books"), 1);
}

#[test]
fn global_key_reads_but_never_borrows_a_same_principals_workspace_write_role() {
    let mut f = fixture();
    f.connection
        .execute(
            "UPDATE ccp_workspaces SET owner_admin_id=?1 WHERE id=?2",
            params![f.global.to_string(), f.workspace.to_string()],
        )
        .unwrap();
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    let request = |scope| AdminKeyRequest {
        principal_id: f.global,
        scope,
        name: "key".into(),
        expires_at: None,
    };
    let global = store
        .issue_admin_key(
            Actor::Admin(f.global),
            &request(AdminKeyScope::Instance),
            NOW,
        )
        .unwrap();
    let scoped = store
        .issue_admin_key(
            Actor::Admin(f.global),
            &request(AdminKeyScope::Workspace(f.workspace)),
            NOW,
        )
        .unwrap();
    let global = store.authenticate_admin_key(global.secret(), NOW).unwrap();
    let scoped = store.authenticate_admin_key(scoped.secret(), NOW).unwrap();
    assert!(
        store
            .create_shelf(global, f.workspace, &draft("global denied"), NOW)
            .is_err()
    );
    let shelf = store
        .create_shelf(scoped, f.workspace, &draft("allowed"), NOW)
        .unwrap();
    assert_eq!(
        store.get_shelf(global, f.workspace, shelf.id, NOW).unwrap(),
        shelf
    );
    assert!(
        store
            .create_book(global, f.workspace, shelf.id, &draft("denied"), NOW)
            .is_err()
    );
    assert!(
        store
            .get_shelf(scoped, f.other_workspace, shelf.id, NOW)
            .is_err()
    );
}

#[test]
fn duplicate_names_and_invalid_payloads_do_not_overwrite_existing_hierarchy() {
    let mut f = fixture();
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    let actor = Actor::Admin(f.owner);
    let shelf = store
        .create_shelf(actor, f.workspace, &draft("same"), NOW)
        .unwrap();
    assert!(
        store
            .create_shelf(actor, f.workspace, &draft(" same "), NOW)
            .is_err()
    );
    let book = store
        .create_book(actor, f.workspace, shelf.id, &draft("same"), NOW)
        .unwrap();
    assert!(
        store
            .create_book(actor, f.workspace, shelf.id, &draft("same"), NOW)
            .is_err()
    );
    for name in ["".to_owned(), "x".repeat(257), "line\nbreak".to_owned()] {
        assert!(
            store
                .create_shelf(actor, f.workspace, &draft(&name), NOW)
                .is_err()
        );
    }
    let too_large = NodeDraft {
        name: "large".into(),
        description: "é".repeat(2049),
    };
    assert!(
        store
            .create_shelf(actor, f.workspace, &too_large, NOW)
            .is_err()
    );
    let another = store
        .create_shelf(actor, f.workspace, &draft("another"), NOW)
        .unwrap();
    assert!(
        store
            .create_book(actor, f.workspace, another.id, &draft("same"), NOW)
            .is_ok()
    );
    assert_eq!(
        store.get_book(actor, f.workspace, book.id, NOW).unwrap(),
        book
    );
    assert_eq!(
        store.get_shelf(actor, f.workspace, shelf.id, NOW).unwrap(),
        shelf
    );
}

#[test]
fn hierarchy_pages_use_ids_and_never_cross_parent_or_workspace_boundaries() {
    let mut f = fixture();
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    let actor = Actor::Admin(f.owner);
    let mut ids = Vec::new();
    for name in ["a", "b", "c"] {
        ids.push(
            store
                .create_shelf(actor, f.workspace, &draft(name), NOW)
                .unwrap()
                .id,
        );
    }
    ids.sort();
    let first = store.shelves(actor, f.workspace, None, 1, NOW).unwrap();
    assert_eq!(first.items[0].id, ids[0]);
    assert_eq!(first.next_cursor, Some(ids[0]));
    let last = store
        .shelves(actor, f.workspace, first.next_cursor, 2, NOW)
        .unwrap();
    assert_eq!(
        last.items.iter().map(|s| s.id).collect::<Vec<_>>(),
        ids[1..]
    );
    assert!(last.next_cursor.is_none());
    for name in ["a", "b", "c"] {
        store
            .create_book(actor, f.workspace, ids[0], &draft(name), NOW)
            .unwrap();
    }
    let first = store
        .books(actor, f.workspace, ids[0], None, 2, NOW)
        .unwrap();
    let last = store
        .books(actor, f.workspace, ids[0], first.next_cursor, 2, NOW)
        .unwrap();
    assert_eq!(first.items.len(), 2);
    assert_eq!(last.items.len(), 1);
    assert!(last.next_cursor.is_none());
    assert!(
        store
            .books(actor, f.workspace, ids[1], None, 2, NOW)
            .unwrap()
            .items
            .is_empty()
    );
    for limit in [0, 501] {
        assert!(store.shelves(actor, f.workspace, None, limit, NOW).is_err());
        assert!(
            store
                .books(actor, f.workspace, ids[0], None, limit, NOW)
                .is_err()
        );
    }
}

#[test]
fn archived_workspaces_stay_readable_and_current_disable_and_revocation_are_enforced() {
    let mut f = fixture();
    let client = enroll(&mut f, DataRole::ReadWrite);
    let still_authorized = enroll(&mut f, DataRole::Read);
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    let actor = Actor::Credential(client.credential_id);
    let shelf = store
        .create_shelf(actor, f.workspace, &draft("existing"), NOW)
        .unwrap();
    store
        .set_archived(Actor::Admin(f.owner), f.workspace, true, NOW)
        .unwrap();
    assert_eq!(
        store.get_shelf(actor, f.workspace, shelf.id, NOW).unwrap(),
        shelf
    );
    assert!(
        store
            .create_shelf(actor, f.workspace, &draft("denied"), NOW)
            .is_err()
    );
    assert!(
        store
            .create_book(
                Actor::Admin(f.owner),
                f.workspace,
                shelf.id,
                &draft("denied"),
                NOW
            )
            .is_err()
    );
    store
        .set_archived(Actor::Admin(f.owner), f.workspace, false, NOW)
        .unwrap();
    store
        .revoke_grant(Actor::Admin(f.owner), client.grant_id, NOW)
        .unwrap();
    assert!(store.get_shelf(actor, f.workspace, shelf.id, NOW).is_err());
    assert_eq!(
        store
            .get_shelf(Actor::Admin(f.owner), f.workspace, shelf.id, NOW)
            .unwrap(),
        shelf
    );
    assert!(
        store
            .get_shelf(
                Actor::Credential(still_authorized.credential_id),
                f.workspace,
                shelf.id,
                NOW
            )
            .is_ok()
    );
    store
        .disable_client(Actor::Admin(f.global), still_authorized.client_id, NOW)
        .unwrap();
    assert!(
        store
            .shelves(
                Actor::Credential(still_authorized.credential_id),
                f.workspace,
                None,
                100,
                NOW
            )
            .is_err()
    );
}

#[test]
fn archived_shelf_state_preserves_children_and_name_collisions_cannot_overwrite_them() {
    let mut f = fixture();
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    let actor = Actor::Admin(f.owner);
    let shelf = store
        .create_shelf(actor, f.workspace, &draft("parent"), NOW)
        .unwrap();
    let book = store
        .create_book(actor, f.workspace, shelf.id, &draft("child"), NOW)
        .unwrap();
    f.connection
        .execute(
            "UPDATE ccp_content_shelves SET archived=1 WHERE id=?1",
            [shelf.id.to_string()],
        )
        .unwrap();
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    assert!(
        store
            .shelves(actor, f.workspace, None, 100, NOW)
            .unwrap()
            .items
            .is_empty()
    );
    assert!(store.get_shelf(actor, f.workspace, shelf.id, NOW).is_err());
    assert!(store.get_book(actor, f.workspace, book.id, NOW).is_err());
    assert!(
        store
            .books(actor, f.workspace, shelf.id, None, 100, NOW)
            .is_err()
    );
    assert!(
        store
            .create_book(actor, f.workspace, shelf.id, &draft("denied"), NOW)
            .is_err()
    );
    let replacement = store
        .create_shelf(actor, f.workspace, &draft("parent"), NOW)
        .unwrap();
    assert_ne!(replacement.id, shelf.id);
    assert!(
        f.connection
            .execute(
                "UPDATE ccp_content_shelves SET archived=0 WHERE id=?1",
                [shelf.id.to_string()]
            )
            .is_err()
    );
    assert_eq!(rows(&f.connection, "ccp_content_books"), 1);
    let retained: String = f
        .connection
        .query_row(
            "SELECT description FROM ccp_content_books WHERE id=?1",
            [book.id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(retained, book.description);
}

#[test]
fn content_audit_failure_rolls_back_the_new_node_and_foreign_keys_reject_cross_scope_parents() {
    let mut f = fixture();
    f.connection.execute_batch("CREATE TRIGGER fail_content BEFORE INSERT ON ccp_access_audit WHEN NEW.action IN ('create_shelf','create_book') BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .create_shelf(Actor::Admin(f.owner), f.workspace, &draft("rollback"), NOW)
            .is_err()
    );
    assert_eq!(rows(&f.connection, "ccp_content_shelves"), 0);
    f.connection
        .execute_batch("DROP TRIGGER fail_content")
        .unwrap();
    let shelf = AccessStore::new(&mut f.connection)
        .unwrap()
        .create_shelf(Actor::Admin(f.owner), f.workspace, &draft("parent"), NOW)
        .unwrap();
    assert!(
        f.connection
            .execute(
                "INSERT INTO ccp_content_books VALUES (?1,?2,?3,'foreign','',1000)",
                params![
                    Uuid::new_v4().to_string(),
                    f.other_workspace.to_string(),
                    shelf.id.to_string()
                ]
            )
            .is_err()
    );
    f.connection.execute_batch("CREATE TRIGGER fail_book BEFORE INSERT ON ccp_access_audit WHEN NEW.action='create_book' BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .create_book(
                Actor::Admin(f.owner),
                f.workspace,
                shelf.id,
                &draft("rollback"),
                NOW
            )
            .is_err()
    );
    assert_eq!(rows(&f.connection, "ccp_content_books"), 0);
}

#[test]
fn mapped_legacy_workspaces_refuse_content_without_hiding_or_modifying_native_payload() {
    let mut f = fixture();
    f.connection.execute_batch("INSERT INTO sessions(id,name) VALUES(99,'legacy'); INSERT INTO message_packs(session_id,name,context) VALUES(99,'entry','native content');").unwrap();
    f.connection
        .execute(
            "UPDATE ccp_workspaces SET legacy_session_id=99 WHERE id=?1",
            [f.workspace.to_string()],
        )
        .unwrap();
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    let error = store
        .shelves(Actor::Admin(f.owner), f.workspace, None, 100, NOW)
        .unwrap_err();
    assert!(error.to_string().contains("verified conversion"));
    assert!(
        store
            .create_shelf(
                Actor::Admin(f.owner),
                f.workspace,
                &draft("no overwrite"),
                NOW
            )
            .is_err()
    );
    assert_eq!(rows(&f.connection, "ccp_content_shelves"), 0);
    let payload: String = f
        .connection
        .query_row(
            "SELECT context FROM message_packs WHERE session_id=99",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(payload, "native content");
}

#[test]
fn schema14_upgrade_preserves_security_native_payload_and_hierarchy_survives_reopen() {
    let path = std::env::temp_dir().join(format!("ccp-hierarchy-{}.sqlite3", Uuid::new_v4()));
    crate::init::init_sqlite(&path).unwrap();
    let mut f = fixture_with_connection(Connection::open(&path).unwrap());
    let client = enroll(&mut f, DataRole::ReadWrite);
    let instance = AccessStore::new(&mut f.connection)
        .unwrap()
        .instance_id()
        .unwrap();
    f.connection.execute_batch("DROP TABLE ccp_content_books; DROP TABLE ccp_content_shelves; DELETE FROM schema_version WHERE version>14; INSERT OR IGNORE INTO schema_version(version) VALUES(14); INSERT INTO sessions(id,name) VALUES(99,'old'); INSERT INTO message_packs(session_id,name,context) VALUES(99,'entry','preserved native payload');").unwrap();
    drop(f.connection);
    crate::init::init_sqlite(&path).unwrap();
    let mut db = Connection::open(&path).unwrap();
    let mut store = AccessStore::new(&mut db).unwrap();
    assert_eq!(store.instance_id().unwrap(), instance);
    let shelf = store
        .create_shelf(
            Actor::Credential(client.credential_id),
            f.workspace,
            &draft("persisted"),
            NOW,
        )
        .unwrap();
    let book = store
        .create_book(
            Actor::Credential(client.credential_id),
            f.workspace,
            shelf.id,
            &draft("persisted"),
            NOW,
        )
        .unwrap();
    drop(db);
    crate::init::init_sqlite(&path).unwrap();
    let mut db = Connection::open(&path).unwrap();
    let mut store = AccessStore::new(&mut db).unwrap();
    assert_eq!(
        store
            .get_book(Actor::Admin(f.owner), f.workspace, book.id, NOW)
            .unwrap(),
        book
    );
    let payload: String = db
        .query_row(
            "SELECT context FROM message_packs WHERE session_id=99",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(payload, "preserved native payload");
    let version: u32 = db
        .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 15);
    drop(db);
    fs::remove_file(path).unwrap();
}
