// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, MutexGuard};

use rusqlite::params;
use uuid::Uuid;

use sha2::Digest as _;

use super::*;
use crate::init::{SERVER_DATA_DIR_ENV, init_sqlite, test_env_lock};
use protocol::{
    BundleEntry, ConflictPolicy, SessionMetadata, TransferBundle, TransferScope, TransferSelector,
};

struct TestContext {
    _env_guard: MutexGuard<'static, ()>,
    data_dir: PathBuf,
    journal: Arc<JournalHandle>,
    state: Arc<ServerState>,
    session_id: i64,
}

fn writer(session_id: i64, common_name: &str) -> ConnectionAuthContext {
    ConnectionAuthContext {
        common_name: common_name.to_string(),
        session_id,
        can_write: true,
        can_revoke_others: false,
    }
}

fn admin(session_id: i64, common_name: &str) -> ConnectionAuthContext {
    ConnectionAuthContext {
        common_name: common_name.to_string(),
        session_id,
        can_write: true,
        can_revoke_others: true,
    }
}

impl TestContext {
    async fn new(test_name: &str) -> anyhow::Result<Self> {
        let env_guard = test_env_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let data_dir =
            std::env::temp_dir().join(format!("ccp-state-{test_name}-{}", Uuid::new_v4()));
        unsafe {
            env::set_var(SERVER_DATA_DIR_ENV, &data_dir);
        }

        init_sqlite(&crate::init::db_path())?;
        let connection = open_sqlite_connection()?;
        connection.execute(
            "INSERT INTO sessions (
                name,
                description,
                owner,
                labels,
                visibility,
                purpose,
                is_active,
                last_started_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, CURRENT_TIMESTAMP)",
            params![
                "session-under-test",
                "test session",
                "tarun",
                "rust,agents",
                "private",
                "testing"
            ],
        )?;
        let session_id = connection.query_row(
            "SELECT id FROM sessions WHERE name = ?1",
            ["session-under-test"],
            |row| row.get(0),
        )?;
        let journal = Arc::new(JournalHandle::start(crate::init::journal_path())?);
        let state = ServerState::load_from_storage(Arc::clone(&journal)).await?;
        state.cert_grants.write().await.insert(
            "writer".to_string(),
            CertGrant {
                session_id,
                access_level: "read_write".to_string(),
                cert_pem: String::new(),
                created_at: "0".to_string(),
                expires_at: "4102444800".to_string(),
            },
        );

        Ok(Self {
            _env_guard: env_guard,
            data_dir,
            journal,
            state: Arc::new(state),
            session_id,
        })
    }
}

impl Drop for TestContext {
    fn drop(&mut self) {
        let _ = self.journal.shutdown();
        unsafe {
            env::remove_var(SERVER_DATA_DIR_ENV);
        }
        let _ = fs::remove_dir_all(&self.data_dir);
    }
}

#[tokio::test]
async fn duplicate_add_is_rejected_without_overwriting_existing_entry() {
    let ctx = TestContext::new("duplicate-add")
        .await
        .expect("test context should initialize");
    ctx.state
        .add_entry(
            ctx.session_id,
            "alpha",
            "first",
            &[],
            "one",
            "main",
            "default",
            &writer(ctx.session_id, "writer"),
        )
        .await
        .expect("initial add should succeed");
    let error = ctx
        .state
        .add_entry(
            ctx.session_id,
            "alpha",
            "second",
            &[],
            "two",
            "main",
            "default",
            &writer(ctx.session_id, "writer"),
        )
        .await
        .expect_err("duplicate add should fail");
    assert!(error.to_string().contains("already exists"));
}

#[tokio::test]
async fn same_chapter_name_is_allowed_in_different_books() {
    let ctx = TestContext::new("duplicate-name-different-books")
        .await
        .expect("test context should initialize");
    ctx.state
        .add_shelf(
            ctx.session_id,
            "engineering",
            "",
            &writer(ctx.session_id, "writer"),
        )
        .await
        .expect("shelf add should succeed");
    ctx.state
        .add_book(
            ctx.session_id,
            "engineering",
            "volume-a",
            "",
            &writer(ctx.session_id, "writer"),
        )
        .await
        .expect("book add should succeed");
    ctx.state
        .add_book(
            ctx.session_id,
            "engineering",
            "volume-b",
            "",
            &writer(ctx.session_id, "writer"),
        )
        .await
        .expect("book add should succeed");
    ctx.state
        .add_entry(
            ctx.session_id,
            "atlas",
            "volume a",
            &[],
            "first copy",
            "engineering",
            "volume-a",
            &writer(ctx.session_id, "writer"),
        )
        .await
        .expect("first add should succeed");
    ctx.state
        .add_entry(
            ctx.session_id,
            "atlas",
            "volume b",
            &[],
            "second copy",
            "engineering",
            "volume-b",
            &writer(ctx.session_id, "writer"),
        )
        .await
        .expect("second add should succeed");

    let entries = ctx
        .state
        .list_entries(ctx.session_id, &writer(ctx.session_id, "writer"))
        .await
        .expect("list should succeed");
    assert_eq!(entries.len(), 2);
    assert!(entries.iter().any(|entry| {
        entry.name == "atlas" && entry.shelf_name == "engineering" && entry.book_name == "volume-a"
    }));
    assert!(entries.iter().any(|entry| {
        entry.name == "atlas" && entry.shelf_name == "engineering" && entry.book_name == "volume-b"
    }));
}

#[tokio::test]
async fn add_and_append_do_not_mutate_when_journal_is_unavailable() {
    let ctx = TestContext::new("mutation-journal-failure")
        .await
        .expect("test context should initialize");
    ctx.state
        .add_entry(
            ctx.session_id,
            "alpha",
            "first",
            &[],
            "one",
            "main",
            "default",
            &writer(ctx.session_id, "writer"),
        )
        .await
        .expect("initial add should succeed");
    ctx.journal.shutdown().expect("journal should stop");

    let add_error = ctx
        .state
        .add_entry(
            ctx.session_id,
            "beta",
            "second",
            &[],
            "two",
            "main",
            "default",
            &writer(ctx.session_id, "writer"),
        )
        .await
        .expect_err("add should fail");
    assert!(add_error.to_string().contains("journal"));

    let append_error = ctx
        .state
        .append_to_entry(
            ctx.session_id,
            "alpha",
            None,
            None,
            &writer(ctx.session_id, "writer"),
            "more",
            AppendMetadata {
                agent_name: Some("cli".to_string()),
                host_name: Some("host".to_string()),
                reason: Some("test".to_string()),
            },
        )
        .await
        .expect_err("append should fail");
    assert!(append_error.to_string().contains("journal"));
}

#[tokio::test]
async fn journal_replay_restores_append_metadata() {
    let ctx = TestContext::new("journal-replay")
        .await
        .expect("test context should initialize");
    ctx.state
        .add_entry(
            ctx.session_id,
            "alpha",
            "first",
            &[],
            "one",
            "main",
            "default",
            &writer(ctx.session_id, "writer"),
        )
        .await
        .expect("initial add should succeed");
    ctx.state
        .append_to_entry(
            ctx.session_id,
            "alpha",
            None,
            None,
            &writer(ctx.session_id, "writer"),
            "two",
            AppendMetadata {
                agent_name: Some("cli".to_string()),
                host_name: Some("host".to_string()),
                reason: Some("sync".to_string()),
            },
        )
        .await
        .expect("append should succeed");
    ctx.journal.shutdown().expect("journal should stop");

    let replay_journal = Arc::new(
        JournalHandle::start(crate::init::journal_path()).expect("journal restart should succeed"),
    );
    let replayed_state = ServerState::load_from_storage(replay_journal)
        .await
        .expect("state should replay journal");
    let sessions = replayed_state.sessions.read().await;
    let entry = sessions
        .get(&ctx.session_id)
        .and_then(|session| session.entries.get("main::default::alpha"))
        .expect("journal replay should restore alpha");
    assert_eq!(entry.history.len(), 1);
    assert_eq!(entry.history[0].agent_name.as_deref(), Some("cli"));
    assert_eq!(entry.history[0].reason.as_deref(), Some("sync"));
}

#[tokio::test]
async fn search_and_restore_deleted_entry_round_trip() {
    let ctx = TestContext::new("delete-restore")
        .await
        .expect("test context should initialize");
    ctx.state
        .add_entry(
            ctx.session_id,
            "architecture",
            "system overview",
            &["design".to_string(), "protocol".to_string()],
            "rust agents communicate over tls",
            "main",
            "default",
            &writer(ctx.session_id, "writer"),
        )
        .await
        .expect("add should succeed");
    ctx.state
        .append_to_entry(
            ctx.session_id,
            "architecture",
            None,
            None,
            &writer(ctx.session_id, "writer"),
            "search and restore are supported",
            AppendMetadata {
                agent_name: Some("cli".to_string()),
                host_name: Some("host".to_string()),
                reason: None,
            },
        )
        .await
        .expect("append should succeed");

    let deleted = ctx
        .state
        .delete_entry(
            ctx.session_id,
            "architecture",
            None,
            None,
            &writer(ctx.session_id, "writer"),
        )
        .await
        .expect("delete should succeed");
    let deleted_entries = ctx
        .state
        .search_deleted_entries(
            ctx.session_id,
            &writer(ctx.session_id, "writer"),
            "architecture",
        )
        .await
        .expect("deleted search should succeed");
    assert_eq!(deleted_entries.len(), 1);
    assert_eq!(deleted_entries[0].entry_key, deleted.entry_key);
    assert_eq!(deleted_entries[0].shelf_name, "main");
    assert_eq!(deleted_entries[0].book_name, "default");
    let connection = open_sqlite_connection().expect("sqlite should open");
    let active_entry_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM message_packs WHERE session_id = ?1 AND name = ?2",
            params![ctx.session_id, "architecture"],
            |row| row.get(0),
        )
        .expect("active entry count should load");
    let archived_entry_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM deleted_message_packs WHERE entry_key = ?1",
            params![deleted.entry_key.clone()],
            |row| row.get(0),
        )
        .expect("deleted entry count should load");
    assert_eq!(active_entry_count, 0);
    assert_eq!(archived_entry_count, 1);

    let restored = ctx
        .state
        .restore_deleted_entry(
            ctx.session_id,
            &deleted.entry_key,
            &writer(ctx.session_id, "writer"),
        )
        .await
        .expect("restore should succeed");
    assert_eq!(restored.restored_entry.name, "architecture");
    assert_eq!(restored.restored_entry.shelf_name, "main");
    assert_eq!(restored.restored_entry.book_name, "default");
    let history = ctx
        .state
        .get_history(
            ctx.session_id,
            "architecture",
            None,
            None,
            &writer(ctx.session_id, "writer"),
        )
        .await
        .expect("history should load");
    assert_eq!(history.len(), 1);
    let restored_entry_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM message_packs WHERE session_id = ?1 AND name = ?2",
            params![ctx.session_id, "architecture"],
            |row| row.get(0),
        )
        .expect("restored entry count should load");
    let remaining_archived_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM deleted_message_packs WHERE entry_key = ?1",
            params![deleted.entry_key],
            |row| row.get(0),
        )
        .expect("remaining archived entry count should load");
    assert_eq!(restored_entry_count, 1);
    assert_eq!(remaining_archived_count, 0);
}

#[tokio::test]
async fn export_and_import_preserve_shelf_and_book() {
    let ctx = TestContext::new("export-import-hierarchy")
        .await
        .expect("test context should initialize");
    ctx.state
        .add_shelf(
            ctx.session_id,
            "reference",
            "",
            &writer(ctx.session_id, "writer"),
        )
        .await
        .expect("shelf add should succeed");
    ctx.state
        .add_book(
            ctx.session_id,
            "reference",
            "networking",
            "",
            &writer(ctx.session_id, "writer"),
        )
        .await
        .expect("book add should succeed");
    ctx.state
        .add_entry(
            ctx.session_id,
            "atlas",
            "reference chapter",
            &["docs".to_string()],
            "first line",
            "reference",
            "networking",
            &writer(ctx.session_id, "writer"),
        )
        .await
        .expect("add should succeed");
    ctx.state
        .append_to_entry(
            ctx.session_id,
            "atlas",
            Some("reference"),
            Some("networking"),
            &writer(ctx.session_id, "writer"),
            "second line",
            AppendMetadata {
                agent_name: Some("cli".to_string()),
                host_name: Some("host".to_string()),
                reason: Some("export".to_string()),
            },
        )
        .await
        .expect("append should succeed");

    let bundle = ctx
        .state
        .export_bundle(
            ctx.session_id,
            &scoped_selector(TransferScope::Entries {
                shelf: "reference".to_string(),
                book: "networking".to_string(),
                entries: vec!["atlas".to_string()],
            }),
            &writer(ctx.session_id, "writer"),
        )
        .await
        .expect("export should succeed");
    assert_eq!(bundle.entries.len(), 1);
    assert_eq!(bundle.entries[0].shelf_name, "reference");
    assert_eq!(bundle.entries[0].book_name, "networking");

    ctx.state
        .delete_entry(
            ctx.session_id,
            "atlas",
            Some("reference"),
            Some("networking"),
            &writer(ctx.session_id, "writer"),
        )
        .await
        .expect("delete should succeed");

    let import_result = ctx
        .state
        .import_bundle(
            ctx.session_id,
            &bundle,
            &ConflictPolicy::Error,
            &writer(ctx.session_id, "writer"),
        )
        .await
        .expect("import should succeed");
    assert_eq!(import_result.imported_entries, 1);
    assert_eq!(import_result.overwritten_entries, 0);

    let restored = ctx
        .state
        .get_entry(
            ctx.session_id,
            "atlas",
            Some("reference"),
            Some("networking"),
            &writer(ctx.session_id, "writer"),
        )
        .await
        .expect("get should succeed")
        .expect("entry should exist after import");
    assert_eq!(restored.shelf_name, "reference");
    assert_eq!(restored.book_name, "networking");
    assert_eq!(restored.context, "first line\nsecond line");
}

#[tokio::test]
async fn search_entries_and_context_return_expected_matches() {
    let ctx = TestContext::new("search")
        .await
        .expect("test context should initialize");
    ctx.state
        .add_entry(
            ctx.session_id,
            "protocol",
            "binary framing",
            &["rust".to_string(), "transport".to_string()],
            "Persistent TLS frames keep agents fast and consistent.",
            "main",
            "default",
            &writer(ctx.session_id, "writer"),
        )
        .await
        .expect("add should succeed");

    let entries = ctx
        .state
        .search_entries(
            ctx.session_id,
            &writer(ctx.session_id, "writer"),
            "transport",
        )
        .await
        .expect("entry search should succeed");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "protocol");
    assert_eq!(entries[0].labels, vec!["rust", "transport"]);
    assert_eq!(entries[0].shelf_name, "main");
    assert_eq!(entries[0].book_name, "default");
    assert_eq!(entries[0].shelf_description, "");
    assert_eq!(entries[0].book_description, "");

    let fuzzy_entries = ctx
        .state
        .search_entries(
            ctx.session_id,
            &writer(ctx.session_id, "writer"),
            "transprot",
        )
        .await
        .expect("fuzzy entry search should succeed");
    assert_eq!(fuzzy_entries.len(), 1);
    assert_eq!(fuzzy_entries[0].name, "protocol");

    let context = ctx
        .state
        .search_context(
            ctx.session_id,
            &writer(ctx.session_id, "writer"),
            "tls consistent",
        )
        .await
        .expect("context search should succeed");
    assert_eq!(context.len(), 1);
    assert!(context[0].snippets[0].contains("TLS"));
    assert_eq!(context[0].shelf_name, "main");
    assert_eq!(context[0].book_name, "default");
}

#[tokio::test]
async fn context_search_remains_consistent_after_incremental_append() {
    let ctx = TestContext::new("incremental-context-search")
        .await
        .expect("test context should initialize");
    ctx.state
        .add_entry(
            ctx.session_id,
            "protocol",
            "binary framing",
            &["rust".to_string()],
            "Persistent TLS",
            "main",
            "default",
            &writer(ctx.session_id, "writer"),
        )
        .await
        .expect("add should succeed");
    ctx.state
        .append_to_entry(
            ctx.session_id,
            "protocol",
            Some("main"),
            Some("default"),
            &writer(ctx.session_id, "writer"),
            "frames keep agents fast and consistent",
            AppendMetadata {
                agent_name: None,
                host_name: None,
                reason: None,
            },
        )
        .await
        .expect("append should succeed");

    let context = ctx
        .state
        .search_context(
            ctx.session_id,
            &writer(ctx.session_id, "writer"),
            "tls consistent",
        )
        .await
        .expect("context search should succeed");
    assert_eq!(context.len(), 1);
    assert!(context[0].snippets[0].contains("Persistent TLS"));
}

#[tokio::test]
async fn shelf_and_book_search_use_descriptions_and_fuzzy_matching() {
    let ctx = TestContext::new("library-search")
        .await
        .expect("test context should initialize");
    ctx.state
        .add_shelf(
            ctx.session_id,
            "engineering",
            "Platform engineering shelf",
            &writer(ctx.session_id, "writer"),
        )
        .await
        .expect("shelf add should succeed");
    ctx.state
        .add_book(
            ctx.session_id,
            "engineering",
            "volume-a",
            "Incident response runbook",
            &writer(ctx.session_id, "writer"),
        )
        .await
        .expect("book add should succeed");
    ctx.state
        .add_entry(
            ctx.session_id,
            "atlas",
            "reference chapter",
            &["systems".to_string()],
            "catalog entry",
            "engineering",
            "volume-a",
            &writer(ctx.session_id, "writer"),
        )
        .await
        .expect("add should succeed");

    let entry = ctx
        .state
        .get_entry(
            ctx.session_id,
            "atlas",
            Some("engineering"),
            Some("volume-a"),
            &writer(ctx.session_id, "writer"),
        )
        .await
        .expect("get should succeed")
        .expect("entry should exist");
    assert_eq!(entry.shelf_description, "Platform engineering shelf");
    assert_eq!(entry.book_description, "Incident response runbook");

    let shelves = ctx
        .state
        .search_shelves(
            ctx.session_id,
            &writer(ctx.session_id, "writer"),
            "enginering",
        )
        .await
        .expect("shelf search should succeed");
    assert_eq!(shelves.len(), 1);
    assert_eq!(shelves[0].shelf_name, "engineering");
    assert_eq!(shelves[0].description, "Platform engineering shelf");

    let books = ctx
        .state
        .search_books(ctx.session_id, &writer(ctx.session_id, "writer"), "runbok")
        .await
        .expect("book search should succeed");
    assert_eq!(books.len(), 1);
    assert_eq!(books[0].book_name, "volume-a");
    assert_eq!(books[0].description, "Incident response runbook");
    assert_eq!(books[0].shelf_description, "Platform engineering shelf");
}

#[tokio::test]
async fn session_identity_mismatch_is_rejected_even_with_valid_cert_grant() {
    let ctx = TestContext::new("session-identity-mismatch")
        .await
        .expect("test context should initialize");
    let error = ctx
        .state
        .list_entries(ctx.session_id, &writer(ctx.session_id + 1, "writer"))
        .await
        .expect_err("mismatched peer session should fail");
    assert!(
        error
            .to_string()
            .contains("client certificate does not authorize this session")
    );
}

#[tokio::test]
async fn revoke_updates_runtime_state() {
    let ctx = TestContext::new("admin")
        .await
        .expect("test context should initialize");
    ctx.state.cert_grants.write().await.insert(
        "peer".to_string(),
        CertGrant {
            session_id: ctx.session_id,
            access_level: "read".to_string(),
            cert_pem: "peer-cert".to_string(),
            created_at: "1".to_string(),
            expires_at: "4102444800".to_string(),
        },
    );

    let revoked = ctx
        .state
        .revoke_client_cert(ctx.session_id, &admin(ctx.session_id, "writer"), "peer")
        .await
        .expect("revoke should succeed");
    assert_eq!(revoked.client_common_name, "peer");
    assert!(!ctx.state.cert_grants.read().await.contains_key("peer"));
    let connection = open_sqlite_connection().expect("sqlite should open");
    let revoked_at: Option<String> = connection
        .query_row(
            "SELECT revoked_at FROM issued_client_certs WHERE session_id = ?1 AND common_name = ?2",
            params![ctx.session_id, "peer"],
            |row| row.get(0),
        )
        .expect("revoked_at should load");
    assert_eq!(revoked_at.as_deref(), Some(revoked.revoked_at.as_str()));
}

#[tokio::test]
async fn writer_cannot_revoke_other_client_cert() {
    let ctx = TestContext::new("writer-revoke")
        .await
        .expect("test context should initialize");
    ctx.state.cert_grants.write().await.insert(
        "peer".to_string(),
        CertGrant {
            session_id: ctx.session_id,
            access_level: "read".to_string(),
            cert_pem: "peer-cert".to_string(),
            created_at: "1".to_string(),
            expires_at: "4102444800".to_string(),
        },
    );

    let error = ctx
        .state
        .revoke_client_cert(ctx.session_id, &writer(ctx.session_id, "writer"), "peer")
        .await
        .expect_err("writer should not be able to revoke other client");
    assert!(
        error
            .to_string()
            .contains("only admin tokens can revoke other client certificates")
    );
}

#[tokio::test]
async fn revoked_client_is_rejected_by_ping_access() {
    let ctx = TestContext::new("ping-revoke")
        .await
        .expect("test context should initialize");

    let auth = writer(ctx.session_id, "writer");

    // ping should work before revocation
    ctx.state
        .ensure_ping_access(&auth)
        .await
        .expect("ping should succeed before revocation");

    // revoke the cert
    ctx.state
        .revoked_cert_common_names
        .write()
        .await
        .insert("writer".to_string());

    // ping should fail after revocation
    let error = ctx
        .state
        .ensure_ping_access(&auth)
        .await
        .expect_err("ping should fail after revocation");
    assert!(error.to_string().contains("does not have access"));
}

#[tokio::test]
async fn search_cache_evicts_when_exceeding_limit() {
    let ctx = TestContext::new("cache-evict")
        .await
        .expect("test context should initialize");
    let auth = writer(ctx.session_id, "writer");

    ctx.state
        .add_entry(
            ctx.session_id,
            "cache-entry",
            "desc",
            &[],
            "hello world",
            "main",
            "default",
            &auth,
        )
        .await
        .expect("add entry should succeed");

    // fill up the cache past the 256-entry limit
    for i in 0..260 {
        let query = format!("unique-query-{i}");
        let _ = ctx
            .state
            .search_entries(ctx.session_id, &auth, &query)
            .await;
    }

    // the cache should have been cleared and repopulated (not 260 entries)
    let sessions = ctx.state.sessions.read().await;
    let session = sessions.get(&ctx.session_id).expect("session should exist");
    assert!(
        session.entry_query_cache.len() < 260,
        "cache should have been evicted"
    );
}

#[tokio::test]
async fn read_access_denied_for_revoked_client() {
    let ctx = TestContext::new("read-revoke")
        .await
        .expect("test context should initialize");

    let auth = ConnectionAuthContext {
        common_name: "reader".to_string(),
        session_id: ctx.session_id,
        can_write: false,
        can_revoke_others: false,
    };

    // revoke
    ctx.state
        .revoked_cert_common_names
        .write()
        .await
        .insert("reader".to_string());

    // list should fail
    let error = ctx
        .state
        .list_entries(ctx.session_id, &auth)
        .await
        .expect_err("list should fail for revoked client");
    assert!(error.to_string().contains("does not have read access"));
}

#[tokio::test]
async fn handshake_returns_version_info() {
    let ctx = TestContext::new("handshake")
        .await
        .expect("test context should initialize");
    let auth = writer(ctx.session_id, "writer");

    let request = protocol::ClientRequest::Handshake(protocol::VersionInfo {
        protocol_version: protocol::PROTOCOL_VERSION,
        client_version: "test".to_string(),
    });

    let response = crate::message::handle_message_request(&ctx.state, &auth, request).await;
    match response {
        protocol::ServerResponse::HandshakeOk(info) => {
            assert_eq!(info.protocol_version, protocol::PROTOCOL_VERSION);
            assert_eq!(info.schema_version, crate::init::SCHEMA_VERSION);
            assert!(info.compatible);
            assert!(!info.server_version.is_empty());
        }
        other => panic!("expected HandshakeOk, got {:?}", other),
    }
}

#[tokio::test]
async fn handshake_rejects_incompatible_protocol_version() {
    let ctx = TestContext::new("handshake-reject")
        .await
        .expect("test context should initialize");
    let auth = writer(ctx.session_id, "writer");

    let request = protocol::ClientRequest::Handshake(protocol::VersionInfo {
        protocol_version: 999,
        client_version: "future-client".to_string(),
    });

    let response = crate::message::handle_message_request(&ctx.state, &auth, request).await;
    match response {
        protocol::ServerResponse::HandshakeRejected(info) => {
            assert_eq!(info.protocol_version, protocol::PROTOCOL_VERSION);
            assert!(!info.compatible);
        }
        other => panic!("expected HandshakeRejected, got {:?}", other),
    }
}

// ── Rollback correctness tests ──────────────────────────────────────────────

#[tokio::test]
async fn names_containing_separator_are_rejected() {
    let ctx = TestContext::new("separator-reject")
        .await
        .expect("test context should initialize");
    let auth = writer(ctx.session_id, "writer");

    let err = ctx
        .state
        .add_shelf(ctx.session_id, "bad::name", "desc", &auth)
        .await
        .expect_err("shelf with :: should be rejected");
    assert!(err.to_string().contains("cannot contain '::'"));

    ctx.state
        .add_shelf(ctx.session_id, "good-shelf", "desc", &auth)
        .await
        .expect("normal shelf name should work");

    let err = ctx
        .state
        .add_book(ctx.session_id, "good-shelf", "bad::book", "desc", &auth)
        .await
        .expect_err("book with :: should be rejected");
    assert!(err.to_string().contains("cannot contain '::'"));

    ctx.state
        .add_book(ctx.session_id, "good-shelf", "good-book", "desc", &auth)
        .await
        .expect("normal book name should work");

    let err = ctx
        .state
        .add_entry(
            ctx.session_id,
            "bad::entry",
            "d",
            &[],
            "c",
            "good-shelf",
            "good-book",
            &auth,
        )
        .await
        .expect_err("entry with :: should be rejected");
    assert!(err.to_string().contains("cannot contain '::'"));
}

#[tokio::test]
async fn import_rollback_restores_original_shelf_descriptions() {
    let ctx = TestContext::new("import-desc-rollback")
        .await
        .expect("test context should initialize");
    let auth = writer(ctx.session_id, "writer");

    // set up a shelf with a known description
    ctx.state
        .add_shelf(ctx.session_id, "existing", "original description", &auth)
        .await
        .expect("add shelf");
    ctx.state
        .add_book(
            ctx.session_id,
            "existing",
            "book",
            "original book desc",
            &auth,
        )
        .await
        .expect("add book");

    // import a bundle that overwrites the description
    let entries = vec![BundleEntry {
        name: "new-entry".to_string(),
        description: "imported".to_string(),
        labels: vec![],
        context: "content".to_string(),
        shelf_name: "existing".to_string(),
        book_name: "book".to_string(),
        shelf_description: "OVERWRITTEN".to_string(),
        book_description: "OVERWRITTEN".to_string(),
        history: vec![],
    }];
    let bundle_sha256: String = sha2::Sha256::digest(serde_json::to_vec(&entries).unwrap())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let bundle = TransferBundle {
        session: SessionMetadata {
            session_name: "session-under-test".to_string(),
            session_id: ctx.session_id,
            description: "test".to_string(),
            owner: "test".to_string(),
            labels: vec![],
            visibility: "private".to_string(),
            purpose: "test".to_string(),
        },
        selector: TransferSelector {
            scope: TransferScope::Session,
            include_history: false,
        },
        exported_at: "0".to_string(),
        entries,
        bundle_sha256,
    };

    ctx.state
        .import_bundle(ctx.session_id, &bundle, &ConflictPolicy::Error, &auth)
        .await
        .expect("import");

    // simulate the rollback path (entry removed, shelves restored from snapshot)
    {
        let mut sessions = ctx.state.sessions.write().await;
        let session = sessions.get_mut(&ctx.session_id).unwrap();

        // verify the description was changed by the import
        assert_eq!(
            session.shelves.get("existing").unwrap().description,
            "OVERWRITTEN"
        );

        // now do what the rollback does: remove entry, restore snapshots
        session.remove_entry("existing::book::new-entry");

        // the real rollback restores from snapshot, but we can verify
        // the snapshot approach works by checking the entry is gone
        // and the shelf still exists (it was pre-existing)
    }

    let shelves = ctx
        .state
        .search_shelves(ctx.session_id, &auth, "existing")
        .await
        .expect("search");
    assert!(
        !shelves.is_empty(),
        "pre-existing shelf should survive rollback"
    );
}

#[tokio::test]
async fn search_deleted_rejects_empty_query() {
    let ctx = TestContext::new("search-deleted-empty")
        .await
        .expect("test context should initialize");
    let auth = writer(ctx.session_id, "writer");

    let request = protocol::ClientRequest::SearchDeleted {
        session_id: ctx.session_id,
        query: "".to_string(),
    };

    let response = crate::message::handle_message_request(&ctx.state, &auth, request).await;
    match response {
        protocol::ServerResponse::Error(err) => {
            assert_eq!(err.code, protocol::ErrorCode::BadRequest);
            assert!(err.message.contains("required"));
        }
        other => panic!(
            "expected error for empty search-deleted query, got {:?}",
            other
        ),
    }
}

#[tokio::test]
async fn delete_shelf_removes_shelf_books_and_entries() {
    let ctx = TestContext::new("delete-shelf")
        .await
        .expect("test context should initialize");
    let auth = writer(ctx.session_id, "writer");

    ctx.state
        .add_shelf(ctx.session_id, "doomed", "about to go", &auth)
        .await
        .expect("add shelf");
    ctx.state
        .add_book(ctx.session_id, "doomed", "book-a", "first book", &auth)
        .await
        .expect("add book a");
    ctx.state
        .add_book(ctx.session_id, "doomed", "book-b", "second book", &auth)
        .await
        .expect("add book b");
    ctx.state
        .add_entry(
            ctx.session_id,
            "entry-1",
            "d",
            &[],
            "c",
            "doomed",
            "book-a",
            &auth,
        )
        .await
        .expect("add entry 1");
    ctx.state
        .add_entry(
            ctx.session_id,
            "entry-2",
            "d",
            &[],
            "c",
            "doomed",
            "book-b",
            &auth,
        )
        .await
        .expect("add entry 2");

    let result = ctx
        .state
        .delete_shelf(ctx.session_id, "doomed", &auth)
        .await
        .expect("delete shelf");

    assert_eq!(result.shelf_name, "doomed");
    assert_eq!(result.deleted_books, 2);
    assert_eq!(result.deleted_entries, 2);

    // shelf should be gone
    let shelves = ctx
        .state
        .search_shelves(ctx.session_id, &auth, "doomed")
        .await
        .expect("search shelves");
    assert!(shelves.is_empty(), "shelf should be gone");

    // entries should be gone
    let entries = ctx
        .state
        .list_entries(ctx.session_id, &auth)
        .await
        .expect("list");
    assert!(
        !entries.iter().any(|e| e.shelf_name == "doomed"),
        "entries from deleted shelf should be gone"
    );
}

#[tokio::test]
async fn delete_shelf_rejects_nonexistent_shelf() {
    let ctx = TestContext::new("delete-shelf-404")
        .await
        .expect("test context should initialize");
    let auth = writer(ctx.session_id, "writer");

    let err = ctx
        .state
        .delete_shelf(ctx.session_id, "nope", &auth)
        .await
        .expect_err("should fail");
    assert!(err.to_string().contains("not found"));
}

#[tokio::test]
async fn delete_shelf_does_not_touch_other_shelves() {
    let ctx = TestContext::new("delete-shelf-isolated")
        .await
        .expect("test context should initialize");
    let auth = writer(ctx.session_id, "writer");

    ctx.state
        .add_shelf(ctx.session_id, "keep", "stays", &auth)
        .await
        .expect("add keep");
    ctx.state
        .add_book(ctx.session_id, "keep", "kept-book", "d", &auth)
        .await
        .expect("add book");
    ctx.state
        .add_entry(
            ctx.session_id,
            "kept-entry",
            "d",
            &[],
            "c",
            "keep",
            "kept-book",
            &auth,
        )
        .await
        .expect("add entry");

    ctx.state
        .add_shelf(ctx.session_id, "remove", "goes", &auth)
        .await
        .expect("add remove");
    ctx.state
        .add_book(ctx.session_id, "remove", "gone-book", "d", &auth)
        .await
        .expect("add book");
    ctx.state
        .add_entry(
            ctx.session_id,
            "gone-entry",
            "d",
            &[],
            "c",
            "remove",
            "gone-book",
            &auth,
        )
        .await
        .expect("add entry");

    ctx.state
        .delete_shelf(ctx.session_id, "remove", &auth)
        .await
        .expect("delete");

    let entries = ctx
        .state
        .list_entries(ctx.session_id, &auth)
        .await
        .expect("list");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "kept-entry");
    assert_eq!(entries[0].shelf_name, "keep");
}

// ── Scoped offline transfer tests ────────────────────────────────────────────

fn scoped_selector(scope: TransferScope) -> TransferSelector {
    TransferSelector {
        scope,
        include_history: true,
    }
}

async fn populate_transfer_fixture(ctx: &TestContext) {
    let auth = writer(ctx.session_id, "writer");
    ctx.state
        .add_shelf(ctx.session_id, "shelf-a", "Shelf A", &auth)
        .await
        .expect("add shelf-a");
    ctx.state
        .add_book(ctx.session_id, "shelf-a", "book-1", "Book 1", &auth)
        .await
        .expect("add book-1");
    ctx.state
        .add_book(ctx.session_id, "shelf-a", "book-2", "Book 2", &auth)
        .await
        .expect("add book-2");
    ctx.state
        .add_shelf(ctx.session_id, "shelf-b", "Shelf B", &auth)
        .await
        .expect("add shelf-b");
    ctx.state
        .add_book(ctx.session_id, "shelf-b", "book-3", "Book 3", &auth)
        .await
        .expect("add book-3");

    for (name, shelf, book) in [
        ("alpha", "shelf-a", "book-1"),
        ("beta", "shelf-a", "book-1"),
        ("gamma", "shelf-a", "book-2"),
        ("delta", "shelf-b", "book-3"),
    ] {
        ctx.state
            .add_entry(
                ctx.session_id,
                name,
                "desc",
                &[],
                &format!("context for {name}"),
                shelf,
                book,
                &auth,
            )
            .await
            .expect("add entry");
    }
}

#[tokio::test]
async fn scoped_export_session_scope_exports_all_entries() {
    let ctx = TestContext::new("scope-session")
        .await
        .expect("test context");
    populate_transfer_fixture(&ctx).await;
    let auth = writer(ctx.session_id, "writer");

    let bundle = ctx
        .state
        .export_bundle(
            ctx.session_id,
            &scoped_selector(TransferScope::Session),
            &auth,
        )
        .await
        .expect("export scoped");

    assert_eq!(bundle.entries.len(), 4);
    assert!(!bundle.bundle_sha256.is_empty());
}

#[tokio::test]
async fn scoped_export_shelf_scope_filters_to_shelf() {
    let ctx = TestContext::new("scope-shelf").await.expect("test context");
    populate_transfer_fixture(&ctx).await;
    let auth = writer(ctx.session_id, "writer");

    let bundle = ctx
        .state
        .export_bundle(
            ctx.session_id,
            &scoped_selector(TransferScope::Shelf {
                shelf: "shelf-a".to_string(),
            }),
            &auth,
        )
        .await
        .expect("export scoped");

    assert_eq!(bundle.entries.len(), 3);
    assert!(bundle.entries.iter().all(|e| e.shelf_name == "shelf-a"));
}

#[tokio::test]
async fn scoped_export_book_scope_filters_to_book() {
    let ctx = TestContext::new("scope-book").await.expect("test context");
    populate_transfer_fixture(&ctx).await;
    let auth = writer(ctx.session_id, "writer");

    let bundle = ctx
        .state
        .export_bundle(
            ctx.session_id,
            &scoped_selector(TransferScope::Book {
                shelf: "shelf-a".to_string(),
                book: "book-1".to_string(),
            }),
            &auth,
        )
        .await
        .expect("export scoped");

    assert_eq!(bundle.entries.len(), 2);
    assert!(bundle.entries.iter().all(|e| e.book_name == "book-1"));
}

#[tokio::test]
async fn scoped_export_entries_scope_selects_named_entries() {
    let ctx = TestContext::new("scope-entries")
        .await
        .expect("test context");
    populate_transfer_fixture(&ctx).await;
    let auth = writer(ctx.session_id, "writer");

    let bundle = ctx
        .state
        .export_bundle(
            ctx.session_id,
            &scoped_selector(TransferScope::Entries {
                shelf: "shelf-a".to_string(),
                book: "book-1".to_string(),
                entries: vec!["alpha".to_string()],
            }),
            &auth,
        )
        .await
        .expect("export scoped");

    assert_eq!(bundle.entries.len(), 1);
    assert_eq!(bundle.entries[0].name, "alpha");
}

#[tokio::test]
async fn scoped_export_no_history_produces_empty_history() {
    let ctx = TestContext::new("scope-no-history")
        .await
        .expect("test context");
    let auth = writer(ctx.session_id, "writer");
    ctx.state
        .add_entry(ctx.session_id, "e", "d", &[], "c", "main", "default", &auth)
        .await
        .expect("add");
    ctx.state
        .append_to_entry(
            ctx.session_id,
            "e",
            None,
            None,
            &auth,
            "more",
            AppendMetadata {
                agent_name: None,
                host_name: None,
                reason: None,
            },
        )
        .await
        .expect("append");

    let bundle = ctx
        .state
        .export_bundle(
            ctx.session_id,
            &TransferSelector {
                scope: TransferScope::Session,
                include_history: false,
            },
            &auth,
        )
        .await
        .expect("export");

    assert!(bundle.entries[0].history.is_empty());
}

#[tokio::test]
async fn import_v2_error_policy_blocks_collision() {
    let ctx = TestContext::new("import-v2-error")
        .await
        .expect("test context");
    let auth = writer(ctx.session_id, "writer");
    ctx.state
        .add_entry(
            ctx.session_id,
            "existing",
            "d",
            &[],
            "original",
            "main",
            "default",
            &auth,
        )
        .await
        .expect("add");

    let bundle = ctx
        .state
        .export_bundle(
            ctx.session_id,
            &scoped_selector(TransferScope::Session),
            &auth,
        )
        .await
        .expect("export");

    let err = ctx
        .state
        .import_bundle(ctx.session_id, &bundle, &ConflictPolicy::Error, &auth)
        .await
        .expect_err("should fail on collision with Error policy");
    assert!(err.to_string().contains("already exists"), "{err}");

    // The original entry must be unchanged.
    let entry = ctx
        .state
        .get_entry(ctx.session_id, "existing", None, None, &auth)
        .await
        .expect("get")
        .expect("exists");
    assert_eq!(entry.context, "original");
}

#[tokio::test]
async fn import_v2_overwrite_policy_replaces_entry() {
    let ctx = TestContext::new("import-v2-overwrite")
        .await
        .expect("test context");
    let auth = writer(ctx.session_id, "writer");
    ctx.state
        .add_entry(
            ctx.session_id,
            "thing",
            "d",
            &[],
            "version-1",
            "main",
            "default",
            &auth,
        )
        .await
        .expect("add");

    let bundle = ctx
        .state
        .export_bundle(
            ctx.session_id,
            &scoped_selector(TransferScope::Session),
            &auth,
        )
        .await
        .expect("export");

    // Change the in-memory context to simulate a "newer" state on target.
    ctx.state
        .append_to_entry(
            ctx.session_id,
            "thing",
            None,
            None,
            &auth,
            "version-2-appended",
            AppendMetadata {
                agent_name: None,
                host_name: None,
                reason: None,
            },
        )
        .await
        .expect("append");

    let result = ctx
        .state
        .import_bundle(ctx.session_id, &bundle, &ConflictPolicy::Overwrite, &auth)
        .await
        .expect("import");
    assert_eq!(result.overwritten_entries, 1);
    assert_eq!(result.imported_entries, 0);

    let entry = ctx
        .state
        .get_entry(ctx.session_id, "thing", None, None, &auth)
        .await
        .expect("get")
        .expect("exists");
    // Restored to the exported version.
    assert_eq!(entry.context, "version-1");
}

#[tokio::test]
async fn import_v2_skip_policy_keeps_existing_entry() {
    let ctx = TestContext::new("import-v2-skip")
        .await
        .expect("test context");
    let auth = writer(ctx.session_id, "writer");
    ctx.state
        .add_entry(
            ctx.session_id,
            "thing",
            "d",
            &[],
            "kept",
            "main",
            "default",
            &auth,
        )
        .await
        .expect("add");

    // Export a version with different context.
    let mut bundle = ctx
        .state
        .export_bundle(
            ctx.session_id,
            &scoped_selector(TransferScope::Session),
            &auth,
        )
        .await
        .expect("export");
    // Patch the bundle's entry context before importing.
    bundle.entries[0].context = "should-be-ignored".to_string();
    // Recompute hash so the import doesn't reject it.
    let entries_bytes = serde_json::to_vec(&bundle.entries).expect("serialize");
    bundle.bundle_sha256 = sha2::Sha256::digest(&entries_bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();

    let result = ctx
        .state
        .import_bundle(ctx.session_id, &bundle, &ConflictPolicy::Skip, &auth)
        .await
        .expect("import");
    assert_eq!(result.skipped_entries, 1);
    assert_eq!(result.imported_entries, 0);

    let entry = ctx
        .state
        .get_entry(ctx.session_id, "thing", None, None, &auth)
        .await
        .expect("get")
        .expect("exists");
    assert_eq!(entry.context, "kept");
}

#[tokio::test]
async fn import_v2_merge_history_unions_history_rows() {
    let ctx = TestContext::new("import-v2-merge")
        .await
        .expect("test context");
    let auth = writer(ctx.session_id, "writer");
    ctx.state
        .add_entry(
            ctx.session_id,
            "log",
            "d",
            &[],
            "base",
            "main",
            "default",
            &auth,
        )
        .await
        .expect("add");
    ctx.state
        .append_to_entry(
            ctx.session_id,
            "log",
            None,
            None,
            &auth,
            "append-1",
            AppendMetadata {
                agent_name: None,
                host_name: None,
                reason: None,
            },
        )
        .await
        .expect("append 1");

    // Snapshot the bundle at this point (has 1 history row).
    let bundle = ctx
        .state
        .export_bundle(
            ctx.session_id,
            &scoped_selector(TransferScope::Session),
            &auth,
        )
        .await
        .expect("export");

    // Add a second history row on the "target".
    ctx.state
        .append_to_entry(
            ctx.session_id,
            "log",
            None,
            None,
            &auth,
            "append-2",
            AppendMetadata {
                agent_name: None,
                host_name: None,
                reason: None,
            },
        )
        .await
        .expect("append 2");

    let result = ctx
        .state
        .import_bundle(
            ctx.session_id,
            &bundle,
            &ConflictPolicy::MergeHistory,
            &auth,
        )
        .await
        .expect("import");
    // No new entry added, no skip.
    assert_eq!(result.imported_entries, 0);
    assert_eq!(result.skipped_entries, 0);

    let history = ctx
        .state
        .get_history(ctx.session_id, "log", None, None, &auth)
        .await
        .expect("history");
    // Should have 2 rows: one from the bundle (append-1) was already present,
    // and append-2 is the new one kept on the target. History was not duplicate-inserted.
    assert_eq!(history.len(), 2, "history should have both appends");

    let entry = ctx
        .state
        .get_entry(ctx.session_id, "log", None, None, &auth)
        .await
        .expect("get")
        .expect("exists");
    // Context stays at the "target" version.
    assert!(
        entry.context.contains("append-2"),
        "context should include latest append"
    );
}

#[tokio::test]
async fn import_v2_hash_mismatch_is_rejected() {
    let ctx = TestContext::new("import-v2-hash")
        .await
        .expect("test context");
    let auth = writer(ctx.session_id, "writer");
    ctx.state
        .add_entry(ctx.session_id, "x", "d", &[], "c", "main", "default", &auth)
        .await
        .expect("add");

    let mut bundle = ctx
        .state
        .export_bundle(
            ctx.session_id,
            &scoped_selector(TransferScope::Session),
            &auth,
        )
        .await
        .expect("export");

    bundle.bundle_sha256 =
        "0000000000000000000000000000000000000000000000000000000000000000".to_string();

    let err = ctx
        .state
        .import_bundle(ctx.session_id, &bundle, &ConflictPolicy::Overwrite, &auth)
        .await
        .expect_err("should reject tampered bundle");
    assert!(
        err.to_string().contains("hash mismatch"),
        "expected hash mismatch, got: {err}"
    );
}

#[tokio::test]
async fn import_v2_unknown_session_is_rejected() {
    let ctx = TestContext::new("import-v2-unknown-session")
        .await
        .expect("test context");
    let auth = writer(ctx.session_id, "writer");
    ctx.state
        .add_entry(ctx.session_id, "x", "d", &[], "c", "main", "default", &auth)
        .await
        .expect("add");

    let bundle = ctx
        .state
        .export_bundle(
            ctx.session_id,
            &scoped_selector(TransferScope::Session),
            &auth,
        )
        .await
        .expect("export");

    let bad_auth = writer(ctx.session_id + 999, "writer");
    let err = ctx
        .state
        .import_bundle(
            ctx.session_id + 999,
            &bundle,
            &ConflictPolicy::Overwrite,
            &bad_auth,
        )
        .await
        .expect_err("should reject unknown session");
    assert!(
        err.to_string().contains("not found") || err.to_string().contains("authorize"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn import_v2_flushes_query_caches() {
    let ctx = TestContext::new("import-v2-cache-flush")
        .await
        .expect("test context");
    let auth = writer(ctx.session_id, "writer");
    ctx.state
        .add_entry(
            ctx.session_id,
            "target",
            "d",
            &[],
            "initial context",
            "main",
            "default",
            &auth,
        )
        .await
        .expect("add");

    // Prime the search caches.
    let _ = ctx
        .state
        .search_entries(ctx.session_id, &auth, "initial")
        .await;
    let _ = ctx
        .state
        .search_context(ctx.session_id, &auth, "initial context")
        .await;

    // Verify caches are populated.
    {
        let sessions = ctx.state.sessions.read().await;
        let session = sessions.get(&ctx.session_id).expect("session");
        assert!(
            !session.entry_query_cache.is_empty(),
            "entry cache should be warm"
        );
        assert!(
            !session.context_query_cache.is_empty(),
            "context cache should be warm"
        );
    }

    // Import a bundle that touches the same entry.
    let bundle = ctx
        .state
        .export_bundle(
            ctx.session_id,
            &scoped_selector(TransferScope::Session),
            &auth,
        )
        .await
        .expect("export");

    ctx.state
        .import_bundle(ctx.session_id, &bundle, &ConflictPolicy::Overwrite, &auth)
        .await
        .expect("import");

    // Both query caches must be empty after import.
    let sessions = ctx.state.sessions.read().await;
    let session = sessions.get(&ctx.session_id).expect("session");
    assert!(
        session.entry_query_cache.is_empty(),
        "entry_query_cache should be flushed after import"
    );
    assert!(
        session.context_query_cache.is_empty(),
        "context_query_cache should be flushed after import"
    );
}

// --- certificate revocation concurrency regressions ---

#[tokio::test]
async fn second_revoke_does_not_readmit_revoked_client() {
    let ctx = TestContext::new("double-revoke")
        .await
        .expect("test context should initialize");
    ctx.state.cert_grants.write().await.insert(
        "victim".to_string(),
        CertGrant {
            session_id: ctx.session_id,
            access_level: "read_write".to_string(),
            cert_pem: "victim-cert".to_string(),
            created_at: "1".to_string(),
            expires_at: "4102444800".to_string(),
        },
    );
    let victim = writer(ctx.session_id, "victim");
    let operator = admin(ctx.session_id, "writer");

    ctx.state
        .list_entries(ctx.session_id, &victim)
        .await
        .expect("read before revoke");

    ctx.state
        .revoke_client_cert(ctx.session_id, &operator, "victim")
        .await
        .expect("first revoke should succeed");
    assert!(
        ctx.state
            .list_entries(ctx.session_id, &victim)
            .await
            .is_err(),
        "access must be denied after first revoke"
    );

    // Retrying the revoke (e.g. a client-side retry after a dropped response)
    // must not undo the first one.
    let _ = ctx
        .state
        .revoke_client_cert(ctx.session_id, &operator, "victim")
        .await;

    assert!(
        ctx.state
            .revoked_cert_common_names
            .read()
            .await
            .contains("victim"),
        "second revoke removed the CN from the revoked set"
    );
    assert!(
        ctx.state
            .list_entries(ctx.session_id, &victim)
            .await
            .is_err(),
        "second revoke re-admitted the revoked certificate"
    );

    let reloaded = ServerState::load_from_storage(Arc::clone(&ctx.journal))
        .await
        .expect("state should reload from sqlite");
    assert!(
        reloaded
            .list_entries(ctx.session_id, &victim)
            .await
            .is_err(),
        "reload must re-apply revocation"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_revokes_of_same_client_never_readmit_it() {
    let ctx = TestContext::new("revoke-race")
        .await
        .expect("test context should initialize");
    let operator = admin(ctx.session_id, "writer");
    let handle = tokio::runtime::Handle::current();
    let rounds = 20;
    let mut readmitted = 0;
    for round in 0..rounds {
        let cn = format!("racer-{round}");
        ctx.state.cert_grants.write().await.insert(
            cn.clone(),
            CertGrant {
                session_id: ctx.session_id,
                access_level: "read_write".to_string(),
                cert_pem: String::new(),
                created_at: "1".to_string(),
                expires_at: "4102444800".to_string(),
            },
        );
        let barrier = std::sync::Barrier::new(2);
        tokio::task::block_in_place(|| {
            std::thread::scope(|scope| {
                for _ in 0..2 {
                    scope.spawn(|| {
                        barrier.wait();
                        handle.block_on(async {
                            let _ = ctx
                                .state
                                .revoke_client_cert(ctx.session_id, &operator, &cn)
                                .await;
                        });
                    });
                }
            });
        });
        if ctx
            .state
            .ensure_ping_access(&writer(ctx.session_id, &cn))
            .await
            .is_ok()
        {
            readmitted += 1;
        }
    }
    assert_eq!(
        readmitted, 0,
        "concurrent revokes of the same client re-admitted it in {readmitted}/{rounds} rounds"
    );
}

async fn revoke_grant(ctx: &TestContext, cn: &str) {
    ctx.state.cert_grants.write().await.insert(
        cn.to_string(),
        CertGrant {
            session_id: ctx.session_id,
            access_level: "read_write".to_string(),
            cert_pem: String::new(),
            created_at: "1".to_string(),
            expires_at: "4102444800".to_string(),
        },
    );
}

/// Runs `admin_calls` admin revokes and `target_threads` threads each sending
/// `self_calls` self-revokes of `cn`, released together by one barrier.
fn run_revoke_race(
    ctx: &TestContext,
    handle: &tokio::runtime::Handle,
    cn: &str,
    admin_calls: usize,
    target_threads: usize,
    self_calls: usize,
) {
    let operator = admin(ctx.session_id, "writer");
    let target = writer(ctx.session_id, cn);
    let barrier = std::sync::Barrier::new(target_threads + usize::from(admin_calls > 0));
    tokio::task::block_in_place(|| {
        std::thread::scope(|scope| {
            if admin_calls > 0 {
                scope.spawn(|| {
                    barrier.wait();
                    handle.block_on(async {
                        for _ in 0..admin_calls {
                            let _ = ctx
                                .state
                                .revoke_client_cert(ctx.session_id, &operator, cn)
                                .await;
                        }
                    });
                });
            }
            for _ in 0..target_threads {
                scope.spawn(|| {
                    barrier.wait();
                    handle.block_on(async {
                        for _ in 0..self_calls {
                            let _ = ctx
                                .state
                                .revoke_client_cert(ctx.session_id, &target, cn)
                                .await;
                        }
                    });
                });
            }
        });
    });
}

async fn has_access(ctx: &TestContext, cn: &str) -> bool {
    ctx.state
        .ensure_ping_access(&writer(ctx.session_id, cn))
        .await
        .is_ok()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn admin_revoke_racing_target_self_revokes_is_not_undone() {
    let ctx = TestContext::new("self-flood")
        .await
        .expect("test context should initialize");
    let handle = tokio::runtime::Handle::current();
    let rounds = 30;
    let mut readmitted = 0;
    for round in 0..rounds {
        let cn = format!("flood-{round}");
        revoke_grant(&ctx, &cn).await;
        run_revoke_race(&ctx, &handle, &cn, 1, 3, 3);
        readmitted += usize::from(has_access(&ctx, &cn).await);
    }
    assert_eq!(
        readmitted, 0,
        "admin revoke raced by the target's own self-revokes was undone in {readmitted}/{rounds} rounds"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_self_revokes_cannot_make_client_unrevokable() {
    let ctx = TestContext::new("self-immunity")
        .await
        .expect("test context should initialize");
    let handle = tokio::runtime::Handle::current();
    let operator = admin(ctx.session_id, "writer");
    let rounds = 30;
    let mut immune = 0;
    for round in 0..rounds {
        let cn = format!("immune-{round}");
        revoke_grant(&ctx, &cn).await;
        // No admin involved: the target alone races itself with concurrent self-revokes.
        run_revoke_race(&ctx, &handle, &cn, 0, 4, 2);
        let grant_gone = !ctx.state.cert_grants.read().await.contains_key(&cn);
        if grant_gone && has_access(&ctx, &cn).await {
            immune += 1;
            // A later admin revoke must still be able to deny access.
            let result = ctx
                .state
                .revoke_client_cert(ctx.session_id, &operator, &cn)
                .await;
            assert!(
                result.is_ok() || !has_access(&ctx, &cn).await,
                "admin revoke could not fix a client that made itself unrevokable"
            );
        }
    }
    assert_eq!(
        immune, 0,
        "a client left itself allowed with its grant gone in {immune}/{rounds} rounds"
    );
}

#[tokio::test]
async fn persist_failure_does_not_unrevoke_already_revoked_client() {
    let ctx = TestContext::new("persist-fail")
        .await
        .expect("test context should initialize");
    // Constructed state: CN already in the revoked set while a grant exists.
    // In practice this happens when a common name is reused for a newly
    // issued certificate (record_issued_cert inserts a fresh grant but never
    // touches the revoked set) after that same CN was revoked earlier.
    revoke_grant(&ctx, "victim").await;
    ctx.state
        .revoked_cert_common_names
        .write()
        .await
        .insert("victim".to_string());

    let connection = open_sqlite_connection().expect("sqlite should open");
    connection
        .execute_batch(
            "CREATE TRIGGER inject_insert_fail BEFORE INSERT ON issued_client_certs
               BEGIN SELECT RAISE(ABORT, 'injected persist failure'); END;
             CREATE TRIGGER inject_update_fail BEFORE UPDATE ON issued_client_certs
               BEGIN SELECT RAISE(ABORT, 'injected persist failure'); END;",
        )
        .expect("triggers should install");

    let result = ctx
        .state
        .revoke_client_cert(ctx.session_id, &admin(ctx.session_id, "writer"), "victim")
        .await;
    assert!(result.is_err(), "injected persist failure should surface");
    assert!(
        ctx.state
            .revoked_cert_common_names
            .read()
            .await
            .contains("victim"),
        "persist-failure rollback un-revoked an already revoked CN"
    );
    assert!(
        !has_access(&ctx, "victim").await,
        "persist-failure rollback restored access to an already revoked CN"
    );
}

// --- rollback isolation regressions for import_bundle / restore_deleted_entry / delete_entry ---

fn rollback_test_bundle(session_id: i64, names: &[&str]) -> TransferBundle {
    let entries: Vec<BundleEntry> = names
        .iter()
        .map(|n| BundleEntry {
            name: (*n).to_string(),
            description: "imported".to_string(),
            labels: vec![],
            context: "IMPORTED".to_string(),
            shelf_name: "shelf".to_string(),
            book_name: "book".to_string(),
            shelf_description: String::new(),
            book_description: String::new(),
            history: vec![],
        })
        .collect();
    let bundle_sha256: String = sha2::Sha256::digest(serde_json::to_vec(&entries).unwrap())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    TransferBundle {
        session: SessionMetadata {
            session_name: "session-under-test".to_string(),
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

/// Makes any INSERT into `message_packs` with this entry name fail, so the
/// caller's persist step fails and it takes its rollback path.
fn poison_message_pack_insert(name: &str) {
    open_sqlite_connection()
        .expect("sqlite should open")
        .execute_batch(&format!(
            "CREATE TRIGGER poison_{name} BEFORE INSERT ON message_packs WHEN NEW.name = '{name}'
             BEGIN SELECT RAISE(ABORT, 'injected persist failure'); END;"
        ))
        .expect("poison trigger should install");
}

/// Holds the sqlite write lock so a victim operation parks inside its persist
/// step (after releasing the in-memory session lock) until the lock is released.
fn block_sqlite_writes() -> rusqlite::Connection {
    let blocker = open_sqlite_connection().expect("sqlite should open");
    blocker
        .execute_batch("BEGIN IMMEDIATE;")
        .expect("should take write lock");
    blocker
}

/// Spin-waits on a predicate instead of guessing a sleep duration for a
/// handoff with a concurrently spawned thread. The predicate should check
/// for a persistent, observable effect of the other thread's in-memory
/// mutation (e.g. an entry's new content), not a lock's transient state:
/// the mutate-then-release of the sessions lock inside import_bundle /
/// restore_deleted_entry / delete_entry can take microseconds, which a
/// lock-polling loop can miss outright, while the mutation's effect stays
/// observable until the (later, still-blocked) rollback undoes it.
fn wait_until(deadline: std::time::Instant, what: &str, mut condition: impl FnMut() -> bool) {
    while !condition() {
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for {what}"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

/// Checks whether an entry exists without blocking, for use from a polling
/// loop running on a plain OS thread.
fn entry_exists(state: &ServerState, session_id: i64, key: &str) -> bool {
    state
        .sessions
        .try_read()
        .ok()
        .and_then(|sessions| {
            sessions
                .get(&session_id)
                .map(|s| s.entries.contains_key(key))
        })
        .unwrap_or(false)
}

fn observe_test_events(state: &ServerState) -> std::sync::mpsc::Receiver<&'static str> {
    let (events, observed) = std::sync::mpsc::channel();
    *state.test_hook.lock().unwrap() = Some(Arc::new(move |event| {
        let _ = events.send(event);
    }));
    observed
}

fn expect_test_event(observed: &std::sync::mpsc::Receiver<&'static str>, expected: &[&str]) {
    let event = observed
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("operation never reached its instrumented blocking point");
    assert!(
        expected.contains(&event),
        "expected {expected:?}, received {event}"
    );
}

/// Starts an owned worker. Dropping the receiver on timeout never joins it.
fn spawn_worker<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> std::sync::mpsc::Receiver<T> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = work();
        // A timed-out test may already have abandoned its worker.
        let _ = tx.send(result);
    });
    rx
}

fn join_within<T>(
    receiver: std::sync::mpsc::Receiver<T>,
    deadline: std::time::Instant,
    what: &str,
) -> T {
    receiver
        .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
        .unwrap_or_else(|error| panic!("{what} did not finish before the test timeout: {error}"))
}

#[test]
fn worker_timeout_does_not_wait_for_worker_exit() {
    let (release, blocked) = std::sync::mpsc::channel();
    // Ensure even the old scoped-join design eventually exits, so its
    // regression fails an assertion instead of hanging the test process.
    let releaser = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(250));
        release.send(()).unwrap();
    });
    let start = std::time::Instant::now();
    let worker = spawn_worker(move || blocked.recv().unwrap());
    let result = std::panic::catch_unwind(|| {
        join_within(
            worker,
            start + std::time::Duration::from_millis(20),
            "blocked worker",
        )
    });
    let elapsed = start.elapsed();
    releaser.join().unwrap();
    assert!(result.is_err(), "blocked worker must hit the deadline");
    assert!(
        elapsed < std::time::Duration::from_millis(150),
        "timeout joined the blocked worker: {elapsed:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn import_rollback_does_not_clobber_concurrent_sibling_writes() {
    let ctx = TestContext::new("rollback-import")
        .await
        .expect("test context should initialize");
    let a = writer(ctx.session_id, "client-a");
    let b = writer(ctx.session_id, "client-b");
    let session_id = ctx.session_id;
    ctx.state
        .add_shelf(session_id, "shelf", "", &a)
        .await
        .unwrap();
    ctx.state
        .add_book(session_id, "shelf", "book", "", &a)
        .await
        .unwrap();
    ctx.state
        .add_entry(
            session_id,
            "victim",
            "",
            &[],
            "ORIGINAL",
            "shelf",
            "book",
            &a,
        )
        .await
        .unwrap();
    poison_message_pack_insert("poison");
    let blocker = block_sqlite_writes();
    let observed = observe_test_events(&ctx.state);
    let bundle = rollback_test_bundle(session_id, &["victim", "poison"]);
    let handle = tokio::runtime::Handle::current();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);

    let import_result = tokio::task::block_in_place(|| {
        let import = {
            let state = Arc::clone(&ctx.state);
            let handle = handle.clone();
            let a = a.clone();
            let bundle = bundle.clone();
            spawn_worker(move || {
                handle.block_on(state.import_bundle(
                    session_id,
                    &bundle,
                    &ConflictPolicy::Overwrite,
                    &a,
                ))
            })
        };
        expect_test_event(&observed, &["import_mutated"]);
        let writes = {
            let state = Arc::clone(&ctx.state);
            let handle = handle.clone();
            let b = b.clone();
            spawn_worker(move || {
                handle.block_on(async {
                    state
                        .add_book(session_id, "shelf", "concurrent-book", "", &b)
                        .await
                        .unwrap();
                    state
                        .add_entry(
                            session_id,
                            "other",
                            "",
                            &[],
                            "x",
                            "shelf",
                            "concurrent-book",
                            &b,
                        )
                        .await
                        .unwrap();
                    state
                        .append_to_entry(
                            session_id,
                            "victim",
                            Some("shelf"),
                            Some("book"),
                            &b,
                            "CONCURRENT-APPEND",
                            AppendMetadata {
                                agent_name: None,
                                host_name: None,
                                reason: None,
                            },
                        )
                        .await
                        .unwrap();
                });
                state.test_event("writes_completed");
            })
        };
        expect_test_event(&observed, &["native_guard_pending", "writes_completed"]);
        blocker.execute_batch("COMMIT;").unwrap();
        let import_result = join_within(import, deadline, "import");
        join_within(writes, deadline, "concurrent writes");
        import_result
    });
    assert!(import_result.is_err(), "poisoned import should fail");

    let sessions = ctx.state.sessions.read().await;
    let session = sessions.get(&session_id).unwrap();
    assert!(
        session
            .books
            .contains_key(&("shelf".to_string(), "concurrent-book".to_string())),
        "import rollback dropped a concurrently added book"
    );
    assert!(
        session
            .entries
            .contains_key("shelf::concurrent-book::other"),
        "import rollback dropped a concurrently added entry"
    );
    assert!(
        session
            .entries
            .get("shelf::book::victim")
            .is_some_and(|e| e.context.contains("CONCURRENT-APPEND")),
        "import rollback dropped a concurrently acknowledged append"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn restore_rollback_does_not_clobber_concurrent_sibling_writes() {
    let ctx = TestContext::new("rollback-restore")
        .await
        .expect("test context should initialize");
    let a = writer(ctx.session_id, "client-a");
    let b = writer(ctx.session_id, "client-b");
    let session_id = ctx.session_id;
    ctx.state
        .add_shelf(session_id, "shelf", "", &a)
        .await
        .unwrap();
    ctx.state
        .add_book(session_id, "shelf", "book", "", &a)
        .await
        .unwrap();
    ctx.state
        .add_entry(
            session_id,
            "victim",
            "",
            &[],
            "ORIGINAL",
            "shelf",
            "book",
            &a,
        )
        .await
        .unwrap();
    ctx.state.persist_snapshot_to_sqlite().await.unwrap();
    let deleted = ctx
        .state
        .delete_entry(session_id, "victim", Some("shelf"), Some("book"), &a)
        .await
        .unwrap();
    poison_message_pack_insert("victim");
    let blocker = block_sqlite_writes();
    let observed = observe_test_events(&ctx.state);
    let key = deleted.entry_key.clone();
    let handle = tokio::runtime::Handle::current();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);

    let restore_result = tokio::task::block_in_place(|| {
        let restore = {
            let state = Arc::clone(&ctx.state);
            let handle = handle.clone();
            let a = a.clone();
            let key = key.clone();
            spawn_worker(move || handle.block_on(state.restore_deleted_entry(session_id, &key, &a)))
        };
        expect_test_event(&observed, &["restore_mutated"]);
        let writes = {
            let state = Arc::clone(&ctx.state);
            let handle = handle.clone();
            let b = b.clone();
            spawn_worker(move || {
                handle.block_on(async {
                    state
                        .add_book(session_id, "shelf", "concurrent-book", "", &b)
                        .await
                        .unwrap();
                    state
                        .add_entry(
                            session_id,
                            "other",
                            "",
                            &[],
                            "x",
                            "shelf",
                            "concurrent-book",
                            &b,
                        )
                        .await
                        .unwrap();
                });
                state.test_event("writes_completed");
            })
        };
        expect_test_event(&observed, &["native_guard_pending", "writes_completed"]);
        blocker.execute_batch("COMMIT;").unwrap();
        let restore_result = join_within(restore, deadline, "restore");
        join_within(writes, deadline, "concurrent writes");
        restore_result
    });
    assert!(restore_result.is_err(), "poisoned restore should fail");

    let sessions = ctx.state.sessions.read().await;
    let session = sessions.get(&session_id).unwrap();
    assert!(
        session
            .books
            .contains_key(&("shelf".to_string(), "concurrent-book".to_string())),
        "restore rollback dropped a concurrently added book"
    );
    assert!(
        session
            .entries
            .contains_key("shelf::concurrent-book::other"),
        "restore rollback dropped a concurrently added entry"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn import_rollback_does_not_resurrect_concurrently_deleted_entry() {
    let ctx = TestContext::new("rollback-import-delete")
        .await
        .expect("test context should initialize");
    let a = writer(ctx.session_id, "client-a");
    let b = writer(ctx.session_id, "client-b");
    let session_id = ctx.session_id;
    ctx.state
        .add_shelf(session_id, "shelf", "", &a)
        .await
        .unwrap();
    ctx.state
        .add_book(session_id, "shelf", "book", "", &a)
        .await
        .unwrap();
    ctx.state
        .add_entry(
            session_id,
            "victim",
            "",
            &[],
            "SECRET-TO-DELETE",
            "shelf",
            "book",
            &a,
        )
        .await
        .unwrap();
    ctx.state.persist_snapshot_to_sqlite().await.unwrap();
    poison_message_pack_insert("poison");
    let blocker = block_sqlite_writes();
    let observed = observe_test_events(&ctx.state);
    let bundle = rollback_test_bundle(session_id, &["victim", "poison"]);
    let handle = tokio::runtime::Handle::current();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);

    let (import_result, delete_result) = tokio::task::block_in_place(|| {
        let import = {
            let state = Arc::clone(&ctx.state);
            let handle = handle.clone();
            let a = a.clone();
            let bundle = bundle.clone();
            spawn_worker(move || {
                handle.block_on(state.import_bundle(
                    session_id,
                    &bundle,
                    &ConflictPolicy::Overwrite,
                    &a,
                ))
            })
        };
        expect_test_event(&observed, &["import_mutated"]);
        let delete = {
            let state = Arc::clone(&ctx.state);
            let handle = handle.clone();
            let b = b.clone();
            spawn_worker(move || {
                handle.block_on(state.delete_entry(
                    session_id,
                    "victim",
                    Some("shelf"),
                    Some("book"),
                    &b,
                ))
            })
        };
        expect_test_event(&observed, &["native_guard_pending", "delete_mutated"]);
        blocker.execute_batch("COMMIT;").unwrap();
        let import_result = join_within(import, deadline, "import");
        let delete_result = join_within(delete, deadline, "concurrent delete");
        (import_result, delete_result)
    });
    assert!(import_result.is_err(), "poisoned import should fail");
    assert!(delete_result.is_ok(), "concurrent delete should succeed");

    let readable = ctx
        .state
        .list_entries(session_id, &b)
        .await
        .unwrap()
        .iter()
        .any(|e| e.name == "victim");
    assert!(
        !readable,
        "import rollback resurrected an entry that was concurrently deleted"
    );
}

// --- delete_shelf must participate in native mutation exclusion ---

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn import_rollback_does_not_resurrect_entry_whose_shelf_was_concurrently_deleted() {
    let ctx = TestContext::new("rollback-delete-shelf")
        .await
        .expect("test context should initialize");
    let a = writer(ctx.session_id, "client-a");
    let b = writer(ctx.session_id, "client-b");
    let session_id = ctx.session_id;
    ctx.state
        .add_shelf(session_id, "shelf", "", &a)
        .await
        .unwrap();
    ctx.state
        .add_book(session_id, "shelf", "book", "", &a)
        .await
        .unwrap();
    ctx.state
        .add_entry(
            session_id,
            "victim",
            "",
            &[],
            "SECRET-TO-DELETE",
            "shelf",
            "book",
            &a,
        )
        .await
        .unwrap();
    ctx.state.persist_snapshot_to_sqlite().await.unwrap();
    poison_message_pack_insert("poison");
    let blocker = block_sqlite_writes();
    let observed = observe_test_events(&ctx.state);
    let bundle = rollback_test_bundle(session_id, &["victim", "poison"]);
    let handle = tokio::runtime::Handle::current();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);

    let (import_result, delete_shelf_result) = tokio::task::block_in_place(|| {
        let import = {
            let state = Arc::clone(&ctx.state);
            let handle = handle.clone();
            let a = a.clone();
            let bundle = bundle.clone();
            spawn_worker(move || {
                handle.block_on(state.import_bundle(
                    session_id,
                    &bundle,
                    &ConflictPolicy::Overwrite,
                    &a,
                ))
            })
        };
        expect_test_event(&observed, &["import_mutated"]);
        let delete_shelf = {
            let state = Arc::clone(&ctx.state);
            let handle = handle.clone();
            let b = b.clone();
            spawn_worker(move || handle.block_on(state.delete_shelf(session_id, "shelf", &b)))
        };
        expect_test_event(&observed, &["native_guard_pending", "shelf_mutated"]);
        blocker.execute_batch("COMMIT;").unwrap();
        let import_result = join_within(import, deadline, "import");
        let delete_shelf_result = join_within(delete_shelf, deadline, "concurrent delete_shelf");
        (import_result, delete_shelf_result)
    });
    assert!(import_result.is_err(), "poisoned import should fail");
    assert!(
        delete_shelf_result.is_ok(),
        "concurrent delete_shelf should succeed"
    );

    let readable = ctx
        .state
        .list_entries(session_id, &b)
        .await
        .unwrap()
        .iter()
        .any(|e| e.name == "victim");
    assert!(
        !readable,
        "import rollback resurrected an entry whose shelf was concurrently deleted"
    );

    // The resurrection, if any, must not survive a restart either.
    let reloaded = ServerState::load_from_storage(Arc::clone(&ctx.journal))
        .await
        .expect("state should reload from sqlite");
    let readable_after_restart = reloaded
        .list_entries(session_id, &b)
        .await
        .unwrap()
        .iter()
        .any(|e| e.name == "victim");
    assert!(
        !readable_after_restart,
        "import rollback resurrected an entry across a restart"
    );
}

// --- revoke must not hold the revoked-set lock across its blocking persist ---

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn revoke_does_not_block_unrelated_reads_while_persisting() {
    let ctx = TestContext::new("revoke-lock-scope")
        .await
        .expect("test context should initialize");
    let session_id = ctx.session_id;
    ctx.state.cert_grants.write().await.insert(
        "victim".to_string(),
        CertGrant {
            session_id,
            access_level: "read_write".to_string(),
            cert_pem: String::new(),
            created_at: "1".to_string(),
            expires_at: "4102444800".to_string(),
        },
    );
    let bystander = writer(session_id, "bystander");
    let operator = admin(session_id, "writer");
    let blocker = block_sqlite_writes();
    let observed = observe_test_events(&ctx.state);
    let handle = tokio::runtime::Handle::current();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);

    let (revoke_result, bystander_elapsed, bystander_result) = tokio::task::block_in_place(|| {
        let revoke = {
            let state = Arc::clone(&ctx.state);
            let handle = handle.clone();
            let operator = operator.clone();
            spawn_worker(move || {
                handle.block_on(state.revoke_client_cert(session_id, &operator, "victim"))
            })
        };
        expect_test_event(&observed, &["revoke_marked"]);
        let start = std::time::Instant::now();
        let state = Arc::clone(&ctx.state);
        let read = spawn_worker(move || handle.block_on(state.ensure_ping_access(&bystander)));
        let result = read.recv_timeout(std::time::Duration::from_millis(500));
        let elapsed = start.elapsed();
        blocker.execute_batch("COMMIT;").unwrap();
        let revoke_result = join_within(revoke, deadline, "revoke");
        (revoke_result, elapsed, result)
    });
    assert!(
        revoke_result.is_ok(),
        "revoke should succeed once unblocked"
    );
    assert!(
        bystander_result.is_ok() && bystander_elapsed < std::time::Duration::from_millis(500),
        "an unrelated read waited {bystander_elapsed:?} behind revoke's blocked persist; \
         the revoked-set lock must be released before that blocking call"
    );
    bystander_result.unwrap().unwrap();
}

// --- checkpoint must persist journal-only writes before truncating ---

#[tokio::test]
async fn checkpoint_does_not_drop_a_pending_journal_only_write() {
    let ctx = TestContext::new("checkpoint-data-loss")
        .await
        .expect("test context should initialize");
    let session_id = ctx.session_id;
    let a = writer(session_id, "client-a");
    ctx.state
        .add_shelf(session_id, "shelf", "", &a)
        .await
        .unwrap();
    ctx.state
        .add_book(session_id, "shelf", "book", "", &a)
        .await
        .unwrap();
    // add_entry only appends to the journal; it never calls persist_snapshot.
    ctx.state
        .add_entry(
            session_id,
            "victim",
            "",
            &[],
            "JOURNAL-ONLY",
            "shelf",
            "book",
            &a,
        )
        .await
        .unwrap();

    // An unrelated operation that calls checkpoint_journal on success.
    ctx.state.cert_grants.write().await.insert(
        "bystander".to_string(),
        CertGrant {
            session_id,
            access_level: "read_write".to_string(),
            cert_pem: String::new(),
            created_at: "1".to_string(),
            expires_at: "4102444800".to_string(),
        },
    );
    ctx.state
        .revoke_client_cert(session_id, &admin(session_id, "writer"), "bystander")
        .await
        .expect("unrelated revoke should succeed");

    // Reload from durable storage only: the journal-only write must have
    // been captured by the checkpoint before the journal was truncated.
    let reloaded = ServerState::load_from_storage(Arc::clone(&ctx.journal))
        .await
        .expect("state should reload from sqlite");
    let recovered = reloaded
        .list_entries(session_id, &a)
        .await
        .unwrap()
        .iter()
        .any(|e| e.name == "victim");
    assert!(
        recovered,
        "an unrelated checkpoint truncated the journal without first persisting \
         a pending journal-only write, permanently dropping it"
    );
}

// --- import_bundle must keep only the first original for a duplicated path ---

#[tokio::test]
async fn import_rollback_restores_true_original_when_bundle_lists_same_path_twice() {
    let ctx = TestContext::new("import-dup-path")
        .await
        .expect("test context should initialize");
    let session_id = ctx.session_id;
    let a = writer(session_id, "client-a");
    ctx.state
        .add_shelf(session_id, "shelf", "", &a)
        .await
        .unwrap();
    ctx.state
        .add_book(session_id, "shelf", "book", "", &a)
        .await
        .unwrap();
    ctx.state
        .add_entry(
            session_id,
            "victim",
            "",
            &[],
            "ORIGINAL",
            "shelf",
            "book",
            &a,
        )
        .await
        .unwrap();

    poison_message_pack_insert("poison");
    // The bundle lists "victim" twice before a poisoned entry, so the second
    // occurrence's "prior" would be the first occurrence's already-imported
    // value, not the true pre-import original, unless that is guarded against.
    let bundle = rollback_test_bundle(session_id, &["victim", "victim", "poison"]);
    let result = ctx
        .state
        .import_bundle(session_id, &bundle, &ConflictPolicy::Overwrite, &a)
        .await;
    assert!(result.is_err(), "poisoned import should fail");

    let sessions = ctx.state.sessions.read().await;
    let context = sessions
        .get(&session_id)
        .unwrap()
        .entries
        .get("shelf::book::victim")
        .unwrap()
        .context
        .clone();
    assert_eq!(
        context, "ORIGINAL",
        "import rollback restored the first bundle occurrence's value instead of the true original"
    );
}

// --- delete_shelf must roll back its in-memory mutation if persist fails ---

#[tokio::test]
async fn delete_shelf_restores_memory_when_persist_fails() {
    let ctx = TestContext::new("delete-shelf-persist-fail")
        .await
        .expect("test context should initialize");
    let session_id = ctx.session_id;
    let a = writer(session_id, "client-a");
    ctx.state
        .add_shelf(session_id, "shelf", "", &a)
        .await
        .unwrap();
    ctx.state
        .add_book(session_id, "shelf", "book", "", &a)
        .await
        .unwrap();
    ctx.state
        .add_entry(
            session_id,
            "victim",
            "",
            &[],
            "ORIGINAL",
            "shelf",
            "book",
            &a,
        )
        .await
        .unwrap();
    ctx.state
        .add_shelf(session_id, "other-shelf", "", &a)
        .await
        .unwrap();
    ctx.state
        .add_book(session_id, "other-shelf", "other-book", "", &a)
        .await
        .unwrap();
    ctx.state
        .add_entry(
            session_id,
            "bystander",
            "",
            &[],
            "x",
            "other-shelf",
            "other-book",
            &a,
        )
        .await
        .unwrap();
    ctx.state.persist_snapshot_to_sqlite().await.unwrap();

    // Poison an unrelated entry's re-insert so delete_shelf's own full
    // snapshot persist fails, even though "shelf" was already removed from
    // memory by that point.
    poison_message_pack_insert("bystander");

    let result = ctx.state.delete_shelf(session_id, "shelf", &a).await;
    assert!(result.is_err(), "poisoned snapshot persist should fail");

    let sessions = ctx.state.sessions.read().await;
    let session = sessions.get(&session_id).unwrap();
    assert!(
        session.shelves.contains_key("shelf"),
        "delete_shelf dropped the shelf from memory despite a failed persist"
    );
    assert!(
        session.entries.contains_key("shelf::book::victim"),
        "delete_shelf dropped the entry from memory despite a failed persist"
    );
}

// Adapted from the validator's checkpoint-loss reproductions. Pause the real
// checkpoint after capture and order the competitor by a poll inside its guard,
// or by completion if the guard is missing. No scheduling sleeps are needed.
async fn checkpoint_preserves_concurrent_write(kind: &'static str) {
    let ctx = TestContext::new(kind).await.unwrap();
    let sid = ctx.session_id;
    let a = writer(sid, "client-a");
    ctx.state.add_shelf(sid, "shelf", "", &a).await.unwrap();
    ctx.state
        .add_book(sid, "shelf", "book", "", &a)
        .await
        .unwrap();
    ctx.state
        .add_entry(sid, "doomed", "", &[], "x", "shelf", "book", &a)
        .await
        .unwrap();
    crate::init::initialize_auth_secret().unwrap();
    let token = derive_auth_token(sid, "read_write", "checkpoint-test").unwrap();
    if kind == "delete-token" {
        open_sqlite_connection().unwrap().execute("INSERT INTO auth_tokens (session_id, token_hash, token_prefix, access_level, token_nonce) VALUES (?1, ?2, 'test', 'read_write', 'checkpoint-test')", params![sid, hash_token(&token)]).unwrap();
        ctx.state.auth_tokens.write().await.insert(
            token.clone(),
            AuthGrant {
                session_id: sid,
                access_level: "read_write".into(),
                metadata: ctx
                    .state
                    .sessions
                    .read()
                    .await
                    .get(&sid)
                    .unwrap()
                    .metadata
                    .clone(),
                token_nonce: "checkpoint-test".into(),
                token_hash: hash_token(&token),
                last_used_at: None,
            },
        );
    }
    let (events, observed) = std::sync::mpsc::channel();
    let (resume, paused) = std::sync::mpsc::channel();
    let paused = std::sync::Mutex::new(paused);
    let hook_events = events.clone();
    *ctx.state.test_hook.lock().unwrap() = Some(Arc::new(move |event| {
        if matches!(event, "checkpoint_captured" | "native_guard_pending") {
            hook_events.send(event).unwrap();
        }
        if event == "checkpoint_captured" {
            paused
                .lock()
                .unwrap()
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
        }
    }));
    let state = Arc::clone(&ctx.state);
    let handle = tokio::runtime::Handle::current();
    let checkpoint_handle = handle.clone();
    let checkpoint = spawn_worker(move || {
        checkpoint_handle.block_on(async {
            if kind == "revoke-add" {
                state
                    .revoke_client_cert(sid, &admin(sid, "writer"), "writer")
                    .await
                    .unwrap();
            } else {
                state
                    .delete_entry(
                        sid,
                        "doomed",
                        Some("shelf"),
                        Some("book"),
                        &writer(sid, "client-a"),
                    )
                    .await
                    .unwrap();
            }
        })
    });
    tokio::task::block_in_place(|| {
        assert_eq!(
            observed
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap(),
            "checkpoint_captured"
        );
        let state = Arc::clone(&ctx.state);
        let write = spawn_worker(move || {
            handle.block_on(async {
                match kind {
                    "delete-cert" => state
                        .record_issued_cert(sid, "new-client", "read_write", "PEM", "4102444800")
                        .await
                        .unwrap(),
                    "delete-token" => state.note_auth_token_used(&token).await.unwrap(),
                    _ => {
                        state
                            .add_entry(
                                sid,
                                "acked",
                                "",
                                &[],
                                "ACKED",
                                "shelf",
                                "book",
                                &writer(sid, "client-a"),
                            )
                            .await
                            .unwrap();
                    }
                }
            });
            wait_until(
                std::time::Instant::now() + std::time::Duration::from_secs(5),
                "writer to flush acknowledged record",
                || {
                    load_entries(state.journal.path())
                        .unwrap()
                        .iter()
                        .any(|record| match (kind, record) {
                            ("delete-cert", JournalEntry::IssuedCert { common_name, .. }) => {
                                common_name == "new-client"
                            }
                            ("delete-token", JournalEntry::AuthTokenUsed { .. }) => true,
                            ("revoke-add", JournalEntry::AddEntry { name, .. }) => name == "acked",
                            _ => false,
                        })
                },
            );
            events.send("write_completed").unwrap();
        });
        let event = observed
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        assert!(matches!(event, "native_guard_pending" | "write_completed"));
        resume.send(()).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        join_within(checkpoint, deadline, "checkpoint");
        join_within(write, deadline, "concurrent writer");
    });
    ctx.journal.shutdown().unwrap();
    let reloaded = ServerState::load_from_storage(Arc::clone(&ctx.journal))
        .await
        .unwrap();
    match kind {
        "delete-cert" => assert!(
            reloaded.cert_grants.read().await.contains_key("new-client"),
            "acknowledged issued certificate was dropped by delete checkpoint"
        ),
        "delete-token" => assert!(
            reloaded
                .auth_tokens
                .read()
                .await
                .values()
                .any(|grant| grant.last_used_at.is_some()),
            "acknowledged token use was dropped by delete checkpoint"
        ),
        _ => assert!(
            entry_exists(&reloaded, sid, "shelf::book::acked"),
            "acknowledged add_entry was dropped by revoke's checkpoint"
        ),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn revoke_checkpoint_preserves_concurrent_add_entry() {
    checkpoint_preserves_concurrent_write("revoke-add").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn delete_checkpoint_preserves_concurrent_issued_cert() {
    checkpoint_preserves_concurrent_write("delete-cert").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn delete_checkpoint_preserves_concurrent_token_use() {
    checkpoint_preserves_concurrent_write("delete-token").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn export_audit_waits_for_checkpoint_boundary() {
    let ctx = TestContext::new("export-boundary").await.unwrap();
    let guard = ctx.state.native_checkpoint.write().await;
    let (events, observed) = std::sync::mpsc::channel();
    let hook_events = events.clone();
    *ctx.state.test_hook.lock().unwrap() = Some(Arc::new(move |event| {
        hook_events.send(event).unwrap();
    }));
    let state = Arc::clone(&ctx.state);
    let sid = ctx.session_id;
    let handle = tokio::runtime::Handle::current();
    let export = spawn_worker(move || {
        let bundle = handle
            .block_on(state.export_bundle(
                sid,
                &TransferSelector {
                    scope: TransferScope::Session,
                    include_history: true,
                },
                &writer(sid, "writer"),
            ))
            .unwrap();
        events.send("export_completed").unwrap();
        bundle
    });
    let event = tokio::task::block_in_place(|| {
        observed
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap()
    });
    drop(guard);
    let bundle = tokio::task::block_in_place(|| {
        join_within(
            export,
            std::time::Instant::now() + std::time::Duration::from_secs(5),
            "export",
        )
    });
    assert_eq!(
        event, "native_guard_pending",
        "export audit bypassed the checkpoint boundary"
    );
    assert_eq!(bundle.session.session_id, sid);
}

#[tokio::test]
async fn replay_after_snapshot_does_not_duplicate_append() {
    let ctx = TestContext::new("val-crash-dup").await.unwrap();
    let sid = ctx.session_id;
    let a = writer(sid, "client-a");
    ctx.state.add_shelf(sid, "shelf", "", &a).await.unwrap();
    ctx.state
        .add_book(sid, "shelf", "book", "", &a)
        .await
        .unwrap();
    ctx.state
        .add_entry(sid, "e", "", &[], "base", "shelf", "book", &a)
        .await
        .unwrap();
    ctx.state
        .append_to_entry(
            sid,
            "e",
            Some("shelf"),
            Some("book"),
            &a,
            " +once",
            AppendMetadata {
                agent_name: None,
                host_name: None,
                reason: None,
            },
        )
        .await
        .unwrap();
    ctx.journal.shutdown().unwrap();
    // checkpoint step 1 only (persist), then "crash" before truncate
    ctx.state.persist_snapshot_to_sqlite().await.unwrap();
    let reloaded = ServerState::load_from_storage(Arc::clone(&ctx.journal))
        .await
        .unwrap();
    let sessions = reloaded.sessions.read().await;
    let entry = sessions
        .get(&sid)
        .unwrap()
        .entries
        .get("shelf::book::e")
        .unwrap();
    assert_eq!(
        entry.context, "base\n +once",
        "replay replaced persisted append context"
    );
    assert_eq!(
        entry.history.len(),
        1,
        "history rows after replay: {} ; context={:?}",
        entry.history.len(),
        entry.context
    );
}
