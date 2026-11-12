// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

mod persistence;

fn draft(name: &str) -> NodeDraft {
    NodeDraft {
        name: name.into(),
        description: "retained metadata".into(),
    }
}

fn tree(f: &mut Fixture) -> (Uuid, Uuid, Enrollment, EntryMutation, EntryReceipt) {
    let writer = enroll(f, DataRole::ReadWrite);
    let actor = Actor::Credential(writer.credential_id);
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    let shelf = store
        .create_shelf(actor, f.workspace, &draft("shelf"), NOW)
        .unwrap();
    let book = store
        .create_book(actor, f.workspace, shelf.id, &draft("book"), NOW)
        .unwrap();
    let request = EntryMutation {
        operation_id: Uuid::new_v4(),
        agent: Some("original agent".into()),
        change: EntryChange::Create {
            book_id: book.id,
            name: "entry".into(),
            content: EntryContent {
                text: "PRIVATE_BODY".into(),
                description: "original".into(),
                labels: vec!["label".into()],
            },
        },
    };
    let receipt = store
        .mutate_entry(actor, f.workspace, &request, NOW)
        .unwrap();
    (shelf.id, book.id, writer, request, receipt)
}

fn transition(shelf_id: Uuid, expected_revision: u64, archived: bool) -> ShelfTransition {
    ShelfTransition {
        operation_id: Uuid::new_v4(),
        shelf_id,
        expected_revision,
        archived,
    }
}

fn apply(f: &mut Fixture, request: &ShelfTransition) -> ShelfState {
    AccessStore::new(&mut f.connection)
        .unwrap()
        .transition_shelf(Actor::Admin(f.admin), f.workspace, request, NOW)
        .unwrap()
}

fn error<T>(result: anyhow::Result<T>, expected: ContentError) {
    match result {
        Ok(_) => panic!("expected {expected:?}"),
        Err(error) => assert_eq!(
            error.downcast_ref::<ContentError>(),
            Some(&expected),
            "{error:#}"
        ),
    }
}

#[test]
fn shelf_archive_restore_preserves_descendant_privacy_ownership_and_archives() {
    let mut f = fixture();
    let (shelf, book, writer, create, entry) = tree(&mut f);
    let actor = Actor::Credential(writer.credential_id);
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    let before = store
        .get_entry(actor, f.workspace, entry.entry_id, NOW)
        .unwrap();
    let other = EntryMutation {
        operation_id: Uuid::new_v4(),
        agent: None,
        change: EntryChange::Create {
            book_id: book,
            name: "already archived".into(),
            content: before.content.clone(),
        },
    };
    let other = store.mutate_entry(actor, f.workspace, &other, NOW).unwrap();
    store
        .mutate_entry(
            actor,
            f.workspace,
            &EntryMutation {
                operation_id: Uuid::new_v4(),
                agent: None,
                change: EntryChange::Archive {
                    id: other.entry_id,
                    expected_revision: 1,
                },
            },
            NOW,
        )
        .unwrap();
    let archived = apply(&mut f, &transition(shelf, 1, true));
    assert_eq!(archived.revision, 2);
    assert_hidden(&mut f, actor, shelf, book, entry.entry_id);
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    assert_eq!(
        store
            .mutate_entry(actor, f.workspace, &create, NOW)
            .unwrap(),
        entry
    );
    assert_eq!(rows(&f.connection, "ccp_content_versions"), 3);
    assert_eq!(rows(&f.connection, "ccp_content_operations"), 3);
    apply(&mut f, &transition(shelf, 2, false));
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    assert_eq!(
        store
            .get_entry(actor, f.workspace, entry.entry_id, NOW)
            .unwrap(),
        before
    );
    error(
        store.get_entry(actor, f.workspace, other.entry_id, NOW),
        ContentError::NotFound,
    );
    let archived = store
        .entries(
            actor,
            f.workspace,
            &EntryPageRequest {
                book_id: book,
                archived: true,
                after: None,
                limit: 100,
            },
            NOW,
        )
        .unwrap();
    assert_eq!(archived.items.len(), 1);
    assert_eq!(archived.items[0].id, other.entry_id);
    assert_eq!(archived.items[0].owner_client_id, Some(writer.client_id));
}

#[test]
fn current_scoped_admin_authority_is_required_even_for_an_exact_retry() {
    let mut f = fixture();
    let (shelf, _, writer, _, _) = tree(&mut f);
    let reader = enroll(&mut f, DataRole::Read);
    let request = transition(shelf, 1, true);
    for actor in [
        Actor::Credential(writer.credential_id),
        Actor::Credential(reader.credential_id),
        Actor::Admin(f.global),
        Actor::Admin(f.other_owner),
    ] {
        error(
            AccessStore::new(&mut f.connection)
                .unwrap()
                .transition_shelf(actor, f.workspace, &request, NOW),
            ContentError::Forbidden,
        );
    }
    apply(&mut f, &request);
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    let summaries = store
        .archived_shelves(
            Actor::Credential(reader.credential_id),
            f.workspace,
            None,
            100,
            NOW,
        )
        .unwrap();
    assert_eq!(summaries.items.len(), 1);
    assert_eq!(summaries.items[0].id, shelf);
    store
        .revoke_workspace_admin(Actor::Admin(f.owner), f.workspace, f.admin, NOW)
        .unwrap();
    error(
        store.transition_shelf(Actor::Admin(f.admin), f.workspace, &request, NOW),
        ContentError::Forbidden,
    );
    let restore = transition(shelf, 2, false);
    store
        .transition_shelf(Actor::Admin(f.owner), f.workspace, &restore, NOW)
        .unwrap();
    store
        .set_archived(Actor::Admin(f.owner), f.workspace, true, NOW)
        .unwrap();
    error(
        store.transition_shelf(Actor::Admin(f.owner), f.workspace, &restore, NOW),
        ContentError::Forbidden,
    );
    assert!(
        store
            .shelf_state(
                Actor::Credential(reader.credential_id),
                f.workspace,
                shelf,
                NOW
            )
            .is_ok()
    );
}

#[test]
fn exact_historical_receipts_never_reapply_after_restore_and_conflicting_or_stale_commands_fail() {
    let mut f = fixture();
    let (shelf, _, _, _, _) = tree(&mut f);
    let request = transition(shelf, 1, true);
    let saved = apply(&mut f, &request);
    assert_eq!(apply(&mut f, &request), saved);
    let restored = apply(&mut f, &transition(shelf, 2, false));
    assert_eq!(apply(&mut f, &request), saved);
    assert_eq!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .shelf_state(Actor::Admin(f.owner), f.workspace, shelf, NOW)
            .unwrap(),
        restored
    );
    for (actor, shelf_id, expected_revision) in [
        (Actor::Admin(f.owner), shelf, 1),
        (Actor::Admin(f.admin), Uuid::new_v4(), 1),
        (Actor::Admin(f.admin), shelf, 2),
    ] {
        let changed = ShelfTransition {
            operation_id: request.operation_id,
            shelf_id,
            expected_revision,
            archived: true,
        };
        error(
            AccessStore::new(&mut f.connection)
                .unwrap()
                .transition_shelf(actor, f.workspace, &changed, NOW),
            ContentError::Conflict,
        );
    }
    let mut changed = ShelfTransition {
        archived: false,
        ..request
    };
    error(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .transition_shelf(Actor::Admin(f.admin), f.workspace, &changed, NOW),
        ContentError::Conflict,
    );
    changed.operation_id = Uuid::new_v4();
    changed.archived = true;
    error(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .transition_shelf(Actor::Admin(f.admin), f.workspace, &changed, NOW),
        ContentError::Conflict,
    );
    let same = transition(shelf, 3, false);
    error(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .transition_shelf(Actor::Admin(f.admin), f.workspace, &same, NOW),
        ContentError::Conflict,
    );
    assert_eq!(rows(&f.connection, "ccp_shelf_operations"), 2);
    assert!(
        f.connection
            .execute("UPDATE ccp_shelf_operations SET archived=0", [])
            .unwrap_err()
            .to_string()
            .contains("immutable")
    );
}

#[test]
fn restore_name_collision_and_audit_or_receipt_failure_leave_all_state_unchanged() {
    let mut f = fixture();
    let (shelf, _, _, _, _) = tree(&mut f);
    apply(&mut f, &transition(shelf, 1, true));
    let duplicate = AccessStore::new(&mut f.connection)
        .unwrap()
        .create_shelf(Actor::Admin(f.admin), f.workspace, &draft("shelf"), NOW)
        .unwrap();
    let restore = transition(shelf, 2, false);
    error(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .transition_shelf(Actor::Admin(f.admin), f.workspace, &restore, NOW),
        ContentError::Conflict,
    );
    apply(&mut f, &transition(duplicate.id, 1, true));
    for (name, sql) in [
        (
            "fail_shelf_audit",
            "CREATE TRIGGER fail_shelf_audit BEFORE INSERT ON ccp_access_audit WHEN NEW.action='restore_shelf' BEGIN SELECT RAISE(ABORT,'audit fault'); END;",
        ),
        (
            "fail_shelf_receipt",
            "CREATE TRIGGER fail_shelf_receipt BEFORE INSERT ON ccp_shelf_operations BEGIN SELECT RAISE(ABORT,'receipt fault'); END;",
        ),
    ] {
        f.connection.execute_batch(sql).unwrap();
        assert!(
            AccessStore::new(&mut f.connection)
                .unwrap()
                .transition_shelf(Actor::Admin(f.admin), f.workspace, &restore, NOW)
                .is_err()
        );
        let state = AccessStore::new(&mut f.connection)
            .unwrap()
            .shelf_state(Actor::Admin(f.owner), f.workspace, shelf, NOW)
            .unwrap();
        assert!(state.archived);
        assert_eq!(state.revision, 2);
        assert_eq!(rows(&f.connection, "ccp_shelf_operations"), 2);
        f.connection
            .execute_batch(&format!("DROP TRIGGER {name}"))
            .unwrap();
    }
    assert!(!apply(&mut f, &restore).archived);
}

fn assert_hidden(f: &mut Fixture, actor: Actor, shelf: Uuid, book: Uuid, entry: Uuid) {
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    assert!(
        store
            .shelves(actor, f.workspace, None, 100, NOW)
            .unwrap()
            .items
            .is_empty()
    );
    error(
        store.get_shelf(actor, f.workspace, shelf, NOW),
        ContentError::NotFound,
    );
    error(
        store.get_book(actor, f.workspace, book, NOW),
        ContentError::NotFound,
    );
    error(
        store.get_entry(actor, f.workspace, entry, NOW),
        ContentError::NotFound,
    );
    error(
        store.entry_version(actor, f.workspace, entry, 1, NOW),
        ContentError::NotFound,
    );
    error(
        store.entry_history(
            actor,
            f.workspace,
            &RevisionPageRequest {
                entry_id: entry,
                after: 0,
                limit: 100,
            },
            NOW,
        ),
        ContentError::NotFound,
    );
    error(
        store.entries(
            actor,
            f.workspace,
            &EntryPageRequest {
                book_id: book,
                archived: true,
                after: None,
                limit: 100,
            },
            NOW,
        ),
        ContentError::NotFound,
    );
    error(
        store.create_book(actor, f.workspace, shelf, &draft("denied"), NOW),
        ContentError::NotFound,
    );
    let append = EntryMutation {
        operation_id: Uuid::new_v4(),
        agent: None,
        change: EntryChange::Append {
            id: entry,
            expected_revision: 1,
            text: "denied".into(),
        },
    };
    error(
        store.mutate_entry(actor, f.workspace, &append, NOW),
        ContentError::NotFound,
    );
}
