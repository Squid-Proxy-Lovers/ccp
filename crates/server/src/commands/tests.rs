// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::sync::MutexGuard;

use sha2::{Digest, Sha256};

use super::super::*;
use crate::init::{SERVER_DATA_DIR_ENV, init_sqlite, test_env_lock};
use protocol::{ConflictPolicy, TransferScope, TransferSelector};

struct Context {
    _guard: MutexGuard<'static, ()>,
    directory: std::path::PathBuf,
    state: ServerState,
    auth: ConnectionAuthContext,
}

impl Context {
    // Serializes process-global environment changes across single-threaded tests;
    // no task spawned by these fixtures ever acquires this synchronous lock.
    #[allow(clippy::await_holding_lock)]
    async fn new() -> Self {
        let guard = test_env_lock().lock().unwrap_or_else(|e| e.into_inner());
        let directory = std::env::temp_dir().join(format!("ccp-commands-{}", Uuid::new_v4()));
        unsafe { std::env::set_var(SERVER_DATA_DIR_ENV, &directory) };
        init_sqlite(&crate::init::db_path()).unwrap();
        open_sqlite_connection()
            .unwrap()
            .execute(
                "INSERT INTO sessions (id, name, is_active) VALUES (1, 'command-tests', 1)",
                [],
            )
            .unwrap();
        let journal = Arc::new(JournalHandle::start(crate::init::journal_path()).unwrap());
        let state = ServerState::load_from_storage(journal).await.unwrap();
        Self {
            _guard: guard,
            directory,
            state,
            auth: ConnectionAuthContext {
                common_name: "worker".to_string(),
                session_id: 1,
                can_write: true,
                can_revoke_others: false,
            },
        }
    }

    async fn add(&self, name: &str, context: &str) {
        self.state
            .add_entry(1, name, "", &[], context, "main", "default", &self.auth)
            .await
            .unwrap();
    }
}

impl Drop for Context {
    fn drop(&mut self) {
        let _ = self.state.journal.shutdown();
        unsafe { std::env::remove_var(SERVER_DATA_DIR_ENV) };
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

#[tokio::test]
async fn archive_search_excludes_unrelated_entries() {
    let ctx = Context::new().await;
    ctx.add("lunar-navigation", "moon").await;
    ctx.add("banana-recipes", "fruit").await;
    for name in ["lunar-navigation", "banana-recipes"] {
        ctx.state
            .delete_entry(1, name, None, None, &ctx.auth)
            .await
            .unwrap();
    }
    let results = ctx
        .state
        .search_deleted_entries(1, &ctx.auth, "lunar")
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "lunar-navigation");
    assert_eq!(
        ctx.state
            .search_deleted_entries(1, &ctx.auth, "")
            .await
            .unwrap()
            .len(),
        2
    );
    assert!(
        ctx.state
            .search_deleted_entries(1, &ctx.auth, "🦑")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn duplicate_bundle_paths_never_mutate_live_entries() {
    let ctx = Context::new().await;
    ctx.add("source", "original").await;
    let mut bundle = ctx
        .state
        .export_bundle(
            1,
            &TransferSelector {
                scope: TransferScope::Session,
                include_history: true,
            },
            &ctx.auth,
        )
        .await
        .unwrap();
    bundle.entries[0].name = "new".to_string();
    let mut duplicate = bundle.entries[0].clone();
    duplicate.name = " new ".to_string();
    bundle.entries.push(duplicate);
    bundle.bundle_sha256 = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&bundle.entries).unwrap())
    );
    for policy in [
        ConflictPolicy::Error,
        ConflictPolicy::Overwrite,
        ConflictPolicy::Skip,
        ConflictPolicy::MergeHistory,
    ] {
        assert!(
            ctx.state
                .import_bundle(1, &bundle, &policy, &ctx.auth)
                .await
                .is_err()
        );
        assert!(
            ctx.state
                .get_entry(1, "new", None, None, &ctx.auth)
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(ctx.state.list_entries(1, &ctx.auth).await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn shelf_delete_restores_memory_when_its_snapshot_fails() {
    let ctx = Context::new().await;
    ctx.add("preserved", "original").await;
    // Warm caches before rollback and remember the generation that a search of
    // the temporary deletion would capture (one removed entry bumps context).
    ctx.state
        .search_entries(1, &ctx.auth, "preserved")
        .await
        .unwrap();
    ctx.state
        .search_context(1, &ctx.auth, "original")
        .await
        .unwrap();
    let deleted_context_generation = ctx.state.sessions.read().await[&1]
        .context_search_generation
        .wrapping_add(1);
    // Persist a baseline, then allow exactly one checkpoint transaction before
    // failing the snapshot of the shelf deletion itself.
    {
        let _mutation = ctx.state.mutation_lock.lock().await;
        ctx.state.checkpoint_locked().await.unwrap();
    }
    open_sqlite_connection()
        .unwrap()
        .execute_batch(
            "CREATE TABLE snapshot_attempts (count INTEGER NOT NULL);
         INSERT INTO snapshot_attempts VALUES (0);
         CREATE TRIGGER fail_second_snapshot BEFORE DELETE ON books
         WHEN (SELECT count FROM snapshot_attempts) >= 1
         BEGIN SELECT RAISE(ABORT, 'injected snapshot failure'); END;
         CREATE TRIGGER count_snapshot AFTER DELETE ON books
         BEGIN UPDATE snapshot_attempts SET count = count + 1; END;",
        )
        .unwrap();
    assert!(ctx.state.delete_shelf(1, "main", &ctx.auth).await.is_err());
    assert_eq!(
        ctx.state
            .get_entry(1, "preserved", None, None, &ctx.auth)
            .await
            .unwrap()
            .unwrap()
            .context,
        "original"
    );
    assert!(
        ctx.state
            .sessions
            .read()
            .await
            .get(&1)
            .unwrap()
            .shelves
            .contains_key("main")
    );
    {
        let sessions = ctx.state.sessions.read().await;
        let restored = &sessions[&1];
        assert_ne!(
            restored.context_search_generation,
            deleted_context_generation
        );
        assert!(restored.entry_query_cache.is_empty());
        assert!(restored.context_query_cache.is_empty());
    }
    // A subsequent real mutation must not reuse the failed deletion's generation,
    // or an outstanding scorer could insert its deleted-state results again.
    ctx.state
        .add_shelf(1, "other", "description", &ctx.auth)
        .await
        .unwrap();
    assert_ne!(
        ctx.state.sessions.read().await[&1].context_search_generation,
        deleted_context_generation
    );
}

#[tokio::test]
async fn parent_metadata_changes_refresh_context_search_results() {
    let ctx = Context::new().await;
    ctx.add("notes", "orbital mission").await;
    let initial = ctx
        .state
        .search_context(1, &ctx.auth, "orbital")
        .await
        .unwrap();
    assert_eq!(initial.len(), 1);
    ctx.state
        .add_shelf(1, "main", "New shelf description", &ctx.auth)
        .await
        .unwrap();
    ctx.state
        .add_book(1, "main", "default", "New book description", &ctx.auth)
        .await
        .unwrap();
    let refreshed = ctx
        .state
        .search_context(1, &ctx.auth, "orbital")
        .await
        .unwrap();
    assert_eq!(refreshed[0].shelf_description, "New shelf description");
    assert_eq!(refreshed[0].book_description, "New book description");
}

#[tokio::test]
async fn instruction_requests_enforce_the_authorized_session() {
    let ctx = Context::new().await;
    let other_session = ConnectionAuthContext {
        session_id: 2,
        common_name: ctx.auth.common_name.clone(),
        can_write: false,
        can_revoke_others: false,
    };
    let response = crate::message::handle_message_request(
        &ctx.state,
        &other_session,
        protocol::ClientRequest::GetMasterInstructions { session_id: 1 },
    )
    .await;
    assert!(matches!(
        response,
        protocol::ServerResponse::Error(protocol::ErrorResponse {
            code: protocol::ErrorCode::Forbidden,
            ..
        })
    ));
    let response = crate::message::handle_message_request(
        &ctx.state,
        &ctx.auth,
        protocol::ClientRequest::GetMasterInstructions { session_id: 1 },
    )
    .await;
    assert!(matches!(
        response,
        protocol::ServerResponse::MasterInstructions(_)
    ));
}

#[tokio::test]
async fn repeated_revocation_preserves_the_existing_runtime_mark() {
    let ctx = Context::new().await;
    let admin = ConnectionAuthContext {
        can_revoke_others: true,
        ..ctx.auth.clone()
    };
    ctx.state
        .record_issued_cert(1, "already-revoked", "read", "", "4102444800")
        .await
        .unwrap();
    ctx.state
        .revoke_client_cert(1, &admin, "already-revoked")
        .await
        .unwrap();
    assert!(
        ctx.state
            .revoke_client_cert(1, &admin, "already-revoked")
            .await
            .is_err()
    );
    assert!(
        ctx.state
            .revoked_cert_common_names
            .read()
            .await
            .contains("already-revoked")
    );
    // A missing target that was never revoked still leaves no new runtime mark.
    assert!(
        ctx.state
            .revoke_client_cert(1, &admin, "missing")
            .await
            .is_err()
    );
    assert!(
        !ctx.state
            .revoked_cert_common_names
            .read()
            .await
            .contains("missing")
    );
}

#[tokio::test]
async fn committed_import_succeeds_even_when_the_audit_insert_fails() {
    let ctx = Context::new().await;
    ctx.add("source", "preserved").await;
    let mut bundle = ctx
        .state
        .export_bundle(
            1,
            &TransferSelector {
                scope: TransferScope::Session,
                include_history: false,
            },
            &ctx.auth,
        )
        .await
        .unwrap();
    bundle.entries[0].name = "imported".to_string();
    bundle.bundle_sha256 = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&bundle.entries).unwrap())
    );
    open_sqlite_connection()
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER fail_import_audit BEFORE INSERT ON transfer_log
         WHEN NEW.direction = 'import'
         BEGIN SELECT RAISE(ABORT, 'injected audit failure'); END;",
        )
        .unwrap();
    let result = ctx
        .state
        .import_bundle(1, &bundle, &ConflictPolicy::Error, &ctx.auth)
        .await
        .unwrap();
    assert_eq!(result.imported_entries, 1);
    assert_eq!(
        ctx.state
            .get_entry(1, "imported", None, None, &ctx.auth)
            .await
            .unwrap()
            .unwrap()
            .context,
        "preserved"
    );
    let count: i64 = open_sqlite_connection()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM message_packs WHERE name = 'imported'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn briefing_counts_labels_once_per_entry_and_sorts_numeric_timestamps() {
    let ctx = Context::new().await;
    ctx.state
        .add_entry(
            1,
            "older",
            "",
            &vec!["repeated".to_string(); 5],
            "",
            "main",
            "default",
            &ctx.auth,
        )
        .await
        .unwrap();
    for name in ["newer", "other"] {
        ctx.state
            .add_entry(
                1,
                name,
                "",
                &["popular".to_string()],
                "",
                "main",
                "default",
                &ctx.auth,
            )
            .await
            .unwrap();
    }
    {
        let mut sessions = ctx.state.sessions.write().await;
        let session = sessions.get_mut(&1).unwrap();
        session
            .entries
            .get_mut("main::default::older")
            .unwrap()
            .updated_at = "9".to_string();
        session
            .entries
            .get_mut("main::default::newer")
            .unwrap()
            .updated_at = "10".to_string();
        session
            .entries
            .get_mut("main::default::other")
            .unwrap()
            .updated_at = "8".to_string();
    }
    let brief = ctx.state.brief_me(1, &ctx.auth).await.unwrap();
    assert_eq!(brief.frequent_labels, ["popular", "repeated"]);
    assert_eq!(
        brief
            .recent_entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        ["newer", "older", "other"]
    );
}
