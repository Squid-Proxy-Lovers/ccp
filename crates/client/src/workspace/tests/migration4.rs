// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

#[test]
fn format3_upgrade_preserves_raw_identity_history_and_pending_operations_with_exact_backup() {
    let (ca, key) = authority();
    let trust = trust(&ca);
    for kind in [
        "empty",
        "enrollment",
        "active",
        "admission",
        "renewal",
        "recovery",
    ] {
        let dir = directory();
        store::create(&dir, &trust).unwrap();
        if matches!(kind, "active" | "admission" | "renewal") {
            let credential = material(&trust, &ca, &key, |_| {});
            rusqlite::Connection::open(dir.join("workspace-profile.sqlite3"))
                .unwrap()
                .execute(
                    "UPDATE profile SET credential=?1",
                    [serde_json::to_string(&credential).unwrap()],
                )
                .unwrap();
        }
        let mut p = WorkspaceProfile::open(&dir).unwrap();
        match kind {
            "enrollment" => p.prepare(&invite()).unwrap(),
            "admission" => p
                .prepare_operation(Operation::Admission, Some(&invite()))
                .unwrap(),
            "renewal" => p.prepare_operation(Operation::Renewal, None).unwrap(),
            "recovery" => p
                .prepare_recovery(
                    &Uuid::new_v4().to_string(),
                    &format!("ccp_recover_{}", "a".repeat(64)),
                )
                .unwrap(),
            _ => {}
        }
        p.connection
            .execute(
                "INSERT INTO completed_invitations VALUES(?1,'recovery')",
                [crypto::hash(b"completed")],
            )
            .unwrap();
        let pending = p.pending_record.clone();
        let credential = p.credential_record.clone();
        p.connection
            .execute_batch("DROP TABLE content_state; PRAGMA user_version=3;")
            .unwrap();
        drop(p);
        let bytes = fs::read(dir.join("workspace-profile.sqlite3")).unwrap();
        let p = WorkspaceProfile::open(&dir).unwrap();
        assert_eq!(p.info().unwrap().format, 4);
        assert_eq!(p.pending_record, pending);
        assert_eq!(p.credential_record, credential);
        assert!(p.info().unwrap().pending_content.is_none());
        assert!(p.info().unwrap().last_content.is_none());
        assert_eq!(
            p.connection
                .query_row("SELECT operation FROM completed_invitations", [], |r| {
                    r.get::<_, String>(0)
                })
                .unwrap(),
            "recovery"
        );
        drop(p);
        let backup = dir.join(format!(
            "workspace-profile-v3-{}.sqlite3",
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
        drop(WorkspaceProfile::open(&dir).unwrap());
        fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn format3_failed_upgrade_keeps_version_and_state_and_old_file_limit_still_applies() {
    let (ca, _key) = authority();
    let dir = directory();
    store::create(&dir, &trust(&ca)).unwrap();
    let db = rusqlite::Connection::open(dir.join("workspace-profile.sqlite3")).unwrap();
    // A conflicting schema object prevents CREATE TABLE inside the upgrade transaction.
    db.execute_batch(
        "DROP TABLE content_state; CREATE VIEW content_state AS SELECT 1; PRAGMA user_version=3;",
    )
    .unwrap();
    drop(db);
    assert!(WorkspaceProfile::open(&dir).is_err());
    let db = rusqlite::Connection::open(dir.join("workspace-profile.sqlite3")).unwrap();
    assert_eq!(
        db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        3
    );
    db.execute_batch("DROP VIEW content_state; CREATE TABLE padding(data BLOB); INSERT INTO padding VALUES(zeroblob(4194304));").unwrap();
    drop(db);
    assert!(WorkspaceProfile::open(&dir).is_err());
    let db = rusqlite::Connection::open(dir.join("workspace-profile.sqlite3")).unwrap();
    assert_eq!(
        db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        3
    );
    drop(db);
    fs::remove_dir_all(dir).unwrap();
}
