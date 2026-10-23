// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;
use std::{
    cell::Cell,
    fs,
    sync::{Arc, Barrier},
};

fn valid(db: &Connection) -> anyhow::Result<()> {
    let version: i64 = db.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version != 1 {
        anyhow::bail!("unsupported fixture format");
    }
    Ok(())
}
fn database() -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "ccp-private-lease-{}.sqlite3",
        uuid::Uuid::new_v4()
    ));
    let db = Connection::open(&path).unwrap();
    db.execute_batch(
        "PRAGMA user_version=1; CREATE TABLE counter(value INTEGER); INSERT INTO counter VALUES(0)",
    )
    .unwrap();
    path
}

#[test]
fn concurrent_validated_openers_release_readers_before_exclusive_acquisition() {
    let path = database();
    let barrier = Arc::new(Barrier::new(2));
    let threads: Vec<_> = (0..2)
        .map(|_| {
            let p = path.clone();
            let gate = barrier.clone();
            std::thread::spawn(move || {
                let first = Cell::new(true);
                let db = exclusive_connection(&p, "busy fixture", |db| {
                    valid(db)?;
                    if first.replace(false) {
                        gate.wait();
                    }
                    Ok(())
                })
                .unwrap();
                db.execute("UPDATE counter SET value=value+1", []).unwrap();
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    let db = Connection::open(&path).unwrap();
    assert_eq!(
        db.query_row("SELECT value FROM counter", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
    drop(db);
    fs::remove_file(path).unwrap();
}

#[test]
fn empty_foreign_and_concurrently_changed_formats_are_never_initialized_or_used() {
    let path = database();
    fs::write(&path, b"").unwrap();
    assert!(exclusive_connection(&path, "busy fixture", valid).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"");
    fs::remove_file(&path).unwrap();
    let path = database();
    let changed = Cell::new(false);
    assert!(
        exclusive_connection(&path, "busy fixture", |db| {
            valid(db)?;
            if !changed.replace(true) {
                let foreign = Connection::open(&path)?;
                foreign.pragma_update(None, "user_version", 99)?;
            }
            Ok(())
        })
        .is_err()
    );
    let before = fs::read(&path).unwrap();
    assert!(exclusive_connection(&path, "busy fixture", valid).is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
    let db = Connection::open(&path).unwrap();
    assert_eq!(
        db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        99
    );
    assert_eq!(
        db.query_row("SELECT value FROM counter", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    drop(db);
    fs::remove_file(path).unwrap();
}

#[test]
fn a_crash_left_hot_journal_is_recovered_before_validation_and_exclusive_use() {
    let directory = std::env::temp_dir().join(format!("ccp-hot-lease-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&directory).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
    let _cleanup = Cleanup(directory.clone());
    let original = directory.join("original.sqlite3");
    let copy = directory.join("interrupted.sqlite3");
    let journal = directory.join("interrupted.sqlite3-journal");
    {
        let db = Connection::open(&original).unwrap();
        db.execute_batch(
            "PRAGMA journal_mode=DELETE; PRAGMA user_version=1;
            CREATE TABLE payload(value BLOB);
            WITH RECURSIVE n(i) AS(SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<4000)
            INSERT INTO payload SELECT zeroblob(500) FROM n;",
        )
        .unwrap();
    }
    {
        let db = Connection::open_with_flags(&original, OpenFlags::SQLITE_OPEN_READ_WRITE).unwrap();
        db.pragma_update(None, "synchronous", "FULL").unwrap();
        db.pragma_update(None, "locking_mode", "EXCLUSIVE").unwrap();
        db.execute_batch(
            "PRAGMA cache_size=2; BEGIN IMMEDIATE;
            UPDATE payload SET value=zeroblob(600); PRAGMA user_version=99;",
        )
        .unwrap();
        // The bundled SQLite pager has spilled dirty pages. Copy both files at
        // this quiescent point while the transaction still holds exclusion:
        // the copy represents process loss without invoking an exit in the test.
        fs::copy(&original, &copy).unwrap();
        fs::copy(directory.join("original.sqlite3-journal"), &journal).unwrap();
    }
    let read_only = Connection::open_with_flags(&copy, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let failure = read_only
        .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
        .unwrap_err();
    assert_eq!(failure.sqlite_error().unwrap().extended_code, 776); // SQLITE_READONLY_ROLLBACK
    drop(read_only);
    let db = exclusive_connection(&copy, "busy fixture", valid).unwrap();
    assert_eq!(
        db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("SELECT count(*),sum(length(value)) FROM payload", [], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
        })
        .unwrap(),
        (4000, 2_000_000)
    );
    assert_eq!(
        db.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
    assert!(!journal.exists());
}
