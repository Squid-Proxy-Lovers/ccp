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

#[tokio::test]
async fn startup_finalizes_all_populated_sessions() {
    let ctx = TestContext::new("multi-session-bulk-load").await.unwrap();
    let mut session_ids = vec![ctx.session_id];
    let connection = open_sqlite_connection().unwrap();
    for name in ["second-bulk-session", "third-bulk-session"] {
        connection
            .execute("INSERT INTO sessions (name) VALUES (?1)", [name])
            .unwrap();
        let session_id = connection.last_insert_rowid();
        let mut metadata = ctx.state.sessions.read().await[&ctx.session_id]
            .metadata
            .clone();
        metadata.session_id = session_id;
        metadata.session_name = name.into();
        ctx.state
            .sessions
            .write()
            .await
            .insert(session_id, SessionCache::new(metadata, true));
        session_ids.push(session_id);
    }
    for &session_id in &session_ids {
        apply_multi_session_records(&ctx.state, session_id, "sqlite", false).await;
    }
    ctx.state.persist_snapshot_to_sqlite().await.unwrap();
    let deferred = ServerState::load_from_storage(Arc::clone(&ctx.journal))
        .await
        .unwrap();
    // Verify every SQLite-loaded session and warm real queries before replay.
    assert_multi_session_results(&deferred, &ctx.state, &session_ids).await;
    for &session_id in &session_ids {
        apply_multi_session_records(&ctx.state, session_id, "journal", false).await;
        apply_multi_session_records(&deferred, session_id, "journal", true).await;
        for record in multi_session_records(session_id, "journal") {
            ctx.journal.append(record).unwrap();
        }
    }
    // Startup caches begin empty. This warmed mirror exercises invalidation
    // after deferred replay, without adding a production-only test hook.
    for (&session_id, session) in deferred.sessions.write().await.iter_mut() {
        assert_eq!(session.entry_query_cache.len(), 2, "session {session_id}");
        assert_eq!(session.context_query_cache.len(), 1, "session {session_id}");
        session.finish_bulk_load();
        assert!(
            session.entry_query_cache.is_empty(),
            "session {session_id}: entry queries survive finalization"
        );
        assert!(
            session.context_query_cache.is_empty(),
            "session {session_id}: context queries survive finalization"
        );
    }
    assert_multi_session_results(&deferred, &ctx.state, &session_ids).await;
    ctx.journal.shutdown().unwrap();
    let journal = Arc::new(JournalHandle::start(ctx.journal.path().to_owned()).unwrap());
    let loaded = ServerState::load_from_storage(journal).await.unwrap();
    // Exercise the actual SQLite-plus-journal startup loop independently of
    // the mirror, including repeated AddEntry records for the replaced entry.
    assert_multi_session_results(&loaded, &ctx.state, &session_ids).await;
    loaded.journal.shutdown().unwrap();
}

fn multi_session_records(session_id: i64, phase: &str) -> Vec<JournalEntry> {
    let mut records = Vec::new();
    // Reverse insertion order and repeated names across books expose sorting
    // and path-key mistakes. Every session has distinct library and entry names.
    for (shelf_suffix, book_suffix, name_suffix) in
        [("z", "z", "z"), ("a", "z", "z"), ("a", "a", "a")]
    {
        let shelf_name = format!("shelf-{session_id}-{shelf_suffix}");
        let book_name = format!("book-{session_id}-{book_suffix}");
        records.push(JournalEntry::AddShelf {
            session_id,
            shelf_name: shelf_name.clone(),
            description: format!("initial shelf {phase}"),
        });
        records.push(JournalEntry::AddBook {
            session_id,
            shelf_name: shelf_name.clone(),
            book_name: book_name.clone(),
            description: format!("initial book {phase}"),
        });
        records.push(JournalEntry::AddEntry {
            session_id,
            name: format!("{phase}-{session_id}-{name_suffix}"),
            description: format!("{phase} entry"),
            labels: vec![format!("label-{session_id}")],
            context: format!("{phase} context {session_id}"),
            shelf_name,
            book_name,
            created_at: "1".into(),
            updated_at: "1".into(),
        });
    }
    let mut replacement = records.last().unwrap().clone();
    if let JournalEntry::AddEntry {
        description,
        context,
        ..
    } = &mut replacement
    {
        description.push_str(" replacement");
        context.push_str(" replacement");
    }
    records.push(replacement);
    for (shelf_suffix, book_suffix) in [("z", "z"), ("a", "z"), ("a", "a")] {
        let shelf_name = format!("shelf-{session_id}-{shelf_suffix}");
        records.push(JournalEntry::AddShelf {
            session_id,
            shelf_name: shelf_name.clone(),
            description: format!("quasar-{session_id} {phase}"),
        });
        records.push(JournalEntry::AddBook {
            session_id,
            shelf_name,
            book_name: format!("book-{session_id}-{book_suffix}"),
            description: format!("nebula-{session_id} {phase}"),
        });
    }
    records
}

async fn apply_multi_session_records(
    state: &ServerState,
    session_id: i64,
    phase: &str,
    deferred: bool,
) {
    let mut sessions = state.sessions.write().await;
    let session = sessions.get_mut(&session_id).unwrap();
    for record in multi_session_records(session_id, phase) {
        match record {
            JournalEntry::AddShelf {
                shelf_name,
                description,
                ..
            } => {
                session.add_shelf(&shelf_name, Some(&description));
                if !deferred {
                    session.refresh_library_metadata(&shelf_name, None);
                }
            }
            JournalEntry::AddBook {
                shelf_name,
                book_name,
                description,
                ..
            } => {
                session
                    .add_book(&shelf_name, &book_name, Some(&description))
                    .unwrap();
                if !deferred {
                    session.refresh_library_metadata(&shelf_name, Some(&book_name));
                }
            }
            JournalEntry::AddEntry {
                name,
                shelf_name,
                book_name,
                description,
                labels,
                context,
                created_at,
                updated_at,
                ..
            } => {
                let (path, key) = entry_path_for(&name, Some(&shelf_name), Some(&book_name));
                let entry = session.build_entry(
                    path,
                    description,
                    labels,
                    context,
                    created_at,
                    updated_at,
                    Vec::new(),
                );
                if deferred {
                    session.insert_entry_deferred(key, entry);
                } else {
                    session.insert_entry(key, entry);
                }
            }
            _ => panic!("unexpected multi-session fixture record"),
        }
    }
}

async fn assert_multi_session_results(
    actual: &ServerState,
    expected: &ServerState,
    session_ids: &[i64],
) {
    assert_eq!(actual.sessions.read().await.len(), 3);
    for &session_id in session_ids {
        let actor = writer(session_id, "writer");
        let entries = actual.list_entries(session_id, &actor).await.unwrap();
        assert_eq!(
            entries,
            expected.list_entries(session_id, &actor).await.unwrap(),
            "session {session_id}"
        );
        let keys = entries
            .iter()
            .map(|entry| (&entry.shelf_name, &entry.book_name, &entry.name))
            .collect::<Vec<_>>();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted, "session {session_id}: entry order");
        for query in [
            format!("quasar-{session_id}"),
            format!("nebula-{session_id}"),
        ] {
            let results = expected
                .search_entries(session_id, &actor, &query)
                .await
                .unwrap();
            assert_eq!(
                results.len(),
                entries.len(),
                "session {session_id}: description query {query}"
            );
            assert_eq!(
                actual
                    .search_entries(session_id, &actor, &query)
                    .await
                    .unwrap(),
                results,
                "session {session_id}: entry query {query}"
            );
        }
        let results = expected
            .search_context(session_id, &actor, "replacement")
            .await
            .unwrap();
        assert!(
            !results.is_empty(),
            "session {session_id}: replaced context"
        );
        assert_eq!(
            actual
                .search_context(session_id, &actor, "replacement")
                .await
                .unwrap(),
            results,
            "session {session_id}: context query"
        );
        let actual = actual.sessions.read().await;
        let expected = expected.sessions.read().await;
        assert_session_caches_equal(&actual[&session_id], &expected[&session_id]);
        assert_eq!(
            actual[&session_id]
                .shelf_entry_counts
                .values()
                .sum::<usize>(),
            entries.len()
        );
    }
}
