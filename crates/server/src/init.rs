// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::{
    env, fs,
    path::{Path, PathBuf},
};

#[cfg(test)]
use std::sync::Mutex;
use std::sync::OnceLock;

use anyhow::{Context, bail};
use hmac::{Hmac, Mac};
use rcgen::{
    BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, KeyPair,
    KeyUsagePurpose,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::{fs::OpenOptions, io::Write};
use time::{Duration as TimeDuration, OffsetDateTime};
use url::Url;
#[cfg(test)]
use uuid::Uuid;

pub const SERVER_DATA_DIR_ENV: &str = "CCP_SERVER_DATA_DIR";
pub const SERVER_HOME_ENV: &str = "CCP_SERVER_HOME";
pub const AUTH_SERVER_BASE_URL_ENV: &str = "CCP_AUTH_BASE_URL";
pub const MTLS_SERVER_BASE_URL_ENV: &str = "CCP_MTLS_BASE_URL";
pub const AUTH_LISTENER_ADDR_ENV: &str = "CCP_AUTH_LISTENER_ADDR";
pub const MTLS_LISTENER_ADDR_ENV: &str = "CCP_MTLS_LISTENER_ADDR";
pub const TLS_SERVER_NAMES_ENV: &str = "CCP_TLS_SERVER_NAMES";
pub const ALLOW_NON_LOOPBACK_AUTH_LISTENER_ENV: &str = "CCP_ALLOW_NON_LOOPBACK_AUTH_LISTENER";
pub const SESSION_OWNER_ENV: &str = "CCP_SESSION_OWNER";
pub const SESSION_LABELS_ENV: &str = "CCP_SESSION_LABELS";
pub const SESSION_VISIBILITY_ENV: &str = "CCP_SESSION_VISIBILITY";
pub const SESSION_PURPOSE_ENV: &str = "CCP_SESSION_PURPOSE";
pub const ENROLLMENT_TOKEN_TTL_SECONDS_ENV: &str = "CCP_ENROLLMENT_TOKEN_TTL_SECONDS";
pub const CLIENT_CERT_TTL_SECONDS_ENV: &str = "CCP_CLIENT_CERT_TTL_SECONDS";
pub const CA_CERT_TTL_DAYS_ENV: &str = "CCP_CA_CERT_TTL_DAYS";
pub const CERT_WARNING_WINDOW_SECONDS_ENV: &str = "CCP_CERT_WARNING_WINDOW_SECONDS";
pub const AUTO_ISSUE_INITIAL_TOKENS_ENV: &str = "CCP_AUTO_ISSUE_INITIAL_TOKENS";

const DEFAULT_DATA_DIR: &str = "data";
const DEFAULT_SERVER_HOME: &str = "sessions";
const DEFAULT_AUTH_SERVER_BASE_URL: &str = "http://127.0.0.1:1337";
const DEFAULT_MTLS_SERVER_BASE_URL: &str = "https://localhost:1338";
const DEFAULT_AUTH_LISTENER_ADDR: &str = "127.0.0.1:1337";
const DEFAULT_MTLS_LISTENER_ADDR: &str = "127.0.0.1:1338";
/// Current schema version. Bump when adding new migrations.
pub const SCHEMA_VERSION: u32 = 2;

const AUTH_TOKEN_PURPOSE: &[u8] = b"ccp-auth-token:v1:";
const DEFAULT_ENROLLMENT_TOKEN_TTL_SECONDS: u64 = 60 * 60;
const DEFAULT_CLIENT_CERT_TTL_SECONDS: u64 = 3650 * 24 * 60 * 60;
const DEFAULT_CA_CERT_TTL_DAYS: i64 = 3650;
const DEFAULT_CERT_WARNING_WINDOW_SECONDS: u64 = 0;

const SCHEMA: &str = include_str!("init-db.sql");

pub struct SessionBootstrap {
    pub session_id: i64,
    pub session_name: String,
    pub auth_redeem_url: String,
    pub initial_read_token: Option<IssuedEnrollmentToken>,
    pub initial_read_write_token: Option<IssuedEnrollmentToken>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct IssuedEnrollmentToken {
    pub session_name: String,
    pub session_id: i64,
    pub access_level: String,
    pub token: String,
    pub expires_at: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionBinding {
    pub session_id: i64,
    pub session_name: String,
}

pub async fn initialize_cpp_server(session_name: &str) -> anyhow::Result<SessionBootstrap> {
    let _ = config_session_dir(session_name)?;
    init_sqlite(&db_path())?;
    validate_auth_formating()?;
    let mut connection = open_sqlite_connection()?;
    let session_id = ensure_runtime_session(&mut connection, session_name)?;
    drop(connection);

    initialize_ca_material(session_id, session_name)?;
    initialize_server_tls_material(session_id, session_name)?;
    let initial_read_token = ensure_initial_enrollment_token(session_name, session_id, "read")?;
    let initial_read_write_token =
        ensure_initial_enrollment_token(session_name, session_id, "read_write")?;
    Ok(SessionBootstrap {
        session_id,
        session_name: session_name.to_string(),
        auth_redeem_url: auth_redeem_url(),
        initial_read_token,
        initial_read_write_token,
    })
}

pub fn open_sqlite_connection() -> anyhow::Result<Connection> {
    let db_path = db_path();
    let connection = Connection::open(&db_path)
        .with_context(|| format!("failed to open sqlite database at {}", db_path.display()))?;
    configure_sqlite(&connection)?;
    Ok(connection)
}

pub fn server_home_dir() -> PathBuf {
    env::var_os(SERVER_HOME_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_SERVER_HOME))
}

static RESOLVED_DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

pub fn server_data_dir() -> PathBuf {
    if let Some(explicit) = env::var_os(SERVER_DATA_DIR_ENV) {
        return PathBuf::from(explicit);
    }
    RESOLVED_DATA_DIR
        .get()
        .cloned()
        .unwrap_or_else(|| PathBuf::from(DEFAULT_DATA_DIR))
}

pub fn session_storage_dir(session_name: &str) -> PathBuf {
    server_home_dir().join(session_slug(session_name))
}

pub fn config_session_dir(session_name: &str) -> anyhow::Result<PathBuf> {
    if let Some(explicit) = env::var_os(SERVER_DATA_DIR_ENV) {
        return Ok(PathBuf::from(explicit));
    }

    let resolved: PathBuf;
    if let Some(existing) = check_existing_session(session_name)? {
        resolved = existing;
    } else {
        resolved = session_storage_dir(session_name);
    }

    let _ = RESOLVED_DATA_DIR.set(resolved.clone());
    Ok(resolved)
}

fn check_existing_session(session_name: &str) -> anyhow::Result<Option<PathBuf>> {
    // Check the legacy data dir first, then scan server home.
    let legacy = PathBuf::from(DEFAULT_DATA_DIR);
    if cmp_dir_session_name(&legacy, session_name)? {
        return Ok(Some(legacy));
    }

    let home = server_home_dir();
    if !home.exists() {
        return Ok(None);
    }

    for entry in
        fs::read_dir(&home).with_context(|| format!("failed to read {}", home.display()))?
    {
        let entry = entry.context("failed to read session directory entry")?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        if cmp_dir_session_name(&path, session_name)? {
            return Ok(Some(path));
        }
        let nested_data_dir = path.join("data");
        if nested_data_dir.is_dir() && cmp_dir_session_name(&nested_data_dir, session_name)? {
            return Ok(Some(nested_data_dir));
        }
    }

    Ok(None)
}

fn cmp_dir_session_name(dir: &Path, session_name: &str) -> anyhow::Result<bool> {
    if let Ok(binding) = load_session_binding_from_path(&session_binding_path_for(dir))
        && binding.session_name == session_name
    {
        return Ok(true);
    }
    // Fall back to checking the sqlite database.
    let db_path = dir.join("ccp.sqlite3");
    if !db_path.exists() {
        return Ok(false);
    }

    let connection = Connection::open(&db_path)
        .with_context(|| format!("failed to open sqlite database at {}", db_path.display()))?;
    configure_sqlite(&connection)?;
    let count = connection
        .query_row(
            "SELECT COUNT(1) FROM sessions WHERE name = ?1",
            params![session_name],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .unwrap_or(0);
    Ok(count > 0)
}

pub fn session_slug(session_name: &str) -> String {
    let mut normalized = String::new();
    let mut last_was_dash = false;
    for ch in session_name.chars().flat_map(|ch| ch.to_lowercase()) {
        if ch.is_ascii_alphanumeric() {
            normalized.push(ch);
            last_was_dash = false;
        } else if !last_was_dash {
            normalized.push('-');
            last_was_dash = true;
        }
    }
    let normalized = normalized.trim_matches('-').to_string();
    let normalized = if normalized.is_empty() {
        "session".to_string()
    } else {
        normalized
    };
    let digest = Sha256::digest(session_name.as_bytes());
    let digest_hex = format!("{digest:x}");
    format!("{normalized}-{}", &digest_hex[..8])
}

pub fn db_path() -> PathBuf {
    server_data_dir().join("ccp.sqlite3")
}

pub fn ca_cert_path() -> PathBuf {
    server_data_dir().join("ccp_ca_cert.pem")
}

pub fn ca_key_path() -> PathBuf {
    server_data_dir().join("ccp_ca_key.pem")
}

pub fn server_cert_path() -> PathBuf {
    server_data_dir().join("ccp_server_cert.pem")
}

pub fn server_key_path() -> PathBuf {
    server_data_dir().join("ccp_server_key.pem")
}

pub fn auth_urls_path() -> PathBuf {
    server_data_dir().join("auth_urls.txt")
}

pub fn journal_path() -> PathBuf {
    server_data_dir().join("runtime-journal.jsonl")
}

pub fn auth_secret_path() -> PathBuf {
    server_data_dir().join("ccp_auth_secret.bin")
}

pub fn session_binding_path() -> PathBuf {
    session_binding_path_for(&server_data_dir())
}

fn session_binding_path_for(data_dir: &Path) -> PathBuf {
    data_dir.join("active_session.json")
}

pub fn auth_server_base_url() -> String {
    env::var(AUTH_SERVER_BASE_URL_ENV).unwrap_or_else(|_| DEFAULT_AUTH_SERVER_BASE_URL.to_string())
}

pub fn auth_redeem_url() -> String {
    format!(
        "{}/auth/redeem",
        auth_server_base_url().trim_end_matches('/')
    )
}

pub fn mtls_server_base_url() -> String {
    env::var(MTLS_SERVER_BASE_URL_ENV).unwrap_or_else(|_| DEFAULT_MTLS_SERVER_BASE_URL.to_string())
}

pub fn auth_listener_addr() -> String {
    env::var(AUTH_LISTENER_ADDR_ENV).unwrap_or_else(|_| DEFAULT_AUTH_LISTENER_ADDR.to_string())
}

pub fn mtls_listener_addr() -> String {
    env::var(MTLS_LISTENER_ADDR_ENV).unwrap_or_else(|_| DEFAULT_MTLS_LISTENER_ADDR.to_string())
}

pub fn enrollment_token_ttl_seconds() -> u64 {
    env::var(ENROLLMENT_TOKEN_TTL_SECONDS_ENV)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_ENROLLMENT_TOKEN_TTL_SECONDS)
}

pub fn client_cert_ttl_seconds() -> u64 {
    env::var(CLIENT_CERT_TTL_SECONDS_ENV)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_CLIENT_CERT_TTL_SECONDS)
}

pub fn ca_cert_ttl_days() -> i64 {
    env::var(CA_CERT_TTL_DAYS_ENV)
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_CA_CERT_TTL_DAYS)
}

pub fn cert_warning_window_seconds() -> u64 {
    env::var(CERT_WARNING_WINDOW_SECONDS_ENV)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(DEFAULT_CERT_WARNING_WINDOW_SECONDS)
}

#[cfg(test)]
pub(crate) fn test_env_lock() -> &'static Mutex<()> {
    static TEST_ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    TEST_ENV_LOCK.get_or_init(|| Mutex::new(()))
}

#[cfg(test)]
#[path = "tests/init.rs"]
mod tests;

pub(crate) fn configure_sqlite(connection: &Connection) -> anyhow::Result<()> {
    connection
        .execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;",
        )
        .context("failed to configure sqlite pragmas")?;
    Ok(())
}

pub fn issue_enrollment_token(
    session_name: &str,
    access_level: &str,
    ttl_seconds: Option<u64>,
) -> anyhow::Result<IssuedEnrollmentToken> {
    let _ = config_session_dir(session_name)?;
    if access_level != "read" && access_level != "read_write" && access_level != "admin" {
        bail!("access_level must be 'read', 'read_write', or 'admin'");
    }

    // user can override the default TTL
    // default TTL is 1 hour
    let ttl_seconds = ttl_seconds.unwrap_or_else(enrollment_token_ttl_seconds);
    if ttl_seconds == 0 {
        bail!("ttl_seconds must be greater than zero");
    }

    let connection = open_sqlite_connection()?;

    let session_id = connection
        .query_row(
            "SELECT id FROM sessions WHERE name = ?1",
            params![session_name],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .with_context(|| format!("unknown session '{session_name}'"))?;

    let token = generate_opaque_token(32)?;
    let token_hash = hash_token(&token);
    let expires_at = unix_timestamp_after(ttl_seconds)?;
    let expires_at_text = unix_timestamp_to_sqlite(expires_at)?;

    connection
        .execute(
            "INSERT INTO auth_tokens (
                session_id,
                token_value,
                token_hash,
                token_prefix,
                access_level,
                expires_at
            ) VALUES (?1, NULL, ?2, ?3, ?4, ?5)",
            params![
                session_id,
                token_hash,
                token_prefix(&token),
                access_level,
                expires_at_text,
            ],
        )
        .with_context(|| format!("failed to issue {access_level} enrollment token"))?;

    Ok(IssuedEnrollmentToken {
        session_name: session_name.to_string(),
        session_id,
        access_level: access_level.to_string(),
        token,
        expires_at,
    })
}

fn ensure_initial_enrollment_token(
    session_name: &str,
    session_id: i64,
    access_level: &str,
) -> anyhow::Result<Option<IssuedEnrollmentToken>> {
    // skip auto-issue if explicitly disabled
    let auto_issue = env::var(AUTO_ISSUE_INITIAL_TOKENS_ENV)
        .map(|value| value != "0")
        .unwrap_or(true);
    if !auto_issue {
        return Ok(None);
    }

    // don't issue if tokens already exist for this session + access level
    let connection = open_sqlite_connection()?;
    let existing_count: i64 = connection.query_row(
        "SELECT COUNT(1) FROM auth_tokens WHERE session_id = ?1 AND access_level = ?2",
        params![session_id, access_level],
        |row| row.get(0),
    )?;
    drop(connection);

    if existing_count > 0 {
        return Ok(None);
    }

    issue_enrollment_token(session_name, access_level, None).map(Some)
}

pub fn hash_token(raw_token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(raw_token.as_bytes());
    format!("{:x}", hasher.finalize())
}

pub fn token_prefix(raw_token: &str) -> String {
    raw_token.chars().take(12).collect()
}

fn validate_auth_formating() -> anyhow::Result<()> {
    let listener_host =
        get_host(&auth_listener_addr()).context("auth listener address is missing a host")?;
    if !is_loopback_host(&listener_host) && !allow_non_loopback_auth_listener() {
        bail!(
            "auth listener must bind to loopback; expose enrollment through an HTTPS reverse proxy or tunnel instead"
        );
    }

    let auth_base = auth_server_base_url();
    let auth_host = get_host(&auth_base).context("auth base URL is missing a host")?;
    let auth_scheme = extract_scheme(&auth_base).unwrap_or("http");
    if auth_scheme != "https" && !is_loopback_host(&auth_host) {
        bail!("auth base URL must use https unless it points to loopback");
    }

    Ok(())
}

fn allow_non_loopback_auth_listener() -> bool {
    env::var(ALLOW_NON_LOOPBACK_AUTH_LISTENER_ENV)
        .map(|value| value == "1")
        .unwrap_or(false)
}

#[allow(dead_code)]
pub(crate) fn initialize_auth_secret() -> anyhow::Result<()> {
    let path = auth_secret_path();
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create auth secret dir at {}", parent.display()))?;
    }
    let mut secret = [0u8; 32];
    getrandom::getrandom(&mut secret)
        .map_err(|error| anyhow::anyhow!("failed to generate auth secret: {error}"))?;
    write_private_file(&path, &secret)
}

fn load_auth_secret() -> anyhow::Result<Vec<u8>> {
    let path = auth_secret_path();
    let secret = fs::read(&path)
        .with_context(|| format!("failed to read auth secret from {}", path.display()))?;
    if secret.len() < 32 {
        bail!("auth secret at {} is too short", path.display());
    }
    Ok(secret)
}

pub fn derive_auth_token(
    session_id: i64,
    access_level: &str,
    token_nonce: &str,
) -> anyhow::Result<String> {
    type HmacSha256 = Hmac<Sha256>;

    let secret = load_auth_secret()?;
    let mut mac =
        HmacSha256::new_from_slice(&secret).context("failed to initialize auth token MAC")?;
    mac.update(AUTH_TOKEN_PURPOSE);
    mac.update(session_id.to_string().as_bytes());
    mac.update(b":");
    mac.update(access_level.as_bytes());
    mac.update(b":");
    mac.update(token_nonce.as_bytes());
    Ok(encode_hex(&mac.finalize().into_bytes()))
}

fn encode_hex(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut encoded, "{byte:02x}");
    }
    encoded
}

fn generate_opaque_token(byte_len: usize) -> anyhow::Result<String> {
    let mut token = vec![0u8; byte_len];
    getrandom::getrandom(&mut token)
        .map_err(|error| anyhow::anyhow!("failed to generate enrollment token: {error}"))?;
    Ok(encode_hex(&token))
}

pub fn load_session_binding() -> anyhow::Result<SessionBinding> {
    load_session_binding_from_path(&session_binding_path())
}

fn load_session_binding_from_path(binding_path: &Path) -> anyhow::Result<SessionBinding> {
    let binding = fs::read_to_string(binding_path).with_context(|| {
        format!(
            "failed to read session binding from {}",
            binding_path.display()
        )
    })?;
    serde_json::from_str(&binding).with_context(|| {
        format!(
            "failed to parse session binding from {}",
            binding_path.display()
        )
    })
}

fn write_session_binding(binding: &SessionBinding) -> anyhow::Result<()> {
    let binding_path = session_binding_path();
    let payload =
        serde_json::to_vec_pretty(binding).context("failed to serialize session binding")?;
    fs::write(&binding_path, payload).with_context(|| {
        format!(
            "failed to write session binding to {}",
            binding_path.display()
        )
    })
}

fn initialize_ca_material(session_id: i64, session_name: &str) -> anyhow::Result<()> {
    let ca_cert_path = ca_cert_path();
    let ca_key_path = ca_key_path();
    let desired_binding = SessionBinding {
        session_id,
        session_name: session_name.to_string(),
    };

    if ca_cert_path.exists()
        && ca_key_path.exists()
        && let Ok(existing_binding) = load_session_binding()
        && existing_binding == desired_binding
    {
        return Ok(());
    }

    if let Some(parent) = ca_cert_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create CA directory at {}", parent.display()))?;
    }

    if ca_cert_path.exists() {
        fs::remove_file(&ca_cert_path).with_context(|| {
            format!(
                "failed to remove stale CA certificate at {}",
                ca_cert_path.display()
            )
        })?;
    }
    if ca_key_path.exists() {
        fs::remove_file(&ca_key_path).with_context(|| {
            format!("failed to remove stale CA key at {}", ca_key_path.display())
        })?;
    }

    let server_cert_path = server_cert_path();
    if server_cert_path.exists() {
        fs::remove_file(&server_cert_path).with_context(|| {
            format!(
                "failed to remove stale server certificate at {}",
                server_cert_path.display()
            )
        })?;
    }
    let server_key_path = server_key_path();
    if server_key_path.exists() {
        fs::remove_file(&server_key_path).with_context(|| {
            format!(
                "failed to remove stale server key at {}",
                server_key_path.display()
            )
        })?;
    }

    let signing_key = KeyPair::generate().context("failed to generate CA key pair")?;
    let mut params = CertificateParams::new(Vec::<String>::new())
        .context("failed to build CA certificate params")?;
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.not_before = OffsetDateTime::now_utc() - TimeDuration::hours(1);
    params.not_after = OffsetDateTime::now_utc() + TimeDuration::days(ca_cert_ttl_days());
    params.distinguished_name.push(
        DnType::CommonName,
        format!("CCP Session CA [{}:{}]", session_name, session_id),
    );

    let certificate = params
        .self_signed(&signing_key)
        .context("failed to self-sign CA certificate")?;

    fs::write(&ca_cert_path, certificate.pem().as_bytes()).with_context(|| {
        format!(
            "failed to write CA certificate to {}",
            ca_cert_path.display()
        )
    })?;
    write_private_file(&ca_key_path, signing_key.serialize_pem().as_bytes())
        .with_context(|| format!("failed to write CA key to {}", ca_key_path.display()))?;
    write_session_binding(&desired_binding)?;

    Ok(())
}

pub fn ensure_active_session_binding(session_id: i64) -> anyhow::Result<SessionBinding> {
    let binding = load_session_binding()?;
    if binding.session_id != session_id {
        anyhow::bail!(
            "session binding mismatch: active session is '{}' ({}) not {}",
            binding.session_name,
            binding.session_id,
            session_id
        );
    }
    Ok(binding)
}

fn ensure_runtime_session(connection: &mut Connection, session_name: &str) -> anyhow::Result<i64> {
    let owner = env::var(SESSION_OWNER_ENV).unwrap_or_default();
    let labels = env::var(SESSION_LABELS_ENV).unwrap_or_default();
    let visibility = env::var(SESSION_VISIBILITY_ENV).unwrap_or_else(|_| "private".to_string());
    let purpose = env::var(SESSION_PURPOSE_ENV)
        .unwrap_or_else(|_| "Runtime session for CCP inter-agent communication".to_string());
    connection
        .execute(
            "INSERT OR IGNORE INTO sessions (
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
                session_name,
                "Runtime session for CCP inter-agent communication",
                owner,
                labels,
                visibility,
                purpose
            ],
        )
        .with_context(|| format!("failed to ensure session {session_name} exists"))?;

    connection
        .execute(
            "UPDATE sessions
             SET is_active = 1,
                 owner = ?2,
                 labels = ?3,
                 visibility = ?4,
                 purpose = ?5,
                 last_started_at = CURRENT_TIMESTAMP
             WHERE name = ?1",
            params![session_name, owner, labels, visibility, purpose],
        )
        .with_context(|| format!("failed to activate session {session_name}"))?;

    connection
        .query_row(
            "SELECT id FROM sessions WHERE name = ?1",
            [session_name],
            |row| row.get(0),
        )
        .with_context(|| format!("failed to load session id for {session_name}"))
}

fn apply_schema_migrations(connection: &Connection) -> anyhow::Result<()> {
    add_column_if_missing(
        connection,
        "ALTER TABLE sessions ADD COLUMN is_active INTEGER NOT NULL DEFAULT 0",
    )?;
    add_column_if_missing(
        connection,
        "ALTER TABLE sessions ADD COLUMN last_started_at TEXT",
    )?;
    add_column_if_missing(
        connection,
        "ALTER TABLE sessions ADD COLUMN last_stopped_at TEXT",
    )?;
    add_column_if_missing(
        connection,
        "ALTER TABLE sessions ADD COLUMN owner TEXT NOT NULL DEFAULT ''",
    )?;
    add_column_if_missing(
        connection,
        "ALTER TABLE sessions ADD COLUMN labels TEXT NOT NULL DEFAULT ''",
    )?;
    add_column_if_missing(
        connection,
        "ALTER TABLE sessions ADD COLUMN visibility TEXT NOT NULL DEFAULT 'private'",
    )?;
    add_column_if_missing(
        connection,
        "ALTER TABLE sessions ADD COLUMN purpose TEXT NOT NULL DEFAULT ''",
    )?;
    add_column_if_missing(
        connection,
        "ALTER TABLE auth_tokens ADD COLUMN token_value TEXT",
    )?;
    add_column_if_missing(
        connection,
        "ALTER TABLE auth_tokens ADD COLUMN token_nonce TEXT NOT NULL DEFAULT ''",
    )?;
    add_column_if_missing(
        connection,
        "ALTER TABLE auth_tokens ADD COLUMN expires_at TEXT",
    )?;
    add_column_if_missing(
        connection,
        "ALTER TABLE message_history ADD COLUMN operation_id TEXT NOT NULL DEFAULT ''",
    )?;
    add_column_if_missing(
        connection,
        "ALTER TABLE message_history ADD COLUMN agent_name TEXT",
    )?;
    add_column_if_missing(
        connection,
        "ALTER TABLE message_history ADD COLUMN host_name TEXT",
    )?;
    add_column_if_missing(
        connection,
        "ALTER TABLE message_history ADD COLUMN reason TEXT",
    )?;
    add_column_if_missing(
        connection,
        "ALTER TABLE message_packs ADD COLUMN labels TEXT NOT NULL DEFAULT ''",
    )?;
    add_column_if_missing(
        connection,
        "ALTER TABLE deleted_message_packs ADD COLUMN labels TEXT NOT NULL DEFAULT ''",
    )?;
    add_column_if_missing(
        connection,
        "ALTER TABLE message_packs ADD COLUMN shelf_name TEXT NOT NULL DEFAULT 'main'",
    )?;
    add_column_if_missing(
        connection,
        "ALTER TABLE message_packs ADD COLUMN book_name TEXT NOT NULL DEFAULT 'default'",
    )?;
    add_column_if_missing(
        connection,
        "ALTER TABLE deleted_message_packs ADD COLUMN shelf_name TEXT NOT NULL DEFAULT 'main'",
    )?;
    add_column_if_missing(
        connection,
        "ALTER TABLE deleted_message_packs ADD COLUMN book_name TEXT NOT NULL DEFAULT 'default'",
    )?;
    add_column_if_missing(
        connection,
        "ALTER TABLE deleted_message_packs ADD COLUMN shelf_description TEXT NOT NULL DEFAULT ''",
    )?;
    add_column_if_missing(
        connection,
        "ALTER TABLE deleted_message_packs ADD COLUMN book_description TEXT NOT NULL DEFAULT ''",
    )?;
    add_column_if_missing(
        connection,
        "ALTER TABLE issued_client_certs ADD COLUMN expires_at TEXT NOT NULL DEFAULT '9999-12-31 23:59:59'",
    )?;
    connection
        .execute_batch(
            "DROP INDEX IF EXISTS idx_auth_tokens_session_access_level;

             CREATE TABLE IF NOT EXISTS message_history (
                 id INTEGER PRIMARY KEY,
                 message_pack_id INTEGER NOT NULL,
                 operation_id TEXT NOT NULL DEFAULT '',
                 client_common_name TEXT NOT NULL,
                 agent_name TEXT,
                 host_name TEXT,
                 reason TEXT,
                 appended_content TEXT NOT NULL,
                 created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                 FOREIGN KEY (message_pack_id) REFERENCES message_packs(id) ON DELETE CASCADE
             );

             CREATE INDEX IF NOT EXISTS idx_message_history_message_pack_id
             ON message_history(message_pack_id);

             CREATE TABLE IF NOT EXISTS deleted_message_history (
                 id INTEGER PRIMARY KEY,
                 entry_key TEXT NOT NULL,
                 operation_id TEXT NOT NULL,
                 client_common_name TEXT NOT NULL,
                 agent_name TEXT,
                 host_name TEXT,
                 reason TEXT,
                 appended_content TEXT NOT NULL,
                 created_at TEXT NOT NULL,
                 FOREIGN KEY (entry_key) REFERENCES deleted_message_packs(entry_key) ON DELETE CASCADE
             );

             CREATE INDEX IF NOT EXISTS idx_deleted_message_history_entry_key
             ON deleted_message_history(entry_key);",
        )
        .context("failed to apply sqlite follow-up migrations")?;

    migrate_access_level_admin(connection)?;

    // v2: offline scoped transfer audit log
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS transfer_log (
                id            INTEGER PRIMARY KEY,
                session_id    INTEGER NOT NULL,
                direction     TEXT NOT NULL CHECK (direction IN ('export', 'import')),
                scope_json    TEXT NOT NULL,
                bundle_sha256 TEXT NOT NULL,
                policy        TEXT,
                outcome       TEXT NOT NULL,
                entry_count   INTEGER NOT NULL,
                created_at    TEXT NOT NULL,
                created_by    TEXT NOT NULL,
                FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_transfer_log_session_id
            ON transfer_log(session_id);",
        )
        .context("failed to apply v2 migration: transfer_log table")?;

    record_schema_version(connection)?;

    Ok(())
}

fn record_schema_version(connection: &Connection) -> anyhow::Result<()> {
    // Ensure the table exists (may not if this is the first run before SCHEMA)
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_version (
                version INTEGER NOT NULL,
                applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            )",
        )
        .context("failed to create schema_version table")?;

    let current: u32 = connection
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_version",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    if current < SCHEMA_VERSION {
        connection
            .execute(
                "INSERT INTO schema_version (version) VALUES (?1)",
                [SCHEMA_VERSION],
            )
            .context("failed to record schema version")?;
    }
    Ok(())
}

pub fn current_schema_version() -> anyhow::Result<u32> {
    let connection = open_sqlite_connection()?;
    let has_table: bool = connection
        .query_row(
            "SELECT COUNT(1) FROM sqlite_master WHERE type = 'table' AND name = 'schema_version'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0)
        > 0;
    if !has_table {
        return Ok(0);
    }
    let version: u32 = connection
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_version",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    Ok(version)
}

fn migrate_access_level_admin(connection: &Connection) -> anyhow::Result<()> {
    let has_auth_tokens: bool = connection
        .query_row(
            "SELECT COUNT(1) FROM sqlite_master WHERE type = 'table' AND name = 'auth_tokens'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0)
        > 0;
    if !has_auth_tokens {
        return Ok(());
    }

    connection.execute_batch(
        "PRAGMA foreign_keys = OFF;

         CREATE TABLE IF NOT EXISTS auth_tokens_admin_new (
             id INTEGER PRIMARY KEY,
             session_id INTEGER NOT NULL,
             token_nonce TEXT NOT NULL DEFAULT '',
             token_value TEXT UNIQUE,
             token_hash TEXT NOT NULL UNIQUE,
             token_prefix TEXT NOT NULL,
             access_level TEXT NOT NULL CHECK (access_level IN ('read', 'read_write', 'admin')),
             created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
             last_used_at TEXT,
             expires_at TEXT,
             FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
         );
         INSERT OR IGNORE INTO auth_tokens_admin_new (
             id,
             session_id,
             token_nonce,
             token_value,
             token_hash,
             token_prefix,
             access_level,
             created_at,
             last_used_at,
             expires_at
         )
         SELECT
             id,
             session_id,
             token_nonce,
             token_value,
             token_hash,
             token_prefix,
             access_level,
             created_at,
             last_used_at,
             expires_at
         FROM auth_tokens;
         DROP TABLE auth_tokens;
         ALTER TABLE auth_tokens_admin_new RENAME TO auth_tokens;
         CREATE INDEX IF NOT EXISTS idx_auth_tokens_session_id ON auth_tokens(session_id);
         CREATE INDEX IF NOT EXISTS idx_auth_tokens_access_level ON auth_tokens(access_level);

         CREATE TABLE IF NOT EXISTS issued_client_certs_admin_new (
             id INTEGER PRIMARY KEY,
             session_id INTEGER NOT NULL,
             common_name TEXT NOT NULL UNIQUE,
             access_level TEXT NOT NULL CHECK (access_level IN ('read', 'read_write', 'admin')),
             cert_pem TEXT NOT NULL,
             created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
             expires_at TEXT NOT NULL,
             revoked_at TEXT,
             FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
         );
         INSERT OR IGNORE INTO issued_client_certs_admin_new SELECT * FROM issued_client_certs;
         DROP TABLE issued_client_certs;
         ALTER TABLE issued_client_certs_admin_new RENAME TO issued_client_certs;
         CREATE INDEX IF NOT EXISTS idx_issued_client_certs_session_id ON issued_client_certs(session_id);

         PRAGMA foreign_keys = ON;",
    )
    .context("failed to migrate access_level admin")?;
    Ok(())
}

fn add_column_if_missing(connection: &Connection, sql: &str) -> anyhow::Result<()> {
    match connection.execute(sql, []) {
        Ok(_) => Ok(()),
        Err(error) if error.to_string().contains("duplicate column name") => Ok(()),
        Err(error) if error.to_string().contains("no such table") => Ok(()),
        Err(error) => Err(error).with_context(|| format!("failed migration: {sql}")),
    }
}

pub fn unix_timestamp_after(ttl_seconds: u64) -> anyhow::Result<u64> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .context("system clock is before UNIX_EPOCH")?
        .as_secs();
    now.checked_add(ttl_seconds)
        .context("timestamp overflow while computing expiry")
}

pub fn unix_timestamp_to_sqlite(timestamp: u64) -> anyhow::Result<String> {
    let datetime = OffsetDateTime::from_unix_timestamp(timestamp as i64)
        .context("failed to convert unix timestamp to datetime")?;
    Ok(format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        datetime.year(),
        u8::from(datetime.month()),
        datetime.day(),
        datetime.hour(),
        datetime.minute(),
        datetime.second()
    ))
}

fn initialize_server_tls_material(session_id: i64, session_name: &str) -> anyhow::Result<()> {
    let server_cert_path = server_cert_path();
    let server_key_path = server_key_path();
    let _ = ensure_active_session_binding(session_id)
        .with_context(|| format!("server TLS material is not bound to session '{session_name}'"))?;
    let ca_cert_path = ca_cert_path();
    let ca_key_path = ca_key_path();

    let ca_cert_pem = fs::read_to_string(&ca_cert_path).with_context(|| {
        format!(
            "failed to read CA certificate from {}",
            ca_cert_path.display()
        )
    })?;
    let ca_key_pem = fs::read_to_string(&ca_key_path)
        .with_context(|| format!("failed to read CA key from {}", ca_key_path.display()))?;

    let ca_key = KeyPair::from_pem(&ca_key_pem).context("failed to parse CA private key")?;
    let ca_params = CertificateParams::from_ca_cert_pem(&ca_cert_pem)
        .context("failed to parse CA certificate")?;
    let ca_cert = ca_params
        .self_signed(&ca_key)
        .context("failed to reconstruct CA certificate")?;

    let server_key = KeyPair::generate().context("failed to generate server TLS key pair")?;
    let server_names = desired_server_tls_names();
    let common_name = server_names
        .first()
        .cloned()
        .unwrap_or_else(|| "localhost".to_string());
    let mut server_params = CertificateParams::new(server_names)
        .context("failed to build server certificate params")?;
    server_params
        .distinguished_name
        .push(DnType::CommonName, common_name);
    server_params.not_before = OffsetDateTime::now_utc() - TimeDuration::hours(1);
    server_params.not_after = OffsetDateTime::now_utc() + TimeDuration::days(ca_cert_ttl_days());
    server_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    server_params.key_usages = vec![KeyUsagePurpose::DigitalSignature];

    let server_cert = server_params
        .signed_by(&server_key, &ca_cert, &ca_key)
        .context("failed to sign server TLS certificate")?;

    fs::write(&server_cert_path, server_cert.pem().as_bytes()).with_context(|| {
        format!(
            "failed to write server cert to {}",
            server_cert_path.display()
        )
    })?;
    write_private_file(&server_key_path, server_key.serialize_pem().as_bytes()).with_context(
        || {
            format!(
                "failed to write server key to {}",
                server_key_path.display()
            )
        },
    )?;

    Ok(())
}

fn desired_server_tls_names() -> Vec<String> {
    let mut names = vec!["localhost".to_string()];

    if let Some(host) = get_host(&mtls_server_base_url()) {
        push_unique(&mut names, host);
    }

    if let Ok(extra_names) = env::var(TLS_SERVER_NAMES_ENV) {
        for candidate in extra_names.split(',') {
            let candidate = candidate.trim();
            if !candidate.is_empty() {
                push_unique(&mut names, candidate.to_string());
            }
        }
    }

    names
}

fn get_host(value: &str) -> Option<String> {
    let to_parse = if value.contains("://") {
        value.to_string()
    } else {
        format!("https://{}", value)
    };
    Url::parse(&to_parse)
        .ok()
        .and_then(|u| u.host_str().map(String::from))
}

fn extract_scheme(base_url: &str) -> Option<&str> {
    base_url.split_once("://").map(|(scheme, _)| scheme)
}

fn is_loopback_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") || host == "::1" || host == "[::1]" {
        return true;
    }
    host.parse::<std::net::IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
}

fn push_unique(values: &mut Vec<String>, candidate: String) {
    if !values.iter().any(|existing| existing == &candidate) {
        values.push(candidate);
    }
}

fn write_private_file(path: &Path, contents: &[u8]) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(path)
            .with_context(|| format!("failed to open private file {}", path.display()))?;
        file.write_all(contents)
            .with_context(|| format!("failed to write private file {}", path.display()))?;
        file.flush()
            .with_context(|| format!("failed to flush private file {}", path.display()))?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .with_context(|| format!("failed to set permissions on {}", path.display()))?;
        Ok(())
    }

    #[cfg(not(unix))]
    {
        fs::write(path, contents)
            .with_context(|| format!("failed to write private file {}", path.display()))
    }
}

pub(crate) fn init_sqlite(db_path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = db_path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create database directory at {}",
                parent.display()
            )
        })?;
    }

    let mut connection = Connection::open(db_path)
        .with_context(|| format!("failed to open sqlite database at {}", db_path.display()))?;
    configure_sqlite(&connection)?;

    apply_schema_migrations(&connection)?;

    let transaction = connection
        .transaction()
        .context("failed to start sqlite initialization transaction")?;

    transaction
        .execute_batch(SCHEMA)
        .context("failed to initialize sqlite schema")?;

    transaction
        .commit()
        .context("failed to commit sqlite initialization transaction")?;

    Ok(())
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ServerHealthStatus {
    pub status: String,
    pub session_name: String,
    pub protocol_version: u32,
    pub schema_version: u32,
    pub server_version: String,
    pub active_sessions: usize,
    pub issued_certs: usize,
    pub revoked_certs: usize,
    pub database_path: String,
    pub journal_path: String,
    pub ca_cert_path: String,
    pub server_cert_path: String,
    pub issued_certs_list: Vec<CertInfo>,
    pub revoked_certs_list: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CertInfo {
    pub common_name: String,
    pub session_id: i64,
    pub access_level: String,
    pub created_at: String,
    pub expires_at: String,
}

pub fn check_server_health(session_name: &str) -> anyhow::Result<ServerHealthStatus> {
    let session_dir = config_session_dir(session_name)?;
    let db_path = session_dir.join("ccp.sqlite3");
    let journal_path = session_dir.join("runtime-journal.jsonl");
    let ca_cert_path = session_dir.join("ccp_ca_cert.pem");
    let server_cert_path = session_dir.join("ccp_server_cert.pem");

    // Open database and read persisted stats
    let connection =
        Connection::open(&db_path).context("failed to open sqlite database for health check")?;

    // Count active sessions from database (includes in-memory active ones)
    let active_sessions: usize = connection
        .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
        .unwrap_or(0);

    // Count issued certs (both in DB and in-memory)
    let db_issued_certs: usize = connection
        .query_row(
            "SELECT COUNT(*) FROM issued_client_certs WHERE revoked_at IS NULL",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    // Get issued certs list from DB
    let mut stmt = connection.prepare(
        "SELECT cert_pem, session_id, access_level, created_at, expires_at FROM issued_client_certs WHERE revoked_at IS NULL ORDER BY created_at DESC",
    )?;

    let mut issued_certs_list = Vec::new();
    let rows = stmt.query_map([], |row| {
        let cert_pem: String = row.get(0)?;
        let session_id: i64 = row.get(1)?;
        let access_level: String = row.get(2)?;
        let created_at: String = row.get(3)?;
        let expires_at: String = row.get(4)?;

        // Extract common name from cert
        let common_name = extract_cn_from_pem(&cert_pem).unwrap_or_else(|| "unknown".to_string());

        Ok(CertInfo {
            common_name,
            session_id,
            access_level,
            created_at,
            expires_at,
        })
    })?;

    for cert_info in rows.flatten() {
        issued_certs_list.push(cert_info);
    }

    // Count revoked certs from database
    let revoked_certs: usize = connection
        .query_row(
            "SELECT COUNT(*) FROM issued_client_certs WHERE revoked_at IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    // Get revoked certs list
    let mut stmt = connection.prepare(
        "SELECT cert_pem FROM issued_client_certs WHERE revoked_at IS NOT NULL ORDER BY revoked_at DESC",
    )?;

    let mut revoked_certs_list = Vec::new();
    let rows = stmt.query_map([], |row| {
        let cert_pem: String = row.get(0)?;
        Ok(cert_pem)
    })?;

    for cert_pem in rows.flatten() {
        let common_name = extract_cn_from_pem(&cert_pem).unwrap_or_else(|| "unknown".to_string());
        revoked_certs_list.push(common_name);
    }

    Ok(ServerHealthStatus {
        status: "healthy".to_string(),
        session_name: session_name.to_string(),
        protocol_version: protocol::PROTOCOL_VERSION,
        schema_version: SCHEMA_VERSION,
        server_version: env!("CARGO_PKG_VERSION").to_string(),
        active_sessions,
        issued_certs: db_issued_certs,
        revoked_certs,
        database_path: db_path.display().to_string(),
        journal_path: journal_path.display().to_string(),
        ca_cert_path: ca_cert_path.display().to_string(),
        server_cert_path: server_cert_path.display().to_string(),
        issued_certs_list,
        revoked_certs_list,
    })
}

fn extract_cn_from_pem(pem_str: &str) -> Option<String> {
    // Try to find CN in the PEM certificate
    // This is a simple approach - looks for subjectAltName URI with session/access info
    // Format: X509v3 Subject Alternative Name: URI:ccp://session/..., URI:ccp://access/...
    if let Some(cn_start) = pem_str.find("CN=") {
        let rest = &pem_str[cn_start + 3..];
        if let Some(cn_end) = rest.find(',') {
            return Some(rest[..cn_end].trim().to_string());
        } else if let Some(cn_end) = rest.find('\n') {
            return Some(rest[..cn_end].trim().to_string());
        }
    }
    None
}
