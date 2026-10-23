// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

#[test]
fn scoped_key_cannot_borrow_global_authority_and_revoked_key_cannot_replay() {
    let mut f = fixture();
    let (shelf, _, _, _, _) = tree(&mut f);
    f.connection
        .execute(
            "UPDATE ccp_workspaces SET owner_admin_id=?1 WHERE id=?2",
            params![f.global.to_string(), f.workspace.to_string()],
        )
        .unwrap();
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    let mut issued = Vec::new();
    for scope in [
        AdminKeyScope::Instance,
        AdminKeyScope::Workspace(f.workspace),
    ] {
        issued.push(
            store
                .issue_admin_key(
                    Actor::Admin(f.global),
                    &AdminKeyRequest {
                        principal_id: f.global,
                        scope,
                        name: "key".into(),
                        expires_at: None,
                    },
                    NOW,
                )
                .unwrap(),
        );
    }
    let global = store
        .authenticate_admin_key(issued[0].secret(), NOW)
        .unwrap();
    let scoped = store
        .authenticate_admin_key(issued[1].secret(), NOW)
        .unwrap();
    let request = transition(shelf, 1, true);
    error(
        store.transition_shelf(global, f.workspace, &request, NOW),
        ContentError::Forbidden,
    );
    store
        .transition_shelf(scoped, f.workspace, &request, NOW)
        .unwrap();
    f.connection
        .execute(
            "UPDATE ccp_admin_keys SET revoked_at=?1 WHERE id=?2",
            params![NOW, issued[1].id.to_string()],
        )
        .unwrap();
    error(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .transition_shelf(scoped, f.workspace, &request, NOW),
        ContentError::Forbidden,
    );
    assert_eq!(rows(&f.connection, "ccp_shelf_operations"), 1);
}

#[test]
fn archived_shelf_pages_and_explicit_state_are_bounded_and_workspace_scoped() {
    let mut f = fixture();
    let (shelf, _, _, _, _) = tree(&mut f);
    apply(&mut f, &transition(shelf, 1, true));
    let second = AccessStore::new(&mut f.connection)
        .unwrap()
        .create_shelf(Actor::Admin(f.admin), f.workspace, &draft("second"), NOW)
        .unwrap();
    apply(&mut f, &transition(second.id, 1, true));
    let reader = Actor::Credential(enroll(&mut f, DataRole::Read).credential_id);
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    let first = store
        .archived_shelves(reader, f.workspace, None, 1, NOW)
        .unwrap();
    assert_eq!(first.items.len(), 1);
    assert_eq!(first.next_cursor, Some(first.items[0].id));
    let second = store
        .archived_shelves(reader, f.workspace, first.next_cursor, 1, NOW)
        .unwrap();
    assert_eq!(second.items.len(), 1);
    assert!(second.next_cursor.is_none());
    assert!(first.items[0].id < second.items[0].id);
    for limit in [0, 501, usize::MAX] {
        error(
            store.archived_shelves(reader, f.workspace, None, limit, NOW),
            ContentError::InvalidInput,
        );
    }
    assert!(
        store
            .archived_shelves(
                Actor::Admin(f.other_owner),
                f.other_workspace,
                None,
                100,
                NOW
            )
            .unwrap()
            .items
            .is_empty()
    );
    error(
        store.shelf_state(Actor::Admin(f.other_owner), f.other_workspace, shelf, NOW),
        ContentError::NotFound,
    );
    error(
        store.archived_shelves(reader, f.other_workspace, None, 100, NOW),
        ContentError::Forbidden,
    );
    error(
        store.transition_shelf(
            Actor::Admin(f.other_owner),
            f.other_workspace,
            &transition(shelf, 2, false),
            NOW,
        ),
        ContentError::NotFound,
    );
    for revision in [0, u64::try_from(i64::MAX).unwrap(), u64::MAX] {
        error(
            store.transition_shelf(
                Actor::Admin(f.admin),
                f.workspace,
                &transition(shelf, revision, false),
                NOW,
            ),
            ContentError::InvalidInput,
        );
    }
    let mut nil = transition(shelf, 2, false);
    nil.operation_id = Uuid::nil();
    error(
        store.transition_shelf(Actor::Admin(f.admin), f.workspace, &nil, NOW),
        ContentError::InvalidInput,
    );
}

#[test]
fn schema17_upgrade_backfills_preserves_content_and_reopen_does_not_reset_lifecycle() {
    let path = std::env::temp_dir().join(format!("ccp-shelf-upgrade-{}.sqlite3", Uuid::new_v4()));
    crate::init::init_sqlite(&path).unwrap();
    let mut f = fixture_with_connection(Connection::open(&path).unwrap());
    let (shelf, _, writer, _, entry) = tree(&mut f);
    let actor = Actor::Credential(writer.credential_id);
    let before = AccessStore::new(&mut f.connection)
        .unwrap()
        .get_entry(actor, f.workspace, entry.entry_id, NOW)
        .unwrap();
    f.connection.execute_batch("DROP TRIGGER IF EXISTS ccp_no_incomplete_workspace_deletion; DROP TABLE ccp_shelf_operations; DROP TABLE ccp_shelf_lifecycle; DELETE FROM schema_version WHERE version>17; INSERT OR IGNORE INTO schema_version(version) VALUES(17);").unwrap();
    drop(f.connection);
    crate::init::init_sqlite(&path).unwrap();
    f.connection = Connection::open(&path).unwrap();
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    assert_eq!(
        store
            .shelf_state(actor, f.workspace, shelf, NOW)
            .unwrap()
            .revision,
        1
    );
    assert_eq!(
        store
            .get_entry(actor, f.workspace, entry.entry_id, NOW)
            .unwrap(),
        before
    );
    let request = transition(shelf, 1, true);
    let saved = apply(&mut f, &request);
    drop(f.connection);
    crate::init::init_sqlite(&path).unwrap();
    f.connection = Connection::open(&path).unwrap();
    assert_eq!(apply(&mut f, &request), saved);
    let mut second = Connection::open(&path).unwrap();
    let stale = AccessStore::new(&mut second)
        .unwrap()
        .shelf_state(actor, f.workspace, shelf, NOW)
        .unwrap();
    apply(&mut f, &transition(shelf, 2, false));
    error(
        AccessStore::new(&mut second).unwrap().transition_shelf(
            Actor::Admin(f.admin),
            f.workspace,
            &transition(shelf, stale.revision, false),
            NOW,
        ),
        ContentError::Conflict,
    );
    assert_eq!(
        AccessStore::new(&mut second)
            .unwrap()
            .get_entry(actor, f.workspace, entry.entry_id, NOW)
            .unwrap(),
        before
    );
    assert_eq!(rows(&second, "ccp_shelf_operations"), 2);
    assert!(
        second
            .prepare("PRAGMA foreign_key_check")
            .unwrap()
            .query([])
            .unwrap()
            .next()
            .unwrap()
            .is_none()
    );
    drop(second);
    drop(f.connection);
    fs::remove_file(path).unwrap();
}

#[test]
fn legacy_content_guard_and_new_control_failures_never_commit_partial_shelf_state() {
    let mut f = fixture();
    f.connection.execute_batch("CREATE TRIGGER fail_control BEFORE INSERT ON ccp_shelf_lifecycle BEGIN SELECT RAISE(ABORT,'control fault'); END;").unwrap();
    assert!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .create_shelf(Actor::Admin(f.admin), f.workspace, &draft("new"), NOW)
            .is_err()
    );
    assert_eq!(rows(&f.connection, "ccp_content_shelves"), 0);
    assert_eq!(rows(&f.connection, "ccp_shelf_lifecycle"), 0);
    f.connection
        .execute_batch("DROP TRIGGER fail_control")
        .unwrap();
    let (shelf, _, _, _, _) = tree(&mut f);
    f.connection.execute_batch("CREATE TRIGGER ignore_control BEFORE UPDATE ON ccp_shelf_lifecycle BEGIN SELECT RAISE(IGNORE); END;").unwrap();
    let request = transition(shelf, 1, true);
    assert!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .transition_shelf(Actor::Admin(f.admin), f.workspace, &request, NOW)
            .is_err()
    );
    let state = AccessStore::new(&mut f.connection)
        .unwrap()
        .shelf_state(Actor::Admin(f.owner), f.workspace, shelf, NOW)
        .unwrap();
    assert!(!state.archived);
    assert_eq!(state.revision, 1);
    assert_eq!(rows(&f.connection, "ccp_shelf_operations"), 0);
    f.connection
        .execute_batch(
            "DROP TRIGGER ignore_control; INSERT INTO sessions(id,name) VALUES(99,'legacy');",
        )
        .unwrap();
    f.connection
        .execute(
            "UPDATE ccp_workspaces SET legacy_session_id=99 WHERE id=?1",
            [f.workspace.to_string()],
        )
        .unwrap();
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    error(
        store.transition_shelf(Actor::Admin(f.admin), f.workspace, &request, NOW),
        ContentError::LegacyConversionRequired,
    );
    error(
        store.shelf_state(Actor::Admin(f.owner), f.workspace, shelf, NOW),
        ContentError::LegacyConversionRequired,
    );
    error(
        store.archived_shelves(Actor::Admin(f.owner), f.workspace, None, 100, NOW),
        ContentError::LegacyConversionRequired,
    );
}
