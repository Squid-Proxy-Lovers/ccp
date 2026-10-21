// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
fn make_format4(p: &WorkspaceProfile) {
    p.connection.execute_batch("DROP TRIGGER status_pending_excludes_other; DROP TRIGGER content_pending_excludes_status; DROP TRIGGER lifecycle_pending_excludes_status; DROP TABLE status_state; PRAGMA user_version=4;").unwrap();
}
#[test]
fn format4_upgrade_keeps_large_raw_state_and_whole_private_backup_without_rewriting_entries() {
    let (dir, p) = profile();
    let credential = p.credential_record.clone();
    let trust: String = p
        .connection
        .query_row("SELECT trust FROM profile", [], |r| r.get(0))
        .unwrap();
    p.connection
        .execute(
            "INSERT INTO completed_invitations VALUES(?1,'admission')",
            [crypto::hash(b"history")],
        )
        .unwrap();
    p.connection
        .execute_batch(
            "CREATE TABLE padding(data BLOB); INSERT INTO padding VALUES(zeroblob(5242880));",
        )
        .unwrap();
    make_format4(&p);
    drop(p);
    let bytes = fs::read(dir.join("workspace-profile.sqlite3")).unwrap();
    assert!(bytes.len() > 4 * 1024 * 1024);
    let p = WorkspaceProfile::open(&dir).unwrap();
    assert_eq!(p.info().unwrap().format, 5);
    assert_eq!(p.credential_record, credential);
    assert_eq!(
        p.connection
            .query_row("SELECT trust FROM profile", [], |r| r.get::<_, String>(0))
            .unwrap(),
        trust
    );
    assert_eq!(
        p.connection
            .query_row("SELECT count(*) FROM completed_invitations", [], |r| r
                .get::<_, usize>(0))
            .unwrap(),
        1
    );
    assert!(p.info().unwrap().pending_status.is_none());
    assert!(p.info().unwrap().last_status.is_none());
    drop(p);
    let backup = dir.join(format!(
        "workspace-profile-v4-{}.sqlite3",
        crypto::hash(&bytes)
    ));
    assert_eq!(fs::read(&backup).unwrap(), bytes);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(backup).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    fs::remove_dir_all(dir).unwrap();
}
#[test]
fn failed_status_table_upgrade_keeps_old_version_and_unsupported_downgrade_never_ignores_pending() {
    let (dir, p) = profile();
    make_format4(&p);
    p.connection
        .execute_batch("CREATE VIEW status_state AS SELECT 1;")
        .unwrap();
    drop(p);
    assert!(WorkspaceProfile::open(&dir).is_err());
    let db = Connection::open(dir.join("workspace-profile.sqlite3")).unwrap();
    assert_eq!(
        db.pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
            .unwrap(),
        4
    );
    db.execute_batch("DROP VIEW status_state").unwrap();
    drop(db);
    let mut p = WorkspaceProfile::open(&dir).unwrap();
    prepare(&mut p, request());
    let raw = p.status_state.pending_record.clone();
    p.connection.pragma_update(None, "user_version", 4).unwrap();
    drop(p);
    assert!(WorkspaceProfile::open(&dir).is_err());
    let db = Connection::open(dir.join("workspace-profile.sqlite3")).unwrap();
    assert_eq!(
        db.query_row("SELECT pending FROM status_state", [], |r| r
            .get::<_, Option<String>>(0))
            .unwrap(),
        raw
    );
    drop(db);
    fs::remove_dir_all(dir).unwrap();
}
