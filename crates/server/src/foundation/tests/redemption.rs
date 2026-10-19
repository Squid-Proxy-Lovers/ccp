// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use rcgen::{
    BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, Ia5String, IsCa, KeyPair,
    SanType,
};
use rustls::pki_types::{CertificateDer, UnixTime};
use sha2::{Digest, Sha256};
use std::sync::Arc;

const CLOCK: i64 = 1_700_000_000;
const DEADLINE: i64 = CLOCK + 3600;

pub(super) fn authority() -> (IssuingAuthority, Vec<u8>) {
    let key = KeyPair::generate().unwrap();
    let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params
        .distinguished_name
        .push(DnType::CommonName, "workspace issuer");
    let cert = params.self_signed(&key).unwrap();
    (
        IssuingAuthority::from_pem(&cert.pem(), &key.serialize_pem()).unwrap(),
        cert.der().to_vec(),
    )
}

pub(super) fn csr_request(key: &KeyPair) -> RedemptionRequest {
    let params = CertificateParams::new(Vec::<String>::new()).unwrap();
    RedemptionRequest {
        request_id: Uuid::new_v4(),
        csr_pem: params.serialize_request(key).unwrap().pem().unwrap(),
        existing_credential: None,
    }
}

pub(super) fn invite(f: &mut Fixture, capacity: u32) -> IssuedInvitation {
    AccessStore::new(&mut f.connection)
        .unwrap()
        .issue_initial_invitation(
            Actor::Admin(f.admin),
            InitialInvitationRequest {
                workspace_id: f.workspace,
                role: DataRole::ReadWrite,
                max_clients: Some(capacity),
                expires_at: Some(DEADLINE),
            },
            CLOCK,
        )
        .unwrap()
}

pub(super) fn redeem(
    f: &mut Fixture,
    token: &IssuedInvitation,
    request: &RedemptionRequest,
    ca: &IssuingAuthority,
    now: i64,
) -> anyhow::Result<EnrollmentReceipt> {
    let mut store = AccessStore::new(&mut f.connection)?;
    match request.existing_credential {
        Some(id) => {
            store.admit_with_invitation(Actor::Credential(id), token.secret(), request, ca, now)
        }
        None => store.redeem_invitation(token.secret(), request, ca, now),
    }
}

#[test]
fn signed_redemption_is_atomic_and_retry_does_not_consume_another_slot() {
    let mut f = fixture();
    let token = invite(&mut f, 1);
    let key = KeyPair::generate().unwrap();
    let request = csr_request(&key);
    let (ca, root) = authority();
    let first = redeem(&mut f, &token, &request, &ca, CLOCK).unwrap();
    let retry = redeem(&mut f, &token, &request, &ca, CLOCK).unwrap();
    assert_eq!(first, retry);
    for table in [
        "ccp_clients",
        "ccp_credentials",
        "ccp_credential_certificates",
        "ccp_grants",
        "ccp_credential_grants",
        "ccp_invitation_receipts",
    ] {
        assert_eq!(rows(&f.connection, table), 1, "{table}");
    }
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    let status = store
        .invitation_status(Actor::Admin(f.admin), f.workspace, token.id, CLOCK)
        .unwrap();
    assert_eq!(status.uses, 1);
    assert!(status.initial_closed);
    let (_, pem) = x509_parser::pem::parse_x509_pem(first.certificate_pem.as_bytes()).unwrap();
    let mut roots = rustls::RootCertStore::empty();
    roots.add(CertificateDer::from(root)).unwrap();
    let verifier = rustls::server::WebPkiClientVerifier::builder_with_provider(
        Arc::new(roots),
        Arc::new(rustls::crypto::ring::default_provider()),
    )
    .build()
    .unwrap();
    let now = UnixTime::since_unix_epoch(std::time::Duration::from_secs(
        u64::try_from(CLOCK).unwrap(),
    ));
    assert!(
        verifier
            .verify_client_cert(&CertificateDer::from(pem.contents.clone()), &[], now)
            .is_ok()
    );
    let fingerprint: Vec<u8> = f
        .connection
        .query_row(
            "SELECT certificate_fingerprint FROM ccp_credential_certificates",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(fingerprint, Sha256::digest(&pem.contents).to_vec());
    assert!(
        authorize(
            &f.connection,
            Actor::Credential(first.credential_id),
            Action::WriteContext,
            Some(f.workspace),
            CLOCK
        )
        .is_ok()
    );
}

#[test]
fn caller_requested_names_and_privileges_are_not_copied_to_the_certificate() {
    let mut f = fixture();
    let token = invite(&mut f, 1);
    let key = KeyPair::generate().unwrap();
    let mut params = CertificateParams::new(vec!["attacker.example".to_string()]).unwrap();
    params
        .distinguished_name
        .push(DnType::CommonName, "global-admin");
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    params.subject_alt_names.push(SanType::URI(
        Ia5String::try_from("ccp://access/admin").unwrap(),
    ));
    let request = RedemptionRequest {
        request_id: Uuid::new_v4(),
        csr_pem: params.serialize_request(&key).unwrap().pem().unwrap(),
        existing_credential: None,
    };
    let (ca, _) = authority();
    let receipt = redeem(&mut f, &token, &request, &ca, CLOCK).unwrap();
    let (_, pem) = x509_parser::pem::parse_x509_pem(receipt.certificate_pem.as_bytes()).unwrap();
    let (_, cert) = x509_parser::parse_x509_certificate(&pem.contents).unwrap();
    let name = cert
        .subject()
        .iter_common_name()
        .next()
        .unwrap()
        .as_str()
        .unwrap();
    assert_eq!(name, receipt.credential_id.to_string());
    let sans = &cert
        .subject_alternative_name()
        .unwrap()
        .unwrap()
        .value
        .general_names;
    let instance = AccessStore::new(&mut f.connection)
        .unwrap()
        .instance_id()
        .unwrap();
    let expected = [
        format!("ccp://instance/{instance}"),
        format!("ccp://client/{}", receipt.client_id),
        format!("ccp://credential/{}", receipt.credential_id),
    ];
    assert_eq!(sans.len(), 3);
    for value in expected {
        assert!(sans.iter().any(
            |name| matches!(name, x509_parser::extensions::GeneralName::URI(uri) if *uri == value)
        ));
    }
    let usage = cert.extended_key_usage().unwrap().unwrap().value;
    assert!(usage.client_auth);
    assert!(!usage.server_auth);
    assert_eq!(rows(&f.connection, "ccp_server_admins"), 1);
}

#[test]
fn different_key_or_identity_claim_cannot_reuse_a_request_receipt() {
    let mut f = fixture();
    let token = invite(&mut f, 2);
    let key = KeyPair::generate().unwrap();
    let request = csr_request(&key);
    let (ca, _) = authority();
    let receipt = redeem(&mut f, &token, &request, &ca, CLOCK).unwrap();
    let mut forged = csr_request(&KeyPair::generate().unwrap());
    forged.request_id = request.request_id;
    assert!(redeem(&mut f, &token, &forged, &ca, CLOCK).is_err());
    let mut changed = request.clone();
    changed.existing_credential = Some(receipt.credential_id);
    assert!(redeem(&mut f, &token, &changed, &ca, CLOCK).is_err());
    let status = AccessStore::new(&mut f.connection)
        .unwrap()
        .invitation_status(Actor::Admin(f.admin), f.workspace, token.id, CLOCK)
        .unwrap();
    assert_eq!(status.uses, 1);
    assert_eq!(rows(&f.connection, "ccp_clients"), 1);
}

#[test]
fn quota_and_expiry_failures_do_not_create_partial_identities_or_reopen() {
    let mut f = fixture();
    let token = invite(&mut f, 1);
    let (ca, _) = authority();
    let first = csr_request(&KeyPair::generate().unwrap());
    redeem(&mut f, &token, &first, &ca, CLOCK).unwrap();
    assert!(
        redeem(
            &mut f,
            &token,
            &csr_request(&KeyPair::generate().unwrap()),
            &ca,
            CLOCK
        )
        .is_err()
    );
    assert_eq!(rows(&f.connection, "ccp_clients"), 1);
    let mut other = fixture();
    let token = invite(&mut other, 2);
    let request = csr_request(&KeyPair::generate().unwrap());
    assert!(redeem(&mut other, &token, &request, &ca, DEADLINE).is_err());
    assert!(redeem(&mut other, &token, &request, &ca, CLOCK).is_err());
    assert_eq!(rows(&other.connection, "ccp_clients"), 0);
    assert_eq!(rows(&other.connection, "ccp_invitation_closures"), 1);
    assert!(
        AccessStore::new(&mut other.connection)
            .unwrap()
            .invitation_status(Actor::Admin(other.admin), other.workspace, token.id, CLOCK)
            .unwrap()
            .initial_closed
    );
}

#[test]
fn existing_registration_reuse_requires_the_matching_private_key_and_preserves_certificate() {
    let mut f = fixture();
    let token = invite(&mut f, 2);
    let key = KeyPair::generate().unwrap();
    let (ca, _) = authority();
    let first = redeem(&mut f, &token, &csr_request(&key), &ca, CLOCK).unwrap();
    let second_token = AccessStore::new(&mut f.connection)
        .unwrap()
        .issue_individual_invitation(
            Actor::Admin(f.other_owner),
            IndividualInvitationRequest {
                workspace_id: f.other_workspace,
                role: DataRole::Read,
                expires_at: DEADLINE,
            },
            CLOCK,
        )
        .unwrap();
    let mut wrong = csr_request(&KeyPair::generate().unwrap());
    wrong.existing_credential = Some(first.credential_id);
    assert!(redeem(&mut f, &second_token, &wrong, &ca, CLOCK).is_err());
    let mut reuse = csr_request(&key);
    reuse.existing_credential = Some(first.credential_id);
    let second = redeem(&mut f, &second_token, &reuse, &ca, CLOCK).unwrap();
    assert_eq!(second.client_id, first.client_id);
    assert_eq!(second.credential_id, first.credential_id);
    assert_eq!(second.certificate_pem, first.certificate_pem);
    assert_eq!(second.expires_at, first.expires_at);
    assert_ne!(second.grant_id, first.grant_id);
    assert_eq!(rows(&f.connection, "ccp_clients"), 1);
    assert_eq!(rows(&f.connection, "ccp_credential_certificates"), 1);
    assert!(
        authorize(
            &f.connection,
            Actor::Credential(first.credential_id),
            Action::ReadContext,
            Some(f.other_workspace),
            CLOCK
        )
        .is_ok()
    );
    assert!(
        authorize(
            &f.connection,
            Actor::Credential(first.credential_id),
            Action::WriteContext,
            Some(f.other_workspace),
            CLOCK
        )
        .is_err()
    );
}

#[test]
fn invalid_signature_and_bad_secret_do_not_consume_an_invitation() {
    let mut f = fixture();
    let token = invite(&mut f, 1);
    let (ca, _) = authority();
    let mut request = csr_request(&KeyPair::generate().unwrap());
    let mut bytes = request.csr_pem.as_bytes().to_vec();
    let end = request.csr_pem.find("-----END").unwrap();
    let position = bytes[..end]
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, byte)| byte.is_ascii_alphanumeric())
        .nth(10)
        .unwrap()
        .0;
    bytes[position] = if bytes[position] == b'A' { b'B' } else { b'A' };
    request.csr_pem = String::from_utf8(bytes).unwrap();
    assert!(redeem(&mut f, &token, &request, &ca, CLOCK).is_err());
    request.csr_pem = "not a CSR".to_string();
    assert!(redeem(&mut f, &token, &request, &ca, CLOCK).is_err());
    request.csr_pem = "x".repeat(16 * 1024 + 1);
    assert!(redeem(&mut f, &token, &request, &ca, CLOCK).is_err());
    let request = csr_request(&KeyPair::generate().unwrap());
    for secret in [
        "".to_string(),
        "wrong".to_string(),
        format!("ccp_join_{}", "0".repeat(64)),
    ] {
        assert!(
            AccessStore::new(&mut f.connection)
                .unwrap()
                .redeem_invitation(&secret, &request, &ca, CLOCK)
                .is_err()
        );
    }
    assert!(redeem(&mut f, &token, &request, &ca, -1).is_err());
    assert_eq!(rows(&f.connection, "ccp_clients"), 0);
    let status = AccessStore::new(&mut f.connection)
        .unwrap()
        .invitation_status(Actor::Admin(f.owner), f.workspace, token.id, CLOCK)
        .unwrap();
    assert_eq!(status.uses, 0);
}

#[test]
fn audit_failure_rolls_back_certificate_grant_receipt_and_slot() {
    let mut f = fixture();
    let token = invite(&mut f, 1);
    let request = csr_request(&KeyPair::generate().unwrap());
    let (ca, _) = authority();
    f.connection.execute_batch("CREATE TRIGGER reject_redemption BEFORE INSERT ON ccp_access_audit WHEN NEW.action = 'redeem_invitation' BEGIN SELECT RAISE(ABORT, 'audit unavailable'); END;").unwrap();
    assert!(redeem(&mut f, &token, &request, &ca, CLOCK).is_err());
    for table in [
        "ccp_clients",
        "ccp_credentials",
        "ccp_grants",
        "ccp_credential_grants",
        "ccp_credential_certificates",
        "ccp_invitation_receipts",
    ] {
        assert_eq!(rows(&f.connection, table), 0, "{table}");
    }
    let status = AccessStore::new(&mut f.connection)
        .unwrap()
        .invitation_status(Actor::Admin(f.owner), f.workspace, token.id, CLOCK)
        .unwrap();
    assert_eq!(status.uses, 0);
    assert!(!status.initial_closed);
    f.connection
        .execute_batch("DROP TRIGGER reject_redemption")
        .unwrap();
    assert!(redeem(&mut f, &token, &request, &ca, CLOCK).is_ok());
}

#[test]
fn archived_workspace_allows_only_existing_receipt_and_revocation_blocks_replay() {
    let mut f = fixture();
    let token = invite(&mut f, 2);
    let request = csr_request(&KeyPair::generate().unwrap());
    let (ca, _) = authority();
    let first = redeem(&mut f, &token, &request, &ca, CLOCK).unwrap();
    AccessStore::new(&mut f.connection)
        .unwrap()
        .set_archived(Actor::Admin(f.owner), f.workspace, true, CLOCK)
        .unwrap();
    assert_eq!(redeem(&mut f, &token, &request, &ca, CLOCK).unwrap(), first);
    assert!(
        redeem(
            &mut f,
            &token,
            &csr_request(&KeyPair::generate().unwrap()),
            &ca,
            CLOCK
        )
        .is_err()
    );
    AccessStore::new(&mut f.connection)
        .unwrap()
        .revoke_grant(Actor::Admin(f.admin), first.grant_id, CLOCK)
        .unwrap();
    assert!(redeem(&mut f, &token, &request, &ca, CLOCK).is_err());
    assert_eq!(rows(&f.connection, "ccp_clients"), 1);
}

#[test]
fn issuer_demotion_or_key_expiry_blocks_fresh_admission_and_receipt_replay() {
    let mut f = fixture();
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    let key = store
        .issue_admin_key(
            Actor::Admin(f.owner),
            &AdminKeyRequest {
                principal_id: f.admin,
                scope: AdminKeyScope::Workspace(f.workspace),
                name: "issuer".to_string(),
                expires_at: Some(CLOCK + 100),
            },
            CLOCK,
        )
        .unwrap();
    let actor = store.authenticate_admin_key(key.secret(), CLOCK).unwrap();
    let token = store
        .issue_initial_invitation(
            actor,
            InitialInvitationRequest {
                workspace_id: f.workspace,
                role: DataRole::Read,
                max_clients: Some(3),
                expires_at: Some(DEADLINE),
            },
            CLOCK,
        )
        .unwrap();
    let (ca, _) = authority();
    let request = csr_request(&KeyPair::generate().unwrap());
    redeem(&mut f, &token, &request, &ca, CLOCK).unwrap();
    assert!(redeem(&mut f, &token, &request, &ca, CLOCK + 100).is_err());
    assert!(
        redeem(
            &mut f,
            &token,
            &csr_request(&KeyPair::generate().unwrap()),
            &ca,
            CLOCK + 100
        )
        .is_err()
    );
    AccessStore::new(&mut f.connection)
        .unwrap()
        .revoke_workspace_admin(Actor::Admin(f.owner), f.workspace, f.admin, CLOCK)
        .unwrap();
    assert!(redeem(&mut f, &token, &request, &ca, CLOCK).is_err());
    assert_eq!(rows(&f.connection, "ccp_clients"), 1);
}

#[test]
fn disabled_clients_cannot_replay_or_reuse_identity_for_a_new_workspace() {
    let mut f = fixture();
    let token = invite(&mut f, 2);
    let key = KeyPair::generate().unwrap();
    let request = csr_request(&key);
    let (ca, _) = authority();
    let first = redeem(&mut f, &token, &request, &ca, CLOCK).unwrap();
    let other_token = AccessStore::new(&mut f.connection)
        .unwrap()
        .issue_individual_invitation(
            Actor::Admin(f.other_owner),
            IndividualInvitationRequest {
                workspace_id: f.other_workspace,
                role: DataRole::ReadWrite,
                expires_at: DEADLINE,
            },
            CLOCK,
        )
        .unwrap();
    AccessStore::new(&mut f.connection)
        .unwrap()
        .disable_client(Actor::Admin(f.global), first.client_id, CLOCK)
        .unwrap();
    assert!(redeem(&mut f, &token, &request, &ca, CLOCK).is_err());
    let mut reuse = csr_request(&key);
    reuse.existing_credential = Some(first.credential_id);
    assert!(redeem(&mut f, &other_token, &reuse, &ca, CLOCK).is_err());
    assert_eq!(rows(&f.connection, "ccp_grants"), 1);
}

#[test]
fn late_receipt_replay_does_not_reopen_admission_and_unused_individual_expiry_is_terminal() {
    let mut f = fixture();
    let token = invite(&mut f, 2);
    let request = csr_request(&KeyPair::generate().unwrap());
    let (ca, _) = authority();
    let first = redeem(&mut f, &token, &request, &ca, CLOCK).unwrap();
    assert_eq!(
        redeem(&mut f, &token, &request, &ca, DEADLINE).unwrap(),
        first
    );
    assert!(
        redeem(
            &mut f,
            &token,
            &csr_request(&KeyPair::generate().unwrap()),
            &ca,
            CLOCK
        )
        .is_err()
    );
    let other = AccessStore::new(&mut f.connection)
        .unwrap()
        .issue_individual_invitation(
            Actor::Admin(f.other_owner),
            IndividualInvitationRequest {
                workspace_id: f.other_workspace,
                role: DataRole::Read,
                expires_at: DEADLINE,
            },
            CLOCK,
        )
        .unwrap();
    let fresh = csr_request(&KeyPair::generate().unwrap());
    assert!(redeem(&mut f, &other, &fresh, &ca, DEADLINE).is_err());
    assert!(redeem(&mut f, &other, &fresh, &ca, CLOCK).is_err());
    assert_eq!(rows(&f.connection, "ccp_clients"), 1);
}

#[test]
fn issuer_ca_checks_prevent_invalid_signing_and_bound_leaf_expiry() {
    let key = KeyPair::generate().unwrap();
    let wrong = KeyPair::generate().unwrap();
    let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.not_after = time::OffsetDateTime::from_unix_timestamp(CLOCK + 120).unwrap();
    let cert = params.self_signed(&key).unwrap();
    assert!(IssuingAuthority::from_pem(&cert.pem(), &wrong.serialize_pem()).is_err());
    assert!(IssuingAuthority::from_pem("invalid certificate", &key.serialize_pem()).is_err());
    assert!(IssuingAuthority::from_pem(&cert.pem(), "invalid key").is_err());
    let leaf = CertificateParams::new(Vec::<String>::new())
        .unwrap()
        .self_signed(&key)
        .unwrap();
    assert!(IssuingAuthority::from_pem(&leaf.pem(), &key.serialize_pem()).is_err());
    let ca = IssuingAuthority::from_pem(&cert.pem(), &key.serialize_pem()).unwrap();
    let mut f = fixture();
    let token = invite(&mut f, 2);
    let receipt = redeem(
        &mut f,
        &token,
        &csr_request(&KeyPair::generate().unwrap()),
        &ca,
        CLOCK,
    )
    .unwrap();
    assert_eq!(receipt.expires_at, CLOCK + 120);
    assert!(
        redeem(
            &mut f,
            &token,
            &csr_request(&KeyPair::generate().unwrap()),
            &ca,
            CLOCK + 120
        )
        .is_err()
    );
    assert_eq!(rows(&f.connection, "ccp_clients"), 1);
}

fn redemption_race(same_request: bool) {
    use std::sync::Barrier;

    let path = std::env::temp_dir().join(format!("ccp-redemption-race-{}.db", Uuid::new_v4()));
    crate::init::init_sqlite(&path).unwrap();
    let mut f = fixture_with_connection(Connection::open(&path).unwrap());
    let token = invite(&mut f, if same_request { 1 } else { 2 });
    drop(f);
    let ca = Arc::new(authority().0);
    let shared = csr_request(&KeyPair::generate().unwrap());
    let barrier = Arc::new(Barrier::new(8));
    let workers: Vec<_> = (0..8)
        .map(|_| {
            let request = if same_request {
                shared.clone()
            } else {
                csr_request(&KeyPair::generate().unwrap())
            };
            let ca = Arc::clone(&ca);
            let barrier = Arc::clone(&barrier);
            let secret = token.secret().to_string();
            let path = path.clone();
            std::thread::spawn(move || {
                let mut connection = Connection::open(path).unwrap();
                connection
                    .busy_timeout(std::time::Duration::from_secs(5))
                    .unwrap();
                let mut store = AccessStore::new(&mut connection).unwrap();
                barrier.wait();
                store
                    .redeem_invitation(&secret, &request, &ca, CLOCK)
                    .map_err(|error| error.to_string())
            })
        })
        .collect();
    let replies: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();
    if same_request {
        let first = replies.first().unwrap().as_ref().unwrap();
        for reply in &replies {
            assert_eq!(reply.as_ref().unwrap(), first);
        }
    } else {
        assert_eq!(replies.iter().filter(|result| result.is_ok()).count(), 2);
        for error in replies.iter().filter_map(|result| result.as_ref().err()) {
            assert!(error.contains("not authorized or is closed"), "{error}");
        }
    }
    let connection = Connection::open(&path).unwrap();
    let expected = if same_request { 1 } else { 2 };
    for table in [
        "ccp_clients",
        "ccp_credentials",
        "ccp_credential_certificates",
        "ccp_grants",
        "ccp_invitation_receipts",
    ] {
        assert_eq!(rows(&connection, table), expected, "{table}");
    }
    let used: i64 = connection
        .query_row("SELECT uses FROM ccp_invitations", [], |row| row.get(0))
        .unwrap();
    assert_eq!(used, expected);
    drop(connection);
    fs::remove_file(path).unwrap();
}

#[test]
fn concurrent_distinct_redemptions_cannot_exceed_capacity() {
    redemption_race(false);
}

#[test]
fn concurrent_retries_return_one_committed_identity_and_certificate() {
    redemption_race(true);
}

#[test]
fn durable_receipt_and_old_keys_survive_schema7_upgrade_and_reopening() {
    let path = std::env::temp_dir().join(format!("ccp-redemption-upgrade-{}.db", Uuid::new_v4()));
    crate::init::init_sqlite(&path).unwrap();
    let mut f = fixture_with_connection(Connection::open(&path).unwrap());
    let old = enroll(&mut f, DataRole::Read);
    let token = AccessStore::new(&mut f.connection)
        .unwrap()
        .issue_individual_invitation(
            Actor::Admin(f.other_owner),
            IndividualInvitationRequest {
                workspace_id: f.other_workspace,
                role: DataRole::Read,
                expires_at: DEADLINE,
            },
            CLOCK,
        )
        .unwrap();
    f.connection.execute_batch("DROP TABLE ccp_invitation_receipts; DROP TABLE ccp_invitation_closures; DROP TABLE ccp_credential_certificates;
        DROP INDEX ccp_grant_workspace_identity; UPDATE schema_version SET version = 7;").unwrap();
    drop(f);
    crate::init::init_sqlite(&path).unwrap();
    let mut connection = Connection::open(&path).unwrap();
    let request = csr_request(&KeyPair::generate().unwrap());
    let (ca, _) = authority();
    let first = AccessStore::new(&mut connection)
        .unwrap()
        .redeem_invitation(token.secret(), &request, &ca, CLOCK)
        .unwrap();
    drop(connection);
    crate::init::init_sqlite(&path).unwrap();
    let mut connection = Connection::open(&path).unwrap();
    let retry = AccessStore::new(&mut connection)
        .unwrap()
        .redeem_invitation(token.secret(), &request, &ca, CLOCK)
        .unwrap();
    assert_eq!(first, retry);
    let old_client: String = connection
        .query_row(
            "SELECT client_id FROM ccp_credentials WHERE id = ?1",
            [old.credential_id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(old_client, old.client_id.to_string());
    assert_eq!(rows(&connection, "ccp_clients"), 2);
    let invalid_fks: i64 = connection
        .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(invalid_fks, 0);
    drop(connection);
    fs::remove_file(path).unwrap();
}

#[test]
fn instance_key_cannot_manage_invitations_even_when_its_principal_owns_the_workspace() {
    let mut f = fixture();
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    let mine = store
        .create_workspace(
            Actor::Admin(f.global),
            "instance owner's workspace",
            f.global,
            CLOCK,
        )
        .unwrap();
    let key = store
        .issue_admin_key(
            Actor::Admin(f.global),
            &AdminKeyRequest {
                principal_id: f.global,
                scope: AdminKeyScope::Instance,
                name: "instance".to_string(),
                expires_at: None,
            },
            CLOCK,
        )
        .unwrap();
    let instance = store.authenticate_admin_key(key.secret(), CLOCK).unwrap();
    let request = InitialInvitationRequest {
        workspace_id: mine,
        role: DataRole::Read,
        max_clients: Some(1),
        expires_at: Some(DEADLINE),
    };
    assert!(
        store
            .issue_initial_invitation(instance, request, CLOCK)
            .is_err()
    );
    let owner_key = store
        .issue_admin_key(
            Actor::Admin(f.global),
            &AdminKeyRequest {
                principal_id: f.global,
                scope: AdminKeyScope::Workspace(mine),
                name: "owner".to_string(),
                expires_at: None,
            },
            CLOCK,
        )
        .unwrap();
    let owner = store
        .authenticate_admin_key(owner_key.secret(), CLOCK)
        .unwrap();
    let token = store
        .issue_initial_invitation(owner, request, CLOCK)
        .unwrap();
    assert!(
        store
            .revoke_invitation(instance, mine, token.id, CLOCK)
            .is_err()
    );
    assert!(
        store
            .close_initial_enrollment(instance, mine, CLOCK)
            .is_err()
    );
    assert!(
        store
            .invitation_status(instance, mine, token.id, CLOCK)
            .is_err()
    );
    assert!(
        store
            .invitation_status(owner, mine, token.id, CLOCK)
            .is_ok()
    );
}

#[test]
fn invitation_ttl_and_expiry_facts_remain_accurate_through_explicit_close() {
    let mut f = fixture();
    let token = invite(&mut f, 2);
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    store
        .close_initial_enrollment(Actor::Admin(f.admin), f.workspace, DEADLINE)
        .unwrap();
    let status = store
        .invitation_status(Actor::Admin(f.admin), f.workspace, token.id, CLOCK)
        .unwrap();
    assert!(status.initial_closed && status.expired);
    let reason: String = f
        .connection
        .query_row(
            "SELECT event FROM ccp_enrollment_events ORDER BY id DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(reason, "deadline_close");
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    assert!(
        store
            .issue_individual_invitation(
                Actor::Admin(f.other_owner),
                IndividualInvitationRequest {
                    workspace_id: f.other_workspace,
                    role: DataRole::Read,
                    expires_at: i64::MAX,
                },
                CLOCK
            )
            .is_err()
    );
}

#[test]
fn revoked_invitation_and_expired_credentials_do_not_regain_authority_through_replay() {
    let mut f = fixture();
    let token = invite(&mut f, 2);
    let key = KeyPair::generate().unwrap();
    let request = csr_request(&key);
    let (ca, _) = authority();
    let first = redeem(&mut f, &token, &request, &ca, CLOCK).unwrap();
    assert!(redeem(&mut f, &token, &request, &ca, first.expires_at).is_err());
    AccessStore::new(&mut f.connection)
        .unwrap()
        .revoke_invitation(Actor::Admin(f.admin), f.workspace, token.id, CLOCK)
        .unwrap();
    assert!(redeem(&mut f, &token, &request, &ca, CLOCK).is_err());
    assert!(
        redeem(
            &mut f,
            &token,
            &csr_request(&KeyPair::generate().unwrap()),
            &ca,
            CLOCK
        )
        .is_err()
    );
    let other = AccessStore::new(&mut f.connection)
        .unwrap()
        .issue_individual_invitation(
            Actor::Admin(f.other_owner),
            IndividualInvitationRequest {
                workspace_id: f.other_workspace,
                role: DataRole::Read,
                expires_at: first.expires_at + 3600,
            },
            first.expires_at,
        )
        .unwrap();
    let mut reuse = csr_request(&key);
    reuse.existing_credential = Some(first.credential_id);
    assert!(redeem(&mut f, &other, &reuse, &ca, first.expires_at).is_err());
    assert_eq!(rows(&f.connection, "ccp_clients"), 1);
    assert_eq!(rows(&f.connection, "ccp_grants"), 1);
}

#[test]
fn existing_workspace_grant_is_not_rebound_or_rewritten_by_another_invitation() {
    let mut f = fixture();
    let token = invite(&mut f, 2);
    let key = KeyPair::generate().unwrap();
    let (ca, _) = authority();
    let first = redeem(&mut f, &token, &csr_request(&key), &ca, CLOCK).unwrap();
    AccessStore::new(&mut f.connection)
        .unwrap()
        .close_initial_enrollment(Actor::Admin(f.admin), f.workspace, CLOCK)
        .unwrap();
    let second = AccessStore::new(&mut f.connection)
        .unwrap()
        .issue_individual_invitation(
            Actor::Admin(f.admin),
            IndividualInvitationRequest {
                workspace_id: f.workspace,
                role: DataRole::Read,
                expires_at: DEADLINE,
            },
            CLOCK,
        )
        .unwrap();
    let mut reuse = csr_request(&key);
    reuse.existing_credential = Some(first.credential_id);
    assert!(redeem(&mut f, &second, &reuse, &ca, CLOCK).is_err());
    let status = AccessStore::new(&mut f.connection)
        .unwrap()
        .invitation_status(Actor::Admin(f.admin), f.workspace, second.id, CLOCK)
        .unwrap();
    assert_eq!(status.uses, 0);
    assert_eq!(rows(&f.connection, "ccp_grants"), 1);
    assert!(
        authorize(
            &f.connection,
            Actor::Credential(first.credential_id),
            Action::WriteContext,
            Some(f.workspace),
            CLOCK
        )
        .is_ok()
    );
}

#[test]
fn an_old_initial_invitation_cannot_restore_a_revoked_workspace_identity() {
    let mut f = fixture();
    let token = invite(&mut f, 3);
    let key = KeyPair::generate().unwrap();
    let (ca, _) = authority();
    let first = redeem(&mut f, &token, &csr_request(&key), &ca, CLOCK).unwrap();
    AccessStore::new(&mut f.connection)
        .unwrap()
        .revoke_grant(Actor::Admin(f.admin), first.grant_id, CLOCK)
        .unwrap();
    let mut reuse = csr_request(&key);
    reuse.existing_credential = Some(first.credential_id);
    assert!(redeem(&mut f, &token, &reuse, &ca, CLOCK).is_err());
    assert_eq!(rows(&f.connection, "ccp_grants"), 1);
    assert!(
        authorize(
            &f.connection,
            Actor::Credential(first.credential_id),
            Action::ReadContext,
            Some(f.workspace),
            CLOCK
        )
        .is_err()
    );
    let status = AccessStore::new(&mut f.connection)
        .unwrap()
        .invitation_status(Actor::Admin(f.owner), f.workspace, token.id, CLOCK)
        .unwrap();
    assert_eq!(status.uses, 1);
}

#[test]
fn ordinary_invitation_cannot_restore_revoked_membership_while_other_grants_remain_live() {
    let mut f = fixture();
    let token = invite(&mut f, 2);
    let key = KeyPair::generate().unwrap();
    let (ca, _) = authority();
    let first = redeem(&mut f, &token, &csr_request(&key), &ca, CLOCK).unwrap();
    let target = f.other_workspace;
    let second_token = AccessStore::new(&mut f.connection)
        .unwrap()
        .issue_individual_invitation(
            Actor::Admin(f.other_owner),
            IndividualInvitationRequest {
                workspace_id: target,
                role: DataRole::Read,
                expires_at: DEADLINE,
            },
            CLOCK,
        )
        .unwrap();
    let mut join = csr_request(&key);
    join.existing_credential = Some(first.credential_id);
    let second = redeem(&mut f, &second_token, &join, &ca, CLOCK).unwrap();
    AccessStore::new(&mut f.connection)
        .unwrap()
        .revoke_grant(Actor::Admin(f.other_owner), second.grant_id, CLOCK)
        .unwrap();
    let third_token = AccessStore::new(&mut f.connection)
        .unwrap()
        .issue_individual_invitation(
            Actor::Admin(f.other_owner),
            IndividualInvitationRequest {
                workspace_id: target,
                role: DataRole::ReadWrite,
                expires_at: DEADLINE,
            },
            CLOCK,
        )
        .unwrap();
    join.request_id = Uuid::new_v4();
    assert!(redeem(&mut f, &third_token, &join, &ca, CLOCK).is_err());
    assert!(
        authorize(
            &f.connection,
            Actor::Credential(first.credential_id),
            Action::WriteContext,
            Some(f.workspace),
            CLOCK
        )
        .is_ok()
    );
    assert!(
        authorize(
            &f.connection,
            Actor::Credential(first.credential_id),
            Action::ReadContext,
            Some(target),
            CLOCK
        )
        .is_err()
    );
    assert_eq!(rows(&f.connection, "ccp_grants"), 2);
}

#[test]
fn a_public_csr_and_claimed_id_cannot_replace_an_authenticated_client_connection() {
    let mut f = fixture();
    let token = invite(&mut f, 2);
    let key = KeyPair::generate().unwrap();
    let (ca, _) = authority();
    let first = redeem(&mut f, &token, &csr_request(&key), &ca, CLOCK).unwrap();
    let other = AccessStore::new(&mut f.connection)
        .unwrap()
        .issue_individual_invitation(
            Actor::Admin(f.other_owner),
            IndividualInvitationRequest {
                workspace_id: f.other_workspace,
                role: DataRole::Read,
                expires_at: DEADLINE,
            },
            CLOCK,
        )
        .unwrap();
    let mut request = csr_request(&key);
    request.existing_credential = Some(first.credential_id);
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    assert!(
        store
            .redeem_invitation(other.secret(), &request, &ca, CLOCK)
            .is_err()
    );
    assert!(
        store
            .admit_with_invitation(Actor::Admin(f.global), other.secret(), &request, &ca, CLOCK)
            .is_err()
    );
    assert!(
        store
            .admit_with_invitation(
                Actor::Credential(Uuid::new_v4()),
                other.secret(),
                &request,
                &ca,
                CLOCK
            )
            .is_err()
    );
    assert!(
        store
            .admit_with_invitation(
                Actor::Credential(first.credential_id),
                other.secret(),
                &request,
                &ca,
                CLOCK
            )
            .is_ok()
    );
}

fn mismatched_csr_envelope(signed_key: &KeyPair, hash_key: &KeyPair) -> String {
    let first = csr_request(signed_key).csr_pem;
    let second = csr_request(hash_key).csr_pem;
    // Corrupt only the second signature: its public key remains parseable.
    let mut bytes = second.as_bytes().to_vec();
    let end = second.find("-----END").unwrap();
    let position = bytes[..end]
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, byte)| byte.is_ascii_alphanumeric())
        .nth(10)
        .unwrap()
        .0;
    bytes[position] = if bytes[position] == b'A' { b'B' } else { b'A' };
    format!("x{first}{}", String::from_utf8(bytes).unwrap())
}

#[test]
fn split_pem_cannot_store_a_different_key_than_the_issued_certificate() {
    let mut f = fixture();
    let token = invite(&mut f, 1);
    let a = KeyPair::generate().unwrap();
    let b = KeyPair::generate().unwrap();
    let mut request = csr_request(&a);
    request.csr_pem = mismatched_csr_envelope(&a, &b);
    let (ca, _) = authority();
    let result = redeem(&mut f, &token, &request, &ca, CLOCK);
    let safe = match result {
        Err(_) => true,
        Ok(receipt) => {
            let (_, pem) =
                x509_parser::pem::parse_x509_pem(receipt.certificate_pem.as_bytes()).unwrap();
            let (_, cert) = x509_parser::parse_x509_certificate(&pem.contents).unwrap();
            let stored: Vec<u8> = f.connection.query_row(
                "SELECT public_key_hash FROM ccp_credential_certificates WHERE credential_id = ?1",
                [receipt.credential_id.to_string()], |row| row.get(0)).unwrap();
            stored == Sha256::digest(cert.public_key().raw).to_vec()
        }
    };
    assert!(
        safe,
        "issued key and credential registry key must be the same"
    );
}

#[test]
fn split_pem_cannot_satisfy_existing_credential_key_matching() {
    let mut f = fixture();
    let token = invite(&mut f, 1);
    let existing_key = KeyPair::generate().unwrap();
    let (ca, _) = authority();
    let first = redeem(&mut f, &token, &csr_request(&existing_key), &ca, CLOCK).unwrap();
    let second_token = AccessStore::new(&mut f.connection)
        .unwrap()
        .issue_individual_invitation(
            Actor::Admin(f.other_owner),
            IndividualInvitationRequest {
                workspace_id: f.other_workspace,
                role: DataRole::Read,
                expires_at: DEADLINE,
            },
            CLOCK,
        )
        .unwrap();
    let other_key = KeyPair::generate().unwrap();
    let mut request = csr_request(&other_key);
    request.csr_pem = mismatched_csr_envelope(&other_key, &existing_key);
    request.existing_credential = Some(first.credential_id);
    let result = redeem(&mut f, &second_token, &request, &ca, CLOCK);
    assert!(
        result.is_err(),
        "CSR signing key must match the authenticated credential key"
    );
}

#[test]
fn split_ca_pem_cannot_mix_one_certificate_with_another_private_key() {
    let a = KeyPair::generate().unwrap();
    let b = KeyPair::generate().unwrap();
    let mut params_a = CertificateParams::new(Vec::<String>::new()).unwrap();
    params_a.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params_a.distinguished_name.push(DnType::CommonName, "CA A");
    let mut params_b = params_a.clone();
    params_b.distinguished_name.push(DnType::CommonName, "CA B");
    let cert_a = params_a.self_signed(&a).unwrap();
    let cert_b = params_b.self_signed(&b).unwrap();
    let envelope = format!("x{}{}", cert_a.pem(), cert_b.pem());
    let result = IssuingAuthority::from_pem(&envelope, &b.serialize_pem());
    assert!(
        result.is_err(),
        "CA parameters and key validation must use the same certificate"
    );
}

#[test]
fn csr_envelopes_have_one_canonical_block_and_allow_ascii_whitespace() {
    let mut f = fixture();
    let token = invite(&mut f, 2);
    let key = KeyPair::generate().unwrap();
    let request = csr_request(&key);
    let (ca, _) = authority();
    let cases = [
        format!("junk\n{}", request.csr_pem),
        format!("{}junk", request.csr_pem),
        format!("{}{}", request.csr_pem, request.csr_pem),
        request
            .csr_pem
            .replace("CERTIFICATE REQUEST", "CERTIFICATE"),
        request.csr_pem.replace(
            "-----END CERTIFICATE REQUEST-----",
            "-----END CERTIFICATE-----",
        ),
    ];
    for pem in cases {
        let mut malformed = request.clone();
        malformed.csr_pem = pem;
        assert!(redeem(&mut f, &token, &malformed, &ca, CLOCK).is_err());
    }
    assert_eq!(rows(&f.connection, "ccp_clients"), 0);
    let mut whitespace = request.clone();
    whitespace.csr_pem = format!(" \t\r\n{}\r\n \t", request.csr_pem.replace('\n', "\r\n"));
    let receipt = redeem(&mut f, &token, &whitespace, &ca, CLOCK).unwrap();
    let (_, pem) = x509_parser::pem::parse_x509_pem(receipt.certificate_pem.as_bytes()).unwrap();
    let (_, cert) = x509_parser::parse_x509_certificate(&pem.contents).unwrap();
    let stored: Vec<u8> = f
        .connection
        .query_row(
            "SELECT public_key_hash FROM ccp_credential_certificates",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stored, Sha256::digest(cert.public_key().raw).to_vec());
}

#[test]
fn supported_csr_algorithms_bind_the_exact_issued_public_key() {
    for algorithm in [&rcgen::PKCS_ECDSA_P256_SHA256, &rcgen::PKCS_ED25519] {
        let mut f = fixture();
        let token = invite(&mut f, 1);
        let key = KeyPair::generate_for(algorithm).unwrap();
        let (ca, _) = authority();
        let receipt = redeem(&mut f, &token, &csr_request(&key), &ca, CLOCK).unwrap();
        let (_, pem) =
            x509_parser::pem::parse_x509_pem(receipt.certificate_pem.as_bytes()).unwrap();
        let (_, cert) = x509_parser::parse_x509_certificate(&pem.contents).unwrap();
        let stored: Vec<u8> = f
            .connection
            .query_row(
                "SELECT public_key_hash FROM ccp_credential_certificates",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored, Sha256::digest(cert.public_key().raw).to_vec());
        assert_eq!(cert.public_key().raw, key.public_key_der());
    }
}
