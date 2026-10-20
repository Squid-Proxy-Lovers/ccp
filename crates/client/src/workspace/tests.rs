// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use rcgen::{
    Certificate, CertificateParams, ExtendedKeyUsagePurpose, Ia5String, IsCa, KeyPair,
    KeyUsagePurpose, SanType, SerialNumber,
};
use std::{fs, path::PathBuf};

mod lifecycle;
mod recovery;
mod responses;

const WHEN: i64 = 1_705_000_000;
fn authority() -> (Certificate, KeyPair) {
    let key = KeyPair::generate().unwrap();
    let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
    params.is_ca = IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    (params.self_signed(&key).unwrap(), key)
}
fn trust(ca: &Certificate) -> WorkspaceTrust {
    WorkspaceTrust::new(
        &Uuid::new_v4().to_string(),
        "https://localhost:8443",
        "https://localhost:8444",
        &ca.pem(),
    )
    .unwrap()
}
fn params(instance: &str, client: &str, credential: &Uuid) -> CertificateParams {
    let mut p = CertificateParams::new(Vec::<String>::new()).unwrap();
    p.not_before = rcgen::date_time_ymd(2024, 1, 1);
    p.not_after = rcgen::date_time_ymd(2024, 1, 25);
    p.serial_number = Some(SerialNumber::from_slice(credential.as_bytes()));
    p.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    p.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
    p.subject_alt_names = [
        ("instance", instance.to_string()),
        ("client", client.to_string()),
        ("credential", credential.to_string()),
    ]
    .into_iter()
    .map(|(kind, id)| SanType::URI(Ia5String::try_from(format!("ccp://{kind}/{id}")).unwrap()))
    .collect();
    p
}
fn material(
    trust: &WorkspaceTrust,
    ca: &Certificate,
    ca_key: &KeyPair,
    edit: impl FnOnce(&mut CertificateParams),
) -> Credential {
    let key = KeyPair::generate().unwrap();
    let client = Uuid::new_v4().to_string();
    let id = Uuid::new_v4();
    let mut p = params(&trust.instance_id, &client, &id);
    edit(&mut p);
    let expires = p.not_after.unix_timestamp();
    let certificate = p.signed_by(&key, ca, ca_key).unwrap();
    Credential {
        receipt: api::CertificateReceipt {
            instance_id: trust.instance_id.clone(),
            client_id: client,
            credential_id: id.to_string(),
            certificate_pem: certificate.pem(),
            expires_at: expires,
        },
        key_pem: key.serialize_pem(),
    }
}
fn directory() -> PathBuf {
    std::env::temp_dir().join(format!("ccp-profile-unit-{}", Uuid::new_v4()))
}
fn invite() -> String {
    format!("ccp_join_{}", "a".repeat(64))
}

#[test]
fn origins_refuse_url_ambiguity_and_never_create_a_plaintext_or_embedded_secret_origin() {
    for value in [
        "http://localhost",
        "https://user:secret@localhost",
        "https://@localhost",
        "https://localhost/path",
        "https://localhost/a/..",
        "https://localhost/%2e",
        "https://localhost//",
        "https://localhost?key=x",
        "https://localhost#x",
        " https://localhost",
        "https:localhost",
        "https://localhost\\x",
        "https://local\nhost",
    ] {
        assert!(network::origin(value).is_err(), "{value:?}");
    }
    assert_eq!(
        network::origin("https://LOCALHOST:8443").unwrap(),
        "https://localhost:8443/"
    );
    assert_eq!(
        network::origin("https://[::1]:8443/").unwrap(),
        "https://[::1]:8443/"
    );
}

#[test]
fn strict_pem_binds_one_block_and_pending_key_possession_to_one_request() {
    let mut pending = crypto::pending(&invite()).unwrap();
    crypto::validate_pending(&pending).unwrap();
    let original = pending.csr_pem.clone();
    for value in [
        format!("{original}{original}"),
        format!("junk\n{original}"),
        format!("{original}junk"),
        original.replace("CERTIFICATE REQUEST", "CERTIFICATE"),
    ] {
        pending.csr_pem = value;
        assert!(crypto::validate_pending(&pending).is_err());
    }
    pending.csr_pem = original;
    pending.key_pem = KeyPair::generate().unwrap().serialize_pem();
    assert!(crypto::validate_pending(&pending).is_err());
    assert!(crypto::invitation("SENTINEL_PRIVATE_VALUE").is_err());
    assert_eq!(
        crypto::invitation(&format!(" \n{}\n", invite())).unwrap(),
        invite()
    );
}

#[test]
fn receipt_requires_exact_signature_key_identity_serial_and_metadata_expiry() {
    let (ca, key) = authority();
    let trust = trust(&ca);
    let mut valid = material(&trust, &ca, &key, |_| {});
    crypto::credential(&trust, &valid, WHEN).unwrap();
    valid.receipt.client_id = Uuid::new_v4().to_string();
    assert!(crypto::credential(&trust, &valid, WHEN).is_err());
    let mut valid = material(&trust, &ca, &key, |_| {});
    valid.receipt.instance_id = Uuid::new_v4().to_string();
    assert!(crypto::credential(&trust, &valid, WHEN).is_err());
    let mut valid = material(&trust, &ca, &key, |_| {});
    valid.receipt.credential_id = Uuid::new_v4().to_string();
    assert!(crypto::credential(&trust, &valid, WHEN).is_err());
    let mut valid = material(&trust, &ca, &key, |_| {});
    valid.receipt.expires_at += 1;
    assert!(crypto::credential(&trust, &valid, WHEN).is_err());
    let mut valid = material(&trust, &ca, &key, |_| {});
    valid.key_pem = KeyPair::generate().unwrap().serialize_pem();
    assert!(crypto::credential(&trust, &valid, WHEN).is_err());
    let (foreign, foreign_key) = authority();
    let wrong = material(&trust, &foreign, &foreign_key, |_| {});
    assert!(crypto::credential(&trust, &wrong, WHEN).is_err());
}

#[test]
fn certificate_profile_rejects_extra_privilege_names_and_exclusive_expiry() {
    let (ca, key) = authority();
    let trust = trust(&ca);
    let variants: [fn(&mut CertificateParams); 6] = [
        |p| {
            p.extended_key_usages
                .push(ExtendedKeyUsagePurpose::ServerAuth)
        },
        |p| p.key_usages.push(KeyUsagePurpose::KeyCertSign),
        |p| {
            p.subject_alt_names.push(SanType::DnsName(
                Ia5String::try_from("wrong.example").unwrap(),
            ))
        },
        |p| p.subject_alt_names[0] = p.subject_alt_names[1].clone(),
        |p| p.serial_number = Some(SerialNumber::from(42)),
        |p| p.not_after = rcgen::date_time_ymd(2025, 1, 1),
    ];
    for edit in variants {
        assert!(crypto::credential(&trust, &material(&trust, &ca, &key, edit), WHEN).is_err());
    }
    let valid = material(&trust, &ca, &key, |_| {});
    assert!(crypto::credential(&trust, &valid, valid.receipt.expires_at).is_err());
    assert!(crypto::credential(&trust, &valid, WHEN - 40 * 24 * 60 * 60).is_err());
}

#[test]
fn profile_publishes_once_and_retains_exclusive_lease_and_pending_nonce_across_reopen() {
    let (ca, _key) = authority();
    let trust = trust(&ca);
    let dir = directory();
    store::create(&dir, &trust).unwrap();
    let mut profile = WorkspaceProfile::open(&dir).unwrap();
    profile.prepare(&invite()).unwrap();
    let first = profile.pending.as_ref().unwrap().request_id.clone();
    let csr = profile.pending.as_ref().unwrap().csr_pem.clone();
    let (other_ca, other_key) = authority();
    let other_trust = self::trust(&other_ca);
    let wrong = material(&other_trust, &other_ca, &other_key, |_| {});
    assert!(profile.activate(wrong).is_err());
    assert_eq!(profile.pending.as_ref().unwrap().request_id, first);
    assert!(WorkspaceProfile::open(&dir).is_err());
    assert!(store::create(&dir, &trust).is_err());
    profile.prepare(&invite()).unwrap();
    assert_eq!(profile.pending.as_ref().unwrap().request_id, first);
    assert!(
        profile
            .prepare(&format!("ccp_join_{}", "b".repeat(64)))
            .is_err()
    );
    let info = serde_json::to_string(&profile.info().unwrap()).unwrap();
    assert!(!info.contains("PRIVATE KEY"));
    assert!(!info.contains(&invite()));
    drop(profile);
    let profile = WorkspaceProfile::open(&dir).unwrap();
    assert_eq!(profile.pending.as_ref().unwrap().request_id, first);
    assert_eq!(profile.pending.as_ref().unwrap().csr_pem, csr);
    drop(profile);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn profile_refuses_missing_future_corrupt_and_foreign_state_without_reinitializing() {
    let (ca, _key) = authority();
    let trust = trust(&ca);
    let dir = directory();
    assert!(WorkspaceProfile::open(&dir).is_err());
    store::create(&dir, &trust).unwrap();
    let path = dir.join("workspace-profile.sqlite3");
    let db = rusqlite::Connection::open(&path).unwrap();
    db.pragma_update(None, "user_version", 99).unwrap();
    drop(db);
    assert!(WorkspaceProfile::open(&dir).is_err());
    let db = rusqlite::Connection::open(&path).unwrap();
    db.pragma_update(None, "user_version", 1).unwrap();
    db.pragma_update(None, "application_id", 123).unwrap();
    drop(db);
    assert!(WorkspaceProfile::open(&dir).is_err());
    let db = rusqlite::Connection::open(&path).unwrap();
    db.pragma_update(None, "application_id", 0x4343_5043)
        .unwrap();
    db.execute("UPDATE profile SET pending='SENTINEL_PRIVATE_VALUE'", [])
        .unwrap();
    drop(db);
    let error = WorkspaceProfile::open(&dir).err().unwrap().to_string();
    assert!(!error.contains("SENTINEL_PRIVATE_VALUE"));
    assert!(store::create(&dir, &trust).is_err());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn certificate_chain_or_broad_ca_input_cannot_replace_profile_trust() {
    let (ca, key) = authority();
    let trust = trust(&ca);
    let valid = material(&trust, &ca, &key, |_| {});
    assert!(
        WorkspaceTrust::new(
            &trust.instance_id,
            &trust.https_origin,
            &trust.mtls_origin,
            &format!("{}{}", ca.pem(), ca.pem())
        )
        .is_err()
    );
    assert!(
        WorkspaceTrust::new(
            &trust.instance_id,
            &trust.https_origin,
            &trust.mtls_origin,
            &valid.receipt.certificate_pem
        )
        .is_err()
    );
    let mut wrong = valid;
    wrong.receipt.certificate_pem.push_str(&ca.pem());
    assert!(crypto::credential(&trust, &wrong, WHEN).is_err());
}

#[cfg(unix)]
#[test]
fn private_profile_files_are_owner_only_and_symlink_or_loose_permissions_are_refused() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let (ca, _key) = authority();
    let dir = directory();
    store::create(&dir, &trust(&ca)).unwrap();
    let path = dir.join("workspace-profile.sqlite3");
    assert_eq!(
        fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let mut profile = WorkspaceProfile::open(&dir).unwrap();
    profile.prepare(&invite()).unwrap();
    profile
        .connection
        .execute_batch("BEGIN EXCLUSIVE; UPDATE profile SET trust=trust||' ';")
        .unwrap();
    assert_eq!(
        fs::metadata(dir.join("workspace-profile.sqlite3-journal"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    profile.connection.execute_batch("ROLLBACK").unwrap();
    drop(profile);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(WorkspaceProfile::open(&dir).is_err());
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let backup = dir.join("original.sqlite3");
    fs::rename(&path, &backup).unwrap();
    symlink(&backup, &path).unwrap();
    assert!(WorkspaceProfile::open(&dir).is_err());
    fs::remove_file(path).unwrap();
    fs::rename(backup, dir.join("workspace-profile.sqlite3")).unwrap();
    fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn expired_profile_metadata_remains_inspectable_but_cannot_be_used_online() {
    let (ca, key) = authority();
    let trust = trust(&ca);
    let dir = directory();
    store::create(&dir, &trust).unwrap();
    let credential = material(&trust, &ca, &key, |_| {});
    crypto::credential(&trust, &credential, WHEN).unwrap();
    // Restore a correctly signed historical profile whose certificate has since expired.
    let db = rusqlite::Connection::open(dir.join("workspace-profile.sqlite3")).unwrap();
    db.execute(
        "UPDATE profile SET credential=?1",
        [serde_json::to_string(&credential).unwrap()],
    )
    .unwrap();
    drop(db);
    let mut profile = WorkspaceProfile::open(&dir).unwrap();
    let info = profile.info().unwrap();
    assert!(info.expired);
    assert_eq!(info.client_id, Some(credential.receipt.client_id));
    let error = profile
        .workspaces(None, 10)
        .await
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("validity"));
    drop(profile);
    fs::remove_dir_all(dir).unwrap();
}
