// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::redemption::{authority, csr_request, invite, redeem};
use super::*;
use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair};
use std::sync::Arc;

const CLOCK: i64 = 1_700_000_000;

struct Scenario {
    f: Fixture,
    ca: IssuingAuthority,
    old: EnrollmentReceipt,
    grant_b: Uuid,
    original_token: IssuedInvitation,
    original_request: RedemptionRequest,
    original_key: KeyPair,
}

fn setup(f: Fixture) -> Scenario {
    let mut f = f;
    let (ca, _) = authority();
    let original_token = invite(&mut f, 1);
    let original_key = KeyPair::generate().unwrap();
    let original_request = csr_request(&original_key);
    let old = redeem(&mut f, &original_token, &original_request, &ca, CLOCK).unwrap();
    let grant_b = AccessStore::new(&mut f.connection)
        .unwrap()
        .admit_existing_client(
            Actor::Admin(f.other_owner),
            Admission {
                workspace_id: f.other_workspace,
                presented_credential: old.credential_id,
                role: DataRole::Read,
            },
            CLOCK,
        )
        .unwrap();
    Scenario {
        f,
        ca,
        old,
        grant_b,
        original_token,
        original_request,
        original_key,
    }
}

fn request() -> RecoveryRedemptionRequest {
    let csr = csr_request(&KeyPair::generate().unwrap());
    RecoveryRedemptionRequest {
        request_id: csr.request_id,
        csr_pem: csr.csr_pem,
    }
}

fn issue(
    s: &mut Scenario,
    actor: Actor,
    scope: RecoveryScope,
    now: i64,
) -> IssuedRecoveryInvitation {
    AccessStore::new(&mut s.f.connection)
        .unwrap()
        .issue_recovery_invitation(
            actor,
            RecoveryInvitationRequest {
                client_id: s.old.client_id,
                scope,
                expires_at: now + 3600,
            },
            now,
        )
        .unwrap()
}

fn scoped(s: &mut Scenario) -> IssuedRecoveryInvitation {
    issue(
        s,
        Actor::Admin(s.f.admin),
        RecoveryScope::Workspace(s.f.workspace),
        CLOCK,
    )
}

fn recover(
    s: &mut Scenario,
    token: &IssuedRecoveryInvitation,
    req: &RecoveryRedemptionRequest,
    now: i64,
) -> anyhow::Result<RecoveryReceipt> {
    AccessStore::new(&mut s.f.connection)?.redeem_recovery_invitation(
        token.secret(),
        req,
        &s.ca,
        now,
    )
}

fn can(s: &Scenario, credential: Uuid, action: Action, workspace: Uuid, now: i64) -> bool {
    authorize(
        &s.f.connection,
        Actor::Credential(credential),
        action,
        Some(workspace),
        now,
    )
    .is_ok()
}

fn key(
    s: &mut Scenario,
    principal: Uuid,
    scope: AdminKeyScope,
    expires_at: Option<i64>,
) -> IssuedAdminKey {
    let actor = match scope {
        AdminKeyScope::Instance => s.f.global,
        AdminKeyScope::Workspace(_) => principal,
    };
    AccessStore::new(&mut s.f.connection)
        .unwrap()
        .issue_admin_key(
            Actor::Admin(actor),
            &AdminKeyRequest {
                principal_id: principal,
                scope,
                name: "recovery admin".into(),
                expires_at,
            },
            CLOCK,
        )
        .unwrap()
}

#[test]
fn scoped_recovery_preserves_identity_and_other_workspace_access() {
    let mut s = setup(fixture());
    let token = scoped(&mut s);
    let new = recover(&mut s, &token, &request(), CLOCK).unwrap();
    assert_eq!(new.client_id, s.old.client_id);
    assert_eq!(new.grant_ids, vec![s.old.grant_id]);
    assert_ne!(new.credential_id, s.old.credential_id);
    assert!(can(
        &s,
        new.credential_id,
        Action::WriteContext,
        s.f.workspace,
        CLOCK
    ));
    assert!(!can(
        &s,
        new.credential_id,
        Action::ReadContext,
        s.f.other_workspace,
        CLOCK
    ));
    assert!(!can(
        &s,
        s.old.credential_id,
        Action::ReadContext,
        s.f.workspace,
        CLOCK
    ));
    assert!(can(
        &s,
        s.old.credential_id,
        Action::ReadContext,
        s.f.other_workspace,
        CLOCK
    ));
    assert_eq!(rows(&s.f.connection, "ccp_clients"), 1);
    assert_eq!(rows(&s.f.connection, "ccp_grants"), 2);
    assert_eq!(rows(&s.f.connection, "ccp_credentials"), 2);
    assert_eq!(rows(&s.f.connection, "ccp_binding_revocations"), 1);
    assert_eq!(rows(&s.f.connection, "ccp_credential_grants"), 3);
    assert_eq!(rows(&s.f.connection, "ccp_invitations"), 1);
    let closed: bool =
        s.f.connection
            .query_row(
                "SELECT closed_at IS NOT NULL FROM ccp_initial_enrollment WHERE workspace_id = ?1",
                [s.f.workspace.to_string()],
                |row| row.get(0),
            )
            .unwrap();
    assert!(closed);
    let actor: (String, String, String) = s.f.connection.query_row(
        "SELECT actor_kind, actor_id, target_id FROM ccp_access_audit WHERE action = 'redeem_client_recovery'", [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).unwrap();
    assert_eq!(
        actor,
        (
            "credential".into(),
            new.credential_id.to_string(),
            token.id.to_string()
        )
    );
    let stored: Vec<u8> =
        s.f.connection
            .query_row(
                "SELECT token_hash FROM ccp_recovery_invitations",
                [],
                |row| row.get(0),
            )
            .unwrap();
    use sha2::{Digest, Sha256};
    assert_eq!(stored, Sha256::digest(token.secret().as_bytes()).to_vec());
    assert_ne!(stored, token.secret().as_bytes());
    let (_, pem) = x509_parser::pem::parse_x509_pem(new.certificate_pem.as_bytes()).unwrap();
    let (_, cert) = x509_parser::parse_x509_certificate(&pem.contents).unwrap();
    assert_eq!(cert.validity().not_after.timestamp(), new.expires_at);
    let key: Vec<u8> =
        s.f.connection
            .query_row(
                "SELECT public_key_hash FROM ccp_credential_certificates WHERE credential_id = ?1",
                [new.credential_id.to_string()],
                |row| row.get(0),
            )
            .unwrap();
    assert_eq!(key, Sha256::digest(cert.public_key().raw).to_vec());
}

#[test]
fn global_recovery_works_after_expiry_and_preserves_current_data_roles() {
    let mut s = setup(fixture());
    let now = s.old.expires_at;
    let global = s.f.global;
    let token = issue(&mut s, Actor::Admin(global), RecoveryScope::Instance, now);
    let new = recover(&mut s, &token, &request(), now).unwrap();
    let mut expected = vec![s.old.grant_id, s.grant_b];
    expected.sort();
    assert_eq!(new.client_id, s.old.client_id);
    assert_eq!(new.grant_ids, expected);
    assert!(can(
        &s,
        new.credential_id,
        Action::WriteContext,
        s.f.workspace,
        now
    ));
    assert!(can(
        &s,
        new.credential_id,
        Action::ReadContext,
        s.f.other_workspace,
        now
    ));
    assert!(!can(
        &s,
        new.credential_id,
        Action::WriteContext,
        s.f.other_workspace,
        now
    ));
    assert!(!can(
        &s,
        s.old.credential_id,
        Action::ReadContext,
        s.f.workspace,
        now
    ));
    assert!(!can(
        &s,
        s.old.credential_id,
        Action::ReadContext,
        s.f.other_workspace,
        now
    ));
    assert_eq!(rows(&s.f.connection, "ccp_binding_revocations"), 2);
    assert_eq!(rows(&s.f.connection, "ccp_clients"), 1);
}

#[test]
fn recovery_filters_grants_revoked_or_purged_after_issuance() {
    for purge in [false, true] {
        let mut s = setup(fixture());
        let global = s.f.global;
        let token = issue(&mut s, Actor::Admin(global), RecoveryScope::Instance, CLOCK);
        if purge {
            s.f.connection
                .execute(
                    "UPDATE ccp_workspaces SET state = 'purged' WHERE id = ?1",
                    [s.f.workspace.to_string()],
                )
                .unwrap();
        } else {
            AccessStore::new(&mut s.f.connection)
                .unwrap()
                .revoke_grant(Actor::Admin(s.f.admin), s.old.grant_id, CLOCK)
                .unwrap();
        }
        s.f.connection
            .execute(
                "UPDATE ccp_grants SET role = 'read_write' WHERE id = ?1",
                [s.grant_b.to_string()],
            )
            .unwrap();
        let new = recover(&mut s, &token, &request(), CLOCK).unwrap();
        assert_eq!(new.grant_ids, vec![s.grant_b]);
        assert!(!can(
            &s,
            new.credential_id,
            Action::ReadContext,
            s.f.workspace,
            CLOCK
        ));
        assert!(can(
            &s,
            new.credential_id,
            Action::WriteContext,
            s.f.other_workspace,
            CLOCK
        ));
        assert_eq!(rows(&s.f.connection, "ccp_grants"), 2);
    }
}

#[test]
fn recovery_does_not_pick_up_new_grants_added_after_issuance() {
    let mut s = setup(fixture());
    AccessStore::new(&mut s.f.connection)
        .unwrap()
        .revoke_grant(Actor::Admin(s.f.other_owner), s.grant_b, CLOCK)
        .unwrap();
    let global = s.f.global;
    let token = issue(&mut s, Actor::Admin(global), RecoveryScope::Instance, CLOCK);
    let later = AccessStore::new(&mut s.f.connection)
        .unwrap()
        .admit_existing_client(
            Actor::Admin(s.f.other_owner),
            Admission {
                workspace_id: s.f.other_workspace,
                presented_credential: s.old.credential_id,
                role: DataRole::ReadWrite,
            },
            CLOCK,
        )
        .unwrap();
    let new = recover(&mut s, &token, &request(), CLOCK).unwrap();
    assert_eq!(new.grant_ids, vec![s.old.grant_id]);
    assert!(!new.grant_ids.contains(&later));
    assert!(!can(
        &s,
        new.credential_id,
        Action::ReadContext,
        s.f.other_workspace,
        CLOCK
    ));
    assert!(can(
        &s,
        s.old.credential_id,
        Action::WriteContext,
        s.f.other_workspace,
        CLOCK
    ));
}

#[test]
fn scoped_recovery_is_available_while_archived_without_admission_or_writes() {
    let mut s = setup(fixture());
    AccessStore::new(&mut s.f.connection)
        .unwrap()
        .set_archived(Actor::Admin(s.f.owner), s.f.workspace, true, CLOCK)
        .unwrap();
    let token = scoped(&mut s);
    let new = recover(&mut s, &token, &request(), CLOCK).unwrap();
    assert!(can(
        &s,
        new.credential_id,
        Action::ReadContext,
        s.f.workspace,
        CLOCK
    ));
    assert!(!can(
        &s,
        new.credential_id,
        Action::WriteContext,
        s.f.workspace,
        CLOCK
    ));
    assert!(
        AccessStore::new(&mut s.f.connection)
            .unwrap()
            .issue_individual_invitation(
                Actor::Admin(s.f.admin),
                IndividualInvitationRequest {
                    workspace_id: s.f.workspace,
                    role: DataRole::Read,
                    expires_at: CLOCK + 3600,
                },
                CLOCK
            )
            .is_err()
    );
    assert_eq!(rows(&s.f.connection, "ccp_grants"), 2);
}

#[test]
fn scoped_and_instance_keys_cannot_borrow_each_others_recovery_powers() {
    let mut s = setup(fixture());
    let global = s.f.global;
    let mine = AccessStore::new(&mut s.f.connection)
        .unwrap()
        .create_workspace(Actor::Admin(global), "mine", global, CLOCK)
        .unwrap();
    AccessStore::new(&mut s.f.connection)
        .unwrap()
        .admit_existing_client(
            Actor::Admin(global),
            Admission {
                workspace_id: mine,
                presented_credential: s.old.credential_id,
                role: DataRole::Read,
            },
            CLOCK,
        )
        .unwrap();
    let instance = key(&mut s, global, AdminKeyScope::Instance, None);
    let workspace = key(&mut s, global, AdminKeyScope::Workspace(mine), None);
    for (actor, scope) in [
        (Actor::AdminKey(instance.id), RecoveryScope::Workspace(mine)),
        (Actor::AdminKey(workspace.id), RecoveryScope::Instance),
        (
            Actor::AdminKey(workspace.id),
            RecoveryScope::Workspace(s.f.workspace),
        ),
        (Actor::Admin(s.f.admin), RecoveryScope::Instance),
        (
            Actor::Admin(s.f.other_owner),
            RecoveryScope::Workspace(s.f.workspace),
        ),
        (
            Actor::Credential(s.old.credential_id),
            RecoveryScope::Workspace(s.f.workspace),
        ),
    ] {
        assert!(
            AccessStore::new(&mut s.f.connection)
                .unwrap()
                .issue_recovery_invitation(
                    actor,
                    RecoveryInvitationRequest {
                        client_id: s.old.client_id,
                        scope,
                        expires_at: CLOCK + 3600,
                    },
                    CLOCK
                )
                .is_err()
        );
    }
    let token = issue(
        &mut s,
        Actor::AdminKey(workspace.id),
        RecoveryScope::Workspace(mine),
        CLOCK,
    );
    assert!(
        AccessStore::new(&mut s.f.connection)
            .unwrap()
            .revoke_recovery_invitation(Actor::AdminKey(instance.id), token.id, CLOCK)
            .is_err()
    );
    assert!(
        AccessStore::new(&mut s.f.connection)
            .unwrap()
            .recovery_invitation_status(Actor::Admin(s.f.other_owner), token.id, CLOCK)
            .is_err()
    );
    let new = recover(&mut s, &token, &request(), CLOCK).unwrap();
    assert!(can(&s, new.credential_id, Action::ReadContext, mine, CLOCK));
    assert!(!can(
        &s,
        new.credential_id,
        Action::ReadContext,
        s.f.workspace,
        CLOCK
    ));
    let global_token = issue(
        &mut s,
        Actor::AdminKey(instance.id),
        RecoveryScope::Instance,
        CLOCK,
    );
    let new_global = recover(&mut s, &global_token, &request(), CLOCK).unwrap();
    assert_eq!(new_global.grant_ids.len(), 3);
    let recorded: String = s.f.connection.query_row("SELECT admin_key_id FROM ccp_access_audit WHERE action = 'issue_client_recovery' AND target_id = ?1",
        [global_token.id.to_string()], |row| row.get(0)).unwrap();
    assert_eq!(recorded, instance.id.to_string());
}

#[test]
fn issuer_role_loss_key_revocation_and_expiry_block_pending_recovery() {
    for scenario in [
        "role",
        "key_expired",
        "key_revoked",
        "principal_disabled",
        "global_role",
    ] {
        let mut s = setup(fixture());
        let owner = s.f.owner;
        let admin = s.f.admin;
        let global = s.f.global;
        let workspace_id = s.f.workspace;
        let (actor, scope, key_id) = if scenario == "global_role" {
            (Actor::Admin(global), RecoveryScope::Instance, None)
        } else if scenario.starts_with("key_") {
            let issued = key(
                &mut s,
                owner,
                AdminKeyScope::Workspace(workspace_id),
                Some(CLOCK + 60),
            );
            (
                Actor::AdminKey(issued.id),
                RecoveryScope::Workspace(s.f.workspace),
                Some(issued.id),
            )
        } else {
            (
                Actor::Admin(admin),
                RecoveryScope::Workspace(s.f.workspace),
                None,
            )
        };
        let token = issue(&mut s, actor, scope, CLOCK);
        match scenario {
            "role" => AccessStore::new(&mut s.f.connection)
                .unwrap()
                .revoke_workspace_admin(Actor::Admin(owner), s.f.workspace, admin, CLOCK)
                .unwrap(),
            "key_revoked" => AccessStore::new(&mut s.f.connection)
                .unwrap()
                .revoke_admin_key(
                    Actor::Admin(owner),
                    AdminKeyScope::Workspace(s.f.workspace),
                    key_id.unwrap(),
                    CLOCK,
                )
                .unwrap(),
            "principal_disabled" => {
                s.f.connection
                    .execute(
                        "UPDATE ccp_admin_principals SET enabled = 0 WHERE id = ?1",
                        [admin.to_string()],
                    )
                    .unwrap();
            }
            "global_role" => {
                s.f.connection
                    .execute(
                        "DELETE FROM ccp_server_admins WHERE admin_id = ?1",
                        [global.to_string()],
                    )
                    .unwrap();
            }
            "key_expired" => {}
            _ => panic!("unknown scenario {scenario}"),
        }
        let now = if scenario == "key_expired" {
            CLOCK + 60
        } else {
            CLOCK
        };
        assert!(
            recover(&mut s, &token, &request(), now).is_err(),
            "{scenario}"
        );
        assert_eq!(rows(&s.f.connection, "ccp_credentials"), 1, "{scenario}");
        assert_eq!(
            rows(&s.f.connection, "ccp_binding_revocations"),
            0,
            "{scenario}"
        );
        if scenario == "role" {
            let status = AccessStore::new(&mut s.f.connection)
                .unwrap()
                .recovery_invitation_status(Actor::Admin(owner), token.id, now)
                .unwrap();
            assert!(!status.issuer_authorized);
        }
    }
}

#[test]
fn invalid_lifetimes_targets_and_empty_scopes_issue_no_recovery() {
    let mut s = setup(fixture());
    for (expires_at, now) in [
        (CLOCK, CLOCK),
        (CLOCK - 1, CLOCK),
        (CLOCK + 7 * 86400 + 1, CLOCK),
        (i64::MAX, -1),
    ] {
        assert!(
            AccessStore::new(&mut s.f.connection)
                .unwrap()
                .issue_recovery_invitation(
                    Actor::Admin(s.f.global),
                    RecoveryInvitationRequest {
                        client_id: s.old.client_id,
                        scope: RecoveryScope::Instance,
                        expires_at,
                    },
                    now
                )
                .is_err()
        );
    }
    assert!(
        AccessStore::new(&mut s.f.connection)
            .unwrap()
            .issue_recovery_invitation(
                Actor::Admin(s.f.global),
                RecoveryInvitationRequest {
                    client_id: Uuid::new_v4(),
                    scope: RecoveryScope::Instance,
                    expires_at: CLOCK + 3600,
                },
                CLOCK
            )
            .is_err()
    );
    AccessStore::new(&mut s.f.connection)
        .unwrap()
        .revoke_grant(Actor::Admin(s.f.admin), s.old.grant_id, CLOCK)
        .unwrap();
    assert!(
        AccessStore::new(&mut s.f.connection)
            .unwrap()
            .issue_recovery_invitation(
                Actor::Admin(s.f.admin),
                RecoveryInvitationRequest {
                    client_id: s.old.client_id,
                    scope: RecoveryScope::Workspace(s.f.workspace),
                    expires_at: CLOCK + 3600,
                },
                CLOCK
            )
            .is_err()
    );
    AccessStore::new(&mut s.f.connection)
        .unwrap()
        .disable_client(Actor::Admin(s.f.global), s.old.client_id, CLOCK)
        .unwrap();
    assert!(
        AccessStore::new(&mut s.f.connection)
            .unwrap()
            .issue_recovery_invitation(
                Actor::Admin(s.f.global),
                RecoveryInvitationRequest {
                    client_id: s.old.client_id,
                    scope: RecoveryScope::Instance,
                    expires_at: CLOCK + 3600,
                },
                CLOCK
            )
            .is_err()
    );
    assert_eq!(rows(&s.f.connection, "ccp_recovery_invitations"), 0);
    assert_eq!(rows(&s.f.connection, "ccp_recovery_grants"), 0);
}

#[test]
fn observed_recovery_expiry_stays_closed_after_clock_rollback() {
    let mut s = setup(fixture());
    let token = scoped(&mut s);
    let req = request();
    assert!(recover(&mut s, &token, &req, CLOCK + 3600).is_err());
    assert!(recover(&mut s, &token, &req, CLOCK).is_err());
    let status = AccessStore::new(&mut s.f.connection)
        .unwrap()
        .recovery_invitation_status(Actor::Admin(s.f.owner), token.id, CLOCK)
        .unwrap();
    assert!(status.expired);
    assert!(!status.redeemed);
    assert_eq!(status.scope, RecoveryScope::Workspace(s.f.workspace));
    assert_eq!(status.client_id, s.old.client_id);
    assert_eq!(status.expires_at, CLOCK + 3600);
    assert_eq!(rows(&s.f.connection, "ccp_credentials"), 1);
    assert!(can(
        &s,
        s.old.credential_id,
        Action::WriteContext,
        s.f.workspace,
        CLOCK
    ));
}

#[test]
fn exact_recovery_retry_survives_deadline_but_never_mints_again() {
    let mut s = setup(fixture());
    let token = scoped(&mut s);
    let req = request();
    let new = recover(&mut s, &token, &req, CLOCK).unwrap();
    assert_eq!(recover(&mut s, &token, &req, CLOCK + 3600).unwrap(), new);
    let status = AccessStore::new(&mut s.f.connection)
        .unwrap()
        .recovery_invitation_status(Actor::Admin(s.f.owner), token.id, CLOCK + 3600)
        .unwrap();
    assert!(status.redeemed);
    assert!(status.expired);
    let changed = request();
    assert!(
        recover(
            &mut s,
            &token,
            &RecoveryRedemptionRequest {
                request_id: req.request_id,
                ..changed
            },
            CLOCK
        )
        .is_err()
    );
    assert!(
        recover(
            &mut s,
            &token,
            &RecoveryRedemptionRequest {
                request_id: Uuid::new_v4(),
                csr_pem: req.csr_pem.clone()
            },
            CLOCK
        )
        .is_err()
    );
    assert_eq!(rows(&s.f.connection, "ccp_recovery_receipts"), 1);
    assert_eq!(rows(&s.f.connection, "ccp_credentials"), 2);
    assert_eq!(rows(&s.f.connection, "ccp_binding_revocations"), 1);
    AccessStore::new(&mut s.f.connection)
        .unwrap()
        .revoke_recovery_invitation(Actor::Admin(s.f.admin), token.id, CLOCK)
        .unwrap();
    assert!(recover(&mut s, &token, &req, CLOCK).is_err());
    assert!(can(
        &s,
        new.credential_id,
        Action::ReadContext,
        s.f.workspace,
        CLOCK
    ));
}

#[test]
fn revoked_recovery_token_never_issues_a_certificate() {
    let mut s = setup(fixture());
    let token = scoped(&mut s);
    AccessStore::new(&mut s.f.connection)
        .unwrap()
        .revoke_recovery_invitation(Actor::Admin(s.f.owner), token.id, CLOCK)
        .unwrap();
    assert!(recover(&mut s, &token, &request(), CLOCK).is_err());
    let status = AccessStore::new(&mut s.f.connection)
        .unwrap()
        .recovery_invitation_status(Actor::Admin(s.f.owner), token.id, CLOCK)
        .unwrap();
    assert!(status.revoked);
    assert!(!status.redeemed);
    assert_eq!(rows(&s.f.connection, "ccp_credentials"), 1);
    assert!(
        AccessStore::new(&mut s.f.connection)
            .unwrap()
            .revoke_recovery_invitation(Actor::Admin(s.f.owner), token.id, CLOCK)
            .is_err()
    );
}

#[test]
fn global_disable_blocks_pending_recovery_and_receipt_retrieval() {
    for already_redeemed in [false, true] {
        let mut s = setup(fixture());
        let token = scoped(&mut s);
        let req = request();
        if already_redeemed {
            recover(&mut s, &token, &req, CLOCK).unwrap();
        }
        AccessStore::new(&mut s.f.connection)
            .unwrap()
            .disable_client(Actor::Admin(s.f.global), s.old.client_id, CLOCK)
            .unwrap();
        assert!(recover(&mut s, &token, &req, CLOCK).is_err());
        assert_eq!(
            rows(&s.f.connection, "ccp_credentials"),
            if already_redeemed { 2 } else { 1 }
        );
    }
}

#[test]
fn no_remaining_captured_access_cannot_be_recovered_from_old_token() {
    let mut s = setup(fixture());
    let token = scoped(&mut s);
    AccessStore::new(&mut s.f.connection)
        .unwrap()
        .revoke_grant(Actor::Admin(s.f.admin), s.old.grant_id, CLOCK)
        .unwrap();
    assert!(recover(&mut s, &token, &request(), CLOCK).is_err());
    assert_eq!(rows(&s.f.connection, "ccp_credentials"), 1);
    assert_eq!(rows(&s.f.connection, "ccp_binding_revocations"), 0);
    assert!(can(
        &s,
        s.old.credential_id,
        Action::ReadContext,
        s.f.other_workspace,
        CLOCK
    ));
    assert_eq!(rows(&s.f.connection, "ccp_grants"), 2);
}

#[test]
fn recovery_receipt_scope_stays_fixed_when_other_bindings_are_added() {
    let mut s = setup(fixture());
    let token = scoped(&mut s);
    let req = request();
    let new = recover(&mut s, &token, &req, CLOCK).unwrap();
    s.f.connection
        .execute(
            "INSERT INTO ccp_credential_grants VALUES (?1, ?2, ?3)",
            params![
                new.credential_id.to_string(),
                s.grant_b.to_string(),
                s.old.client_id.to_string()
            ],
        )
        .unwrap();
    assert_eq!(recover(&mut s, &token, &req, CLOCK).unwrap(), new);
    AccessStore::new(&mut s.f.connection)
        .unwrap()
        .revoke_grant(Actor::Admin(s.f.admin), s.old.grant_id, CLOCK)
        .unwrap();
    assert!(can(
        &s,
        new.credential_id,
        Action::ReadContext,
        s.f.other_workspace,
        CLOCK
    ));
    assert!(recover(&mut s, &token, &req, CLOCK).is_err());
}

#[test]
fn renewal_after_scoped_recovery_cannot_copy_a_revoked_binding() {
    let mut s = setup(fixture());
    let token = scoped(&mut s);
    let recovered = recover(&mut s, &token, &request(), CLOCK).unwrap();
    let csr = csr_request(&KeyPair::generate().unwrap());
    let req = RenewalRequest {
        request_id: csr.request_id,
        csr_pem: csr.csr_pem,
    };
    let renewed = AccessStore::new(&mut s.f.connection)
        .unwrap()
        .renew_credential(Actor::Credential(s.old.credential_id), &req, &s.ca, CLOCK)
        .unwrap();
    assert_eq!(renewed.grant_ids, vec![s.grant_b]);
    assert!(!can(
        &s,
        renewed.credential_id,
        Action::ReadContext,
        s.f.workspace,
        CLOCK
    ));
    assert!(can(
        &s,
        renewed.credential_id,
        Action::ReadContext,
        s.f.other_workspace,
        CLOCK
    ));
    assert!(can(
        &s,
        recovered.credential_id,
        Action::WriteContext,
        s.f.workspace,
        CLOCK
    ));
}

#[test]
fn recovered_binding_invalidates_old_admission_receipt_and_is_not_copied_back() {
    let mut s = setup(fixture());
    let token = scoped(&mut s);
    recover(&mut s, &token, &request(), CLOCK).unwrap();
    assert!(
        redeem(
            &mut s.f,
            &s.original_token,
            &s.original_request,
            &s.ca,
            CLOCK
        )
        .is_err()
    );
    assert!(can(
        &s,
        s.old.credential_id,
        Action::ReadContext,
        s.f.other_workspace,
        CLOCK
    ));
    assert_eq!(rows(&s.f.connection, "ccp_grants"), 2);
}

#[test]
fn recovery_audit_failures_roll_back_issue_redeem_and_revoke() {
    let mut s = setup(fixture());
    s.f.connection.execute_batch("CREATE TRIGGER no_recovery_issue BEFORE INSERT ON ccp_access_audit WHEN NEW.action = 'issue_client_recovery' BEGIN SELECT RAISE(ABORT, 'audit down'); END;").unwrap();
    assert!(
        AccessStore::new(&mut s.f.connection)
            .unwrap()
            .issue_recovery_invitation(
                Actor::Admin(s.f.admin),
                RecoveryInvitationRequest {
                    client_id: s.old.client_id,
                    scope: RecoveryScope::Workspace(s.f.workspace),
                    expires_at: CLOCK + 3600,
                },
                CLOCK
            )
            .is_err()
    );
    assert_eq!(rows(&s.f.connection, "ccp_recovery_invitations"), 0);
    assert_eq!(rows(&s.f.connection, "ccp_recovery_grants"), 0);
    s.f.connection.execute_batch("DROP TRIGGER no_recovery_issue; CREATE TRIGGER no_recovery_redeem BEFORE INSERT ON ccp_access_audit WHEN NEW.action = 'redeem_client_recovery' BEGIN SELECT RAISE(ABORT, 'audit down'); END;").unwrap();
    let token = scoped(&mut s);
    let req = request();
    assert!(recover(&mut s, &token, &req, CLOCK).is_err());
    assert_eq!(rows(&s.f.connection, "ccp_credentials"), 1);
    assert_eq!(rows(&s.f.connection, "ccp_credential_certificates"), 1);
    assert_eq!(rows(&s.f.connection, "ccp_binding_revocations"), 0);
    assert_eq!(rows(&s.f.connection, "ccp_recovery_receipts"), 0);
    assert_eq!(rows(&s.f.connection, "ccp_recovery_receipt_grants"), 0);
    assert!(can(
        &s,
        s.old.credential_id,
        Action::WriteContext,
        s.f.workspace,
        CLOCK
    ));
    s.f.connection.execute_batch("DROP TRIGGER no_recovery_redeem; CREATE TRIGGER no_recovery_revoke BEFORE INSERT ON ccp_access_audit WHEN NEW.action = 'revoke_client_recovery' BEGIN SELECT RAISE(ABORT, 'audit down'); END;").unwrap();
    assert!(
        AccessStore::new(&mut s.f.connection)
            .unwrap()
            .revoke_recovery_invitation(Actor::Admin(s.f.admin), token.id, CLOCK)
            .is_err()
    );
    assert!(recover(&mut s, &token, &req, CLOCK).is_ok());
}

#[test]
fn partial_multiscope_binding_failure_rolls_back_all_changes() {
    let mut s = setup(fixture());
    let global = s.f.global;
    let token = issue(&mut s, Actor::Admin(global), RecoveryScope::Instance, CLOCK);
    let last = [s.old.grant_id, s.grant_b].into_iter().max().unwrap();
    s.f.connection.execute_batch(&format!("CREATE TRIGGER fail_second BEFORE INSERT ON ccp_binding_revocations WHEN NEW.grant_id = '{last}' BEGIN SELECT RAISE(ABORT, 'binding down'); END;")).unwrap();
    assert!(recover(&mut s, &token, &request(), CLOCK).is_err());
    assert_eq!(rows(&s.f.connection, "ccp_binding_revocations"), 0);
    assert_eq!(rows(&s.f.connection, "ccp_credentials"), 1);
    assert_eq!(rows(&s.f.connection, "ccp_credential_grants"), 2);
    assert_eq!(rows(&s.f.connection, "ccp_recovery_receipts"), 0);
    assert!(can(
        &s,
        s.old.credential_id,
        Action::WriteContext,
        s.f.workspace,
        CLOCK
    ));
    assert!(can(
        &s,
        s.old.credential_id,
        Action::ReadContext,
        s.f.other_workspace,
        CLOCK
    ));
}

#[test]
fn invalid_secrets_csrs_and_ca_do_not_replace_any_bindings() {
    let mut s = setup(fixture());
    let token = scoped(&mut s);
    for secret in [
        "",
        "ccp_recover_bad",
        "ccp_join_00",
        &format!("ccp_recover_{}", "0".repeat(64)),
    ] {
        assert!(
            AccessStore::new(&mut s.f.connection)
                .unwrap()
                .redeem_recovery_invitation(secret, &request(), &s.ca, CLOCK)
                .is_err()
        );
    }
    assert!(recover(&mut s, &token, &request(), -1).is_err());
    assert!(
        recover(
            &mut s,
            &token,
            &RecoveryRedemptionRequest {
                request_id: Uuid::new_v4(),
                csr_pem: "invalid".into()
            },
            CLOCK
        )
        .is_err()
    );
    let key = KeyPair::generate().unwrap();
    let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.not_before = time::OffsetDateTime::from_unix_timestamp(CLOCK + 7200).unwrap();
    let cert = params.self_signed(&key).unwrap();
    let future = IssuingAuthority::from_pem(&cert.pem(), &key.serialize_pem()).unwrap();
    assert!(
        AccessStore::new(&mut s.f.connection)
            .unwrap()
            .redeem_recovery_invitation(token.secret(), &request(), &future, CLOCK)
            .is_err()
    );
    assert_eq!(rows(&s.f.connection, "ccp_credentials"), 1);
    assert_eq!(rows(&s.f.connection, "ccp_binding_revocations"), 0);
}

#[test]
fn recovery_capture_foreign_keys_and_scope_filter_reject_other_authority() {
    let mut s = setup(fixture());
    let token = scoped(&mut s);
    let other = enroll(&mut s.f, DataRole::Read);
    assert!(
        s.f.connection
            .execute(
                "INSERT INTO ccp_recovery_grants VALUES (?1, ?2, ?3)",
                params![
                    token.id.to_string(),
                    other.grant_id.to_string(),
                    s.old.client_id.to_string()
                ]
            )
            .is_err()
    );
    assert!(
        s.f.connection
            .execute(
                "INSERT INTO ccp_binding_revocations VALUES (?1, ?2, ?3, ?4)",
                params![
                    s.old.credential_id.to_string(),
                    other.grant_id.to_string(),
                    CLOCK,
                    token.id.to_string()
                ]
            )
            .is_err()
    );
    // Even a same-client malformed capture row cannot widen a scoped token.
    s.f.connection
        .execute(
            "INSERT INTO ccp_recovery_grants VALUES (?1, ?2, ?3)",
            params![
                token.id.to_string(),
                s.grant_b.to_string(),
                s.old.client_id.to_string()
            ],
        )
        .unwrap();
    let new = recover(&mut s, &token, &request(), CLOCK).unwrap();
    assert_eq!(new.grant_ids, vec![s.old.grant_id]);
    assert!(!can(
        &s,
        new.credential_id,
        Action::ReadContext,
        s.f.other_workspace,
        CLOCK
    ));
    assert!(can(
        &s,
        s.old.credential_id,
        Action::ReadContext,
        s.f.other_workspace,
        CLOCK
    ));
    assert_eq!(
        s.f.connection
            .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        0
    );
}

#[test]
fn receipt_cannot_survive_new_credential_expiry_revoke_or_rotation() {
    for scenario in [
        "expired",
        "revoked",
        "rotated",
        "recovered_again",
        "issuer_lost",
    ] {
        let mut s = setup(fixture());
        let token = scoped(&mut s);
        let req = request();
        let new = recover(&mut s, &token, &req, CLOCK).unwrap();
        let mut now = CLOCK;
        match scenario {
            "expired" => now = new.expires_at,
            "revoked" => {
                s.f.connection
                    .execute(
                        "UPDATE ccp_credentials SET revoked_at = ?1 WHERE id = ?2",
                        params![CLOCK, new.credential_id.to_string()],
                    )
                    .unwrap();
            }
            "rotated" => {
                let csr = csr_request(&KeyPair::generate().unwrap());
                AccessStore::new(&mut s.f.connection)
                    .unwrap()
                    .renew_credential(
                        Actor::Credential(new.credential_id),
                        &RenewalRequest {
                            request_id: csr.request_id,
                            csr_pem: csr.csr_pem,
                        },
                        &s.ca,
                        CLOCK,
                    )
                    .unwrap();
            }
            "recovered_again" => {
                let next = scoped(&mut s);
                recover(&mut s, &next, &request(), CLOCK).unwrap();
            }
            "issuer_lost" => AccessStore::new(&mut s.f.connection)
                .unwrap()
                .revoke_workspace_admin(Actor::Admin(s.f.owner), s.f.workspace, s.f.admin, CLOCK)
                .unwrap(),
            _ => panic!("unknown scenario {scenario}"),
        }
        assert!(recover(&mut s, &token, &req, now).is_err(), "{scenario}");
    }
}

#[test]
fn schema9_upgrade_preserves_renewal_and_durable_scoped_recovery() {
    let path = std::env::temp_dir().join(format!("ccp-recovery-upgrade-{}.db", Uuid::new_v4()));
    crate::init::init_sqlite(&path).unwrap();
    let mut s = setup(fixture_with_connection(Connection::open(&path).unwrap()));
    let csr = csr_request(&KeyPair::generate().unwrap());
    let renewal = RenewalRequest {
        request_id: csr.request_id,
        csr_pem: csr.csr_pem,
    };
    let renewed = AccessStore::new(&mut s.f.connection)
        .unwrap()
        .renew_credential(
            Actor::Credential(s.old.credential_id),
            &renewal,
            &s.ca,
            CLOCK,
        )
        .unwrap();
    s.f.connection.execute_batch("DROP TABLE ccp_recovery_receipt_grants; DROP TABLE ccp_recovery_receipts; DROP TABLE ccp_binding_revocations;
        DROP TABLE ccp_recovery_grants; DROP TABLE ccp_recovery_invitations; UPDATE schema_version SET version = 9;").unwrap();
    drop(s.f.connection);
    crate::init::init_sqlite(&path).unwrap();
    s.f.connection = Connection::open(&path).unwrap();
    let token = scoped(&mut s);
    let req = request();
    let new = recover(&mut s, &token, &req, CLOCK).unwrap();
    drop(s.f.connection);
    crate::init::init_sqlite(&path).unwrap();
    s.f.connection = Connection::open(&path).unwrap();
    assert_eq!(recover(&mut s, &token, &req, CLOCK).unwrap(), new);
    assert_eq!(
        AccessStore::new(&mut s.f.connection)
            .unwrap()
            .renew_credential(
                Actor::Credential(s.old.credential_id),
                &renewal,
                &s.ca,
                CLOCK
            )
            .unwrap(),
        renewed
    );
    assert!(!can(
        &s,
        renewed.credential_id,
        Action::ReadContext,
        s.f.workspace,
        CLOCK
    ));
    assert!(can(
        &s,
        renewed.credential_id,
        Action::ReadContext,
        s.f.other_workspace,
        CLOCK
    ));
    assert_eq!(
        s.f.connection
            .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        0
    );
    assert_eq!(
        s.f.connection
            .query_row("SELECT MAX(version) FROM schema_version", [], |row| row
                .get::<_, u32>(0))
            .unwrap(),
        crate::init::SCHEMA_VERSION
    );
    drop(s);
    fs::remove_file(path).unwrap();
}

fn race(same_request: bool) {
    let path = std::env::temp_dir().join(format!("ccp-recovery-race-{}.db", Uuid::new_v4()));
    crate::init::init_sqlite(&path).unwrap();
    let mut s = setup(fixture_with_connection(Connection::open(&path).unwrap()));
    let token = scoped(&mut s);
    let req = request();
    let ca = Arc::new(s.ca);
    let barrier = Arc::new(std::sync::Barrier::new(8));
    drop(s.f.connection);
    let mut workers = Vec::new();
    for _ in 0..8 {
        let path = path.clone();
        let ca = ca.clone();
        let barrier = barrier.clone();
        let secret = token.secret().to_string();
        let req = if same_request { req.clone() } else { request() };
        workers.push(std::thread::spawn(move || {
            let mut connection = Connection::open(path).unwrap();
            connection
                .busy_timeout(std::time::Duration::from_secs(20))
                .unwrap();
            barrier.wait();
            AccessStore::new(&mut connection)
                .unwrap()
                .redeem_recovery_invitation(&secret, &req, &ca, CLOCK)
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
        assert!(error.to_string().contains("already redeemed"), "{error:#}");
    }
    let connection = Connection::open(&path).unwrap();
    assert_eq!(rows(&connection, "ccp_credentials"), 2);
    assert_eq!(rows(&connection, "ccp_recovery_receipts"), 1);
    assert_eq!(rows(&connection, "ccp_binding_revocations"), 1);
    assert_eq!(rows(&connection, "ccp_grants"), 2);
    drop(connection);
    fs::remove_file(path).unwrap();
}

#[test]
fn concurrent_same_recovery_request_returns_one_certificate() {
    race(true);
}

#[test]
fn concurrent_distinct_recovery_requests_consume_invitation_once() {
    race(false);
}

#[test]
fn recovery_rejects_the_current_compromised_key_without_changing_access() {
    let mut s = setup(fixture());
    let token = scoped(&mut s);
    let csr = csr_request(&s.original_key);
    let request = RecoveryRedemptionRequest {
        request_id: csr.request_id,
        csr_pem: csr.csr_pem,
    };
    let result = recover(&mut s, &token, &request, CLOCK);
    assert!(
        result.is_err(),
        "recovery must reject the previously registered client key"
    );
    assert_eq!(rows(&s.f.connection, "ccp_credentials"), 1);
    assert_eq!(rows(&s.f.connection, "ccp_binding_revocations"), 0);
    assert_eq!(rows(&s.f.connection, "ccp_recovery_receipts"), 0);
    assert!(can(
        &s,
        s.old.credential_id,
        Action::WriteContext,
        s.f.workspace,
        CLOCK
    ));
    assert!(
        recover(
            &mut s,
            &token,
            &RecoveryRedemptionRequest {
                request_id: Uuid::new_v4(),
                csr_pem: csr_request(&KeyPair::generate().unwrap()).csr_pem
            },
            CLOCK
        )
        .is_ok()
    );
}

#[test]
fn recovery_rejects_a_key_from_an_expired_credential() {
    let mut s = setup(fixture());
    let now = s.old.expires_at;
    let global = s.f.global;
    let token = issue(&mut s, Actor::Admin(global), RecoveryScope::Instance, now);
    let csr = csr_request(&s.original_key);
    let request = RecoveryRedemptionRequest {
        request_id: csr.request_id,
        csr_pem: csr.csr_pem,
    };
    let result = recover(&mut s, &token, &request, now);
    assert!(
        result.is_err(),
        "recovery must reject an expired credential's known key"
    );
    assert_eq!(rows(&s.f.connection, "ccp_credentials"), 1);
    assert_eq!(rows(&s.f.connection, "ccp_binding_revocations"), 0);
}

#[test]
fn scoped_recovery_rejects_a_known_key_now_bound_only_to_another_workspace() {
    let mut s = setup(fixture());
    let other_key = KeyPair::generate().unwrap();
    let csr = csr_request(&other_key);
    let renewed = AccessStore::new(&mut s.f.connection)
        .unwrap()
        .renew_credential(
            Actor::Credential(s.old.credential_id),
            &RenewalRequest {
                request_id: csr.request_id,
                csr_pem: csr.csr_pem,
            },
            &s.ca,
            CLOCK,
        )
        .unwrap();
    let first_token = scoped(&mut s);
    let first = recover(&mut s, &first_token, &request(), CLOCK).unwrap();
    let token = scoped(&mut s);
    let csr = csr_request(&other_key);
    let request = RecoveryRedemptionRequest {
        request_id: csr.request_id,
        csr_pem: csr.csr_pem,
    };
    let result = recover(&mut s, &token, &request, CLOCK);
    assert!(
        result.is_err(),
        "recovery must reject a previously registered key outside the recovered scope"
    );
    assert!(can(
        &s,
        first.credential_id,
        Action::WriteContext,
        s.f.workspace,
        CLOCK
    ));
    assert!(can(
        &s,
        renewed.credential_id,
        Action::ReadContext,
        s.f.other_workspace,
        CLOCK
    ));
    assert_eq!(rows(&s.f.connection, "ccp_credentials"), 3);
}
