// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

fn legacy(dir: &Path) {
    let db = rusqlite::Connection::open(dir.join("workspace-profile.sqlite3")).unwrap();
    let raw: Option<String> = db
        .query_row("SELECT pending FROM profile", [], |r| r.get(0))
        .unwrap();
    let pending = raw.map(|s| {
        let mut p: serde_json::Value = serde_json::from_str(&s).unwrap();
        for field in ["operation", "expected_client_id", "expected_credential_id"] {
            p.as_object_mut().unwrap().remove(field);
        }
        p.to_string()
    });
    db.execute("UPDATE profile SET pending=?1", [pending])
        .unwrap();
    db.execute_batch("DROP TABLE completed_invitations")
        .unwrap();
    db.pragma_update(None, "user_version", 1).unwrap();
}
fn snapshots(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("workspace-profile-v1-")
        })
        .collect()
}
fn version(dir: &Path) -> i64 {
    rusqlite::Connection::open(dir.join("workspace-profile.sqlite3"))
        .unwrap()
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap()
}

#[test]
fn format1_pending_migration_keeps_exact_nonce_key_csr_and_immutable_private_snapshot() {
    let (ca, _key) = authority();
    let dir = directory();
    store::create(&dir, &trust(&ca)).unwrap();
    let mut p = WorkspaceProfile::open(&dir).unwrap();
    p.prepare(&invite()).unwrap();
    let nonce = p.pending.as_ref().unwrap().request_id.clone();
    let key = p.pending.as_ref().unwrap().key_pem.clone();
    let csr = p.pending.as_ref().unwrap().csr_pem.clone();
    drop(p);
    legacy(&dir);
    let bytes = fs::read(dir.join("workspace-profile.sqlite3")).unwrap();
    let p = WorkspaceProfile::open(&dir).unwrap();
    assert_eq!(p.info().unwrap().format, 2);
    assert_eq!(p.pending.as_ref().unwrap().operation, Operation::Enrollment);
    assert_eq!(p.pending.as_ref().unwrap().request_id, nonce);
    assert_eq!(p.pending.as_ref().unwrap().key_pem, key);
    assert_eq!(p.pending.as_ref().unwrap().csr_pem, csr);
    assert_eq!(
        p.pending.as_ref().unwrap().invitation_hash,
        Some(crypto::hash(invite().as_bytes()))
    );
    drop(p);
    let saved = snapshots(&dir);
    assert_eq!(saved.len(), 1);
    assert_eq!(fs::read(&saved[0]).unwrap(), bytes);
    assert_eq!(
        rusqlite::Connection::open(&saved[0])
            .unwrap()
            .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&saved[0]).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    drop(WorkspaceProfile::open(&dir).unwrap());
    assert_eq!(snapshots(&dir).len(), 1);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn format1_empty_and_active_migrate_without_claiming_identity_or_losing_the_key() {
    let (ca, key) = authority();
    let trust = trust(&ca);
    for active in [false, true] {
        let dir = directory();
        store::create(&dir, &trust).unwrap();
        let m = material(&trust, &ca, &key, |_| {});
        if active {
            rusqlite::Connection::open(dir.join("workspace-profile.sqlite3"))
                .unwrap()
                .execute(
                    "UPDATE profile SET credential=?1",
                    [serde_json::to_string(&m).unwrap()],
                )
                .unwrap();
        }
        legacy(&dir);
        let p = WorkspaceProfile::open(&dir).unwrap();
        assert_eq!(p.info().unwrap().format, 2);
        assert_eq!(
            p.info().unwrap().client_id,
            active.then(|| m.receipt.client_id.clone())
        );
        if active {
            assert_eq!(p.credential.as_ref().unwrap().key_pem, m.key_pem);
        }
        assert!(p.pending.is_none());
        drop(p);
        assert_eq!(snapshots(&dir).len(), 1);
        fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn snapshot_publication_or_migration_failure_never_changes_the_old_profile() {
    let (ca, _key) = authority();
    let dir = directory();
    store::create(&dir, &trust(&ca)).unwrap();
    legacy(&dir);
    let bytes = fs::read(dir.join("workspace-profile.sqlite3")).unwrap();
    let blocked = dir.join(format!(
        "workspace-profile-v1-{}.sqlite3",
        crypto::hash(&bytes)
    ));
    fs::create_dir(&blocked).unwrap();
    assert!(WorkspaceProfile::open(&dir).is_err());
    assert_eq!(version(&dir), 1);
    assert_eq!(
        fs::read(dir.join("workspace-profile.sqlite3")).unwrap(),
        bytes
    );
    fs::remove_dir(blocked).unwrap();
    let db = rusqlite::Connection::open(dir.join("workspace-profile.sqlite3")).unwrap();
    db.execute_batch("CREATE TRIGGER fail_upgrade BEFORE UPDATE ON profile BEGIN SELECT RAISE(ABORT,'injected upgrade failure'); END;").unwrap();
    drop(db);
    assert!(WorkspaceProfile::open(&dir).is_err());
    assert_eq!(version(&dir), 1);
    assert_eq!(snapshots(&dir).len(), 1);
    let db = rusqlite::Connection::open(dir.join("workspace-profile.sqlite3")).unwrap();
    db.execute_batch("DROP TRIGGER fail_upgrade").unwrap();
    drop(db);
    drop(WorkspaceProfile::open(&dir).unwrap());
    assert_eq!(version(&dir), 2);
    assert_eq!(snapshots(&dir).len(), 2);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn typed_operations_bind_original_identity_and_cannot_reuse_the_wrong_key_or_invitation() {
    let (ca, key) = authority();
    let trust = trust(&ca);
    let active = material(&trust, &ca, &key, |_| {});
    let mut join =
        crypto::pending_operation(Operation::Admission, Some(&invite()), Some(&active)).unwrap();
    crypto::validate_state(Some(&join), Some(&active)).unwrap();
    assert_eq!(join.key_pem, active.key_pem);
    assert!(crypto::validate_state(Some(&join), None).is_err());
    join.expected_client_id = Some(Uuid::new_v4().to_string());
    assert!(crypto::validate_state(Some(&join), Some(&active)).is_err());
    let mut renew = crypto::pending_operation(Operation::Renewal, None, Some(&active)).unwrap();
    crypto::validate_state(Some(&renew), Some(&active)).unwrap();
    assert_ne!(renew.key_pem, active.key_pem);
    renew.key_pem = active.key_pem.clone();
    assert!(crypto::validate_state(Some(&renew), Some(&active)).is_err());
    let mut renew = crypto::pending_operation(Operation::Renewal, None, Some(&active)).unwrap();
    renew.invitation_hash = Some(crypto::hash(invite().as_bytes()));
    assert!(crypto::validate_pending(&renew).is_err());
    let dir = directory();
    store::create(&dir, &trust).unwrap();
    let mut p = WorkspaceProfile::open(&dir).unwrap();
    p.prepare(&invite()).unwrap();
    assert!(p.prepare_operation(Operation::Renewal, None).is_err());
    assert_eq!(p.pending.as_ref().unwrap().operation, Operation::Enrollment);
    drop(p);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn lifecycle_receipts_must_preserve_client_and_admission_credential_but_rotate_renewal() {
    let (ca, key) = authority();
    let trust = trust(&ca);
    let active = material(&trust, &ca, &key, |_| {});
    let join =
        crypto::pending_operation(Operation::Admission, Some(&invite()), Some(&active)).unwrap();
    super::super::lifecycle::validate_result(&join, &active, Some(&active)).unwrap();
    let mut wrong: Credential =
        serde_json::from_str(&serde_json::to_string(&active).unwrap()).unwrap();
    wrong.receipt.client_id = Uuid::new_v4().to_string();
    assert!(super::super::lifecycle::validate_result(&join, &wrong, Some(&active)).is_err());
    wrong.receipt.client_id = active.receipt.client_id.clone();
    wrong.receipt.credential_id = Uuid::new_v4().to_string();
    assert!(super::super::lifecycle::validate_result(&join, &wrong, Some(&active)).is_err());
    let renewal = crypto::pending_operation(Operation::Renewal, None, Some(&active)).unwrap();
    assert!(super::super::lifecycle::validate_result(&renewal, &active, Some(&active)).is_err());
    let newkey = KeyPair::from_pem(&renewal.key_pem).unwrap();
    let id = Uuid::new_v4();
    let parameters = params(&trust.instance_id, &active.receipt.client_id, &id);
    let expires = parameters.not_after.unix_timestamp();
    let signed = parameters.signed_by(&newkey, &ca, &key).unwrap();
    let valid = Credential {
        receipt: api::CertificateReceipt {
            instance_id: trust.instance_id.clone(),
            client_id: active.receipt.client_id.clone(),
            credential_id: id.to_string(),
            certificate_pem: signed.pem(),
            expires_at: expires,
        },
        key_pem: renewal.key_pem.clone(),
    };
    crypto::credential(&trust, &valid, WHEN).unwrap();
    super::super::lifecycle::validate_result(&renewal, &valid, Some(&active)).unwrap();
}

#[test]
fn automatic_renewal_window_is_bounded_and_a_root_capped_credential_does_not_spin() {
    let (ca, key) = authority();
    let trust = trust(&ca);
    let dir = directory();
    store::create(&dir, &trust).unwrap();
    let m = material(&trust, &ca, &key, |_| {});
    let expiry = m.receipt.expires_at;
    rusqlite::Connection::open(dir.join("workspace-profile.sqlite3"))
        .unwrap()
        .execute(
            "UPDATE profile SET credential=?1",
            [serde_json::to_string(&m).unwrap()],
        )
        .unwrap();
    let mut p = WorkspaceProfile::open(&dir).unwrap();
    assert!(!p.renewal_due(WHEN).unwrap());
    assert!(p.renewal_due(expiry - 6 * 24 * 60 * 60).unwrap());
    assert!(!p.renewal_due(expiry).unwrap());
    let mut cp = CertificateParams::new(Vec::<String>::new()).unwrap();
    cp.is_ca = IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    cp.key_usages = vec![KeyUsagePurpose::KeyCertSign];
    cp.not_after = rcgen::date_time_ymd(2024, 1, 25);
    let capped = cp.self_signed(&key).unwrap();
    p.trust = super::trust(&capped);
    p.credential.as_mut().unwrap().receipt.expires_at = capped.params().not_after.unix_timestamp();
    assert!(!p.renewal_due(expiry - 24 * 60 * 60).unwrap());
    drop(p);
    fs::remove_dir_all(dir).unwrap();
}
