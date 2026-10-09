// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::path::PathBuf;

use crate::init::*;

fn temp_db_path(test_name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("ccp-{test_name}-{}.sqlite3", Uuid::new_v4()))
}

#[test]
fn hash_token_is_stable_and_non_plaintext() {
    let token = "agent-bootstrap-token";

    let first = hash_token(token);
    let second = hash_token(token);

    assert_eq!(first, second);
    assert_ne!(first, token);
    assert_eq!(first.len(), 64);
}

#[test]
fn init_sqlite_creates_expected_tables() {
    let db_path = temp_db_path("schema");
    init_sqlite(&db_path).expect("schema should initialize");

    let connection = Connection::open(&db_path).expect("sqlite should open");
    let mut stmt = connection
        .prepare(
            "SELECT name
             FROM sqlite_master
             WHERE type IN ('table', 'view')
               AND name IN ('sessions', 'auth_tokens', 'shelves', 'books', 'message_packs', 'issued_client_certs', 'message_history')",
        )
        .expect("sqlite should prepare");

    let names = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .expect("sqlite should query")
        .collect::<Result<Vec<_>, _>>()
        .expect("sqlite should collect");

    assert_eq!(names.len(), 7);

    fs::remove_file(&db_path).expect("temp db should be removed");
}

#[test]
fn issued_enrollment_tokens_store_hash_only_with_expiry() {
    let _env_guard = test_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let data_dir = std::env::temp_dir().join(format!("ccp-init-test-{}", Uuid::new_v4()));
    let db_path = data_dir.join("ccp.sqlite3");
    fs::create_dir_all(&data_dir).expect("temp data dir should be created");
    unsafe {
        std::env::set_var(SERVER_DATA_DIR_ENV, &data_dir);
    }
    init_sqlite(&db_path).expect("schema should initialize");

    let mut connection = Connection::open(&db_path).expect("sqlite should open");
    configure_sqlite(&connection).expect("sqlite pragmas should apply");
    let session_id =
        ensure_runtime_session(&mut connection, "shared-agents").expect("session should exist");
    drop(connection);

    let issued = issue_enrollment_token("shared-agents", "read", Some(3600))
        .expect("read token should issue");

    let connection = Connection::open(&db_path).expect("sqlite should reopen");
    let stored = connection
        .query_row(
            "SELECT token_value, token_hash, token_prefix, expires_at
             FROM auth_tokens
             WHERE session_id = ?1 AND access_level = 'read'",
            [session_id],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .expect("stored token row should load");

    assert_eq!(stored.0, None);
    assert_eq!(stored.1, hash_token(&issued.token));
    assert_eq!(stored.2, token_prefix(&issued.token));
    assert!(!stored.3.is_empty());

    fs::remove_file(&db_path).expect("temp db should be removed");
    unsafe {
        std::env::remove_var(SERVER_DATA_DIR_ENV);
    }
    fs::remove_dir_all(&data_dir).expect("temp data dir should be removed");
}

#[test]
fn init_sqlite_auth_tokens_schema_omits_consumed_at() {
    let db_path = temp_db_path("auth-token-columns");
    init_sqlite(&db_path).expect("schema should initialize");

    let connection = Connection::open(&db_path).expect("sqlite should open");
    let mut stmt = connection
        .prepare("PRAGMA table_info(auth_tokens)")
        .expect("sqlite should prepare table info query");
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .expect("sqlite should query table info")
        .collect::<Result<Vec<_>, _>>()
        .expect("sqlite should collect auth_tokens columns");

    assert!(!columns.iter().any(|column| column == "consumed_at"));

    fs::remove_file(&db_path).expect("temp db should be removed");
}

#[test]
fn session_slug_is_stable_and_sanitized() {
    let slug = session_slug("Alpha / Beta");
    assert!(slug.starts_with("alpha-beta-"));
    assert_eq!(slug.len(), "alpha-beta-".len() + 8);
    assert_eq!(slug, session_slug("Alpha / Beta"));
}

#[test]
fn configure_session_data_dir_defaults_to_per_session_home() {
    let _env_guard = test_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let server_home = std::env::temp_dir().join(format!("ccp-session-home-{}", Uuid::new_v4()));
    unsafe {
        std::env::remove_var(SERVER_DATA_DIR_ENV);
        std::env::set_var(SERVER_HOME_ENV, &server_home);
    }

    let resolved = config_session_dir("Alpha / Beta").expect("data dir should resolve");

    assert_eq!(resolved, server_home.join(session_slug("Alpha / Beta")));
    assert_eq!(server_data_dir(), resolved);

    unsafe {
        std::env::remove_var(SERVER_DATA_DIR_ENV);
        std::env::remove_var(SERVER_HOME_ENV);
    }
}

#[test]
fn configure_session_data_dir_prefers_existing_session_directory() {
    let _env_guard = test_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let server_home = std::env::temp_dir().join(format!("ccp-session-home-{}", Uuid::new_v4()));
    let existing_dir = server_home.join("custom-session-dir");
    fs::create_dir_all(&existing_dir).expect("existing session dir should be created");
    fs::write(
        existing_dir.join("active_session.json"),
        serde_json::to_vec_pretty(&SessionBinding {
            session_id: 7,
            session_name: "existing-session".to_string(),
        })
        .expect("binding should serialize"),
    )
    .expect("binding should write");

    unsafe {
        std::env::remove_var(SERVER_DATA_DIR_ENV);
        std::env::set_var(SERVER_HOME_ENV, &server_home);
    }

    let resolved =
        config_session_dir("existing-session").expect("existing data dir should resolve");

    assert_eq!(resolved, existing_dir);

    unsafe {
        std::env::remove_var(SERVER_DATA_DIR_ENV);
        std::env::remove_var(SERVER_HOME_ENV);
    }
    fs::remove_dir_all(&server_home).expect("temp server home should be removed");
}

#[test]
fn validate_auth_formating_rejects_remote_http_auth() {
    let _env_guard = test_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    unsafe {
        std::env::set_var(AUTH_LISTENER_ADDR_ENV, "127.0.0.1:1337");
        std::env::set_var(AUTH_SERVER_BASE_URL_ENV, "http://192.168.1.10:1337");
    }
    let error = validate_auth_formating().expect_err("remote HTTP auth should be rejected");
    assert!(error.to_string().contains("auth base URL must use https"));
    unsafe {
        std::env::remove_var(AUTH_LISTENER_ADDR_ENV);
        std::env::remove_var(AUTH_SERVER_BASE_URL_ENV);
    }
}

#[test]
fn validate_auth_formating_allows_non_loopback_listener_with_opt_in() {
    let _env_guard = test_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    unsafe {
        std::env::set_var(AUTH_LISTENER_ADDR_ENV, "0.0.0.0:1337");
        std::env::set_var(AUTH_SERVER_BASE_URL_ENV, "http://127.0.0.1:1337");
        std::env::set_var(ALLOW_NON_LOOPBACK_AUTH_LISTENER_ENV, "1");
    }

    validate_auth_formating().expect("container opt-in should allow non-loopback auth listener");

    unsafe {
        std::env::remove_var(AUTH_LISTENER_ADDR_ENV);
        std::env::remove_var(AUTH_SERVER_BASE_URL_ENV);
        std::env::remove_var(ALLOW_NON_LOOPBACK_AUTH_LISTENER_ENV);
    }
}

#[test]
fn schema_version_is_recorded_after_init() {
    let _env_guard = test_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let data_dir = std::env::temp_dir().join(format!("ccp-schema-ver-{}", Uuid::new_v4()));
    unsafe {
        std::env::set_var(SERVER_DATA_DIR_ENV, &data_dir);
    }

    init_sqlite(&db_path()).expect("schema should initialize");

    let connection = open_sqlite_connection().expect("should open db");
    let version: u32 = connection
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_version",
            [],
            |row| row.get(0),
        )
        .expect("should query schema version");

    assert_eq!(
        version, SCHEMA_VERSION,
        "schema version should match constant"
    );
    assert!(version > 0, "schema version should be positive");

    unsafe {
        std::env::remove_var(SERVER_DATA_DIR_ENV);
    }
    let _ = std::fs::remove_dir_all(&data_dir);
}

#[test]
fn health_check_includes_version_info() {
    let _env_guard = test_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let data_dir = std::env::temp_dir().join(format!("ccp-health-ver-{}", Uuid::new_v4()));
    unsafe {
        std::env::set_var(SERVER_DATA_DIR_ENV, &data_dir);
    }

    init_sqlite(&db_path()).expect("schema should initialize");
    let mut connection = open_sqlite_connection().expect("should open db");
    let _session_id =
        ensure_runtime_session(&mut connection, "health-ver-test").expect("should create session");
    drop(connection);

    let health = check_server_health("health-ver-test").expect("health check should work");
    assert_eq!(health.protocol_version, protocol::PROTOCOL_VERSION);
    assert_eq!(health.schema_version, SCHEMA_VERSION);
    assert!(!health.server_version.is_empty());

    unsafe {
        std::env::remove_var(SERVER_DATA_DIR_ENV);
    }
    let _ = std::fs::remove_dir_all(&data_dir);
}

#[test]
fn agent_status_ttl_defaults_and_rejects_invalid_values() {
    let _env_guard = test_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    for invalid in ["0", "not-a-number"] {
        unsafe { std::env::set_var(AGENT_STATUS_TTL_SECONDS_ENV, invalid) };
        assert_eq!(agent_status_ttl_seconds(), 10_800);
    }
    unsafe { std::env::set_var(AGENT_STATUS_TTL_SECONDS_ENV, "7200") };
    assert_eq!(agent_status_ttl_seconds(), 7200);
    unsafe { std::env::remove_var(AGENT_STATUS_TTL_SECONDS_ENV) };
}

#[test]
fn schema_v3_creates_agent_status_storage_and_index() {
    let db_path = temp_db_path("agent-status-schema");
    init_sqlite(&db_path).expect("schema should initialize");
    let connection = Connection::open(&db_path).expect("sqlite should open");
    let table_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'agent_statuses'",
            [],
            |row| row.get(0),
        )
        .expect("table lookup should work");
    let index_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = 'idx_agent_statuses_team_expiry'",
            [],
            |row| row.get(0),
        )
        .expect("index lookup should work");
    assert_eq!(table_count, 1);
    assert_eq!(index_count, 1);
    fs::remove_file(db_path).expect("temp db should be removed");
}

#[test]
fn schema_v2_upgrade_preserves_existing_sessions_and_adds_status_storage() {
    let db_path = temp_db_path("agent-status-v2-upgrade");
    init_sqlite(&db_path).expect("baseline schema should initialize");
    let connection = Connection::open(&db_path).expect("sqlite should open");
    connection
        .execute(
            "INSERT INTO sessions (name, description) VALUES ('existing-session', 'keep me')",
            [],
        )
        .expect("legacy session should insert");
    connection
        .execute_batch(
            "DROP TABLE agent_statuses;
             DELETE FROM schema_version;
             INSERT INTO schema_version (version) VALUES (2);",
        )
        .expect("database should be staged as schema v2");
    drop(connection);

    init_sqlite(&db_path).expect("v2 database should upgrade");
    let connection = Connection::open(&db_path).expect("upgraded sqlite should open");
    let description: String = connection
        .query_row(
            "SELECT description FROM sessions WHERE name = 'existing-session'",
            [],
            |row| row.get(0),
        )
        .expect("existing session should survive migration");
    let version: u32 = connection
        .query_row("SELECT MAX(version) FROM schema_version", [], |row| {
            row.get(0)
        })
        .expect("schema version should query");
    let status_table: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'agent_statuses'",
            [],
            |row| row.get(0),
        )
        .expect("status table should exist after upgrade");
    assert_eq!(description, "keep me");
    assert_eq!(version, SCHEMA_VERSION);
    assert_eq!(status_table, 1);

    fs::remove_file(db_path).expect("temp db should be removed");
}

#[test]
fn initialization_preserves_existing_metadata_without_environment_overrides() {
    let _env_guard = test_env_lock().lock().unwrap_or_else(|e| e.into_inner());
    let keys = [
        SESSION_OWNER_ENV,
        SESSION_LABELS_ENV,
        SESSION_VISIBILITY_ENV,
        SESSION_PURPOSE_ENV,
    ];
    let previous = keys.map(std::env::var_os);
    for key in keys {
        unsafe {
            std::env::remove_var(key);
        }
    }
    let db_path = temp_db_path("metadata-preserved");
    init_sqlite(&db_path).unwrap();
    let mut connection = Connection::open(&db_path).unwrap();
    connection.execute("INSERT INTO sessions(name, owner, labels, visibility, purpose) VALUES('existing', 'owner', 'one,two', 'private', 'custom')", []).unwrap();
    ensure_runtime_session(&mut connection, "existing").unwrap();
    let metadata: (String, String, String, String) = connection
        .query_row(
            "SELECT owner, labels, visibility, purpose FROM sessions WHERE name='existing'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    for (key, value) in keys.into_iter().zip(previous) {
        unsafe {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
    assert_eq!(
        metadata,
        (
            "owner".into(),
            "one,two".into(),
            "private".into(),
            "custom".into()
        )
    );
    drop(connection);
    fs::remove_file(db_path).unwrap();
}

#[test]
fn initialization_refuses_a_newer_database_schema() {
    let db_path = temp_db_path("future-schema");
    let connection = Connection::open(&db_path).unwrap();
    connection.execute_batch("CREATE TABLE schema_version(version INTEGER NOT NULL); INSERT INTO schema_version VALUES(999);").unwrap();
    assert!(
        init_sqlite(&db_path)
            .unwrap_err()
            .to_string()
            .contains("newer than supported")
    );
    assert_eq!(read_schema_version(&connection).unwrap(), 999);
    drop(connection);
    fs::remove_file(db_path).unwrap();
}

#[test]
fn session_ids_are_not_reused_after_deletion_or_reinitialization() {
    let _env_guard = test_env_lock().lock().unwrap_or_else(|e| e.into_inner());
    let db_path = temp_db_path("session-sequence");
    init_sqlite(&db_path).unwrap();
    let mut connection = Connection::open(&db_path).unwrap();
    connection
        .execute("INSERT INTO sessions(id, name) VALUES(42, 'migrated')", [])
        .unwrap();
    let first = ensure_runtime_session(&mut connection, "new").unwrap();
    assert_eq!(first, 43);
    connection
        .execute("DELETE FROM sessions WHERE id = ?1", [first])
        .unwrap();
    drop(connection);
    init_sqlite(&db_path).unwrap();
    let mut connection = Connection::open(&db_path).unwrap();
    assert_eq!(ensure_runtime_session(&mut connection, "next").unwrap(), 44);
    connection
        .execute("UPDATE session_id_sequence SET last_id=?1", [i64::MAX])
        .unwrap();
    assert!(
        ensure_runtime_session(&mut connection, "exhausted")
            .unwrap_err()
            .to_string()
            .contains("exhausted")
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM sessions WHERE name='exhausted'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    drop(connection);
    fs::remove_file(db_path).unwrap();
}

#[test]
fn access_level_rebuild_rolls_back_dropped_tables_and_restores_foreign_keys() {
    let connection = Connection::open_in_memory().unwrap();
    configure_sqlite(&connection).unwrap();
    connection
        .execute_batch(&SCHEMA.replace(", 'admin'", ""))
        .unwrap();
    connection
        .execute("INSERT INTO sessions(id, name) VALUES(1, 'existing')", [])
        .unwrap();
    connection.execute("INSERT INTO auth_tokens(session_id, token_hash, token_prefix, access_level) VALUES(1, 'preserved', 'prefix', 'read')", []).unwrap();
    // A failure in the second rebuild occurs after the first DROP/RENAME.
    connection
        .execute_batch("CREATE TABLE issued_client_certs_admin_new(id INTEGER);")
        .unwrap();
    assert!(migrate_access_level_admin(&connection).is_err());
    let hash: String = connection
        .query_row("SELECT token_hash FROM auth_tokens", [], |row| row.get(0))
        .unwrap();
    assert_eq!(hash, "preserved");
    assert!(
        connection
            .pragma_query_value(None, "foreign_keys", |row| row.get::<_, bool>(0))
            .unwrap()
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name='auth_tokens_admin_new'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}

#[test]
fn ambiguous_legacy_journal_preflight_preserves_database_and_journal() {
    let directory = std::env::temp_dir().join(format!("ccp-legacy-preflight-{}", Uuid::new_v4()));
    fs::create_dir_all(&directory).unwrap();
    let db_path = directory.join("ccp.sqlite3");
    let journal_path = directory.join("runtime-journal.jsonl");
    let connection = Connection::open(&db_path).unwrap();
    connection.execute_batch(SCHEMA).unwrap();
    connection
        .execute("INSERT INTO schema_version(version) VALUES(3)", [])
        .unwrap();
    connection
        .execute("INSERT INTO sessions(id, name) VALUES(1, 'existing')", [])
        .unwrap();
    connection.execute("INSERT INTO message_packs(session_id, name, context) VALUES(1, 'existing', 'preserved')", []).unwrap();
    let entry = crate::journal::JournalEntry::AddShelf {
        session_id: 1,
        shelf_name: "team".into(),
        description: "legacy".into(),
    };
    let journal_bytes = format!("{}\n", serde_json::to_string(&entry).unwrap()).into_bytes();
    fs::write(&journal_path, &journal_bytes).unwrap();
    let database_bytes = fs::read(&db_path).unwrap();
    let error = init_sqlite(&db_path).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("legacy journal overlaps persisted data")
    );
    assert_eq!(read_schema_version(&connection).unwrap(), 3);
    assert_eq!(fs::read(&db_path).unwrap(), database_bytes);
    assert_eq!(fs::read(&journal_path).unwrap(), journal_bytes);
    drop(connection);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn unsnapshotted_legacy_journal_can_upgrade_but_persisted_shelves_are_ambiguous() {
    let directory = std::env::temp_dir().join(format!("ccp-legacy-shelf-{}", Uuid::new_v4()));
    fs::create_dir_all(&directory).unwrap();
    let db_path = directory.join("ccp.sqlite3");
    let journal_path = directory.join("runtime-journal.jsonl");
    let connection = Connection::open(&db_path).unwrap();
    connection.execute_batch(SCHEMA).unwrap();
    connection
        .execute("INSERT INTO schema_version(version) VALUES(3)", [])
        .unwrap();
    connection
        .execute("INSERT INTO sessions(id, name) VALUES(1, 'existing')", [])
        .unwrap();
    fs::write(
        &journal_path,
        "{\"AddShelf\":{\"session_id\":1,\"shelf_name\":\"team\",\"description\":\"legacy\"}}\n",
    )
    .unwrap();
    // No snapshot state means this old journal has an unambiguous recovery path.
    init_sqlite(&db_path).unwrap();
    connection
        .execute(
            "INSERT INTO shelves(session_id, shelf_name) VALUES(1, 'persisted')",
            [],
        )
        .unwrap();
    assert!(
        init_sqlite(&db_path)
            .unwrap_err()
            .to_string()
            .contains("legacy journal overlaps persisted data")
    );
    drop(connection);
    fs::remove_dir_all(directory).unwrap();
}
