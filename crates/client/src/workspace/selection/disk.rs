// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use rusqlite::{Connection, OpenFlags, params};
use std::time::Duration;

const FILE: &str = "caller-selection.sqlite3";
const APPLICATION_ID: i64 = 0x4343_5053;

pub(super) fn create(directory: &Path, state: &Selection) -> anyhow::Result<()> {
    store::publish(directory, FILE, |path| {
        let mut connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        connection.pragma_update(None, "journal_mode", "DELETE")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        let tx = connection.transaction()?;
        tx.pragma_update(None, "application_id", APPLICATION_ID)?;
        tx.pragma_update(None, "user_version", 1)?;
        tx.execute_batch("CREATE TABLE caller_selection(singleton INTEGER PRIMARY KEY CHECK(singleton=1),state TEXT NOT NULL)")?;
        tx.execute(
            "INSERT INTO caller_selection VALUES (1,?1)",
            [serde_json::to_string(state)?],
        )?;
        tx.commit()?;
        Ok(())
    })
}

pub(super) fn open(directory: &Path) -> anyhow::Result<Connection> {
    store::private_directory(directory, false)?;
    let path = directory.join(FILE);
    store::private_file(&path)?;
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    connection.busy_timeout(Duration::from_secs(2))?;
    check_format(&connection)?;
    let mode: String = connection.pragma_query_value(None, "journal_mode", |r| r.get(0))?;
    if mode != "delete" {
        bail!("unsupported caller selection journal mode");
    }
    connection.pragma_update(None, "synchronous", "FULL")?;
    connection.pragma_update(None, "locking_mode", "EXCLUSIVE")?;
    connection.execute_batch("BEGIN EXCLUSIVE; COMMIT;")?;
    check_format(&connection)?;
    Ok(connection)
}

fn check_format(connection: &Connection) -> anyhow::Result<()> {
    let app: i64 = connection.pragma_query_value(None, "application_id", |r| r.get(0))?;
    let version: i64 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if app != APPLICATION_ID || version != 1 {
        bail!("unrecognized or unsupported caller selection format");
    }
    Ok(())
}

pub(super) fn read(connection: &Connection) -> anyhow::Result<(Selection, String)> {
    let raw: String = connection.query_row(
        "SELECT state FROM caller_selection WHERE singleton=1",
        [],
        |r| r.get(0),
    )?;
    let state = serde_json::from_str(&raw)
        .map_err(|_data| anyhow::anyhow!("invalid caller selection data"))?;
    Ok((state, raw))
}

pub(super) fn replace(directory: &Path, state: &Selection, expected: u64) -> anyhow::Result<()> {
    let connection = open(directory)?;
    let (old, raw) = read(&connection)?;
    if old.instance_id != state.instance_id
        || old.client_id != state.client_id
        || old.revision != expected
    {
        bail!("caller selection changed; inspect it before retrying");
    }
    let changed = connection.execute(
        "UPDATE caller_selection SET state=?1 WHERE singleton=1 AND state=?2",
        params![serde_json::to_string(state)?, raw],
    )?;
    if changed != 1 {
        bail!("caller selection changed before commit");
    }
    Ok(())
}
