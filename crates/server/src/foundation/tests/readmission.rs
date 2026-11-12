// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::redemption::{authority, csr_request, invite, redeem};
use super::*;
use rcgen::KeyPair;
use std::sync::Arc;

const CLOCK: i64 = 1_700_000_000;

struct Scenario {
    f: Fixture,
    ca: IssuingAuthority,
    old: EnrollmentReceipt,
    original_key: KeyPair,
    original_token: IssuedInvitation,
    grant_b: Uuid,
}

fn setup(f: Fixture, capacity: u32) -> Scenario {
    let mut f = f;
    let (ca, _) = authority();
    let original_key = KeyPair::generate().unwrap();
    let original_token = invite(&mut f, capacity);
    let old = redeem(
        &mut f,
        &original_token,
        &csr_request(&original_key),
        &ca,
        CLOCK,
    )
    .unwrap();
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
    AccessStore::new(&mut f.connection)
        .unwrap()
        .revoke_grant(Actor::Admin(f.admin), old.grant_id, CLOCK)
        .unwrap();
    Scenario {
        f,
        ca,
        old,
        original_key,
        original_token,
        grant_b,
    }
}

fn issue(s: &mut Scenario) -> IssuedInvitation {
    AccessStore::new(&mut s.f.connection)
        .unwrap()
        .issue_readmission_invitation(
            Actor::Admin(s.f.admin),
            ReadmissionInvitationRequest {
                client_id: s.old.client_id,
                workspace_id: s.f.workspace,
                role: DataRole::Read,
                expires_at: CLOCK + 3600,
            },
            CLOCK,
        )
        .unwrap()
}

fn can(s: &Scenario, credential: Uuid, action: Action, workspace: Uuid) -> bool {
    authorize(
        &s.f.connection,
        Actor::Credential(credential),
        action,
        Some(workspace),
        CLOCK,
    )
    .is_ok()
}

fn redeem_target(
    s: &mut Scenario,
    token: &IssuedInvitation,
    req: &RedemptionRequest,
    now: i64,
) -> anyhow::Result<EnrollmentReceipt> {
    AccessStore::new(&mut s.f.connection)?.redeem_invitation(token.secret(), req, &s.ca, now)
}

fn request() -> RedemptionRequest {
    csr_request(&KeyPair::generate().unwrap())
}

#[test]
fn explicit_readmission_creates_a_new_scoped_grant_for_the_same_client() {
    let mut s = setup(fixture(), 1);
    let token = issue(&mut s);
    let receipt = redeem_target(&mut s, &token, &request(), CLOCK).unwrap();
    assert_eq!(receipt.client_id, s.old.client_id);
    assert_ne!(receipt.grant_id, s.old.grant_id);
    assert_ne!(receipt.credential_id, s.old.credential_id);
    assert!(can(
        &s,
        receipt.credential_id,
        Action::ReadContext,
        s.f.workspace
    ));
    assert!(!can(
        &s,
        receipt.credential_id,
        Action::WriteContext,
        s.f.workspace
    ));
    assert!(!can(
        &s,
        receipt.credential_id,
        Action::ReadContext,
        s.f.other_workspace
    ));
    assert!(!can(
        &s,
        s.old.credential_id,
        Action::ReadContext,
        s.f.workspace
    ));
    assert!(can(
        &s,
        s.old.credential_id,
        Action::ReadContext,
        s.f.other_workspace
    ));
    assert_eq!(rows(&s.f.connection, "ccp_clients"), 1);
    assert_eq!(rows(&s.f.connection, "ccp_grants"), 3);
    assert_eq!(rows(&s.f.connection, "ccp_credentials"), 2);
    let revoked: bool =
        s.f.connection
            .query_row(
                "SELECT revoked_at IS NOT NULL FROM ccp_grants WHERE id = ?1",
                [s.old.grant_id.to_string()],
                |row| row.get(0),
            )
            .unwrap();
    assert!(revoked);
    let mut store = AccessStore::new(&mut s.f.connection).unwrap();
    let status = store
        .invitation_status(Actor::Admin(s.f.admin), s.f.workspace, token.id, CLOCK)
        .unwrap();
    assert_eq!(status.target_client_id, Some(s.old.client_id));
    assert_eq!(status.uses, 1);
    assert_eq!(status.max_uses, Some(1));
    let original = store
        .invitation_status(
            Actor::Admin(s.f.admin),
            s.f.workspace,
            s.original_token.id,
            CLOCK,
        )
        .unwrap();
    assert_eq!(original.target_client_id, None);
    assert_eq!(original.uses, 1);
    assert!(original.initial_closed);
}

#[test]
fn existing_credential_claim_cannot_override_a_readmission_target() {
    let mut s = setup(fixture(), 1);
    let token = issue(&mut s);
    let mut claimed = csr_request(&s.original_key);
    claimed.existing_credential = Some(s.old.credential_id);
    assert!(
        AccessStore::new(&mut s.f.connection)
            .unwrap()
            .admit_with_invitation(
                Actor::Credential(s.old.credential_id),
                token.secret(),
                &claimed,
                &s.ca,
                CLOCK
            )
            .is_err()
    );
    assert!(redeem_target(&mut s, &token, &claimed, CLOCK).is_err());
    assert_eq!(rows(&s.f.connection, "ccp_credentials"), 1);
    let req = request();
    let new = redeem_target(&mut s, &token, &req, CLOCK).unwrap();
    assert_eq!(new.client_id, s.old.client_id);
    let mut claimed_retry = req.clone();
    claimed_retry.existing_credential = Some(s.old.credential_id);
    assert!(
        AccessStore::new(&mut s.f.connection)
            .unwrap()
            .admit_with_invitation(
                Actor::Credential(s.old.credential_id),
                token.secret(),
                &claimed_retry,
                &s.ca,
                CLOCK
            )
            .is_err()
    );
}

#[test]
fn readmission_requires_existing_fully_revoked_history_and_enabled_target() {
    for scenario in ["unknown", "no_history", "live", "disabled"] {
        let mut s = setup(fixture(), 1);
        let mut client = s.old.client_id;
        let mut workspace = s.f.workspace;
        let mut actor = s.f.admin;
        match scenario {
            "unknown" => client = Uuid::new_v4(),
            "no_history" => {
                let empty = AccessStore::new(&mut s.f.connection)
                    .unwrap()
                    .create_workspace(Actor::Admin(s.f.global), "empty", s.f.owner, CLOCK)
                    .unwrap();
                workspace = empty;
                actor = s.f.owner;
            }
            "live" => {
                workspace = s.f.other_workspace;
                actor = s.f.other_owner;
            }
            "disabled" => AccessStore::new(&mut s.f.connection)
                .unwrap()
                .disable_client(Actor::Admin(s.f.global), client, CLOCK)
                .unwrap(),
            _ => panic!("unknown scenario {scenario}"),
        }
        assert!(
            AccessStore::new(&mut s.f.connection)
                .unwrap()
                .issue_readmission_invitation(
                    Actor::Admin(actor),
                    ReadmissionInvitationRequest {
                        client_id: client,
                        workspace_id: workspace,
                        role: DataRole::ReadWrite,
                        expires_at: CLOCK + 3600,
                    },
                    CLOCK
                )
                .is_err(),
            "{scenario}"
        );
        assert_eq!(rows(&s.f.connection, "ccp_invitations"), 1, "{scenario}");
        assert_eq!(
            rows(&s.f.connection, "ccp_readmission_targets"),
            0,
            "{scenario}"
        );
        assert_eq!(
            rows(&s.f.connection, "ccp_readmission_history"),
            0,
            "{scenario}"
        );
    }
}

#[test]
fn an_intervening_readmission_permanently_invalidates_older_pending_tickets() {
    let mut s = setup(fixture(), 1);
    let first = issue(&mut s);
    let stale = issue(&mut s);
    let req = request();
    let new = redeem_target(&mut s, &first, &req, CLOCK).unwrap();
    let stale_req = request();
    assert!(redeem_target(&mut s, &stale, &stale_req, CLOCK).is_err());
    AccessStore::new(&mut s.f.connection)
        .unwrap()
        .revoke_grant(Actor::Admin(s.f.admin), new.grant_id, CLOCK)
        .unwrap();
    assert!(redeem_target(&mut s, &stale, &stale_req, CLOCK).is_err());
    assert!(redeem_target(&mut s, &first, &req, CLOCK).is_err());
    let fresh = issue(&mut s);
    let newest = redeem_target(&mut s, &fresh, &request(), CLOCK).unwrap();
    assert_eq!(newest.client_id, s.old.client_id);
    assert_ne!(newest.grant_id, new.grant_id);
    assert_ne!(newest.grant_id, s.old.grant_id);
    assert_eq!(rows(&s.f.connection, "ccp_grants"), 4);
    let status = AccessStore::new(&mut s.f.connection)
        .unwrap()
        .invitation_status(Actor::Admin(s.f.admin), s.f.workspace, stale.id, CLOCK)
        .unwrap();
    assert_eq!(status.uses, 0);
    let captured: i64 =
        s.f.connection
            .query_row(
                "SELECT COUNT(*) FROM ccp_readmission_history WHERE invitation_id = ?1",
                [fresh.id.to_string()],
                |row| row.get(0),
            )
            .unwrap();
    assert_eq!(captured, 2);
}

#[test]
fn a_direct_intervening_admission_and_revocation_also_invalidates_target_snapshot() {
    let mut s = setup(fixture(), 1);
    let stale = issue(&mut s);
    let intervening = AccessStore::new(&mut s.f.connection)
        .unwrap()
        .admit_existing_client(
            Actor::Admin(s.f.admin),
            Admission {
                workspace_id: s.f.workspace,
                presented_credential: s.old.credential_id,
                role: DataRole::ReadWrite,
            },
            CLOCK,
        )
        .unwrap();
    AccessStore::new(&mut s.f.connection)
        .unwrap()
        .revoke_grant(Actor::Admin(s.f.admin), intervening, CLOCK)
        .unwrap();
    assert!(redeem_target(&mut s, &stale, &request(), CLOCK).is_err());
    assert_eq!(rows(&s.f.connection, "ccp_credentials"), 1);
    assert_eq!(rows(&s.f.connection, "ccp_grants"), 3);
    let fresh = issue(&mut s);
    assert!(redeem_target(&mut s, &fresh, &request(), CLOCK).is_ok());
}

#[test]
fn old_generic_invitation_and_group_token_cannot_restore_the_client() {
    let mut s = setup(fixture(), 1);
    let generic = AccessStore::new(&mut s.f.connection)
        .unwrap()
        .issue_individual_invitation(
            Actor::Admin(s.f.admin),
            IndividualInvitationRequest {
                workspace_id: s.f.workspace,
                role: DataRole::ReadWrite,
                expires_at: CLOCK + 3600,
            },
            CLOCK,
        )
        .unwrap();
    let mut req = csr_request(&s.original_key);
    req.existing_credential = Some(s.old.credential_id);
    for token in [&generic, &s.original_token] {
        assert!(
            AccessStore::new(&mut s.f.connection)
                .unwrap()
                .admit_with_invitation(
                    Actor::Credential(s.old.credential_id),
                    token.secret(),
                    &req,
                    &s.ca,
                    CLOCK
                )
                .is_err()
        );
    }
    assert_eq!(rows(&s.f.connection, "ccp_grants"), 2);
    assert_eq!(rows(&s.f.connection, "ccp_credentials"), 1);
    let targeted = issue(&mut s);
    assert!(redeem_target(&mut s, &targeted, &request(), CLOCK).is_ok());
}

#[test]
fn initial_group_must_be_closed_explicitly_before_targeted_readmission() {
    let mut s = setup(fixture(), 2);
    let spec = ReadmissionInvitationRequest {
        client_id: s.old.client_id,
        workspace_id: s.f.workspace,
        role: DataRole::Read,
        expires_at: CLOCK + 3600,
    };
    let error = AccessStore::new(&mut s.f.connection)
        .unwrap()
        .issue_readmission_invitation(Actor::Admin(s.f.admin), spec, CLOCK)
        .err()
        .unwrap();
    assert!(error.to_string().contains("close initial group"));
    assert_eq!(rows(&s.f.connection, "ccp_invitations"), 1);
    AccessStore::new(&mut s.f.connection)
        .unwrap()
        .close_initial_enrollment(Actor::Admin(s.f.admin), s.f.workspace, CLOCK)
        .unwrap();
    let token = issue(&mut s);
    assert!(redeem_target(&mut s, &token, &request(), CLOCK).is_ok());
    assert!(
        AccessStore::new(&mut s.f.connection)
            .unwrap()
            .issue_initial_invitation(
                Actor::Admin(s.f.admin),
                InitialInvitationRequest {
                    workspace_id: s.f.workspace,
                    role: DataRole::Read,
                    max_clients: Some(2),
                    expires_at: None,
                },
                CLOCK
            )
            .is_err()
    );
}

#[test]
fn individual_readmission_seals_an_unused_initial_window() {
    let mut f = fixture();
    let old = enroll(&mut f, DataRole::Read);
    AccessStore::new(&mut f.connection)
        .unwrap()
        .revoke_grant(Actor::Admin(f.admin), old.grant_id, CLOCK)
        .unwrap();
    let token = AccessStore::new(&mut f.connection)
        .unwrap()
        .issue_readmission_invitation(
            Actor::Admin(f.admin),
            ReadmissionInvitationRequest {
                client_id: old.client_id,
                workspace_id: f.workspace,
                role: DataRole::ReadWrite,
                expires_at: CLOCK + 3600,
            },
            CLOCK,
        )
        .unwrap();
    let status = AccessStore::new(&mut f.connection)
        .unwrap()
        .invitation_status(Actor::Admin(f.admin), f.workspace, token.id, CLOCK)
        .unwrap();
    assert!(!status.initial_closed); // This token is individual, not the initial-window token.
    let closed: bool = f
        .connection
        .query_row(
            "SELECT closed_at IS NOT NULL FROM ccp_initial_enrollment WHERE workspace_id = ?1",
            [f.workspace.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert!(closed);
    assert_eq!(status.target_client_id, Some(old.client_id));
    let (ca, _) = authority();
    let new = AccessStore::new(&mut f.connection)
        .unwrap()
        .redeem_invitation(token.secret(), &request(), &ca, CLOCK)
        .unwrap();
    assert_eq!(new.client_id, old.client_id);
}

#[test]
fn readmission_requires_current_scoped_admin_authority_and_preserves_key_ceiling() {
    let mut s = setup(fixture(), 1);
    let global = s.f.global;
    let mine = AccessStore::new(&mut s.f.connection)
        .unwrap()
        .create_workspace(Actor::Admin(global), "mine", global, CLOCK)
        .unwrap();
    let grant = AccessStore::new(&mut s.f.connection)
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
    AccessStore::new(&mut s.f.connection)
        .unwrap()
        .revoke_grant(Actor::Admin(global), grant, CLOCK)
        .unwrap();
    let mut store = AccessStore::new(&mut s.f.connection).unwrap();
    let instance_key = store
        .issue_admin_key(
            Actor::Admin(global),
            &AdminKeyRequest {
                principal_id: global,
                scope: AdminKeyScope::Instance,
                name: "instance".into(),
                expires_at: None,
            },
            CLOCK,
        )
        .unwrap();
    let workspace_key = store
        .issue_admin_key(
            Actor::Admin(global),
            &AdminKeyRequest {
                principal_id: global,
                scope: AdminKeyScope::Workspace(mine),
                name: "workspace".into(),
                expires_at: None,
            },
            CLOCK,
        )
        .unwrap();
    for (actor, workspace) in [
        (Actor::Admin(global), s.f.workspace),
        (Actor::Admin(s.f.other_owner), s.f.workspace),
        (Actor::Credential(s.old.credential_id), s.f.workspace),
        (Actor::AdminKey(instance_key.id), mine),
        (Actor::AdminKey(workspace_key.id), s.f.workspace),
    ] {
        assert!(
            store
                .issue_readmission_invitation(
                    actor,
                    ReadmissionInvitationRequest {
                        client_id: s.old.client_id,
                        workspace_id: workspace,
                        role: DataRole::Read,
                        expires_at: CLOCK + 3600,
                    },
                    CLOCK
                )
                .is_err()
        );
    }
    let token = store
        .issue_readmission_invitation(
            Actor::AdminKey(workspace_key.id),
            ReadmissionInvitationRequest {
                client_id: s.old.client_id,
                workspace_id: mine,
                role: DataRole::Read,
                expires_at: CLOCK + 3600,
            },
            CLOCK,
        )
        .unwrap();
    assert_eq!(
        store
            .invitation_status(Actor::Admin(global), mine, token.id, CLOCK)
            .unwrap()
            .target_client_id,
        Some(s.old.client_id)
    );
}

#[test]
fn role_loss_key_expiry_and_key_revocation_invalidate_pending_readmission() {
    for scenario in ["role", "key_expired", "key_revoked"] {
        let mut s = setup(fixture(), 1);
        let owner = s.f.owner;
        let admin = s.f.admin;
        let workspace = s.f.workspace;
        let mut store = AccessStore::new(&mut s.f.connection).unwrap();
        let key = store
            .issue_admin_key(
                Actor::Admin(owner),
                &AdminKeyRequest {
                    principal_id: admin,
                    scope: AdminKeyScope::Workspace(workspace),
                    name: "issuer".into(),
                    expires_at: Some(CLOCK + 60),
                },
                CLOCK,
            )
            .unwrap();
        let token = store
            .issue_readmission_invitation(
                Actor::AdminKey(key.id),
                ReadmissionInvitationRequest {
                    client_id: s.old.client_id,
                    workspace_id: workspace,
                    role: DataRole::Read,
                    expires_at: CLOCK + 3600,
                },
                CLOCK,
            )
            .unwrap();
        match scenario {
            "role" => store
                .revoke_workspace_admin(Actor::Admin(owner), workspace, admin, CLOCK)
                .unwrap(),
            "key_revoked" => store
                .revoke_admin_key(
                    Actor::Admin(owner),
                    AdminKeyScope::Workspace(workspace),
                    key.id,
                    CLOCK,
                )
                .unwrap(),
            "key_expired" => {}
            _ => panic!("unknown scenario {scenario}"),
        }
        assert!(
            redeem_target(
                &mut s,
                &token,
                &request(),
                if scenario == "key_expired" {
                    CLOCK + 60
                } else {
                    CLOCK
                }
            )
            .is_err()
        );
        assert_eq!(rows(&s.f.connection, "ccp_credentials"), 1);
        assert_eq!(rows(&s.f.connection, "ccp_grants"), 2);
    }
}

#[test]
fn known_client_key_is_rejected_without_consuming_targeted_invitation() {
    let mut s = setup(fixture(), 1);
    let token = issue(&mut s);
    let req = csr_request(&s.original_key);
    let error = redeem_target(&mut s, &token, &req, CLOCK).err().unwrap();
    assert!(error.to_string().contains("not previously registered"));
    assert_eq!(rows(&s.f.connection, "ccp_credentials"), 1);
    assert_eq!(rows(&s.f.connection, "ccp_grants"), 2);
    assert_eq!(rows(&s.f.connection, "ccp_invitation_receipts"), 1);
    let status = AccessStore::new(&mut s.f.connection)
        .unwrap()
        .invitation_status(Actor::Admin(s.f.admin), s.f.workspace, token.id, CLOCK)
        .unwrap();
    assert_eq!(status.uses, 0);
    assert!(redeem_target(&mut s, &token, &request(), CLOCK).is_ok());
}

#[test]
fn archived_workspace_blocks_new_readmission_but_allows_committed_receipt() {
    let mut s = setup(fixture(), 1);
    let token = issue(&mut s);
    let req = request();
    AccessStore::new(&mut s.f.connection)
        .unwrap()
        .set_archived(Actor::Admin(s.f.owner), s.f.workspace, true, CLOCK)
        .unwrap();
    assert!(redeem_target(&mut s, &token, &req, CLOCK).is_err());
    assert!(
        AccessStore::new(&mut s.f.connection)
            .unwrap()
            .issue_readmission_invitation(
                Actor::Admin(s.f.owner),
                ReadmissionInvitationRequest {
                    client_id: s.old.client_id,
                    workspace_id: s.f.workspace,
                    role: DataRole::Read,
                    expires_at: CLOCK + 3600,
                },
                CLOCK
            )
            .is_err()
    );
    AccessStore::new(&mut s.f.connection)
        .unwrap()
        .set_archived(Actor::Admin(s.f.owner), s.f.workspace, false, CLOCK)
        .unwrap();
    let new = redeem_target(&mut s, &token, &req, CLOCK).unwrap();
    AccessStore::new(&mut s.f.connection)
        .unwrap()
        .set_archived(Actor::Admin(s.f.owner), s.f.workspace, true, CLOCK)
        .unwrap();
    assert_eq!(redeem_target(&mut s, &token, &req, CLOCK).unwrap(), new);
    assert!(can(
        &s,
        new.credential_id,
        Action::ReadContext,
        s.f.workspace
    ));
    assert!(!can(
        &s,
        new.credential_id,
        Action::WriteContext,
        s.f.workspace
    ));
}

#[test]
fn targeted_deadline_persists_and_cached_retry_is_exact() {
    let mut s = setup(fixture(), 1);
    let expired = issue(&mut s);
    assert!(redeem_target(&mut s, &expired, &request(), CLOCK + 3600).is_err());
    assert!(redeem_target(&mut s, &expired, &request(), CLOCK).is_err());
    let token = issue(&mut s);
    let req = request();
    let new = redeem_target(&mut s, &token, &req, CLOCK).unwrap();
    assert_eq!(
        redeem_target(&mut s, &token, &req, CLOCK + 3600).unwrap(),
        new
    );
    let mut different = request();
    different.request_id = req.request_id;
    assert!(redeem_target(&mut s, &token, &different, CLOCK).is_err());
    let changed_id = RedemptionRequest {
        request_id: Uuid::new_v4(),
        ..req.clone()
    };
    assert!(redeem_target(&mut s, &token, &changed_id, CLOCK).is_err());
    assert_eq!(rows(&s.f.connection, "ccp_credentials"), 2);
    assert_eq!(rows(&s.f.connection, "ccp_grants"), 3);
    AccessStore::new(&mut s.f.connection)
        .unwrap()
        .revoke_invitation(Actor::Admin(s.f.admin), s.f.workspace, token.id, CLOCK)
        .unwrap();
    assert!(redeem_target(&mut s, &token, &req, CLOCK).is_err());
}

#[test]
fn global_disable_blocks_target_redemption_and_receipt_retrieval() {
    for redeemed in [false, true] {
        let mut s = setup(fixture(), 1);
        let token = issue(&mut s);
        let req = request();
        if redeemed {
            redeem_target(&mut s, &token, &req, CLOCK).unwrap();
        }
        AccessStore::new(&mut s.f.connection)
            .unwrap()
            .disable_client(Actor::Admin(s.f.global), s.old.client_id, CLOCK)
            .unwrap();
        assert!(redeem_target(&mut s, &token, &req, CLOCK).is_err());
        assert_eq!(
            rows(&s.f.connection, "ccp_credentials"),
            if redeemed { 2 } else { 1 }
        );
    }
}

#[test]
fn target_issuance_and_redemption_audit_failure_roll_back_all_new_state() {
    let mut s = setup(fixture(), 1);
    s.f.connection.execute_batch("CREATE TRIGGER reject_target_issue BEFORE INSERT ON ccp_access_audit WHEN NEW.action = 'issue_readmission_invitation' BEGIN SELECT RAISE(ABORT, 'audit down'); END;").unwrap();
    assert!(
        AccessStore::new(&mut s.f.connection)
            .unwrap()
            .issue_readmission_invitation(
                Actor::Admin(s.f.admin),
                ReadmissionInvitationRequest {
                    client_id: s.old.client_id,
                    workspace_id: s.f.workspace,
                    role: DataRole::Read,
                    expires_at: CLOCK + 3600,
                },
                CLOCK
            )
            .is_err()
    );
    assert_eq!(rows(&s.f.connection, "ccp_invitations"), 1);
    assert_eq!(rows(&s.f.connection, "ccp_readmission_targets"), 0);
    assert_eq!(rows(&s.f.connection, "ccp_readmission_history"), 0);
    s.f.connection.execute_batch("DROP TRIGGER reject_target_issue; CREATE TRIGGER reject_target_redeem BEFORE INSERT ON ccp_access_audit WHEN NEW.action = 'redeem_invitation' BEGIN SELECT RAISE(ABORT, 'audit down'); END;").unwrap();
    let token = issue(&mut s);
    let req = request();
    assert!(redeem_target(&mut s, &token, &req, CLOCK).is_err());
    assert_eq!(rows(&s.f.connection, "ccp_credentials"), 1);
    assert_eq!(rows(&s.f.connection, "ccp_credential_certificates"), 1);
    assert_eq!(rows(&s.f.connection, "ccp_grants"), 2);
    let status = AccessStore::new(&mut s.f.connection)
        .unwrap()
        .invitation_status(Actor::Admin(s.f.admin), s.f.workspace, token.id, CLOCK)
        .unwrap();
    assert_eq!(status.uses, 0);
    s.f.connection
        .execute_batch("DROP TRIGGER reject_target_redeem;")
        .unwrap();
    assert!(redeem_target(&mut s, &token, &req, CLOCK).is_ok());
}

#[test]
fn failed_grant_insert_leaves_no_orphan_credential_and_keeps_other_workspace_working() {
    let mut s = setup(fixture(), 1);
    let token = issue(&mut s);
    let req = request();
    s.f.connection.execute_batch("CREATE TRIGGER reject_readmission_grant BEFORE INSERT ON ccp_grants BEGIN SELECT RAISE(ABORT, 'grant down'); END;").unwrap();
    assert!(redeem_target(&mut s, &token, &req, CLOCK).is_err());
    assert_eq!(rows(&s.f.connection, "ccp_credentials"), 1);
    assert_eq!(rows(&s.f.connection, "ccp_credential_certificates"), 1);
    assert_eq!(rows(&s.f.connection, "ccp_grants"), 2);
    assert_eq!(rows(&s.f.connection, "ccp_credential_grants"), 2);
    assert!(can(
        &s,
        s.old.credential_id,
        Action::ReadContext,
        s.f.other_workspace
    ));
    s.f.connection
        .execute_batch("DROP TRIGGER reject_readmission_grant;")
        .unwrap();
    assert!(redeem_target(&mut s, &token, &req, CLOCK).is_ok());
}

#[test]
fn target_and_snapshot_foreign_keys_prevent_other_clients_or_workspaces() {
    let mut s = setup(fixture(), 1);
    let token = issue(&mut s);
    let other = enroll(&mut s.f, DataRole::Read);
    assert!(
        s.f.connection
            .execute(
                "INSERT INTO ccp_readmission_history VALUES (?1, ?2, ?3, ?4)",
                params![
                    token.id.to_string(),
                    other.grant_id.to_string(),
                    s.old.client_id.to_string(),
                    s.f.workspace.to_string()
                ]
            )
            .is_err()
    );
    assert!(
        s.f.connection
            .execute(
                "INSERT INTO ccp_readmission_history VALUES (?1, ?2, ?3, ?4)",
                params![
                    token.id.to_string(),
                    s.grant_b.to_string(),
                    s.old.client_id.to_string(),
                    s.f.workspace.to_string()
                ]
            )
            .is_err()
    );
    assert!(
        s.f.connection
            .execute(
                "INSERT INTO ccp_readmission_history VALUES (?1, ?2, ?3, ?4)",
                params![
                    token.id.to_string(),
                    s.grant_b.to_string(),
                    s.old.client_id.to_string(),
                    s.f.other_workspace.to_string()
                ]
            )
            .is_err()
    );
    assert!(
        s.f.connection
            .execute(
                "INSERT INTO ccp_readmission_targets VALUES (?1, ?2, ?3)",
                params![
                    s.original_token.id.to_string(),
                    s.old.client_id.to_string(),
                    s.f.other_workspace.to_string()
                ]
            )
            .is_err()
    );
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
fn an_initial_group_cannot_be_repurposed_as_a_targeted_ticket() {
    let mut s = setup(fixture(), 2);
    s.f.connection
        .execute(
            "INSERT INTO ccp_readmission_targets VALUES (?1, ?2, ?3)",
            params![
                s.original_token.id.to_string(),
                s.old.client_id.to_string(),
                s.f.workspace.to_string()
            ],
        )
        .unwrap();
    s.f.connection
        .execute(
            "INSERT INTO ccp_readmission_history VALUES (?1, ?2, ?3, ?4)",
            params![
                s.original_token.id.to_string(),
                s.old.grant_id.to_string(),
                s.old.client_id.to_string(),
                s.f.workspace.to_string()
            ],
        )
        .unwrap();
    assert!(
        AccessStore::new(&mut s.f.connection)
            .unwrap()
            .redeem_invitation(s.original_token.secret(), &request(), &s.ca, CLOCK)
            .is_err()
    );
    assert_eq!(rows(&s.f.connection, "ccp_clients"), 1);
    assert_eq!(rows(&s.f.connection, "ccp_credentials"), 1);
    assert_eq!(rows(&s.f.connection, "ccp_grants"), 2);
    let status = AccessStore::new(&mut s.f.connection)
        .unwrap()
        .invitation_status(
            Actor::Admin(s.f.admin),
            s.f.workspace,
            s.original_token.id,
            CLOCK,
        )
        .unwrap();
    assert_eq!(status.uses, 1);
}

#[test]
fn schema10_upgrade_keeps_recovery_and_new_readmission_receipts_after_restart() {
    let path = std::env::temp_dir().join(format!("ccp-readmission-upgrade-{}.db", Uuid::new_v4()));
    crate::init::init_sqlite(&path).unwrap();
    let mut s = setup(fixture_with_connection(Connection::open(&path).unwrap()), 1);
    let recovery = AccessStore::new(&mut s.f.connection)
        .unwrap()
        .issue_recovery_invitation(
            Actor::Admin(s.f.other_owner),
            RecoveryInvitationRequest {
                client_id: s.old.client_id,
                scope: RecoveryScope::Workspace(s.f.other_workspace),
                expires_at: CLOCK + 3600,
            },
            CLOCK,
        )
        .unwrap();
    let csr = request();
    let recovery_req = RecoveryRedemptionRequest {
        request_id: csr.request_id,
        csr_pem: csr.csr_pem,
    };
    let recovered_b = AccessStore::new(&mut s.f.connection)
        .unwrap()
        .redeem_recovery_invitation(recovery.secret(), &recovery_req, &s.ca, CLOCK)
        .unwrap();
    s.f.connection.execute_batch("DROP TABLE ccp_readmission_history; DROP TABLE ccp_readmission_targets; UPDATE schema_version SET version = 10;").unwrap();
    drop(s.f.connection);
    crate::init::init_sqlite(&path).unwrap();
    s.f.connection = Connection::open(&path).unwrap();
    let token = issue(&mut s);
    let req = request();
    let new = redeem_target(&mut s, &token, &req, CLOCK).unwrap();
    drop(s.f.connection);
    crate::init::init_sqlite(&path).unwrap();
    s.f.connection = Connection::open(&path).unwrap();
    assert_eq!(redeem_target(&mut s, &token, &req, CLOCK).unwrap(), new);
    assert_eq!(
        AccessStore::new(&mut s.f.connection)
            .unwrap()
            .redeem_recovery_invitation(recovery.secret(), &recovery_req, &s.ca, CLOCK)
            .unwrap(),
        recovered_b
    );
    assert!(can(
        &s,
        recovered_b.credential_id,
        Action::ReadContext,
        s.f.other_workspace
    ));
    assert!(!can(
        &s,
        new.credential_id,
        Action::ReadContext,
        s.f.other_workspace
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
    let path = std::env::temp_dir().join(format!("ccp-readmission-race-{}.db", Uuid::new_v4()));
    crate::init::init_sqlite(&path).unwrap();
    let mut s = setup(fixture_with_connection(Connection::open(&path).unwrap()), 1);
    let token = issue(&mut s);
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
                .redeem_invitation(&secret, &req, &ca, CLOCK)
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
            error.to_string().contains("not authorized or is closed"),
            "{error:#}"
        );
    }
    let connection = Connection::open(&path).unwrap();
    assert_eq!(rows(&connection, "ccp_credentials"), 2);
    assert_eq!(rows(&connection, "ccp_grants"), 3);
    let uses: u64 = connection
        .query_row(
            "SELECT uses FROM ccp_invitations WHERE id = ?1",
            [token.id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(uses, 1);
    drop(connection);
    fs::remove_file(path).unwrap();
}

#[test]
fn concurrent_same_targeted_request_returns_one_grant_and_certificate() {
    race(true);
}

#[test]
fn concurrent_distinct_targeted_requests_consume_only_one_slot() {
    race(false);
}
