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
