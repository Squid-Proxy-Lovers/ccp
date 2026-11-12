// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

#[test]
fn exact_utf8_bounds_and_invalid_metadata_never_create_partial_entries() {
    let mut f = fixture();
    let book = book(&mut f);
    let actor = Actor::Admin(f.admin);
    let mut invalid = vec![
        create(book, "", "x"),
        create(book, &"é".repeat(129), "x"),
        create(book, "line\nbreak", "x"),
    ];
    for content in [
        content(&"é".repeat(131073)),
        EntryContent {
            description: "é".repeat(2049),
            ..content("x")
        },
        EntryContent {
            labels: vec!["duplicate".into(), "duplicate".into()],
            ..content("x")
        },
        EntryContent {
            labels: vec!["".into()],
            ..content("x")
        },
        EntryContent {
            labels: vec!["bad\nlabel".into()],
            ..content("x")
        },
        EntryContent {
            labels: vec!["é".repeat(65)],
            ..content("x")
        },
        EntryContent {
            labels: (0..33).map(|i| i.to_string()).collect(),
            ..content("x")
        },
    ] {
        invalid.push(command(EntryChange::Create {
            book_id: book,
            name: "invalid".into(),
            content,
        }));
    }
    let mut agent = create(book, "agent", "x");
    agent.agent = Some("bad\nagent".into());
    invalid.push(agent);
    let mut agent = create(book, "agent", "x");
    agent.agent = Some("é".repeat(129));
    invalid.push(agent);
    let mut nil = create(book, "nil", "x");
    nil.operation_id = Uuid::nil();
    invalid.push(nil);
    for request in invalid {
        fail(&mut f, actor, &request, ContentError::InvalidInput);
    }
    assert_eq!(rows(&f.connection, "ccp_content_entries"), 0);
    assert_eq!(rows(&f.connection, "ccp_content_versions"), 0);
    assert_eq!(rows(&f.connection, "ccp_content_operations"), 0);
    let maximum = EntryContent {
        text: "é".repeat(131072),
        description: "é".repeat(2048),
        labels: (0..32)
            .map(|i| format!("{i:02}{}", "é".repeat(63)))
            .collect(),
    };
    let mut request = command(EntryChange::Create {
        book_id: book,
        name: "é".repeat(128),
        content: maximum.clone(),
    });
    request.agent = Some("é".repeat(128));
    let id = apply(&mut f, actor, &request).entry_id;
    assert_eq!(get(&mut f, id).content, maximum);
    fail(
        &mut f,
        actor,
        &command(EntryChange::Append {
            id,
            expected_revision: 1,
            text: "x".into(),
        }),
        ContentError::InvalidInput,
    );
    fail(
        &mut f,
        actor,
        &command(EntryChange::Append {
            id,
            expected_revision: 1,
            text: String::new(),
        }),
        ContentError::InvalidInput,
    );
    assert_eq!(get(&mut f, id).entry.revision, 1);
}

#[test]
fn stale_and_out_of_range_revisions_and_pages_are_typed_errors() {
    let mut f = fixture();
    let book = book(&mut f);
    let actor = Actor::Admin(f.admin);
    let id = apply(&mut f, actor, &create(book, "entry", "x")).entry_id;
    for revision in [0, u64::MAX] {
        for change in [
            EntryChange::Append {
                id,
                expected_revision: revision,
                text: "x".into(),
            },
            EntryChange::Revise {
                id,
                expected_revision: revision,
                content: content("x"),
            },
            EntryChange::Archive {
                id,
                expected_revision: revision,
            },
            EntryChange::Restore {
                id,
                expected_revision: revision,
            },
        ] {
            fail(&mut f, actor, &command(change), ContentError::InvalidInput);
        }
    }
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    error(
        store.entry_version(actor, f.workspace, id, 0, NOW),
        ContentError::InvalidInput,
    );
    error(
        store.entry_version(actor, f.workspace, id, 2, NOW),
        ContentError::NotFound,
    );
    for limit in [0, 501, usize::MAX] {
        error(
            store.entries(
                actor,
                f.workspace,
                &EntryPageRequest {
                    limit,
                    ..page(book, false)
                },
                NOW,
            ),
            ContentError::InvalidInput,
        );
        error(
            store.entry_history(
                actor,
                f.workspace,
                &RevisionPageRequest {
                    entry_id: id,
                    after: 0,
                    limit,
                },
                NOW,
            ),
            ContentError::InvalidInput,
        );
    }
    error(
        store.entry_history(
            actor,
            f.workspace,
            &RevisionPageRequest {
                entry_id: id,
                after: u64::MAX,
                limit: 1,
            },
            NOW,
        ),
        ContentError::InvalidInput,
    );
    error(
        store.mutate_entry(actor, f.workspace, &create(book, "clock", "x"), -1),
        ContentError::Forbidden,
    );
}

#[test]
fn entry_pages_never_cross_parent_or_workspace_and_inactive_parents_are_unavailable() {
    let mut f = fixture();
    let book = book(&mut f);
    let actor = Actor::Admin(f.admin);
    let mut expected = Vec::new();
    for name in ["one", "two", "three"] {
        expected.push(apply(&mut f, actor, &create(book, name, name)).entry_id);
    }
    expected.sort();
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    let first = store
        .entries(
            actor,
            f.workspace,
            &EntryPageRequest {
                limit: 2,
                ..page(book, false)
            },
            NOW,
        )
        .unwrap();
    assert_eq!(
        first.items.iter().map(|v| v.id).collect::<Vec<_>>(),
        expected[..2]
    );
    assert_eq!(first.next_cursor, Some(expected[1]));
    let second = store
        .entries(
            actor,
            f.workspace,
            &EntryPageRequest {
                limit: 2,
                after: first.next_cursor,
                ..page(book, false)
            },
            NOW,
        )
        .unwrap();
    assert_eq!(second.items[0].id, expected[2]);
    assert_eq!(second.next_cursor, None);
    error(
        store.entries(
            Actor::Admin(f.other_owner),
            f.other_workspace,
            &page(book, false),
            NOW,
        ),
        ContentError::NotFound,
    );
    error(
        store.get_entry(
            Actor::Admin(f.other_owner),
            f.other_workspace,
            expected[0],
            NOW,
        ),
        ContentError::NotFound,
    );
    error(
        store.get_entry(actor, f.other_workspace, expected[0], NOW),
        ContentError::Forbidden,
    );
    error(
        store.mutate_entry(actor, f.workspace, &create(book, "one", "replace"), NOW),
        ContentError::Conflict,
    );
    assert_eq!(
        store
            .get_entry(actor, f.workspace, expected[0], NOW)
            .unwrap()
            .entry
            .revision,
        1
    );
    f.connection
        .execute("UPDATE ccp_content_shelves SET archived=1", [])
        .unwrap();
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    error(
        store.get_entry(actor, f.workspace, expected[0], NOW),
        ContentError::NotFound,
    );
    error(
        store.entries(actor, f.workspace, &page(book, false), NOW),
        ContentError::NotFound,
    );
    error(
        store.mutate_entry(actor, f.workspace, &create(book, "hidden", "x"), NOW),
        ContentError::NotFound,
    );
    assert_eq!(rows(&f.connection, "ccp_content_entries"), 3);
}

#[test]
fn legacy_guard_follows_authorization_and_never_converts_or_reassigns_native_content() {
    let mut f = fixture();
    let book = book(&mut f);
    f.connection.execute_batch("INSERT INTO sessions(id,name) VALUES(99,'old'); INSERT INTO message_packs(session_id,name,context) VALUES(99,'entry','native original');").unwrap();
    f.connection
        .execute(
            "UPDATE ccp_workspaces SET legacy_session_id=99 WHERE id=?1",
            [f.workspace.to_string()],
        )
        .unwrap();
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    error(
        store.mutate_entry(
            Actor::Admin(f.admin),
            f.workspace,
            &create(book, "entry", "x"),
            NOW,
        ),
        ContentError::LegacyConversionRequired,
    );
    error(
        store.entries(Actor::Admin(f.admin), f.workspace, &page(book, false), NOW),
        ContentError::LegacyConversionRequired,
    );
    error(
        store.entries(
            Actor::Admin(f.other_owner),
            f.workspace,
            &page(book, false),
            NOW,
        ),
        ContentError::Forbidden,
    );
    assert_eq!(rows(&f.connection, "ccp_content_entries"), 0);
    let text: String = f
        .connection
        .query_row(
            "SELECT context FROM message_packs WHERE session_id=99",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(text, "native original");
}
