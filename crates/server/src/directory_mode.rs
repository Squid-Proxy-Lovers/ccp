// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Local data-directory mode isolation, independent of client authorization.
//! The bundled SQLite VFS owns the kernel locks. Shared read transactions let
//! native server/tools coexist; exclusive locking mode retains locks across the
//! durable activation commit. Keep a lease in every runtime and writer task.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, bail};
use rusqlite::{Connection, ErrorCode, OpenFlags, TransactionBehavior};

const LEDGER_NAME: &str = "ccp-directory-mode.sqlite3";
const APPLICATION_ID: u32 = 0x4343_504d;
const FORMAT_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectoryMode {
    Native,
    Workspace,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectoryConflict {
    Busy,
    WorkspaceActivated,
}

impl fmt::Display for DirectoryConflict {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Busy => "data directory is in use by another server or maintenance operation",
            Self::WorkspaceActivated => "workspace mode is activated; native access is refused",
        })
    }
}

impl std::error::Error for DirectoryConflict {}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LeaseKind {
    Native,
    Workspace,
    Maintenance,
}

struct LeaseInner {
    // RAII retains the SQLite transaction/locks. Mutex makes the held connection
    // safely shareable by runtime and journal-writer lifetimes; it is not exposed.
    _connection: Mutex<Connection>,
    directory: PathBuf,
    mode: DirectoryMode,
    kind: LeaseKind,
}

#[derive(Clone)]
pub struct DirectoryLease(Arc<LeaseInner>);

impl DirectoryLease {
    pub fn native(directory: &Path) -> anyhow::Result<Self> {
        Self::acquire(directory, LeaseKind::Native)
    }

    /// Explicit local activation only. This persists workspace mode and upgrades
    /// the schema before returning; it does not migrate legacy content or start
    /// a listener. There is no in-place deactivation API; rollback uses a backup.
    pub fn activate_workspace(directory: &Path) -> anyhow::Result<Self> {
        Self::acquire(directory, LeaseKind::Workspace)
    }

    /// Resume only an already activated workspace directory. The initial
    /// maintenance check never activates Native mode; activation is explicit.
    pub fn resume_workspace(directory: &Path) -> anyhow::Result<Self> {
        let existing = Self::maintenance(directory)?;
        if existing.mode() != DirectoryMode::Workspace {
            bail!("workspace activation is required; use the explicit activation option");
        }
        drop(existing);
        Self::activate_workspace(directory)
    }

    /// Exclusive local operator boundary for future provisioning/reset tools.
    /// It preserves the persisted mode and grants no remote administrative role.
    pub fn maintenance(directory: &Path) -> anyhow::Result<Self> {
        Self::acquire(directory, LeaseKind::Maintenance)
    }

    /// Promote the uniquely held maintenance lease without a release/reacquire
    /// gap. Data activation precedes ledger activation, as in acquire(); a crash
    /// between those commits still makes native callers refuse the directory.
    pub(crate) fn promote_workspace(self) -> anyhow::Result<Self> {
        let mut inner =
            Arc::try_unwrap(self.0).map_err(|_shared| anyhow::anyhow!("setup lease is shared"))?;
        if inner.kind != LeaseKind::Maintenance {
            bail!("setup promotion requires maintenance authority");
        }
        require_clean_legacy_journal(&inner.directory)?;
        crate::init::init_sqlite(&inner.directory.join("ccp.sqlite3"))?;
        persist_data_activation(&inner.directory)?;
        let connection = inner
            ._connection
            .get_mut()
            .map_err(|_poisoned| anyhow::anyhow!("setup lease is poisoned"))?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if transaction.execute("UPDATE directory_mode SET mode='workspace', revision=revision+1 WHERE singleton=1 AND revision < 9223372036854775807",[])? != 1 {
            bail!("invalid or exhausted directory-mode revision");
        }
        transaction.commit()?;
        inner.mode = DirectoryMode::Workspace;
        inner.kind = LeaseKind::Workspace;
        Ok(Self(Arc::new(inner)))
    }

    pub fn directory(&self) -> &Path {
        &self.0.directory
    }

    pub fn mode(&self) -> DirectoryMode {
        self.0.mode
    }

    pub(crate) fn allows_maintenance(&self) -> bool {
        self.0.kind == LeaseKind::Maintenance
    }

    pub(crate) fn allows_native(&self) -> bool {
        self.0.kind == LeaseKind::Native
    }

    pub(crate) fn allows_workspace(&self) -> bool {
        self.0.kind == LeaseKind::Workspace
    }

    fn acquire(directory: &Path, kind: LeaseKind) -> anyhow::Result<Self> {
        acquire(directory, kind)
            .map_err(classify_conflict)
            .with_context(|| format!("failed to lease data directory {}", directory.display()))
    }
}

fn acquire(directory: &Path, kind: LeaseKind) -> anyhow::Result<DirectoryLease> {
    std::fs::create_dir_all(directory).context("failed to create data directory")?;
    let directory = std::fs::canonicalize(directory).context("failed to resolve data directory")?;
    let mut connection = Connection::open(directory.join(LEDGER_NAME))?;
    connection.busy_timeout(Duration::ZERO)?;
    connection.pragma_update(None, "synchronous", "FULL")?;
    if kind != LeaseKind::Native {
        connection.pragma_update(None, "locking_mode", "EXCLUSIVE")?;
    }
    let mode = if kind == LeaseKind::Native {
        ensure_ledger(&mut connection)?;
        connection.execute_batch("BEGIN DEFERRED;")?;
        let mode = read_mode(&connection)?;
        if mode == DirectoryMode::Workspace
            || read_data_mode(&directory)? == DirectoryMode::Workspace
        {
            return Err(DirectoryConflict::WorkspaceActivated.into());
        }
        mode
    } else {
        connection.execute_batch("BEGIN EXCLUSIVE;")?;
        initialize_ledger(&connection)?;
        let prior = read_mode(&connection)?;
        let mode = if kind == LeaseKind::Workspace {
            require_clean_legacy_journal(&directory)?;
            crate::init::init_sqlite(&directory.join("ccp.sqlite3"))?;
            persist_data_activation(&directory)?;
            DirectoryMode::Workspace
        } else if prior == DirectoryMode::Workspace
            || read_data_mode(&directory)? == DirectoryMode::Workspace
        {
            DirectoryMode::Workspace
        } else {
            prior
        };
        let name = match mode {
            DirectoryMode::Native => "native",
            DirectoryMode::Workspace => "workspace",
        };
        let changed = connection.execute(
            "UPDATE directory_mode SET mode = ?1, revision = revision + 1
             WHERE singleton = 1 AND revision < 9223372036854775807",
            [name],
        )?;
        if changed != 1 {
            bail!("invalid or exhausted directory-mode revision");
        }
        connection.execute_batch("COMMIT;")?;
        mode
    };
    Ok(DirectoryLease(Arc::new(LeaseInner {
        _connection: Mutex::new(connection),
        directory,
        mode,
        kind,
    })))
}

fn ensure_ledger(connection: &mut Connection) -> anyhow::Result<()> {
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name = 'directory_mode' AND type = 'table')",
        [], |row| row.get(0),
    )?;
    if !exists {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Exclusive)?;
        initialize_ledger(&transaction)?;
        transaction.commit()?;
    }
    Ok(())
}

fn initialize_ledger(connection: &Connection) -> anyhow::Result<()> {
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name = 'directory_mode' AND type = 'table')",
        [], |row| row.get(0),
    )?;
    if exists {
        return Ok(());
    }
    let app: u32 = connection.pragma_query_value(None, "application_id", |row| row.get(0))?;
    let version: u32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    let objects: u64 =
        connection.query_row("SELECT COUNT(*) FROM sqlite_schema", [], |row| row.get(0))?;
    if app != 0 || version != 0 || objects != 0 {
        bail!("unrecognized directory-mode ledger");
    }
    connection.execute_batch(
        "CREATE TABLE directory_mode (
            singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
            mode TEXT NOT NULL CHECK (mode IN ('native', 'workspace')),
            revision INTEGER NOT NULL CHECK (revision >= 0)
         );
         INSERT INTO directory_mode VALUES (1, 'native', 0);",
    )?;
    connection.pragma_update(None, "application_id", APPLICATION_ID)?;
    connection.pragma_update(None, "user_version", FORMAT_VERSION)?;
    Ok(())
}

fn read_mode(connection: &Connection) -> anyhow::Result<DirectoryMode> {
    let app: u32 = connection.pragma_query_value(None, "application_id", |row| row.get(0))?;
    let version: u32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    let journal: String = connection.pragma_query_value(None, "journal_mode", |row| row.get(0))?;
    if app != APPLICATION_ID || version != FORMAT_VERSION || journal != "delete" {
        bail!("unsupported directory-mode ledger format or journal mode");
    }
    let (mode, count): (String, u64) = connection.query_row(
        "SELECT mode, (SELECT COUNT(*) FROM directory_mode) FROM directory_mode WHERE singleton = 1",
        [], |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if count != 1 {
        bail!("invalid directory-mode ledger row count");
    }
    match mode.as_str() {
        "native" => Ok(DirectoryMode::Native),
        "workspace" => Ok(DirectoryMode::Workspace),
        _ => bail!("invalid persisted directory mode"),
    }
}

fn classify_conflict(error: anyhow::Error) -> anyhow::Error {
    if error
        .downcast_ref::<rusqlite::Error>()
        .is_some_and(|error| {
            matches!(
                error.sqlite_error_code(),
                Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked)
            )
        })
    {
        DirectoryConflict::Busy.into()
    } else {
        error
    }
}

#[cfg(test)]
mod tests;

fn require_clean_legacy_journal(directory: &Path) -> anyhow::Result<()> {
    match std::fs::metadata(directory.join("runtime-journal.jsonl")) {
        Ok(metadata) if metadata.len() != 0 => {
            bail!(
                "recover the legacy journal and stop the native server cleanly before workspace activation"
            );
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).context("failed to inspect legacy journal before activation"),
    }
}

fn read_data_mode(directory: &Path) -> anyhow::Result<DirectoryMode> {
    let path = directory.join("ccp.sqlite3");
    if !path.try_exists()? {
        return Ok(DirectoryMode::Native);
    }
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    connection.busy_timeout(Duration::ZERO)?;
    crate::init::reject_unsupported_schema(&connection)?;
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'ccp_runtime_mode')",
        [], |row| row.get(0),
    )?;
    if !exists {
        return Ok(DirectoryMode::Native);
    }
    let name: String = connection.query_row(
        "SELECT mode FROM ccp_runtime_mode WHERE singleton = 1",
        [],
        |row| row.get(0),
    )?;
    match name.as_str() {
        "native" => Ok(DirectoryMode::Native),
        "workspace" => Ok(DirectoryMode::Workspace),
        _ => bail!("invalid data-database runtime mode"),
    }
}

fn persist_data_activation(directory: &Path) -> anyhow::Result<()> {
    let mut connection = Connection::open(directory.join("ccp.sqlite3"))?;
    connection.busy_timeout(Duration::ZERO)?;
    connection.pragma_update(None, "synchronous", "FULL")?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mode: String = transaction.query_row(
        "SELECT mode FROM ccp_runtime_mode WHERE singleton = 1",
        [],
        |row| row.get(0),
    )?;
    match mode.as_str() {
        "workspace" => {}
        "native" => {
            let now = i64::try_from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_secs(),
            )?;
            transaction.execute(
                "UPDATE ccp_runtime_mode SET mode = 'workspace' WHERE singleton = 1",
                [],
            )?;
            let recorded = transaction.execute("INSERT INTO ccp_access_audit(actor_kind, actor_id, action, target_id, recorded_at)
                SELECT 'local_operator', 'local', 'activate_workspace_mode', instance_id, ?1 FROM ccp_instance WHERE singleton = 1", [now])?;
            if recorded != 1 {
                bail!("activation requires a persisted instance identity");
            }
        }
        _ => bail!("invalid data-database runtime mode"),
    }
    transaction.commit()?;
    Ok(())
}
