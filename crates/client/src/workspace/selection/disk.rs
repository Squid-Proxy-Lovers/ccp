// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use rusqlite::{Connection, OpenFlags, params};

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
    store::exclusive_connection(&path, "caller selection is busy or unavailable", |db| {
        check_format(db)?;
        let mode: String = db.pragma_query_value(None, "journal_mode", |r| r.get(0))?;
        if mode != "delete" {
            bail!("unsupported caller selection journal mode");
        }
        Ok(())
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier, mpsc};
    use std::time::Duration;
    use uuid::Uuid;

    enum CommitOrder {
        First(mpsc::Sender<()>),
        After(mpsc::Receiver<()>),
    }

    fn commit_from_shared_snapshot(
        path: std::path::PathBuf,
        state: Selection,
        gate: Arc<Barrier>,
        order: CommitOrder,
    ) -> (String, anyhow::Result<()>) {
        let expected = {
            let connection =
                Connection::open_with_flags(path.join(FILE), OpenFlags::SQLITE_OPEN_READ_ONLY)
                    .unwrap();
            read(&connection).unwrap().0.revision
        };
        assert_eq!(expected, 0);
        gate.wait();
        // Both callers validated revision 0 before discovery. Reproduce the
        // commit ordering after that work, without assuming that simultaneous
        // SQLite EXCLUSIVE lock contention always becomes a revision error.
        let result = match order {
            CommitOrder::First(done) => {
                let result = replace(&path, &state, expected);
                assert!(result.is_ok(), "{result:?}");
                done.send(()).unwrap();
                result
            }
            CommitOrder::After(done) => {
                done.recv_timeout(Duration::from_secs(5)).unwrap();
                replace(&path, &state, expected)
            }
        };
        (state.workspace_ids[0].clone(), result)
    }

    #[test]
    fn independent_selection_commits_recheck_the_revision_after_a_shared_start() {
        let directory = std::env::temp_dir().join(format!("ccp-selection-race-{}", Uuid::new_v4()));
        let initial = Selection {
            instance_id: Uuid::new_v4().to_string(),
            client_id: Uuid::new_v4().to_string(),
            workspace_ids: Vec::new(),
            default_workspace_id: None,
            revision: 0,
        };
        create(&directory, &initial).unwrap();
        let barrier = Arc::new(Barrier::new(2));
        let [first, second] = [Uuid::new_v4(), Uuid::new_v4()].map(|id| Selection {
            instance_id: initial.instance_id.clone(),
            client_id: initial.client_id.clone(),
            workspace_ids: vec![id.to_string()],
            default_workspace_id: Some(id.to_string()),
            revision: 1,
        });
        let (done, completed) = mpsc::channel();
        let path = directory.clone();
        let gate = barrier.clone();
        let one = std::thread::spawn(move || {
            commit_from_shared_snapshot(path, first, gate, CommitOrder::First(done))
        });
        let path = directory.clone();
        let two = std::thread::spawn(move || {
            commit_from_shared_snapshot(path, second, barrier, CommitOrder::After(completed))
        });
        let (one, two) = (one.join().unwrap(), two.join().unwrap());
        assert_ne!(one.1.is_ok(), two.1.is_ok());
        let (winner, error) = match (one, two) {
            ((id, Ok(())), (_, Err(error))) | ((_, Err(error)), (id, Ok(()))) => (id, error),
            _ => panic!("exactly one selection update must succeed"),
        };
        assert!(error.to_string().contains("selection changed"), "{error:#}");
        let (saved, _) = read(&open(&directory).unwrap()).unwrap();
        assert_eq!(saved.revision, 1);
        assert_eq!(saved.workspace_ids, vec![winner.clone()]);
        assert_eq!(saved.default_workspace_id, Some(winner));
        std::fs::remove_dir_all(directory).unwrap();
    }
}
