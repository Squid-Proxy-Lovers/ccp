// SPDX-License-Identifier: AGPL-3.0-or-later
use super::redemption::{authority, csr_request, redeem};
use super::*;
mod migration;
use rcgen::KeyPair;
use sha2::{Digest, Sha256};
const CLOCK: i64 = 1_700_000_000;
fn request(workspace_id: Uuid) -> LinkInvitationRequest {
    LinkInvitationRequest {
        workspace_id,
        role: DataRole::ReadWrite,
        admission: LinkAdmission::Individual,
        lifetime: LinkLifetime::Short,
    }
}
fn issue(f: &mut Fixture, request: LinkInvitationRequest) -> anyhow::Result<IssuedInvitation> {
    AccessStore::new(&mut f.connection)?.issue_link_invitation(
        Actor::Admin(f.admin),
        request,
        CLOCK,
    )
}
#[test]
fn link_default_custom_and_confirmed_no_expiry_are_stored_and_audited() {
    for lifetime in [
        LinkLifetime::Short,
        LinkLifetime::At {
            expires_at: CLOCK + 90 * 86400,
            confirmed: true,
        },
        LinkLifetime::Never { confirmed: true },
    ] {
        let mut f = fixture();
        let mut r = request(f.workspace);
        r.lifetime = lifetime;
        let issued = issue(&mut f, r).unwrap();
        let observed = AccessStore::new(&mut f.connection)
            .unwrap()
            .invitation_status(Actor::Admin(f.owner), f.workspace, issued.id, CLOCK)
            .unwrap();
        assert_eq!(observed.expires_at, lifetime.expires_at(CLOCK).unwrap());
        assert_eq!(observed.max_uses, Some(1));
        let confirmations:i64=f.connection.query_row("SELECT COUNT(*) FROM ccp_access_audit WHERE action='confirm_non_expiring_invitation' AND target_id=?1",[issued.id.to_string()],|row|row.get(0)).unwrap();
        assert_eq!(confirmations, i64::from(observed.expires_at.is_none()));
        let hashed: Vec<u8> = f
            .connection
            .query_row(
                "SELECT token_hash FROM ccp_invitations WHERE id=?1",
                [issued.id.to_string()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(hashed, Sha256::digest(issued.secret().as_bytes()).to_vec());
        assert_ne!(hashed, issued.secret().as_bytes());
    }
    assert_eq!(
        LinkLifetime::Short.expires_at(CLOCK).unwrap(),
        Some(CLOCK + 1800)
    );
    assert!(LinkLifetime::Short.expires_at(i64::MAX).is_err());
}
#[test]
fn no_expiry_requires_confirmation_and_positive_capacity_without_side_effects() {
    for (lifetime, admission) in [
        (
            LinkLifetime::Never { confirmed: false },
            LinkAdmission::Individual,
        ),
        (
            LinkLifetime::Never { confirmed: true },
            LinkAdmission::InitialGroup { max_clients: None },
        ),
        (
            LinkLifetime::Never { confirmed: true },
            LinkAdmission::InitialGroup {
                max_clients: Some(0),
            },
        ),
        (
            LinkLifetime::At {
                expires_at: CLOCK,
                confirmed: false,
            },
            LinkAdmission::Individual,
        ),
        (
            LinkLifetime::At {
                expires_at: -1,
                confirmed: false,
            },
            LinkAdmission::Individual,
        ),
    ] {
        let mut f = fixture();
        let mut r = request(f.workspace);
        r.lifetime = lifetime;
        r.admission = admission;
        let before = rows(&f.connection, "ccp_access_audit");
        assert!(issue(&mut f, r).is_err());
        assert_eq!(rows(&f.connection, "ccp_invitations"), 0);
        assert_eq!(rows(&f.connection, "ccp_initial_enrollment"), 0);
        assert_eq!(rows(&f.connection, "ccp_access_audit"), before);
    }
}
#[test]
fn link_issuance_keeps_current_scope_lifecycle_and_revoked_key_rules() {
    for allowed in [true, false] {
        let mut f = fixture();
        let r = request(f.workspace);
        let actors = if allowed {
            vec![Actor::Admin(f.owner), Actor::Admin(f.admin)]
        } else {
            vec![
                Actor::Admin(f.global),
                Actor::Admin(f.other_owner),
                Actor::Credential(Uuid::new_v4()),
            ]
        };
        for actor in actors {
            let result = AccessStore::new(&mut f.connection)
                .unwrap()
                .issue_link_invitation(actor, r, CLOCK);
            assert_eq!(result.is_ok(), allowed);
        }
    }
    let mut f = fixture();
    let r = request(f.workspace);
    let key = AccessStore::new(&mut f.connection)
        .unwrap()
        .issue_admin_key(
            Actor::Admin(f.owner),
            &AdminKeyRequest {
                principal_id: f.admin,
                scope: AdminKeyScope::Workspace(f.workspace),
                name: "link key".into(),
                expires_at: None,
            },
            CLOCK,
        )
        .unwrap();
    let actor = AccessStore::new(&mut f.connection)
        .unwrap()
        .authenticate_admin_key(key.secret(), CLOCK)
        .unwrap();
    f.connection
        .execute(
            "UPDATE ccp_admin_keys SET revoked_at=?1 WHERE id=?2",
            params![CLOCK, key.id.to_string()],
        )
        .unwrap();
    assert!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .issue_link_invitation(actor, r, CLOCK)
            .is_err()
    );
    AccessStore::new(&mut f.connection)
        .unwrap()
        .set_archived(Actor::Admin(f.owner), f.workspace, true, CLOCK)
        .unwrap();
    assert!(issue(&mut f, r).is_err());
    assert_eq!(rows(&f.connection, "ccp_invitations"), 0);
}
#[test]
fn exact_redemption_retry_survives_deadline_while_fresh_reuse_is_refused() {
    let mut f = fixture();
    let workspace = f.workspace;
    let issued = issue(&mut f, request(workspace)).unwrap();
    let (ca, _) = authority();
    let proof = csr_request(&KeyPair::generate().unwrap());
    let receipt = redeem(&mut f, &issued, &proof, &ca, CLOCK).unwrap();
    assert_eq!(
        redeem(&mut f, &issued, &proof, &ca, CLOCK + 1801).unwrap(),
        receipt
    );
    let fresh = csr_request(&KeyPair::generate().unwrap());
    assert!(redeem(&mut f, &issued, &fresh, &ca, CLOCK + 1801).is_err());
    assert_eq!(rows(&f.connection, "ccp_clients"), 1);
    assert_eq!(rows(&f.connection, "ccp_grants"), 1);
}
#[test]
fn non_expiring_individual_is_single_use_and_revocation_still_closes_it() {
    let mut f = fixture();
    let mut r = request(f.workspace);
    r.lifetime = LinkLifetime::Never { confirmed: true };
    let issued = issue(&mut f, r).unwrap();
    let (ca, _) = authority();
    let proof = csr_request(&KeyPair::generate().unwrap());
    let receipt = redeem(&mut f, &issued, &proof, &ca, CLOCK + 90 * 86400).unwrap();
    assert_eq!(
        redeem(&mut f, &issued, &proof, &ca, CLOCK + 90 * 86400 + 1).unwrap(),
        receipt
    );
    assert!(
        redeem(
            &mut f,
            &issued,
            &csr_request(&KeyPair::generate().unwrap()),
            &ca,
            CLOCK + 90 * 86400 + 1
        )
        .is_err()
    );
    let revoked = issue(&mut f, r).unwrap();
    AccessStore::new(&mut f.connection)
        .unwrap()
        .revoke_invitation(Actor::Admin(f.owner), f.workspace, revoked.id, CLOCK)
        .unwrap();
    assert!(
        redeem(
            &mut f,
            &revoked,
            &csr_request(&KeyPair::generate().unwrap()),
            &ca,
            CLOCK
        )
        .is_err()
    );
}
#[test]
fn bounded_non_expiring_initial_group_never_reopens_bulk_admission() {
    let mut f = fixture();
    let mut r = request(f.workspace);
    r.lifetime = LinkLifetime::Never { confirmed: true };
    r.admission = LinkAdmission::InitialGroup {
        max_clients: Some(2),
    };
    let group = issue(&mut f, r).unwrap();
    let (ca, _) = authority();
    for _ in 0..2 {
        redeem(
            &mut f,
            &group,
            &csr_request(&KeyPair::generate().unwrap()),
            &ca,
            CLOCK,
        )
        .unwrap();
    }
    assert!(
        redeem(
            &mut f,
            &group,
            &csr_request(&KeyPair::generate().unwrap()),
            &ca,
            CLOCK
        )
        .is_err()
    );
    assert!(issue(&mut f, r).is_err());
    r.admission = LinkAdmission::Individual;
    assert!(issue(&mut f, r).is_ok());
    r.admission = LinkAdmission::InitialGroup {
        max_clients: Some(3),
    };
    assert!(issue(&mut f, r).is_err());
}
#[test]
fn ignored_confirmation_audit_rolls_back_invitation_and_mode_transition() {
    let mut f = fixture();
    let mut r = request(f.workspace);
    r.lifetime = LinkLifetime::Never { confirmed: true };
    let before = rows(&f.connection, "ccp_access_audit");
    f.connection.execute_batch("CREATE TRIGGER fail_link_confirmation BEFORE INSERT ON ccp_access_audit WHEN NEW.action='confirm_non_expiring_invitation' BEGIN SELECT RAISE(IGNORE); END;").unwrap();
    assert!(issue(&mut f, r).is_err());
    assert_eq!(rows(&f.connection, "ccp_invitations"), 0);
    assert_eq!(rows(&f.connection, "ccp_initial_enrollment"), 0);
    assert_eq!(rows(&f.connection, "ccp_access_audit"), before);
    f.connection
        .execute_batch("DROP TRIGGER fail_link_confirmation")
        .unwrap();
    assert!(issue(&mut f, r).is_ok());
}

#[test]
fn dated_links_beyond_seven_days_require_confirmation_and_groups_require_a_cap() {
    for (seconds, confirmed, group, cap, allowed) in [
        (7 * 86400, false, false, None, true),
        (7 * 86400 + 1, false, false, None, false),
        (7 * 86400 + 1, true, false, None, true),
        (90 * 86400, true, true, None, false),
        (90 * 86400, false, true, Some(2), false),
        (90 * 86400, true, true, Some(2), true),
    ] {
        let mut f = fixture();
        let mut r = request(f.workspace);
        r.lifetime = LinkLifetime::At {
            expires_at: CLOCK + seconds,
            confirmed,
        };
        if group {
            r.admission = LinkAdmission::InitialGroup { max_clients: cap };
        }
        let result = issue(&mut f, r);
        assert_eq!(result.is_ok(), allowed);
        assert_eq!(rows(&f.connection, "ccp_invitations"), i64::from(allowed));
        let audited:i64=f.connection.query_row("SELECT COUNT(*) FROM ccp_access_audit WHERE action='confirm_long_lived_invitation'",[],|row|row.get(0)).unwrap();
        assert_eq!(audited, i64::from(allowed && seconds > 7 * 86400));
    }
}
