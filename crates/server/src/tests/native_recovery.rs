// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;

async fn add(ctx: &TestContext, name: &str, text: &str) {
    ctx.state
        .add_entry(
            ctx.session_id,
            name,
            "",
            &[],
            text,
            "main",
            "default",
            &writer(ctx.session_id, "writer"),
        )
        .await
        .unwrap();
}
async fn reopen(ctx: &TestContext) -> ServerState {
    ctx.journal.shutdown().unwrap();
    let journal = Arc::new(JournalHandle::start(ctx.journal.path().to_owned()).unwrap());
    ServerState::load_from_storage(journal).await.unwrap()
}

#[tokio::test]
async fn unrelated_recovery_survives_delete_without_resurrecting_target() {
    let ctx = TestContext::new("delete-recovery").await.unwrap();
    add(&ctx, "removed", "target").await;
    ctx.state.persist_snapshot_to_sqlite().await.unwrap();
    ctx.journal.checkpoint_blocking().unwrap();
    add(&ctx, "kept", "before").await;
    ctx.state
        .append_to_entry(
            ctx.session_id,
            "kept",
            None,
            None,
            &writer(ctx.session_id, "writer"),
            "+after",
            AppendMetadata {
                agent_name: None,
                host_name: None,
                reason: None,
            },
        )
        .await
        .unwrap();
    ctx.state
        .delete_entry(
            ctx.session_id,
            "removed",
            None,
            None,
            &writer(ctx.session_id, "writer"),
        )
        .await
        .unwrap();
    let loaded = reopen(&ctx).await;
    let kept = loaded
        .get_entry(
            ctx.session_id,
            "kept",
            None,
            None,
            &writer(ctx.session_id, "writer"),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(kept.context, "before\n+after");
    assert!(
        loaded
            .get_entry(
                ctx.session_id,
                "removed",
                None,
                None,
                &writer(ctx.session_id, "writer")
            )
            .await
            .unwrap()
            .is_none()
    );
    loaded.journal.shutdown().unwrap();
}

#[tokio::test]
async fn snapshot_with_retained_records_replays_once() {
    let ctx = TestContext::new("snapshot-replay").await.unwrap();
    add(&ctx, "entry", "before").await;
    ctx.state
        .append_to_entry(
            ctx.session_id,
            "entry",
            None,
            None,
            &writer(ctx.session_id, "writer"),
            "+after",
            AppendMetadata {
                agent_name: None,
                host_name: None,
                reason: None,
            },
        )
        .await
        .unwrap();
    ctx.state.persist_snapshot_to_sqlite().await.unwrap();
    let loaded = reopen(&ctx).await;
    let entry = loaded
        .get_entry(
            ctx.session_id,
            "entry",
            None,
            None,
            &writer(ctx.session_id, "writer"),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(entry.context, "before\n+after");
    assert_eq!(
        loaded
            .get_history(
                ctx.session_id,
                "entry",
                None,
                None,
                &writer(ctx.session_id, "writer")
            )
            .await
            .unwrap()
            .len(),
        1
    );
    loaded.journal.shutdown().unwrap();
}

#[tokio::test]
async fn failed_precheckpoint_preserves_target_and_journal() {
    let ctx = TestContext::new("checkpoint-failure").await.unwrap();
    add(&ctx, "entry", "before").await;
    ctx.journal.shutdown().unwrap();
    let before = fs::read(ctx.journal.path()).unwrap();
    let error = ctx
        .state
        .delete_entry(
            ctx.session_id,
            "entry",
            None,
            None,
            &writer(ctx.session_id, "writer"),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("journal writer"));
    assert_eq!(fs::read(ctx.journal.path()).unwrap(), before);
    assert_eq!(
        ctx.state
            .get_entry(
                ctx.session_id,
                "entry",
                None,
                None,
                &writer(ctx.session_id, "writer")
            )
            .await
            .unwrap()
            .unwrap()
            .context,
        "before"
    );
    let journal = Arc::new(JournalHandle::start(ctx.journal.path().to_owned()).unwrap());
    let loaded = ServerState::load_from_storage(journal).await.unwrap();
    assert_eq!(
        loaded
            .get_entry(
                ctx.session_id,
                "entry",
                None,
                None,
                &writer(ctx.session_id, "writer")
            )
            .await
            .unwrap()
            .unwrap()
            .context,
        "before"
    );
    loaded.journal.shutdown().unwrap();
}

#[tokio::test]
async fn repeated_and_foreign_revocations_preserve_denial() {
    let ctx = TestContext::new("repeat-revoke").await.unwrap();
    ctx.state
        .record_issued_cert(
            ctx.session_id,
            "peer",
            "read_write",
            "test-certificate",
            "4102444800",
        )
        .await
        .unwrap();
    let peer = writer(ctx.session_id, "peer");
    ctx.state.ensure_ping_access(&peer).await.unwrap();
    ctx.state
        .revoke_client_cert(ctx.session_id, &admin(ctx.session_id, "writer"), "peer")
        .await
        .unwrap();
    for session in [ctx.session_id, ctx.session_id + 1] {
        assert!(
            ctx.state
                .revoke_client_cert(session, &admin(session, "writer"), "peer")
                .await
                .unwrap_err()
                .to_string()
                .contains("not found")
        );
        assert!(ctx.state.ensure_ping_access(&peer).await.is_err());
    }
    let loaded = reopen(&ctx).await;
    assert!(loaded.ensure_ping_access(&peer).await.is_err());
    assert!(!loaded.cert_grants.read().await.contains_key("peer"));
    loaded.persist_snapshot_to_sqlite().await.unwrap();
    loaded.journal.shutdown().unwrap();
}

#[tokio::test]
async fn concurrent_revocations_have_one_success_and_keep_the_target_denied() {
    let ctx = TestContext::new("concurrent-revoke").await.unwrap();
    ctx.state
        .record_issued_cert(
            ctx.session_id,
            "peer",
            "read_write",
            "test-certificate",
            "4102444800",
        )
        .await
        .unwrap();
    let actor = admin(ctx.session_id, "writer");
    let (first, second) = tokio::join!(
        ctx.state.revoke_client_cert(ctx.session_id, &actor, "peer"),
        ctx.state.revoke_client_cert(ctx.session_id, &actor, "peer")
    );
    assert_ne!(first.is_ok(), second.is_ok());
    assert!(
        ctx.state
            .ensure_ping_access(&writer(ctx.session_id, "peer"))
            .await
            .is_err()
    );
    assert!(!ctx.state.cert_grants.read().await.contains_key("peer"));
}

#[tokio::test]
async fn failed_revocation_restores_grant_without_clearing_a_prior_mark() {
    let ctx = TestContext::new("revoke-failure").await.unwrap();
    ctx.state
        .record_issued_cert(
            ctx.session_id,
            "peer",
            "read_write",
            "test-certificate",
            "4102444800",
        )
        .await
        .unwrap();
    open_sqlite_connection().unwrap().execute_batch("CREATE TRIGGER refuse_revoke BEFORE INSERT ON issued_client_certs BEGIN SELECT RAISE(ABORT,'forced revocation failure'); END;").unwrap();
    let actor = admin(ctx.session_id, "writer");
    let peer = writer(ctx.session_id, "peer");
    assert!(
        ctx.state
            .revoke_client_cert(ctx.session_id, &actor, "peer")
            .await
            .is_err()
    );
    assert!(ctx.state.cert_grants.read().await.contains_key("peer"));
    ctx.state.ensure_ping_access(&peer).await.unwrap();
    ctx.state
        .revoked_cert_common_names
        .write()
        .await
        .insert("peer".into());
    assert!(
        ctx.state
            .revoke_client_cert(ctx.session_id, &actor, "peer")
            .await
            .is_err()
    );
    assert!(ctx.state.cert_grants.read().await.contains_key("peer"));
    assert!(ctx.state.ensure_ping_access(&peer).await.is_err());
}

#[tokio::test]
async fn legacy_repeated_add_records_keep_the_last_replayed_value() {
    let ctx = TestContext::new("legacy-replay").await.unwrap();
    for text in ["first generation", "second generation"] {
        ctx.journal
            .append(JournalEntry::AddEntry {
                session_id: ctx.session_id,
                name: "entry".into(),
                description: String::new(),
                labels: vec![],
                context: text.into(),
                shelf_name: "main".into(),
                book_name: "default".into(),
                created_at: "1".into(),
                updated_at: "1".into(),
            })
            .unwrap();
    }
    let loaded = reopen(&ctx).await;
    assert_eq!(
        loaded
            .get_entry(
                ctx.session_id,
                "entry",
                None,
                None,
                &writer(ctx.session_id, "writer")
            )
            .await
            .unwrap()
            .unwrap()
            .context,
        "second generation"
    );
    loaded.journal.shutdown().unwrap();
}

#[tokio::test]
async fn shelf_snapshot_failure_restores_memory_and_sql() {
    let ctx = TestContext::new("shelf-rollback").await.unwrap();
    add(&ctx, "entry", "preserved").await;
    open_sqlite_connection().unwrap().execute_batch("CREATE TRIGGER refuse_deleted_shelf BEFORE DELETE ON message_packs WHEN NOT EXISTS(SELECT 1 FROM shelves WHERE shelf_name='main') BEGIN SELECT RAISE(ABORT,'forced shelf failure'); END;").unwrap();
    assert!(
        ctx.state
            .delete_shelf(ctx.session_id, "main", &writer(ctx.session_id, "writer"))
            .await
            .is_err()
    );
    assert_eq!(
        ctx.state
            .get_entry(
                ctx.session_id,
                "entry",
                None,
                None,
                &writer(ctx.session_id, "writer")
            )
            .await
            .unwrap()
            .unwrap()
            .context,
        "preserved"
    );
    let loaded = reopen(&ctx).await;
    assert_eq!(
        loaded
            .get_entry(
                ctx.session_id,
                "entry",
                None,
                None,
                &writer(ctx.session_id, "writer")
            )
            .await
            .unwrap()
            .unwrap()
            .context,
        "preserved"
    );
    loaded.journal.shutdown().unwrap();
}

#[tokio::test]
async fn ordered_writer_checkpoint_retains_only_later_records() {
    let ctx = TestContext::new("writer-barrier").await.unwrap();
    ctx.journal
        .append(JournalEntry::AddShelf {
            session_id: ctx.session_id,
            shelf_name: "before".into(),
            description: String::new(),
        })
        .unwrap();
    ctx.journal.checkpoint_blocking().unwrap();
    assert!(load_entries(ctx.journal.path()).unwrap().is_empty());
    ctx.journal
        .append(JournalEntry::AddShelf {
            session_id: ctx.session_id,
            shelf_name: "after".into(),
            description: String::new(),
        })
        .unwrap();
    ctx.journal.shutdown().unwrap();
    let entries = load_entries(ctx.journal.path()).unwrap();
    assert_eq!(entries.len(), 1);
    assert!(matches!(
        &entries[0],
        JournalEntry::AddShelf { shelf_name, .. } if shelf_name == "after"
    ));
}

#[tokio::test]
async fn append_before_and_after_checkpoint_remains_recoverable() {
    for deletion_first in [false, true] {
        let ctx = TestContext::new("append-checkpoint-order").await.unwrap();
        add(&ctx, "removed", "target").await;
        add(&ctx, "kept", "before").await;
        let actor = writer(ctx.session_id, "writer");
        let append = ctx.state.append_to_entry(
            ctx.session_id,
            "kept",
            None,
            None,
            &actor,
            "after",
            AppendMetadata {
                agent_name: None,
                host_name: None,
                reason: None,
            },
        );
        let delete = ctx
            .state
            .delete_entry(ctx.session_id, "removed", None, None, &actor);
        if deletion_first {
            let (a, b) = tokio::join!(delete, append);
            a.unwrap();
            b.unwrap();
        } else {
            let (a, b) = tokio::join!(append, delete);
            a.unwrap();
            b.unwrap();
        }
        let loaded = reopen(&ctx).await;
        assert_eq!(
            loaded
                .get_entry(ctx.session_id, "kept", None, None, &actor)
                .await
                .unwrap()
                .unwrap()
                .context,
            "before\nafter"
        );
        assert_eq!(
            loaded
                .get_history(ctx.session_id, "kept", None, None, &actor)
                .await
                .unwrap()
                .len(),
            1
        );
        assert!(
            loaded
                .get_entry(ctx.session_id, "removed", None, None, &actor)
                .await
                .unwrap()
                .is_none()
        );
        loaded.journal.shutdown().unwrap();
    }
}

#[tokio::test]
async fn panic_snapshot_declines_while_any_mutation_is_in_progress() {
    let ctx = TestContext::new("snapshot-gate").await.unwrap();
    let mutation = ctx.state.native_checkpoint.read().await;
    assert!(!ctx.state.try_persist_snapshot_to_sqlite().unwrap());
    drop(mutation);
    assert!(ctx.state.try_persist_snapshot_to_sqlite().unwrap());
}

#[tokio::test]
async fn empty_legacy_operation_ids_do_not_collapse_distinct_appends() {
    let ctx = TestContext::new("legacy-empty-operations").await.unwrap();
    add(&ctx, "entry", "base").await;
    for text in ["first", "second"] {
        ctx.journal
            .append(JournalEntry::AppendEntry {
                session_id: ctx.session_id,
                name: "entry".into(),
                operation_id: String::new(),
                client_common_name: "writer".into(),
                agent_name: None,
                host_name: None,
                reason: None,
                appended_content: text.into(),
                shelf_name: "main".into(),
                book_name: "default".into(),
                created_at: "1".into(),
            })
            .unwrap();
    }
    let loaded = reopen(&ctx).await;
    let actor = writer(ctx.session_id, "writer");
    assert_eq!(
        loaded
            .get_entry(ctx.session_id, "entry", None, None, &actor)
            .await
            .unwrap()
            .unwrap()
            .context,
        "base\nfirst\nsecond"
    );
    assert_eq!(
        loaded
            .get_history(ctx.session_id, "entry", None, None, &actor)
            .await
            .unwrap()
            .len(),
        2
    );
    loaded.journal.shutdown().unwrap();
}

#[tokio::test]
async fn earlier_legacy_generation_cannot_append_into_a_newer_sql_snapshot() {
    let ctx = TestContext::new("generation-replay").await.unwrap();
    add(&ctx, "entry", "new generation").await;
    let actor = writer(ctx.session_id, "writer");
    ctx.state
        .append_to_entry(
            ctx.session_id,
            "entry",
            None,
            None,
            &actor,
            "new append",
            AppendMetadata {
                agent_name: None,
                host_name: None,
                reason: None,
            },
        )
        .await
        .unwrap();
    let operation = ctx
        .state
        .get_history(ctx.session_id, "entry", None, None, &actor)
        .await
        .unwrap()[0]
        .operation_id
        .clone();
    ctx.state.persist_snapshot_to_sqlite().await.unwrap();
    ctx.journal.checkpoint_blocking().unwrap();
    for (text, op, append) in [
        ("old generation", "", false),
        ("old append", "old-operation", true),
        ("new generation", "", false),
        ("new append", operation.as_str(), true),
    ] {
        let record = if append {
            JournalEntry::AppendEntry {
                session_id: ctx.session_id,
                name: "entry".into(),
                operation_id: op.into(),
                client_common_name: "writer".into(),
                agent_name: None,
                host_name: None,
                reason: None,
                appended_content: text.into(),
                shelf_name: "main".into(),
                book_name: "default".into(),
                created_at: "1".into(),
            }
        } else {
            JournalEntry::AddEntry {
                session_id: ctx.session_id,
                name: "entry".into(),
                description: String::new(),
                labels: vec![],
                context: text.into(),
                shelf_name: "main".into(),
                book_name: "default".into(),
                created_at: "1".into(),
                updated_at: "1".into(),
            }
        };
        ctx.journal.append(record).unwrap();
    }
    let loaded = reopen(&ctx).await;
    assert_eq!(
        loaded
            .get_entry(ctx.session_id, "entry", None, None, &actor)
            .await
            .unwrap()
            .unwrap()
            .context,
        "new generation\nnew append"
    );
    assert_eq!(
        loaded
            .get_history(ctx.session_id, "entry", None, None, &actor)
            .await
            .unwrap()
            .len(),
        1
    );
    loaded.journal.shutdown().unwrap();
}
