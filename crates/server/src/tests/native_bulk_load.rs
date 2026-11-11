// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;

enum Op {
    Shelf(&'static str, &'static str),
    Book(&'static str, &'static str, &'static str),
    Entry(
        String,
        Option<&'static str>,
        Option<&'static str>,
        &'static str,
    ),
}

fn bulk_load_ops() -> Vec<Op> {
    let mut ops = vec![
        Op::Shelf("main", "Main shelf"),
        Op::Book("main", "alpha", "Alpha book"),
    ];
    for index in 0..40 {
        ops.push(Op::Entry(
            format!("a-{index:02}"),
            Some("main"),
            Some("alpha"),
            "alpha entry",
        ));
    }
    ops.push(Op::Book("main", "beta", ""));
    for index in 0..15 {
        ops.push(Op::Entry(
            format!("b-{index:02}"),
            Some("main"),
            Some("beta"),
            "beta entry",
        ));
    }
    // Library changes after entries exist, a replaced entry and default-shelf entries.
    ops.push(Op::Shelf("main", "Main shelf, renamed"));
    ops.push(Op::Book("main", "alpha", "Alpha book v2"));
    ops.push(Op::Entry(
        "a-03".to_string(),
        Some("main"),
        Some("alpha"),
        "replacement",
    ));
    for index in 0..5 {
        ops.push(Op::Entry(
            format!("loose-{index}"),
            None,
            None,
            "default shelf",
        ));
    }
    ops.push(Op::Shelf("other", "Other shelf"));
    ops.push(Op::Entry(
        "o-1".to_string(),
        Some("other"),
        Some("missing-book"),
        "unknown book",
    ));
    ops
}

#[test]
fn bulk_load_matches_incremental_library_state() {
    let metadata = SessionMetadata {
        session_name: "bulk".to_string(),
        session_id: 1,
        description: String::new(),
        owner: String::new(),
        labels: Vec::new(),
        visibility: "public".to_string(),
        purpose: String::new(),
    };
    let mut incremental = SessionCache::new(metadata.clone(), true);
    let mut bulk = SessionCache::new(metadata, true);
    for op in &bulk_load_ops() {
        match op {
            Op::Shelf(shelf, description) => {
                incremental.add_shelf(shelf, Some(description));
                incremental.refresh_library_metadata(shelf, None);
                bulk.add_shelf(shelf, Some(description));
            }
            Op::Book(shelf, book, description) => {
                incremental
                    .add_book(shelf, book, Some(description))
                    .unwrap();
                incremental.refresh_library_metadata(shelf, Some(book));
                bulk.add_book(shelf, book, Some(description)).unwrap();
            }
            Op::Entry(name, shelf, book, description) => {
                for (session, deferred) in [(&mut incremental, false), (&mut bulk, true)] {
                    let (path, key) = entry_path_for(name, *shelf, *book);
                    let entry = session.build_entry(
                        path,
                        description.to_string(),
                        vec!["label".to_string()],
                        format!("context for {name}"),
                        "1".to_string(),
                        "1".to_string(),
                        Vec::new(),
                    );
                    if deferred {
                        session.insert_entry_deferred(key, entry);
                    } else {
                        session.insert_entry(key, entry);
                    }
                }
            }
        }
    }
    bulk.cache_entry_query("stale".into(), Vec::new());
    bulk.cache_context_query("stale".into(), Vec::new());
    bulk.finish_bulk_load();

    assert_session_caches_equal(&bulk, &incremental);
    assert!(bulk.entry_query_cache.is_empty());
    assert!(bulk.context_query_cache.is_empty());
    assert!(
        bulk.list_entries_cache
            .iter()
            .any(|summary| summary.book_description == "Alpha book v2")
    );
}

fn assert_session_caches_equal(bulk: &SessionCache, incremental: &SessionCache) {
    assert_eq!(bulk.list_entries_cache, incremental.list_entries_cache);
    assert_eq!(bulk.entries.len(), incremental.entries.len());
    for (key, expected) in &incremental.entries {
        let actual = &bulk.entries[key];
        assert_eq!(actual.summary, expected.summary, "summary for {key}");
        assert_eq!(message_entry_from(actual), message_entry_from(expected));
        assert_eq!(actual.history, expected.history);
        assert_eq!(
            actual.context_search.normalized_context,
            expected.context_search.normalized_context
        );
        assert_eq!(
            actual.context_search.fuzzy_tokens,
            expected.context_search.fuzzy_tokens
        );
        assert_eq!(
            actual.entry_search.literal_haystack, expected.entry_search.literal_haystack,
            "{key}"
        );
        assert_eq!(
            actual.entry_search.fuzzy_fields, expected.entry_search.fuzzy_fields,
            "{key}"
        );
        assert_eq!(
            actual.entry_search.fuzzy_tokens, expected.entry_search.fuzzy_tokens,
            "{key}"
        );
    }
    assert_eq!(bulk.shelf_entry_counts, incremental.shelf_entry_counts);
    assert_eq!(bulk.book_entry_counts, incremental.book_entry_counts);
    assert_eq!(bulk.shelf_book_counts, incremental.shelf_book_counts);
}

/// Mix a checkpointed delete/restore snapshot with journal-only additions,
/// metadata updates and duplicate append records. Startup must match live state.
#[tokio::test]
async fn bulk_load_preserves_native_recovery_state() {
    let ctx = TestContext::new("bulk-native-recovery").await.unwrap();
    let actor = writer(ctx.session_id, "writer");
    for name in ["sqlite", "removed", "restored"] {
        add_bulk_fixture_entry(&ctx, name).await;
    }
    let deleted = ctx
        .state
        .delete_entry(ctx.session_id, "restored", None, None, &actor)
        .await
        .unwrap();
    ctx.state
        .restore_deleted_entry(ctx.session_id, &deleted.entry_key, &actor)
        .await
        .unwrap();
    ctx.state
        .delete_entry(ctx.session_id, "removed", None, None, &actor)
        .await
        .unwrap();
    add_bulk_fixture_entry(&ctx, "journal").await;
    ctx.state
        .append_to_entry(
            ctx.session_id,
            "journal",
            None,
            None,
            &actor,
            "appended context",
            AppendMetadata {
                agent_name: Some("agent".into()),
                host_name: None,
                reason: Some("recovery".into()),
            },
        )
        .await
        .unwrap();
    ctx.state
        .add_shelf(ctx.session_id, "main", "updated shelf", &actor)
        .await
        .unwrap();
    ctx.state
        .add_book(ctx.session_id, "main", "default", "updated book", &actor)
        .await
        .unwrap();
    ctx.journal.shutdown().unwrap();
    let records = load_entries(ctx.journal.path()).unwrap();
    let journal = Arc::new(JournalHandle::start(ctx.journal.path().to_owned()).unwrap());
    for record in records {
        if matches!(record, JournalEntry::AppendEntry { .. }) {
            journal.append(record).unwrap();
        }
    }
    journal.shutdown().unwrap();
    let journal = Arc::new(JournalHandle::start(ctx.journal.path().to_owned()).unwrap());
    let loaded = ServerState::load_from_storage(journal).await.unwrap();
    let expected = ctx.state.sessions.read().await;
    let actual = loaded.sessions.read().await;
    let expected = &expected[&ctx.session_id];
    let actual = &actual[&ctx.session_id];
    assert_session_caches_equal(actual, expected);
    assert_eq!(actual.entries.len(), 3);
    assert!(!actual.entries.contains_key("main::default::removed"));
    assert!(actual.entries.contains_key("main::default::restored"));
    assert_eq!(actual.entries["main::default::journal"].history.len(), 1);
    assert!(
        actual
            .list_entries_cache
            .iter()
            .all(|entry| entry.shelf_description == "updated shelf"
                && entry.book_description == "updated book")
    );
    loaded.journal.shutdown().unwrap();
}

async fn add_bulk_fixture_entry(ctx: &TestContext, name: &str) {
    ctx.state
        .add_entry(
            ctx.session_id,
            name,
            "entry",
            &["label".into()],
            "context",
            "main",
            "default",
            &writer(ctx.session_id, "writer"),
        )
        .await
        .unwrap();
}
