// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::redemption::{authority, csr_request, invite, redeem};
use super::*;
use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair};
use std::sync::Arc;

const CLOCK: i64 = 1_700_000_000;

fn request() -> RenewalRequest {
    let csr = csr_request(&KeyPair::generate().unwrap());
    RenewalRequest {
        request_id: csr.request_id,
        csr_pem: csr.csr_pem,
    }
}

fn setup(f: &mut Fixture) -> (IssuingAuthority, EnrollmentReceipt, RenewalRequest) {
    let (ca, _) = authority();
    let token = invite(f, 1);
    let initial = redeem(
        f,
        &token,
        &csr_request(&KeyPair::generate().unwrap()),
        &ca,
        CLOCK,
    )
    .unwrap();
    (ca, initial, request())
}

fn renew(
    f: &mut Fixture,
    credential: Uuid,
    request: &RenewalRequest,
    ca: &IssuingAuthority,
    now: i64,
) -> anyhow::Result<RenewalReceipt> {
    AccessStore::new(&mut f.connection)?.renew_credential(
        Actor::Credential(credential),
        request,
        ca,
        now,
    )
}

fn add_binding(f: &Fixture, initial: &EnrollmentReceipt, credential: Uuid) -> Uuid {
    let grant = Uuid::new_v4();
    f.connection
        .execute(
            "INSERT INTO ccp_grants(id, workspace_id, client_id, role) VALUES (?1, ?2, ?3, 'read')",
            params![
                grant.to_string(),
                f.other_workspace.to_string(),
                initial.client_id.to_string()
            ],
        )
        .unwrap();
    f.connection
        .execute(
            "INSERT INTO ccp_credential_grants VALUES (?1, ?2, ?3)",
            params![
                credential.to_string(),
                grant.to_string(),
                initial.client_id.to_string()
            ],
        )
        .unwrap();
    grant
}

fn can(f: &Fixture, credential: Uuid, action: Action, workspace: Uuid, now: i64) -> bool {
    authorize(
        &f.connection,
        Actor::Credential(credential),
        action,
        Some(workspace),
        now,
    )
    .is_ok()
}

#[test]
fn rotation_preserves_identity_and_current_scope_but_ends_old_authority() {
    let mut f = fixture();
    let (ca, old, request) = setup(&mut f);
    let second = add_binding(&f, &old, old.credential_id);
    let mut expected = vec![old.grant_id, second];
    expected.sort();
    let new = renew(&mut f, old.credential_id, &request, &ca, CLOCK + 60).unwrap();
    assert_eq!(new.client_id, old.client_id);
    assert_ne!(new.credential_id, old.credential_id);
    assert_eq!(new.grant_ids, expected);
    assert!(new.expires_at > old.expires_at);
    assert_ne!(new.certificate_pem, old.certificate_pem);
    assert!(can(
        &f,
        new.credential_id,
        Action::WriteContext,
        f.workspace,
        CLOCK + 60
    ));
    assert!(can(
        &f,
        new.credential_id,
        Action::ReadContext,
        f.other_workspace,
        CLOCK + 60
    ));
    assert!(!can(
        &f,
        new.credential_id,
        Action::WriteContext,
        f.other_workspace,
        CLOCK + 60
    ));
    assert!(!can(
        &f,
        old.credential_id,
        Action::ReadContext,
        f.workspace,
        CLOCK + 60
    ));
    assert!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .visible_workspaces(Actor::Credential(old.credential_id), CLOCK + 60)
            .is_err()
    );
    assert_eq!(rows(&f.connection, "ccp_clients"), 1);
    assert_eq!(rows(&f.connection, "ccp_credentials"), 2);
    assert_eq!(rows(&f.connection, "ccp_grants"), 2);
    let (_, pem) = x509_parser::pem::parse_x509_pem(new.certificate_pem.as_bytes()).unwrap();
    let (_, cert) = x509_parser::parse_x509_certificate(&pem.contents).unwrap();
    assert_eq!(cert.validity().not_after.timestamp(), new.expires_at);
    let recorded: Vec<u8> = f
        .connection
        .query_row(
            "SELECT public_key_hash FROM ccp_credential_certificates WHERE credential_id = ?1",
            [new.credential_id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    use sha2::{Digest, Sha256};
    assert_eq!(recorded, Sha256::digest(cert.public_key().raw).to_vec());
}

#[test]
fn retry_is_exact_and_conflicts_never_rotate_again() {
    let mut f = fixture();
    let (ca, old, mut req) = setup(&mut f);
    let new = renew(&mut f, old.credential_id, &req, &ca, CLOCK + 60).unwrap();
    assert_eq!(
        renew(&mut f, old.credential_id, &req, &ca, CLOCK + 61).unwrap(),
        new
    );
    let changed_key = request();
    assert!(
        renew(
            &mut f,
            old.credential_id,
            &RenewalRequest {
                request_id: req.request_id,
                ..changed_key
            },
            &ca,
            CLOCK + 61
        )
        .is_err()
    );
    req.request_id = Uuid::new_v4();
    assert!(renew(&mut f, old.credential_id, &req, &ca, CLOCK + 61).is_err());
    assert_eq!(rows(&f.connection, "ccp_credentials"), 2);
    assert_eq!(rows(&f.connection, "ccp_credential_renewals"), 1);
    let audits: i64 = f.connection.query_row("SELECT COUNT(*) FROM ccp_access_audit WHERE action = 'renew_credential' AND actor_id = ?1",
        [old.credential_id.to_string()], |row| row.get(0)).unwrap();
    assert_eq!(audits, 1);
}

#[test]
fn renewal_never_collects_another_credentials_grants() {
    let mut f = fixture();
    let (ca, old, req) = setup(&mut f);
    let other = Uuid::new_v4();
    f.connection
        .execute(
            "INSERT INTO ccp_credentials(id, client_id, expires_at) VALUES (?1, ?2, ?3)",
            params![other.to_string(), old.client_id.to_string(), old.expires_at],
        )
        .unwrap();
    add_binding(&f, &old, other);
    let new = renew(&mut f, old.credential_id, &req, &ca, CLOCK).unwrap();
    assert_eq!(new.grant_ids, vec![old.grant_id]);
    assert!(!can(
        &f,
        new.credential_id,
        Action::ReadContext,
        f.other_workspace,
        CLOCK
    ));
    assert!(can(
        &f,
        other,
        Action::ReadContext,
        f.other_workspace,
        CLOCK
    ));
}

#[test]
fn revoked_or_purged_bindings_are_excluded_and_archived_reads_can_renew() {
    for purge in [false, true] {
        let mut f = fixture();
        let (ca, old, req) = setup(&mut f);
        let second = add_binding(&f, &old, old.credential_id);
        if purge {
            f.connection
                .execute(
                    "UPDATE ccp_workspaces SET state = 'purged' WHERE id = ?1",
                    [f.other_workspace.to_string()],
                )
                .unwrap();
        } else {
            AccessStore::new(&mut f.connection)
                .unwrap()
                .revoke_grant(Actor::Admin(f.other_owner), second, CLOCK)
                .unwrap();
        }
        AccessStore::new(&mut f.connection)
            .unwrap()
            .set_archived(Actor::Admin(f.owner), f.workspace, true, CLOCK)
            .unwrap();
        let new = renew(&mut f, old.credential_id, &req, &ca, CLOCK).unwrap();
        assert_eq!(new.grant_ids, vec![old.grant_id]);
        assert!(can(
            &f,
            new.credential_id,
            Action::ReadContext,
            f.workspace,
            CLOCK
        ));
        assert!(!can(
            &f,
            new.credential_id,
            Action::WriteContext,
            f.workspace,
            CLOCK
        ));
        assert!(!can(
            &f,
            new.credential_id,
            Action::ReadContext,
            f.other_workspace,
            CLOCK
        ));
    }
}

#[test]
fn no_current_binding_means_no_renewal_and_no_partial_certificate() {
    let mut f = fixture();
    let (ca, old, req) = setup(&mut f);
    AccessStore::new(&mut f.connection)
        .unwrap()
        .revoke_grant(Actor::Admin(f.admin), old.grant_id, CLOCK)
        .unwrap();
    assert!(renew(&mut f, old.credential_id, &req, &ca, CLOCK).is_err());
    assert_eq!(rows(&f.connection, "ccp_credential_certificates"), 1);
    assert_eq!(rows(&f.connection, "ccp_credential_renewals"), 0);
}

#[test]
fn disabled_revoked_expired_unknown_and_nonclient_actors_cannot_renew() {
    for scenario in [
        "disabled",
        "revoked",
        "expired",
        "unknown",
        "admin",
        "negative",
        "uncertified",
    ] {
        let mut f = fixture();
        let (ca, old, req) = setup(&mut f);
        let mut actor = Actor::Credential(old.credential_id);
        let mut now = CLOCK;
        match scenario {
            "disabled" => AccessStore::new(&mut f.connection)
                .unwrap()
                .disable_client(Actor::Admin(f.global), old.client_id, CLOCK)
                .unwrap(),
            "revoked" => {
                f.connection
                    .execute(
                        "UPDATE ccp_credentials SET revoked_at = ?1 WHERE id = ?2",
                        params![CLOCK, old.credential_id.to_string()],
                    )
                    .unwrap();
            }
            "expired" => now = old.expires_at,
            "unknown" => actor = Actor::Credential(Uuid::new_v4()),
            "admin" => actor = Actor::Admin(f.global),
            "negative" => now = -1,
            "uncertified" => {
                f.connection
                    .execute("DELETE FROM ccp_credential_certificates", [])
                    .unwrap();
            }
            _ => panic!("unknown test scenario: {scenario}"),
        }
        assert!(
            AccessStore::new(&mut f.connection)
                .unwrap()
                .renew_credential(actor, &req, &ca, now)
                .is_err(),
            "{scenario}"
        );
        assert_eq!(rows(&f.connection, "ccp_credentials"), 1, "{scenario}");
        assert_eq!(
            rows(&f.connection, "ccp_credential_renewals"),
            0,
            "{scenario}"
        );
    }
}

#[test]
fn retry_does_not_bypass_revocation_disable_expiry_or_later_rotation() {
    for scenario in [
        "disabled",
        "old_revoked",
        "new_revoked",
        "grant",
        "expired",
        "rotated",
        "purged",
    ] {
        let mut f = fixture();
        let (ca, old, req) = setup(&mut f);
        let new = renew(&mut f, old.credential_id, &req, &ca, CLOCK).unwrap();
        let mut now = CLOCK;
        match scenario {
            "disabled" => AccessStore::new(&mut f.connection)
                .unwrap()
                .disable_client(Actor::Admin(f.global), old.client_id, CLOCK)
                .unwrap(),
            "old_revoked" | "new_revoked" => {
                let id = if scenario == "old_revoked" {
                    old.credential_id
                } else {
                    new.credential_id
                };
                f.connection
                    .execute(
                        "UPDATE ccp_credentials SET revoked_at = ?1 WHERE id = ?2",
                        params![CLOCK, id.to_string()],
                    )
                    .unwrap();
            }
            "grant" => AccessStore::new(&mut f.connection)
                .unwrap()
                .revoke_grant(Actor::Admin(f.admin), old.grant_id, CLOCK)
                .unwrap(),
            "expired" => now = old.expires_at,
            "rotated" => {
                renew(&mut f, new.credential_id, &request(), &ca, CLOCK).unwrap();
            }
            "purged" => {
                f.connection
                    .execute(
                        "UPDATE ccp_workspaces SET state = 'purged' WHERE id = ?1",
                        [f.workspace.to_string()],
                    )
                    .unwrap();
            }
            _ => panic!("unknown test scenario: {scenario}"),
        }
        assert!(
            renew(&mut f, old.credential_id, &req, &ca, now).is_err(),
            "{scenario}"
        );
    }
}

#[test]
fn partial_grant_revocation_changes_authority_without_changing_receipt() {
    let mut f = fixture();
    let (ca, old, req) = setup(&mut f);
    let second = add_binding(&f, &old, old.credential_id);
    let new = renew(&mut f, old.credential_id, &req, &ca, CLOCK).unwrap();
    AccessStore::new(&mut f.connection)
        .unwrap()
        .revoke_grant(Actor::Admin(f.other_owner), second, CLOCK)
        .unwrap();
    assert_eq!(
        renew(&mut f, old.credential_id, &req, &ca, CLOCK).unwrap(),
        new
    );
    assert!(!can(
        &f,
        new.credential_id,
        Action::ReadContext,
        f.other_workspace,
        CLOCK
    ));
    f.connection
        .execute(
            "UPDATE ccp_grants SET role = 'read' WHERE id = ?1",
            [old.grant_id.to_string()],
        )
        .unwrap();
    assert!(can(
        &f,
        new.credential_id,
        Action::ReadContext,
        f.workspace,
        CLOCK
    ));
    assert!(!can(
        &f,
        new.credential_id,
        Action::WriteContext,
        f.workspace,
        CLOCK
    ));
}

#[test]
fn audit_failure_rolls_back_rotation_and_preserves_old_credential() {
    let mut f = fixture();
    let (ca, old, req) = setup(&mut f);
    f.connection.execute_batch("CREATE TRIGGER no_renewal BEFORE INSERT ON ccp_access_audit WHEN NEW.action = 'renew_credential' BEGIN SELECT RAISE(ABORT, 'audit unavailable'); END;").unwrap();
    assert!(renew(&mut f, old.credential_id, &req, &ca, CLOCK).is_err());
    assert_eq!(rows(&f.connection, "ccp_credentials"), 1);
    assert_eq!(rows(&f.connection, "ccp_credential_certificates"), 1);
    assert_eq!(rows(&f.connection, "ccp_credential_grants"), 1);
    assert_eq!(rows(&f.connection, "ccp_credential_renewals"), 0);
    assert!(can(
        &f,
        old.credential_id,
        Action::WriteContext,
        f.workspace,
        CLOCK
    ));
    f.connection
        .execute_batch("DROP TRIGGER no_renewal;")
        .unwrap();
    assert!(renew(&mut f, old.credential_id, &req, &ca, CLOCK).is_ok());
}

#[test]
fn invalid_csr_or_issuing_ca_never_supersedes_working_credentials() {
    let mut f = fixture();
    let (ca, old, mut req) = setup(&mut f);
    req.csr_pem = "invalid".into();
    assert!(renew(&mut f, old.credential_id, &req, &ca, CLOCK).is_err());
    let key = KeyPair::generate().unwrap();
    let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.not_before = time::OffsetDateTime::from_unix_timestamp(CLOCK + 3600).unwrap();
    let cert = params.self_signed(&key).unwrap();
    let future = IssuingAuthority::from_pem(&cert.pem(), &key.serialize_pem()).unwrap();
    assert!(renew(&mut f, old.credential_id, &request(), &future, CLOCK).is_err());
    assert_eq!(rows(&f.connection, "ccp_credentials"), 1);
    assert!(can(
        &f,
        old.credential_id,
        Action::WriteContext,
        f.workspace,
        CLOCK
    ));
}

#[test]
fn superseded_identity_cannot_admit_itself_using_another_invitation() {
    let mut f = fixture();
    let (ca, _) = authority();
    let key = KeyPair::generate().unwrap();
    let mut old_req = csr_request(&key);
    let token = invite(&mut f, 1);
    let old = redeem(&mut f, &token, &old_req, &ca, CLOCK).unwrap();
    renew(&mut f, old.credential_id, &request(), &ca, CLOCK).unwrap();
    let next = AccessStore::new(&mut f.connection)
        .unwrap()
        .issue_individual_invitation(
            Actor::Admin(f.other_owner),
            IndividualInvitationRequest {
                workspace_id: f.other_workspace,
                role: DataRole::Read,
                expires_at: CLOCK + 3600,
            },
            CLOCK,
        )
        .unwrap();
    old_req.existing_credential = Some(old.credential_id);
    assert!(redeem(&mut f, &next, &old_req, &ca, CLOCK).is_err());
    old_req.existing_credential = None;
    assert!(redeem(&mut f, &token, &old_req, &ca, CLOCK).is_err());
    assert_eq!(rows(&f.connection, "ccp_grants"), 1);
}

#[test]
fn schema8_upgrade_and_reopening_preserve_identity_and_exact_retry() {
    let path = std::env::temp_dir().join(format!("ccp-renewal-upgrade-{}.db", Uuid::new_v4()));
    crate::init::init_sqlite(&path).unwrap();
    let mut f = fixture_with_connection(Connection::open(&path).unwrap());
    let (ca, old, req) = setup(&mut f);
    f.connection
        .execute_batch("DROP TABLE ccp_renewal_grants; DROP TABLE ccp_credential_renewals; UPDATE schema_version SET version = 8;")
        .unwrap();
    drop(f);
    crate::init::init_sqlite(&path).unwrap();
    let mut connection = Connection::open(&path).unwrap();
    let new = AccessStore::new(&mut connection)
        .unwrap()
        .renew_credential(Actor::Credential(old.credential_id), &req, &ca, CLOCK)
        .unwrap();
    drop(connection);
    crate::init::init_sqlite(&path).unwrap();
    let mut connection = Connection::open(&path).unwrap();
    assert_eq!(
        AccessStore::new(&mut connection)
            .unwrap()
            .renew_credential(Actor::Credential(old.credential_id), &req, &ca, CLOCK)
            .unwrap(),
        new
    );
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        0
    );
    assert_eq!(
        connection
            .query_row("SELECT MAX(version) FROM schema_version", [], |row| row
                .get::<_, u32>(0))
            .unwrap(),
        crate::init::SCHEMA_VERSION
    );
    drop(connection);
    fs::remove_file(path).unwrap();
}

fn race(same_request: bool) {
    let path = std::env::temp_dir().join(format!("ccp-renewal-race-{}.db", Uuid::new_v4()));
    crate::init::init_sqlite(&path).unwrap();
    let mut f = fixture_with_connection(Connection::open(&path).unwrap());
    let (ca, old, req) = setup(&mut f);
    let authority = Arc::new(ca);
    drop(f);
    let barrier = Arc::new(std::sync::Barrier::new(8));
    let mut workers = Vec::new();
    for _ in 0..8 {
        let path = path.clone();
        let ca = authority.clone();
        let barrier = barrier.clone();
        let req = if same_request { req.clone() } else { request() };
        workers.push(std::thread::spawn(move || {
            let mut connection = Connection::open(path).unwrap();
            connection
                .busy_timeout(std::time::Duration::from_secs(20))
                .unwrap();
            barrier.wait();
            AccessStore::new(&mut connection).unwrap().renew_credential(
                Actor::Credential(old.credential_id),
                &req,
                &ca,
                CLOCK,
            )
        }));
    }
    let outcomes: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();
    let successes: Vec<_> = outcomes
        .iter()
        .filter_map(|result| result.as_ref().ok())
        .collect();
    assert_eq!(successes.len(), if same_request { 8 } else { 1 });
    for success in &successes {
        assert_eq!(success, &successes[0]);
    }
    for error in outcomes.iter().filter_map(|result| result.as_ref().err()) {
        assert!(
            error.to_string().contains("retry does not match"),
            "{error:#}"
        );
    }
    let connection = Connection::open(&path).unwrap();
    assert_eq!(rows(&connection, "ccp_credentials"), 2);
    assert_eq!(rows(&connection, "ccp_credential_renewals"), 1);
    drop(connection);
    fs::remove_file(path).unwrap();
}

#[test]
fn concurrent_identical_retries_return_one_rotation() {
    race(true);
}

#[test]
fn concurrent_different_requests_cannot_rotate_twice() {
    race(false);
}

#[test]
fn later_admission_does_not_change_a_captured_receipt_or_keep_it_alive() {
    let mut f = fixture();
    let (ca, old, req) = setup(&mut f);
    let new = renew(&mut f, old.credential_id, &req, &ca, CLOCK).unwrap();
    add_binding(&f, &old, new.credential_id);
    assert_eq!(
        renew(&mut f, old.credential_id, &req, &ca, CLOCK).unwrap(),
        new
    );
    AccessStore::new(&mut f.connection)
        .unwrap()
        .revoke_grant(Actor::Admin(f.admin), old.grant_id, CLOCK)
        .unwrap();
    assert!(can(
        &f,
        new.credential_id,
        Action::ReadContext,
        f.other_workspace,
        CLOCK
    ));
    assert!(renew(&mut f, old.credential_id, &req, &ca, CLOCK).is_err());
}
