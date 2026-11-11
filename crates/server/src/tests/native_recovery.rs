// SPDX-License-Identifier: AGPL-3.0-or-later
use std::time::Duration;

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
    tokio::time::timeout(Duration::from_secs(20), async {
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
    })
    .await
    .unwrap();
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
    tokio::time::timeout(Duration::from_secs(20), async {
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
    })
    .await
    .unwrap();
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

fn read_write_grant(session_id: i64) -> CertGrant {
    CertGrant {
        session_id,
        access_level: "read_write".to_string(),
        cert_pem: String::new(),
        created_at: "1".to_string(),
        expires_at: "4102444800".to_string(),
    }
}

#[tokio::test]
async fn revocation_is_idempotent_and_stays_effective() {
    let ctx = TestContext::new("revoke-idempotent").await.unwrap();
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
    let operator = admin(ctx.session_id, "writer");
    ctx.state.list_entries(ctx.session_id, &peer).await.unwrap();
    ctx.state
        .ensure_write_access(ctx.session_id, &peer)
        .await
        .unwrap();
    ctx.state.ensure_ping_access(&peer).await.unwrap();

    ctx.state
        .revoke_client_cert(ctx.session_id, &operator, "peer")
        .await
        .unwrap();
    assert!(ctx.state.list_entries(ctx.session_id, &peer).await.is_err());
    assert!(
        ctx.state
            .ensure_write_access(ctx.session_id, &peer)
            .await
            .is_err()
    );
    assert!(ctx.state.ensure_ping_access(&peer).await.is_err());

    assert!(
        ctx.state
            .revoke_client_cert(ctx.session_id, &operator, "peer")
            .await
            .unwrap_err()
            .to_string()
            .contains("not found")
    );
    assert!(ctx.state.list_entries(ctx.session_id, &peer).await.is_err());
    assert!(
        ctx.state
            .ensure_write_access(ctx.session_id, &peer)
            .await
            .is_err()
    );
    assert!(ctx.state.ensure_ping_access(&peer).await.is_err());
}

#[tokio::test]
async fn concurrent_admin_revokes_of_the_same_cert_always_end_denied() {
    tokio::time::timeout(Duration::from_secs(20), async {
        let ctx = TestContext::new("concurrent-admin-revoke").await.unwrap();
        let operator = admin(ctx.session_id, "writer");
        for round in 0..150 {
            let cn = format!("peer-{round}");
            ctx.state
                .cert_grants
                .write()
                .await
                .insert(cn.clone(), read_write_grant(ctx.session_id));
            let (first, second) = tokio::join!(
                ctx.state.revoke_client_cert(ctx.session_id, &operator, &cn),
                ctx.state.revoke_client_cert(ctx.session_id, &operator, &cn)
            );
            assert_ne!(first.is_ok(), second.is_ok());
            assert!(
                ctx.state
                    .ensure_ping_access(&writer(ctx.session_id, &cn))
                    .await
                    .is_err()
            );
            assert!(!ctx.state.cert_grants.read().await.contains_key(&cn));
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn admin_revoke_racing_target_self_revokes_always_ends_denied() {
    tokio::time::timeout(Duration::from_secs(20), async {
        let ctx = TestContext::new("admin-vs-self-revoke").await.unwrap();
        let operator = admin(ctx.session_id, "writer");
        for round in 0..150 {
            let cn = format!("peer-{round}");
            let target = writer(ctx.session_id, &cn);
            ctx.state
                .cert_grants
                .write()
                .await
                .insert(cn.clone(), read_write_grant(ctx.session_id));
            let (admin_result, self_one, self_two, self_three) = tokio::join!(
                ctx.state.revoke_client_cert(ctx.session_id, &operator, &cn),
                ctx.state.revoke_client_cert(ctx.session_id, &target, &cn),
                ctx.state.revoke_client_cert(ctx.session_id, &target, &cn),
                ctx.state.revoke_client_cert(ctx.session_id, &target, &cn),
            );
            let any_succeeded = [
                admin_result.is_ok(),
                self_one.is_ok(),
                self_two.is_ok(),
                self_three.is_ok(),
            ]
            .into_iter()
            .any(|succeeded| succeeded);
            assert!(any_succeeded, "one of the racing revokes should win");
            assert!(ctx.state.ensure_ping_access(&target).await.is_err());
            assert!(!ctx.state.cert_grants.read().await.contains_key(&cn));
        }

        // A later admin revoke, issued after the race, must still behave: idempotent
        // "not found" rather than an error that would suggest the lock or the
        // revocation set were left in a broken state.
        let cn = "after-the-race";
        ctx.state
            .cert_grants
            .write()
            .await
            .insert(cn.to_string(), read_write_grant(ctx.session_id));
        let target = writer(ctx.session_id, cn);
        ctx.state
            .revoke_client_cert(ctx.session_id, &target, cn)
            .await
            .unwrap();
        assert!(
            ctx.state
                .revoke_client_cert(ctx.session_id, &operator, cn)
                .await
                .unwrap_err()
                .to_string()
                .contains("not found")
        );
        assert!(ctx.state.ensure_ping_access(&target).await.is_err());
    })
    .await
    .unwrap();
}

#[tokio::test]
// Defence test: the native exclusive guard already serializes self-revokes.
// This verifies denial and the single winner, not the original lock-scope bug.
async fn concurrent_self_revokes_always_end_denied() {
    tokio::time::timeout(Duration::from_secs(20), async {
        let ctx = TestContext::new("self-revoke-race").await.unwrap();
        for round in 0..150 {
            let cn = format!("peer-{round}");
            let target = writer(ctx.session_id, &cn);
            ctx.state
                .cert_grants
                .write()
                .await
                .insert(cn.clone(), read_write_grant(ctx.session_id));
            let (first, second, third) = tokio::join!(
                ctx.state.revoke_client_cert(ctx.session_id, &target, &cn),
                ctx.state.revoke_client_cert(ctx.session_id, &target, &cn),
                ctx.state.revoke_client_cert(ctx.session_id, &target, &cn),
            );
            let successes = [first.is_ok(), second.is_ok(), third.is_ok()]
                .into_iter()
                .filter(|succeeded| *succeeded)
                .count();
            assert_eq!(successes, 1, "exactly one self-revoke should win the race");
            assert!(ctx.state.ensure_ping_access(&target).await.is_err());
            assert!(!ctx.state.cert_grants.read().await.contains_key(&cn));
        }
    })
    .await
    .unwrap();
}

async fn rollback_isolation_fixture(
    tag: &str,
) -> (Arc<ServerState>, Arc<JournalHandle>, i64, PathBuf) {
    let data_dir = std::env::temp_dir().join(format!("ccp-rollback-{tag}-{}", Uuid::new_v4()));
    unsafe {
        env::set_var(SERVER_DATA_DIR_ENV, &data_dir);
    }
    init_sqlite(&crate::init::db_path()).unwrap();
    let connection = open_sqlite_connection().unwrap();
    connection
        .execute(
            "INSERT INTO sessions (
                name, description, owner, labels, visibility, purpose, is_active, last_started_at
             ) VALUES ('rollback-session', 'd', 'o', '', 'private', 'p', 1, CURRENT_TIMESTAMP)",
            [],
        )
        .unwrap();
    let session_id: i64 = connection
        .query_row(
            "SELECT id FROM sessions WHERE name = 'rollback-session'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let journal = Arc::new(JournalHandle::start(crate::init::journal_path()).unwrap());
    let state = Arc::new(
        ServerState::load_from_storage(Arc::clone(&journal))
            .await
            .unwrap(),
    );
    (state, journal, session_id, data_dir)
}

/// Blocks `event` on `table` for rows matching `condition`, so the owning
/// command's SQLite persist step fails and rolls back its in-memory mutation.
fn poison_before(table: &str, event: &str, condition: &str) {
    open_sqlite_connection()
        .unwrap()
        .execute_batch(&format!(
            "CREATE TRIGGER sibling_rollback_poison BEFORE {event} ON {table}
             WHEN {condition}
             BEGIN SELECT RAISE(ABORT, 'injected persist failure'); END;"
        ))
        .unwrap();
}

/// Poll the sibling through its first lock attempt before releasing the final
/// persist. With exclusivity broken, these uncontended writes finish here.
async fn assert_sibling_still_parked_then_release<F: std::future::Future>(
    gate: &test_hooks::Gate,
    mut sibling: std::pin::Pin<&mut F>,
) {
    let pending =
        std::future::poll_fn(|cx| std::task::Poll::Ready(sibling.as_mut().poll(cx).is_pending()))
            .await;
    gate.release();
    assert!(
        pending,
        "sibling writes finished inside another mutation's rollback window"
    );
}

async fn finish<T>(task: tokio::task::JoinHandle<T>) -> T {
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
}

fn poison_bundle(session_id: i64) -> TransferBundle {
    let entries = vec![BundleEntry {
        name: "poison".to_string(),
        description: "imported".to_string(),
        labels: vec![],
        context: "IMPORTED".to_string(),
        shelf_name: "shelf".to_string(),
        book_name: "book".to_string(),
        shelf_description: String::new(),
        book_description: String::new(),
        history: vec![],
    }];
    let bundle_sha256: String = sha2::Sha256::digest(serde_json::to_vec(&entries).unwrap())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    TransferBundle {
        session: SessionMetadata {
            session_name: "rollback-session".to_string(),
            session_id,
            description: String::new(),
            owner: String::new(),
            labels: vec![],
            visibility: "private".to_string(),
            purpose: String::new(),
        },
        selector: TransferSelector {
            scope: TransferScope::Session,
            include_history: false,
        },
        exported_at: "0".to_string(),
        entries,
        bundle_sha256,
    }
}

async fn seed_shelf_book_and_anchor(
    state: &ServerState,
    session_id: i64,
    actor: &ConnectionAuthContext,
) {
    state
        .add_shelf(session_id, "shelf", "", actor)
        .await
        .unwrap();
    state
        .add_book(session_id, "shelf", "book", "", actor)
        .await
        .unwrap();
    state
        .add_entry(
            session_id,
            "anchor",
            "",
            &[],
            "ANCHOR",
            "shelf",
            "book",
            actor,
        )
        .await
        .unwrap();
}

/// A different client's add-book, add-entry and append, run while another
/// client's mutation is parked mid-rollback.
async fn sibling_writes(
    state: &ServerState,
    session_id: i64,
    actor: &ConnectionAuthContext,
) -> anyhow::Result<()> {
    state
        .add_book(
            session_id,
            "shelf",
            "concurrent-book",
            "CONCURRENT BOOK",
            actor,
        )
        .await?;
    state
        .add_entry(
            session_id,
            "sibling-entry",
            "",
            &[],
            "x",
            "shelf",
            "concurrent-book",
            actor,
        )
        .await?;
    state
        .append_to_entry(
            session_id,
            "anchor",
            Some("shelf"),
            Some("book"),
            actor,
            "ANCHOR-APPEND",
            AppendMetadata {
                agent_name: None,
                host_name: None,
                reason: None,
            },
        )
        .await?;
    Ok(())
}

async fn assert_sibling_writes_survived(
    state: &ServerState,
    session_id: i64,
    actor: &ConnectionAuthContext,
) {
    assert!(
        state
            .get_entry(
                session_id,
                "sibling-entry",
                Some("shelf"),
                Some("concurrent-book"),
                actor
            )
            .await
            .unwrap()
            .is_some(),
        "a sibling's concurrent add must survive another client's rolled-back mutation"
    );
    assert!(
        state
            .get_entry(session_id, "anchor", Some("shelf"), Some("book"), actor)
            .await
            .unwrap()
            .unwrap()
            .context
            .contains("ANCHOR-APPEND"),
        "a sibling's concurrent append must survive another client's rolled-back mutation"
    );
    assert!(
        state
            .search_books(session_id, actor, "concurrent-book")
            .await
            .unwrap()
            .iter()
            .any(|book| book.shelf_name == "shelf"
                && book.book_name == "concurrent-book"
                && book.description == "CONCURRENT BOOK"
                && book.entry_count == 1),
        "a sibling's concurrent book metadata must survive another client's \
         rolled-back mutation, not just the entry row under it"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[expect(
    clippy::await_holding_lock,
    reason = "Serializes process-global test environment; server tasks do not acquire this lock."
)]
async fn sibling_import_rollback_preserves_concurrent_writes() {
    tokio::time::timeout(Duration::from_secs(20), async {
        let _env = test_env_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (state, journal, session_id, data_dir) = rollback_isolation_fixture("import").await;
        let client_a = writer(session_id, "client-a");
        let client_b = writer(session_id, "client-b");
        seed_shelf_book_and_anchor(&state, session_id, &client_a).await;

        poison_before("message_packs", "INSERT", "NEW.name = 'poison'");
        let gate = test_hooks::arm("import_bundle::persist");

        let bundle = poison_bundle(session_id);
        let import = tokio::spawn({
            let state = Arc::clone(&state);
            let actor = client_a.clone();
            async move {
                state
                    .import_bundle(session_id, &bundle, &ConflictPolicy::Overwrite, &actor)
                    .await
            }
        });
        gate.wait_until_reached().await;
        assert!(
            state
                .get_entry(session_id, "poison", Some("shelf"), Some("book"), &client_a)
                .await
                .unwrap()
                .is_some()
        );

        let writes = sibling_writes(&state, session_id, &client_b);
        tokio::pin!(writes);
        assert_sibling_still_parked_then_release(&gate, writes.as_mut()).await;

        assert!(finish(import).await.is_err());
        assert!(
            state
                .get_entry(session_id, "poison", Some("shelf"), Some("book"), &client_a)
                .await
                .unwrap()
                .is_none()
        );
        writes.await.unwrap();
        assert_sibling_writes_survived(&state, session_id, &client_b).await;

        journal.shutdown().unwrap();
        #[expect(
            clippy::let_underscore_must_use,
            reason = "best-effort cleanup of a temporary test directory"
        )]
        let _ = std::fs::remove_dir_all(&data_dir);
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[expect(
    clippy::await_holding_lock,
    reason = "Serializes process-global test environment; server tasks do not acquire this lock."
)]
async fn sibling_restore_rollback_preserves_concurrent_writes() {
    tokio::time::timeout(Duration::from_secs(20), async {
        let _env = test_env_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (state, journal, session_id, data_dir) = rollback_isolation_fixture("restore").await;
        let client_a = writer(session_id, "client-a");
        let client_b = writer(session_id, "client-b");
        seed_shelf_book_and_anchor(&state, session_id, &client_a).await;
        state
            .add_entry(
                session_id,
                "victim",
                "",
                &[],
                "ORIGINAL",
                "shelf",
                "book",
                &client_a,
            )
            .await
            .unwrap();
        state.persist_snapshot_to_sqlite().await.unwrap();
        let deleted = state
            .delete_entry(session_id, "victim", Some("shelf"), Some("book"), &client_a)
            .await
            .unwrap();

        poison_before("message_packs", "INSERT", "NEW.name = 'victim'");
        let gate = test_hooks::arm("restore_deleted_entry::persist");

        let restore = tokio::spawn({
            let state = Arc::clone(&state);
            let actor = client_a.clone();
            let entry_key = deleted.entry_key.clone();
            async move {
                state
                    .restore_deleted_entry(session_id, &entry_key, &actor)
                    .await
            }
        });
        gate.wait_until_reached().await;
        assert!(
            state
                .get_entry(session_id, "victim", Some("shelf"), Some("book"), &client_a)
                .await
                .unwrap()
                .is_some()
        );

        let writes = sibling_writes(&state, session_id, &client_b);
        tokio::pin!(writes);
        assert_sibling_still_parked_then_release(&gate, writes.as_mut()).await;

        assert!(finish(restore).await.is_err());
        assert!(
            state
                .get_entry(session_id, "victim", Some("shelf"), Some("book"), &client_a)
                .await
                .unwrap()
                .is_none()
        );
        writes.await.unwrap();
        assert_sibling_writes_survived(&state, session_id, &client_b).await;

        journal.shutdown().unwrap();
        #[expect(
            clippy::let_underscore_must_use,
            reason = "best-effort cleanup of a temporary test directory"
        )]
        let _ = std::fs::remove_dir_all(&data_dir);
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[expect(
    clippy::await_holding_lock,
    reason = "Serializes process-global test environment; server tasks do not acquire this lock."
)]
async fn sibling_delete_rollback_preserves_concurrent_writes() {
    tokio::time::timeout(Duration::from_secs(20), async {
        let _env = test_env_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (state, journal, session_id, data_dir) = rollback_isolation_fixture("delete").await;
        let client_a = writer(session_id, "client-a");
        let client_b = writer(session_id, "client-b");
        seed_shelf_book_and_anchor(&state, session_id, &client_a).await;
        state
            .add_entry(
                session_id,
                "victim",
                "",
                &[],
                "ORIGINAL",
                "shelf",
                "book",
                &client_a,
            )
            .await
            .unwrap();
        state.persist_snapshot_to_sqlite().await.unwrap();

        // Poison the archive-table insert specific to persist_deleted_entry,
        // not the general message_packs table: checkpoint_before_mutation's
        // earlier full-snapshot resync also deletes from message_packs
        // before the in-memory removal happens, and would otherwise fail
        // the operation before it ever reached the rollback under test.
        poison_before("deleted_message_packs", "INSERT", "NEW.name = 'victim'");
        let gate = test_hooks::arm("delete_entry::persist");

        let delete = tokio::spawn({
            let state = Arc::clone(&state);
            let actor = client_a.clone();
            async move {
                state
                    .delete_entry(session_id, "victim", Some("shelf"), Some("book"), &actor)
                    .await
            }
        });
        gate.wait_until_reached().await;
        assert!(
            state
                .get_entry(session_id, "victim", Some("shelf"), Some("book"), &client_a)
                .await
                .unwrap()
                .is_none()
        );

        let writes = sibling_writes(&state, session_id, &client_b);
        tokio::pin!(writes);
        assert_sibling_still_parked_then_release(&gate, writes.as_mut()).await;

        assert!(finish(delete).await.is_err());
        assert!(
            state
                .get_entry(session_id, "victim", Some("shelf"), Some("book"), &client_a)
                .await
                .unwrap()
                .is_some(),
            "a failed delete persist must restore the entry it tried to remove"
        );
        writes.await.unwrap();
        assert_sibling_writes_survived(&state, session_id, &client_b).await;

        journal.shutdown().unwrap();
        #[expect(
            clippy::let_underscore_must_use,
            reason = "best-effort cleanup of a temporary test directory"
        )]
        let _ = std::fs::remove_dir_all(&data_dir);
    })
    .await
    .unwrap();
}

async fn seed_doomed_and_safe_shelves(
    state: &ServerState,
    session_id: i64,
    client_a: &ConnectionAuthContext,
    client_b: &ConnectionAuthContext,
) {
    state
        .add_shelf(session_id, "doomed-shelf", "", client_a)
        .await
        .unwrap();
    state
        .add_book(session_id, "doomed-shelf", "book", "", client_a)
        .await
        .unwrap();
    state
        .add_entry(
            session_id,
            "victim",
            "",
            &[],
            "ORIGINAL",
            "doomed-shelf",
            "book",
            client_a,
        )
        .await
        .unwrap();
    state
        .add_shelf(session_id, "safe-shelf", "", client_b)
        .await
        .unwrap();
    state
        .add_book(session_id, "safe-shelf", "book", "", client_b)
        .await
        .unwrap();
    state
        .add_entry(
            session_id,
            "anchor",
            "",
            &[],
            "ANCHOR",
            "safe-shelf",
            "book",
            client_b,
        )
        .await
        .unwrap();
}

async fn safe_shelf_sibling_writes(
    state: &ServerState,
    session_id: i64,
    actor: &ConnectionAuthContext,
) -> anyhow::Result<()> {
    state
        .add_book(
            session_id,
            "safe-shelf",
            "concurrent-book",
            "CONCURRENT BOOK",
            actor,
        )
        .await?;
    state
        .add_entry(
            session_id,
            "sibling-entry",
            "",
            &[],
            "x",
            "safe-shelf",
            "concurrent-book",
            actor,
        )
        .await?;
    state
        .append_to_entry(
            session_id,
            "anchor",
            Some("safe-shelf"),
            Some("book"),
            actor,
            "ANCHOR-APPEND",
            AppendMetadata {
                agent_name: None,
                host_name: None,
                reason: None,
            },
        )
        .await?;
    Ok(())
}

async fn assert_safe_shelf_writes_survived(
    state: &ServerState,
    session_id: i64,
    actor: &ConnectionAuthContext,
) {
    assert!(
        state
            .get_entry(
                session_id,
                "sibling-entry",
                Some("safe-shelf"),
                Some("concurrent-book"),
                actor
            )
            .await
            .unwrap()
            .is_some(),
        "a sibling's concurrent add must survive another client's rolled-back delete_shelf"
    );
    assert!(
        state
            .get_entry(
                session_id,
                "anchor",
                Some("safe-shelf"),
                Some("book"),
                actor
            )
            .await
            .unwrap()
            .unwrap()
            .context
            .contains("ANCHOR-APPEND"),
        "a sibling's concurrent append must survive another client's rolled-back delete_shelf"
    );
    assert!(
        state
            .search_books(session_id, actor, "concurrent-book")
            .await
            .unwrap()
            .iter()
            .any(|book| book.shelf_name == "safe-shelf"
                && book.book_name == "concurrent-book"
                && book.description == "CONCURRENT BOOK"
                && book.entry_count == 1),
        "a sibling's book description and count must survive a rolled-back shelf deletion"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[expect(
    clippy::await_holding_lock,
    reason = "Serializes process-global test environment; server tasks do not acquire this lock."
)]
async fn sibling_delete_shelf_rollback_preserves_concurrent_writes() {
    tokio::time::timeout(Duration::from_secs(20), async {
        let _env = test_env_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (state, journal, session_id, data_dir) =
            rollback_isolation_fixture("delete-shelf").await;
        let client_a = writer(session_id, "client-a");
        let client_b = writer(session_id, "client-b");
        seed_doomed_and_safe_shelves(&state, session_id, &client_a, &client_b).await;

        let gate = test_hooks::arm("delete_shelf::persist");

        let delete = tokio::spawn({
            let state = Arc::clone(&state);
            let actor = client_a.clone();
            async move { state.delete_shelf(session_id, "doomed-shelf", &actor).await }
        });
        gate.wait_until_reached().await;
        assert!(
            state
                .get_entry(
                    session_id,
                    "victim",
                    Some("doomed-shelf"),
                    Some("book"),
                    &client_a
                )
                .await
                .unwrap()
                .is_none()
        );
        poison_before("message_packs", "DELETE", "OLD.name = 'victim'");

        let writes = safe_shelf_sibling_writes(&state, session_id, &client_b);
        tokio::pin!(writes);
        assert_sibling_still_parked_then_release(&gate, writes.as_mut()).await;

        assert!(finish(delete).await.is_err());
        assert!(
            state
                .get_entry(
                    session_id,
                    "victim",
                    Some("doomed-shelf"),
                    Some("book"),
                    &client_a
                )
                .await
                .unwrap()
                .is_some(),
            "a failed delete_shelf persist must restore the shelf it tried to remove"
        );
        writes.await.unwrap();
        assert_safe_shelf_writes_survived(&state, session_id, &client_b).await;

        journal.shutdown().unwrap();
        #[expect(
            clippy::let_underscore_must_use,
            reason = "best-effort cleanup of a temporary test directory"
        )]
        let _ = std::fs::remove_dir_all(&data_dir);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn revoke_of_another_certificate_does_not_drop_unrelated_journaled_writes() {
    let ctx = TestContext::new("revoke-journal-safety").await.unwrap();
    add(&ctx, "kept", "before").await;
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
    ctx.state
        .revoke_client_cert(ctx.session_id, &admin(ctx.session_id, "writer"), "peer")
        .await
        .unwrap();

    let loaded = reopen(&ctx).await;
    assert_eq!(
        loaded
            .get_entry(
                ctx.session_id,
                "kept",
                None,
                None,
                &writer(ctx.session_id, "writer")
            )
            .await
            .unwrap()
            .unwrap()
            .context,
        "before",
        "revoking an unrelated certificate must not drop a write acknowledged \
         before the revoke"
    );
    loaded.journal.shutdown().unwrap();
}
