// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

mod ledger;
mod lifecycle;
mod ownership;
mod persistence;
mod purge;
mod validation;

fn content(text: &str) -> EntryContent {
    EntryContent {
        text: text.into(),
        description: "exact\nmetadata".into(),
        labels: vec!["shared".into()],
    }
}

fn command(change: EntryChange) -> EntryMutation {
    EntryMutation {
        operation_id: Uuid::new_v4(),
        agent: Some("agent one".into()),
        change,
    }
}

fn book(f: &mut Fixture) -> Uuid {
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    let node = NodeDraft {
        name: "notes".into(),
        description: String::new(),
    };
    let shelf = store
        .create_shelf(Actor::Admin(f.owner), f.workspace, &node, NOW)
        .unwrap();
    store
        .create_book(Actor::Admin(f.owner), f.workspace, shelf.id, &node, NOW)
        .unwrap()
        .id
}

fn create(book_id: Uuid, name: &str, text: &str) -> EntryMutation {
    command(EntryChange::Create {
        book_id,
        name: name.into(),
        content: content(text),
    })
}

fn apply(f: &mut Fixture, actor: Actor, request: &EntryMutation) -> EntryReceipt {
    AccessStore::new(&mut f.connection)
        .unwrap()
        .mutate_entry(actor, f.workspace, request, NOW)
        .unwrap()
}

fn get(f: &mut Fixture, id: Uuid) -> EntrySnapshot {
    AccessStore::new(&mut f.connection)
        .unwrap()
        .get_entry(Actor::Admin(f.owner), f.workspace, id, NOW)
        .unwrap()
}

fn error<T>(result: anyhow::Result<T>, expected: ContentError) {
    let error = match result {
        Ok(_) => panic!("expected {expected:?}"),
        Err(e) => e,
    };
    assert_eq!(
        error.downcast_ref::<ContentError>(),
        Some(&expected),
        "{error:#}"
    );
}

fn fail(f: &mut Fixture, actor: Actor, request: &EntryMutation, expected: ContentError) {
    error(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .mutate_entry(actor, f.workspace, request, NOW),
        expected,
    );
}

fn page(book_id: Uuid, archived: bool) -> EntryPageRequest {
    EntryPageRequest {
        book_id,
        archived,
        after: None,
        limit: 100,
    }
}

mod resolution;
