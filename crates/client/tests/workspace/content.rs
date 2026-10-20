// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use client::workspace::content as api;
use server::foundation::{Actor, EntryChange, EntryContent, EntryMutation, NodeDraft};

mod create;
mod lifecycle;
mod mutation;

struct Records {
    shelf: String,
    book: String,
    entry: String,
}

fn records(f: &Fixture, info: &client::workspace::ProfileInfo) -> Records {
    let mut db = f.db();
    let mut store = AccessStore::new(&mut db).unwrap();
    let actor = Actor::Credential(Uuid::parse_str(info.credential_id.as_ref().unwrap()).unwrap());
    let draft = NodeDraft {
        name: "notes".into(),
        description: "exact\nmetadata".into(),
    };
    let shelf = store
        .create_shelf(actor, f.setup.workspace_id, &draft, clock())
        .unwrap();
    let book = store
        .create_book(actor, f.setup.workspace_id, shelf.id, &draft, clock())
        .unwrap();
    let content = EntryContent {
        text: "original".into(),
        description: "description".into(),
        labels: vec!["label".into()],
    };
    let created = store
        .mutate_entry(
            actor,
            f.setup.workspace_id,
            &EntryMutation {
                operation_id: Uuid::new_v4(),
                agent: Some("agent".into()),
                change: EntryChange::Create {
                    book_id: book.id,
                    name: "entry".into(),
                    content,
                },
            },
            clock(),
        )
        .unwrap();
    store
        .mutate_entry(
            actor,
            f.setup.workspace_id,
            &EntryMutation {
                operation_id: Uuid::new_v4(),
                agent: Some("other agent".into()),
                change: EntryChange::Append {
                    id: created.entry_id,
                    expected_revision: 1,
                    text: "+contribution".into(),
                },
            },
            clock(),
        )
        .unwrap();
    Records {
        shelf: shelf.id.to_string(),
        book: book.id.to_string(),
        entry: created.entry_id.to_string(),
    }
}

fn entry_request(f: &Fixture, r: &Records) -> api::EntryTarget {
    api::EntryTarget {
        workspace_id: f.setup.workspace_id.to_string(),
        entry_id: r.entry.clone(),
        revision: None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn typed_library_reads_all_content_routes_and_keeps_profile_and_choices_unchanged() {
    let f = Fixture::new().await;
    let info = enroll_profile(&f).await;
    let r = records(&f, &info);
    let mut p = WorkspaceProfile::open(&f.profile).unwrap();
    let directory = f.root.join("caller");
    let selection = p.initialize_selection(&directory).unwrap();
    let shelves = p
        .content_shelves(&api::ShelfPage {
            workspace_id: f.setup.workspace_id.to_string(),
            after: None,
            limit: Some(1),
        })
        .await
        .unwrap();
    assert_eq!(shelves.items[0].id, r.shelf);
    assert!(shelves.next_cursor.is_none());
    let shelf = p
        .content_shelf(&api::NodeTarget {
            workspace_id: f.setup.workspace_id.to_string(),
            node_id: r.shelf.clone(),
        })
        .await
        .unwrap();
    assert_eq!(shelf.description, "exact\nmetadata");
    let books = p
        .content_books(&api::BookPage {
            workspace_id: f.setup.workspace_id.to_string(),
            shelf_id: r.shelf.clone(),
            after: None,
            limit: None,
        })
        .await
        .unwrap();
    assert_eq!(books.items[0].id, r.book);
    let book = p
        .content_book(&api::NodeTarget {
            workspace_id: f.setup.workspace_id.to_string(),
            node_id: r.book.clone(),
        })
        .await
        .unwrap();
    assert_eq!(book.shelf_id, r.shelf);
    let snapshot = p.content_entry(&entry_request(&f, &r)).await.unwrap();
    assert_eq!(snapshot.content.text, "original+contribution");
    assert_eq!(snapshot.entry.owner_client_id, info.client_id);
    let mut request = entry_request(&f, &r);
    request.revision = Some(1);
    let old = p.content_entry(&request).await.unwrap();
    assert_eq!(old.content.text, "original");
    assert_eq!(old.entry.revision, 2);
    assert_eq!(old.revision.revision, 1);
    let entries = p
        .content_entries(&api::EntryPage {
            workspace_id: f.setup.workspace_id.to_string(),
            book_id: r.book.clone(),
            archived: false,
            after: None,
            limit: None,
        })
        .await
        .unwrap();
    assert_eq!(entries.items[0].id, r.entry);
    let history = p
        .content_history(&api::HistoryPage {
            workspace_id: f.setup.workspace_id.to_string(),
            entry_id: r.entry.clone(),
            after: None,
            limit: Some(1),
        })
        .await
        .unwrap();
    assert_eq!(history.items[0].action, "create");
    assert_eq!(history.next_cursor, Some(1));
    let tail = p
        .content_history(&api::HistoryPage {
            workspace_id: f.setup.workspace_id.to_string(),
            entry_id: r.entry.clone(),
            after: Some(1),
            limit: Some(1),
        })
        .await
        .unwrap();
    assert_eq!(tail.items[0].action, "append");
    assert!(tail.next_cursor.is_none());
    assert_eq!(p.info().unwrap().credential_id, info.credential_id);
    assert_eq!(
        serde_json::to_value(p.selection(&directory).unwrap()).unwrap(),
        serde_json::to_value(selection).unwrap()
    );
    drop(p);
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn actual_cli_exposes_all_seven_explicit_content_queries_and_refuses_invalid_targets() {
    let f = Fixture::new().await;
    let info = enroll_profile(&f).await;
    let r = records(&f, &info);
    let workspace = f.setup.workspace_id.to_string();
    for (name, extra) in [
        ("shelves", vec![]),
        ("shelf", vec!["--node-id", r.shelf.as_str()]),
        ("books", vec!["--shelf-id", r.shelf.as_str()]),
        ("book", vec!["--node-id", r.book.as_str()]),
        ("entry", vec!["--entry-id", r.entry.as_str()]),
        ("entries", vec!["--book-id", r.book.as_str()]),
        ("history", vec!["--entry-id", r.entry.as_str()]),
    ] {
        let mut args = vec![
            "workspace",
            "content",
            "--profile",
            f.profile.to_str().unwrap(),
            name,
            "--workspace-id",
            workspace.as_str(),
        ];
        args.extend(extra);
        let result = value(f.cli(&args).await);
        assert!(result.is_object());
        if name == "entry" {
            assert_eq!(result["content"]["text"], "original+contribution");
        }
    }
    let bad = f
        .cli(&[
            "workspace",
            "content",
            "--profile",
            f.profile.to_str().unwrap(),
            "entry",
            "--workspace-id",
            "invalid",
            "--entry-id",
            r.entry.as_str(),
        ])
        .await;
    assert!(!bad.status.success());
    assert!(bad.stdout.is_empty());
    assert!(!String::from_utf8_lossy(&bad.stderr).contains("original"));
    let missing = f
        .cli(&[
            "workspace",
            "content",
            "--profile",
            f.profile.to_str().unwrap(),
            "shelves",
        ])
        .await;
    assert!(!missing.status.success());
    assert!(missing.stdout.is_empty());
    assert_eq!(f.count("ccp_clients"), 1);
    assert_eq!(f.count("ccp_credentials"), 1);
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn large_snapshots_and_archived_summaries_preserve_exact_data_and_never_preview_archived_bodies()
 {
    let f = Fixture::new().await;
    let info = enroll_profile(&f).await;
    let r = records(&f, &info);
    let actor = Actor::Credential(Uuid::parse_str(info.credential_id.as_ref().unwrap()).unwrap());
    let id = Uuid::parse_str(&r.entry).unwrap();
    let text = "\0".repeat(256 * 1024 - 1);
    {
        let mut db = f.db();
        let mut store = AccessStore::new(&mut db).unwrap();
        store
            .mutate_entry(
                actor,
                f.setup.workspace_id,
                &EntryMutation {
                    operation_id: Uuid::new_v4(),
                    agent: None,
                    change: EntryChange::Revise {
                        id,
                        expected_revision: 2,
                        content: EntryContent {
                            text: String::new(),
                            description: String::new(),
                            labels: vec![],
                        },
                    },
                },
                clock(),
            )
            .unwrap();
        store
            .mutate_entry(
                actor,
                f.setup.workspace_id,
                &EntryMutation {
                    operation_id: Uuid::new_v4(),
                    agent: None,
                    change: EntryChange::Append {
                        id,
                        expected_revision: 3,
                        text: text.clone(),
                    },
                },
                clock(),
            )
            .unwrap();
    }
    let mut p = WorkspaceProfile::open(&f.profile).unwrap();
    let full = p.content_entry(&entry_request(&f, &r)).await.unwrap();
    assert_eq!(full.content.text, text);
    assert_eq!(full.appended_text, Some(text));
    {
        let mut db = f.db();
        AccessStore::new(&mut db)
            .unwrap()
            .mutate_entry(
                actor,
                f.setup.workspace_id,
                &EntryMutation {
                    operation_id: Uuid::new_v4(),
                    agent: None,
                    change: EntryChange::Archive {
                        id,
                        expected_revision: 4,
                    },
                },
                clock(),
            )
            .unwrap();
    }
    assert!(p.content_entry(&entry_request(&f, &r)).await.is_err());
    let mut old = entry_request(&f, &r);
    old.revision = Some(1);
    assert!(p.content_entry(&old).await.is_err());
    assert!(
        p.content_history(&api::HistoryPage {
            workspace_id: f.setup.workspace_id.to_string(),
            entry_id: r.entry.clone(),
            after: None,
            limit: None
        })
        .await
        .is_err()
    );
    let archived = p
        .content_entries(&api::EntryPage {
            workspace_id: f.setup.workspace_id.to_string(),
            book_id: r.book.clone(),
            archived: true,
            after: None,
            limit: None,
        })
        .await
        .unwrap();
    assert_eq!(archived.items[0].id, r.entry);
    assert!(archived.items[0].archived);
    let serialized = serde_json::to_value(archived).unwrap();
    assert!(serialized["items"][0].get("content").is_none());
    drop(p);
    f.finish().await;
}
