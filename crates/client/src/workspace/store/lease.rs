// SPDX-License-Identifier: AGPL-3.0-or-later
use anyhow::Context;
use rusqlite::{Connection, ErrorCode, OpenFlags};
use std::{
    path::Path,
    time::{Duration, Instant},
};

pub(in crate::workspace) fn exclusive_connection(
    path: &Path,
    busy_message: &'static str,
    validate: impl Fn(&Connection) -> anyhow::Result<()>,
) -> anyhow::Result<Connection> {
    // Validate without CREATE or an application write transaction. Read-write
    // access lets SQLite recover a crash-left hot journal before the first read.
    // Retire this connection completely before competing for EXCLUSIVE use.
    {
        let validation = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        validation.busy_timeout(Duration::from_secs(2))?;
        validate(&validation)?;
    }
    let connection = acquire(path).context(busy_message)?;
    connection.busy_timeout(Duration::from_secs(2))?;
    connection.pragma_update(None, "synchronous", "FULL")?;
    // Another cooperating opener may have migrated the file while we waited.
    validate(&connection)?;
    Ok(connection)
}

fn acquire(path: &Path) -> rusqlite::Result<Connection> {
    let deadline = Instant::now() + Duration::from_secs(2);
    acquire_with_retry(path, || {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return false;
        }
        std::thread::sleep(Duration::from_millis(5).min(remaining));
        true
    })
}

// Keep the retry policy separate so tests can release a competing lease only
// after an observed BUSY, without depending on scheduling or wall-clock time.
fn acquire_with_retry(
    path: &Path,
    mut retry: impl FnMut() -> bool,
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
                // SQLite's busy wait can keep a contender's reader lock while
                // another opener needs it released. Drop the entire attempt
                // before waiting; no profile mutation has run at this point.
                drop(connection);
                if !busy || !retry() {
                    return Err(error);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
