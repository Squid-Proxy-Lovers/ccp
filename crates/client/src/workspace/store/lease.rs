// SPDX-License-Identifier: AGPL-3.0-or-later
use anyhow::{Context, bail};
use rusqlite::{Connection, ErrorCode, OpenFlags};
use std::{
    ffi::OsStr,
    path::Path,
    time::{Duration, Instant},
};

/// Overrides how long an opener waits for another holder to release the lease.
const LEASE_WAIT_ENV: &str = "CCP_WORKSPACE_LEASE_WAIT_MS";
// A holder keeps the lease across its network round trip, so concurrent calls
// sharing one profile queue behind each other. Wait long enough for several
// bounded requests to finish, yet below the MCP bridge's 60s child timeout.
const DEFAULT_LEASE_WAIT: Duration = Duration::from_secs(30);
const MAX_LEASE_WAIT_MS: u64 = 600_000;

#[cfg(test)]
thread_local! {
    // Lets in-crate tests prove that a held lease is refused without the full
    // production wait. Never compiled into the client.
    pub(in crate::workspace) static TEST_LEASE_WAIT: std::cell::Cell<Option<Duration>> =
        const { std::cell::Cell::new(None) };
}

pub(in crate::workspace) fn exclusive_connection(
    path: &Path,
    busy_message: &'static str,
    validate: impl Fn(&Connection) -> anyhow::Result<()>,
) -> anyhow::Result<Connection> {
    #[cfg(test)]
    if let Some(wait) = TEST_LEASE_WAIT.get() {
        return exclusive_connection_within(path, busy_message, wait, validate);
    }
    let wait = lease_wait(std::env::var_os(LEASE_WAIT_ENV).as_deref())?;
    exclusive_connection_within(path, busy_message, wait, validate)
}

fn exclusive_connection_within(
    path: &Path,
    busy_message: &'static str,
    wait: Duration,
    validate: impl Fn(&Connection) -> anyhow::Result<()>,
) -> anyhow::Result<Connection> {
    // One budget covers both phases: a validation read also waits while
    // another opener holds the exclusive lease.
    let deadline = Instant::now() + wait;
    // Validate without CREATE or an application write transaction. Read-write
    // access lets SQLite recover a crash-left hot journal before the first read.
    // Retire this connection completely before competing for EXCLUSIVE use.
    {
        let validation = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        validation.busy_timeout(deadline.saturating_duration_since(Instant::now()))?;
        validate(&validation)?;
    }
    let connection = acquire(path, deadline).context(busy_message)?;
    connection.busy_timeout(Duration::from_secs(2))?;
    connection.pragma_update(None, "synchronous", "FULL")?;
    // Another cooperating opener may have migrated the file while we waited.
    validate(&connection)?;
    Ok(connection)
}

// Parse the optional override strictly: a malformed value fails closed rather
// than silently choosing a different wait.
fn lease_wait(value: Option<&OsStr>) -> anyhow::Result<Duration> {
    let Some(value) = value else {
        return Ok(DEFAULT_LEASE_WAIT);
    };
    let millis = value
        .to_str()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|ms| *ms <= MAX_LEASE_WAIT_MS);
    let Some(millis) = millis else {
        bail!("{LEASE_WAIT_ENV} must be whole milliseconds from 0 to {MAX_LEASE_WAIT_MS}");
    };
    Ok(Duration::from_millis(millis))
}

fn acquire(path: &Path, deadline: Instant) -> rusqlite::Result<Connection> {
    acquire_with_retry(
        path,
        || Instant::now() < deadline,
        || {
            std::thread::sleep(
                Duration::from_millis(5).min(deadline.saturating_duration_since(Instant::now())),
            );
        },
    )
}

// Keep the retry policy separate so tests can release a competing lease only
// after an observed BUSY, without depending on scheduling or wall-clock time.
// The deadline is checked before the attempt is dropped, so an attempt that
// starts before the deadline always gets one more try after its wait.
fn acquire_with_retry(
    path: &Path,
    mut may_retry: impl FnMut() -> bool,
    mut wait: impl FnMut(),
) -> rusqlite::Result<Connection> {
    loop {
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        connection.busy_timeout(Duration::ZERO)?;
        // rusqlite's typed pragma_update validates the pragma name with its own
        // query first, which can itself return BUSY under contention; a raw
        // statement sets the pragma without touching the database at all.
        connection.execute_batch("PRAGMA locking_mode = EXCLUSIVE;")?;
        match connection.execute_batch("BEGIN EXCLUSIVE; COMMIT;") {
            Ok(()) => return Ok(connection),
            Err(error) => {
                let busy = matches!(
                    error.sqlite_error_code(),
                    Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked)
                );
                if !busy || !may_retry() {
                    return Err(error);
                }
                // SQLite's busy wait can keep a contender's reader lock while
                // another opener needs it released. Drop the entire attempt
                // before waiting; no profile mutation has run at this point.
                drop(connection);
                wait();
            }
        }
    }
}

#[cfg(test)]
mod tests;
