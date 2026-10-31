// SPDX-License-Identifier: AGPL-3.0-or-later
use rusqlite::Connection;
use serde_json::Value;
use std::fs;
use std::process::Command;
use uuid::Uuid;

#[test]
fn local_inventory_cli_emits_only_complete_metadata_and_never_initializes_source() {
    let directory = std::env::temp_dir().join(format!("ccp-legacy-inspect-cli-{}", Uuid::new_v4()));
    fs::create_dir(&directory).unwrap();
    let path = directory.join("ccp.sqlite3");
    let db = Connection::open(&path).unwrap();
    db.pragma_update(None, "journal_mode", "WAL").unwrap();
    db.execute_batch(include_str!("../src/init-db.sql"))
        .unwrap();
    db.execute_batch("INSERT INTO schema_version(version) VALUES(2); INSERT INTO sessions(id,name) VALUES(1,'PRIVATE_SESSION'); INSERT INTO message_packs(session_id,name,context) VALUES(1,'PRIVATE_ENTRY','PRIVATE_BODY');").unwrap();
    drop(db);
    let before = fs::read(&path).unwrap();
    let run = Command::new(env!("CARGO_BIN_EXE_server"))
        .args(["legacy-inspect", "--data-dir"])
        .arg(&directory)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let value: Value = serde_json::from_slice(&run.stdout).unwrap();
    assert_eq!(value["format_version"], 1);
    assert_eq!(value["source_schema"], 2);
    assert_eq!(value["native"]["entries"], 1);
    assert_eq!(value["workspace"]["clients"], 0);
    assert_eq!(value["observation_only"], true);
    assert_eq!(value["conversion_performed"], false);
    assert!(!String::from_utf8_lossy(&run.stdout).contains("PRIVATE"));
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
    fs::write(
        directory.join("runtime-journal.jsonl"),
        b"PRIVATE_PENDING_BODY",
    )
    .unwrap();
    let refused = Command::new(env!("CARGO_BIN_EXE_server"))
        .args(["legacy-inspect", "--data-dir"])
        .arg(&directory)
        .output()
        .unwrap();
    assert!(!refused.status.success());
    assert!(refused.stdout.is_empty());
    assert!(!String::from_utf8_lossy(&refused.stderr).contains("PRIVATE_PENDING_BODY"));
    assert!(String::from_utf8_lossy(&refused.stderr).contains("producing software"));
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(
        fs::read(directory.join("runtime-journal.jsonl")).unwrap(),
        b"PRIVATE_PENDING_BODY"
    );
    fs::remove_dir_all(directory).unwrap();
}
