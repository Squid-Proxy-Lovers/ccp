// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Local-only first-boot reservation, never a remote ownership/reset operation.
//! An instance becomes authoritative only when its complete database is published.
use anyhow::{Context, bail};
use rusqlite::{Connection, OpenFlags, params};
use std::{
    fs::{self, File, OpenOptions},
    path::Path,
};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Pending(Uuid),
    Initialized(Uuid),
}

/// Reserve a previously empty directory without creating any administrator.
/// Classification precedes the mutating DirectoryLease APIs. Existing directories
/// are inspected without migrations, journal recovery or sidecar creation.
/// Pending is an observation, not authority to mint a code: the runtime must
/// reread owner state after acquiring its exclusive workspace lease.
pub fn prepare(directory: &Path) -> anyhow::Result<State> {
    if let Some(state) = inspect(directory)? {
        return Ok(state);
    }
    fs::create_dir_all(directory).context("cannot create setup directory")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
    }
    // Recheck after mkdir: another initializer may have won in the meantime.
    if let Some(state) = inspect(directory)? {
        return Ok(state);
    }
    let temporary = directory.join(format!(".ccp-setup-{}.sqlite3", Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(&temporary)?;
    let result = publish(directory, &temporary);
    fs::remove_file(&temporary).context("cannot remove owned setup staging file")?;
    sync_directory(directory)?;
    result
}
/// Provision TLS only after an explicit reservation or an existing initialized
/// workspace was validated. Legacy workspace-tls-init remains owner-first.
/// This does not hold a runtime lease or return any remote claim authority.
pub fn provision(
    directory: &Path,
    names: &[String],
) -> anyhow::Result<crate::instance_tls::TrustInfo> {
    prepare(directory)?;
    crate::instance_tls::initialize_setup(directory, names)
}

/// Revalidate after taking the exclusive lease. The immutable preflight may have
/// raced a native writer or omitted a WAL; never activate on that snapshot alone.
pub(crate) fn validate_leased(lease: &crate::directory_mode::DirectoryLease) -> anyhow::Result<()> {
    if !lease.allows_maintenance() {
        bail!("setup revalidation requires maintenance authority");
    }
    if !clean_journal(lease.directory())? {
        bail!("recover the native journal before setup");
    }
    let connection = Connection::open_with_flags(
        lease.directory().join("ccp.sqlite3"),
        OpenFlags::SQLITE_OPEN_READ_WRITE,
    )?;
    crate::init::reject_unsupported_schema(&connection)?;
    let mode: String = connection.query_row(
        "SELECT mode FROM ccp_runtime_mode WHERE singleton=1",
        [],
        |row| row.get(0),
    )?;
    if mode == "native" {
        let count: u64 =
            connection.query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))?;
        if count != 0 {
            bail!("native data appeared during setup; use explicit conversion");
        }
    } else if mode != "workspace" {
        bail!("invalid runtime mode");
    }
    Ok(())
}

fn clean_journal(directory: &Path) -> anyhow::Result<bool> {
    match fs::symlink_metadata(directory.join("runtime-journal.jsonl")) {
        Ok(meta) => Ok(meta.is_file() && meta.len() == 0),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(error.into()),
    }
}

fn publish(directory: &Path, temporary: &Path) -> anyhow::Result<State> {
    crate::init::init_sqlite(temporary)?;
    let mut connection = Connection::open(temporary)?;
    connection.pragma_update(None, "synchronous", "FULL")?;
    let transaction = connection.transaction()?;
    let instance: String = transaction.query_row(
        "SELECT instance_id FROM ccp_instance WHERE singleton=1",
        [],
        |r| r.get(0),
    )?;
    let id = Uuid::parse_str(&instance)?;
    let now = crate::instance_tls::now()?;
    transaction.execute(
        "INSERT INTO ccp_setup_reservation(singleton,instance_id,created_at) VALUES(1,?1,?2)",
        params![instance, now],
    )?;
    transaction.execute(
        "UPDATE ccp_runtime_mode SET mode='workspace' WHERE singleton=1",
        [],
    )?;
    transaction.commit()?;
    connection
        .close()
        .map_err(|(_connection, error)| error)
        .context("cannot close staged setup database")?;
    File::open(temporary)?.sync_all()?;
    // Never overwrite a concurrently created database (native, foreign or ours).
    fs::hard_link(temporary, directory.join("ccp.sqlite3"))
        .context("data appeared during setup; existing database was not replaced")?;
    sync_directory(directory)?;
    Ok(State::Pending(id))
}
fn inspect(directory: &Path) -> anyhow::Result<Option<State>> {
    let metadata = match fs::symlink_metadata(directory) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        bail!("setup requires a real directory");
    }
    if fs::read_dir(directory)?.next().is_none() {
        return Ok(None);
    }
    let database = directory.join("ccp.sqlite3");
    if !database.try_exists()? {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with(".ccp-setup-") && name.ends_with(".sqlite3") {
                bail!(
                    "unpublished setup file remains at {}; stop all processes and inspect or restore a matching backup; no files were removed",
                    entry.path().display()
                );
            }
        }
    }
    let meta = fs::symlink_metadata(&database)
        .context("nonempty directory is not a reserved CCP instance; inspect it before setup")?;
    if !meta.is_file() || meta.file_type().is_symlink() {
        bail!("setup database must be a regular file");
    }
    let absolute = if database.is_absolute() {
        database
    } else {
        std::env::current_dir()?.join(database)
    };
    let mut uri = url::Url::from_file_path(absolute)
        .map_err(|_path| anyhow::anyhow!("invalid setup database path"))?;
    uri.set_query(Some("mode=ro&immutable=1"));
    let connection = Connection::open_with_flags(
        uri.as_str(),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )?;
    crate::init::reject_unsupported_schema(&connection)?;
    let mode: String = connection.query_row(
        "SELECT mode FROM ccp_runtime_mode WHERE singleton=1",
        [],
        |r| r.get(0),
    )?;
    let instance: String = connection.query_row(
        "SELECT instance_id FROM ccp_instance WHERE singleton=1",
        [],
        |r| r.get(0),
    )?;
    let id = Uuid::parse_str(&instance)?;
    let admins: u64 =
        connection.query_row("SELECT COUNT(*) FROM ccp_server_admins", [], |r| r.get(0))?;
    if mode == "native" {
        let sessions: u64 =
            connection.query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))?;
        let clean = clean_journal(directory)?;
        // Explicit up may finish an earlier workspace-init sequence, but never
        // adopts native sessions or replays a native journal as workspace data.
        if admins == 0 || sessions != 0 || !clean {
            bail!("native data cannot be adopted by setup; use explicit conversion");
        }
        return Ok(Some(State::Initialized(id)));
    }
    if mode != "workspace" {
        bail!("invalid runtime mode");
    }
    if admins > 0 {
        return Ok(Some(State::Initialized(id)));
    }
    let reserved: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM ccp_setup_reservation WHERE singleton=1 AND instance_id=?1)",
        [instance],
        |r| r.get(0),
    )?;
    if !reserved {
        bail!("unowned database has no setup reservation; refusing to claim it");
    }
    let claimed: bool = connection.query_row(
        "SELECT claimed_at IS NOT NULL FROM ccp_setup_reservation WHERE singleton=1",
        [],
        |row| row.get(0),
    )?;
    Ok(Some(if claimed {
        State::Initialized(id)
    } else {
        State::Pending(id)
    }))
}
fn sync_directory(directory: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    File::open(directory)?.sync_all()?;
    #[cfg(not(unix))]
    {
        let _directory = directory;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
