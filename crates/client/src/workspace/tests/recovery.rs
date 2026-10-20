// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

fn token() -> String {
    format!("ccp_recover_{}", "a".repeat(64))
}

#[test]
fn recovery_pending_retains_exact_target_key_and_nonce_and_refuses_replacement() {
    let (ca, _key) = authority();
    let dir = directory();
    store::create(&dir, &trust(&ca)).unwrap();
    let target = Uuid::new_v4().to_string();
    let mut p = WorkspaceProfile::open(&dir).unwrap();
    assert!(p.prepare_recovery("invalid", &token()).is_err());
    assert!(p.pending.is_none());
    p.prepare_recovery(&target, &token()).unwrap();
    let saved = serde_json::to_string(p.pending.as_ref().unwrap()).unwrap();
    assert_eq!(
        p.info().unwrap().pending_client_id.as_deref(),
        Some(target.as_str())
    );
    assert!(p.info().unwrap().client_id.is_none());
    assert!(
        p.prepare_recovery(&Uuid::new_v4().to_string(), &token())
            .is_err()
    );
    assert!(
        p.prepare_recovery(&target, &token().to_uppercase())
            .is_err()
    );
    assert!(p.prepare(&invite()).is_err());
    assert!(p.prepare_operation(Operation::Renewal, None).is_err());
    drop(p);
    let mut p = WorkspaceProfile::open(&dir).unwrap();
    p.prepare_recovery(&target, &token()).unwrap();
    assert_eq!(
        serde_json::to_string(p.pending.as_ref().unwrap()).unwrap(),
        saved
    );
    let bytes = fs::read(dir.join("workspace-profile.sqlite3")).unwrap();
    assert!(
        !bytes
            .windows(token().len())
            .any(|s| s == token().as_bytes())
    );
    drop(p);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn recovery_checks_expected_identity_and_rejects_active_or_malformed_state() {
    let (ca, key) = authority();
    let trust = trust(&ca);
    let mut recovered = material(&trust, &ca, &key, |_| {});
    let mut pending = crypto::pending_operation(Operation::Recovery, Some(&token()), None).unwrap();
    let proof = KeyPair::from_pem(&recovered.key_pem).unwrap();
    pending.key_pem = recovered.key_pem.clone();
    pending.csr_pem = CertificateParams::new(Vec::<String>::new())
        .unwrap()
        .serialize_request(&proof)
        .unwrap()
        .pem()
        .unwrap();
    pending.expected_client_id = Some(recovered.receipt.client_id.clone());
    crypto::validate_state(Some(&pending), None).unwrap();
    crypto::credential(&trust, &recovered, WHEN).unwrap();
    super::super::lifecycle::validate_result(&pending, &recovered, None).unwrap();
    recovered.receipt.client_id = Uuid::new_v4().to_string();
    assert!(super::super::lifecycle::validate_result(&pending, &recovered, None).is_err());
    assert!(crypto::validate_state(Some(&pending), Some(&recovered)).is_err());
    pending.expected_credential_id = Some(Uuid::new_v4().to_string());
    assert!(crypto::validate_state(Some(&pending), None).is_err());
    pending.expected_credential_id = None;
    pending.expected_client_id = None;
    assert!(crypto::validate_state(Some(&pending), None).is_err());
    assert!(crypto::invitation(&token()).is_err());
    assert!(crypto::recovery_invitation(&invite()).is_err());
}

fn version2(dir: &Path) {
    let db = rusqlite::Connection::open(dir.join("workspace-profile.sqlite3")).unwrap();
    db.execute_batch("ALTER TABLE completed_invitations RENAME TO history3; CREATE TABLE completed_invitations(invitation_hash TEXT PRIMARY KEY,operation TEXT NOT NULL CHECK(operation IN ('enrollment','admission'))); INSERT INTO completed_invitations SELECT * FROM history3; DROP TABLE history3; PRAGMA user_version=2;").unwrap();
}

#[test]
fn format2_migrates_all_existing_pending_states_and_completed_history_with_exact_snapshots() {
    let (ca, key) = authority();
    let trust = trust(&ca);
    for kind in ["empty", "enrollment", "active", "admission", "renewal"] {
        let dir = directory();
        store::create(&dir, &trust).unwrap();
        let active = material(&trust, &ca, &key, |_| {});
        if !matches!(kind, "empty" | "enrollment") {
            rusqlite::Connection::open(dir.join("workspace-profile.sqlite3"))
                .unwrap()
                .execute(
                    "UPDATE profile SET credential=?1",
                    [serde_json::to_string(&active).unwrap()],
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
            _ => {}
        }
        let pending = p.pending_record.clone();
        let credential = p.credential_record.clone();
        p.connection
            .execute(
                "INSERT INTO completed_invitations VALUES (?1,'admission')",
                [crypto::hash(b"completed")],
            )
            .unwrap();
        drop(p);
        version2(&dir);
        let bytes = fs::read(dir.join("workspace-profile.sqlite3")).unwrap();
        let p = WorkspaceProfile::open(&dir).unwrap();
        assert_eq!(p.info().unwrap().format, 3);
        assert_eq!(p.pending_record, pending);
        assert_eq!(p.credential_record, credential);
        assert_eq!(
            p.connection
                .query_row(
                    "SELECT COUNT(*) FROM completed_invitations WHERE invitation_hash=?1",
                    [crypto::hash(b"completed")],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
        let snapshot = dir.join(format!(
            "workspace-profile-v2-{}.sqlite3",
            crypto::hash(&bytes)
        ));
        assert_eq!(fs::read(snapshot).unwrap(), bytes);
        drop(p);
        fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn format2_migration_failure_keeps_history_pending_and_working_format_unchanged() {
    let (ca, _key) = authority();
    let dir = directory();
    store::create(&dir, &trust(&ca)).unwrap();
    version2(&dir);
    let db = rusqlite::Connection::open(dir.join("workspace-profile.sqlite3")).unwrap();
    db.execute_batch("INSERT INTO completed_invitations VALUES ('saved','admission'); CREATE TRIGGER refuse_upgrade BEFORE UPDATE ON profile BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    drop(db);
    let bytes = fs::read(dir.join("workspace-profile.sqlite3")).unwrap();
    assert!(WorkspaceProfile::open(&dir).is_err());
    assert_eq!(
        fs::read(dir.join("workspace-profile.sqlite3")).unwrap(),
        bytes
    );
    let db = rusqlite::Connection::open(dir.join("workspace-profile.sqlite3")).unwrap();
    assert_eq!(
        db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        db.query_row(
            "SELECT invitation_hash FROM completed_invitations",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "saved"
    );
    db.execute_batch("DROP TRIGGER refuse_upgrade").unwrap();
    drop(db);
    drop(WorkspaceProfile::open(&dir).unwrap());
    fs::remove_dir_all(dir).unwrap();
}
