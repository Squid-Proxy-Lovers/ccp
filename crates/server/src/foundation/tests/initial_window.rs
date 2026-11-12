// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;
fn observe(f: &mut Fixture, actor: Actor, now: i64) -> anyhow::Result<InitialEnrollmentStatus> {
    AccessStore::new(&mut f.connection)?.initial_enrollment_status(actor, f.workspace, now)
}
fn open(f: &mut Fixture) -> IssuedInvitation {
    AccessStore::new(&mut f.connection)
        .unwrap()
        .issue_initial_invitation(
            Actor::Admin(f.admin),
            InitialInvitationRequest {
                workspace_id: f.workspace,
                role: DataRole::ReadWrite,
                max_clients: Some(2),
                expires_at: Some(EXPIRES),
            },
            NOW,
        )
        .unwrap()
}
#[test]
fn initial_window_status_uses_real_row_history_and_current_scope() {
    let mut f = fixture();
    let owner = Actor::Admin(f.owner);
    let global = Actor::Admin(f.global);
    assert!(observe(&mut f, global, NOW).is_err());
    let empty = observe(&mut f, owner, NOW).unwrap();
    assert_eq!(empty.phase, InitialEnrollmentPhase::Unused);
    assert!(empty.can_open);
    assert!(!empty.has_admission_history);
    assert!(empty.invitation.is_none());
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    assert!(
        store
            .initial_enrollment_status(owner, f.other_workspace, NOW)
            .is_err()
    );
    store.set_archived(owner, f.workspace, true, NOW).unwrap();
    let archived = observe(&mut f, owner, NOW).unwrap();
    assert!(archived.workspace_archived);
    assert!(!archived.can_open);
    assert_eq!(archived.phase, InitialEnrollmentPhase::Unused);
    AccessStore::new(&mut f.connection)
        .unwrap()
        .set_archived(owner, f.workspace, false, NOW)
        .unwrap();
    assert!(observe(&mut f, owner, NOW).unwrap().can_open);
}
#[test]
fn initial_window_status_never_infers_eligibility_from_empty_inventory() {
    let mut f = fixture();
    let owner = Actor::Admin(f.owner);
    let client = enroll(&mut f, DataRole::Read);
    AccessStore::new(&mut f.connection)
        .unwrap()
        .revoke_grant(owner, client.grant_id, NOW)
        .unwrap();
    assert_eq!(rows(&f.connection, "ccp_invitations"), 0);
    let state = observe(&mut f, owner, NOW).unwrap();
    assert_eq!(state.phase, InitialEnrollmentPhase::Unused);
    assert!(state.has_admission_history);
    assert!(!state.can_open);
    AccessStore::new(&mut f.connection)
        .unwrap()
        .close_initial_enrollment(owner, f.workspace, NOW)
        .unwrap();
    let closed = observe(&mut f, owner, NOW).unwrap();
    assert_eq!(closed.phase, InitialEnrollmentPhase::Closed);
    assert!(closed.invitation.is_none());
    assert_eq!(closed.closed_at, Some(NOW));
    assert!(!closed.can_open);
}
#[test]
fn initial_window_status_tracks_open_capacity_and_terminal_expiry() {
    let mut f = fixture();
    let owner = Actor::Admin(f.owner);
    let token = open(&mut f);
    let current = observe(&mut f, owner, NOW).unwrap();
    assert_eq!(current.phase, InitialEnrollmentPhase::Open);
    assert!(!current.can_open);
    assert_eq!(current.invitation.unwrap().id, token.id);
    let closed = observe(&mut f, owner, EXPIRES).unwrap();
    assert_eq!(closed.phase, InitialEnrollmentPhase::Closed);
    assert!(closed.invitation.unwrap().expired);
    assert_eq!(
        observe(&mut f, owner, NOW).unwrap().phase,
        InitialEnrollmentPhase::Closed
    );
    let mut other = fixture();
    let owner = Actor::Admin(other.owner);
    let token = open(&mut other);
    other
        .connection
        .execute(
            "UPDATE ccp_invitations SET uses=2 WHERE id=?1",
            [token.id.to_string()],
        )
        .unwrap();
    let closed = observe(&mut other, owner, NOW).unwrap();
    assert_eq!(closed.phase, InitialEnrollmentPhase::Closed);
    assert_eq!(closed.invitation.unwrap().uses, 2);
}
#[test]
fn initial_window_status_rolls_back_closure_on_failure_and_rejects_unauthorized_refresh() {
    let mut f = fixture();
    let owner = Actor::Admin(f.owner);
    let global = Actor::Admin(f.global);
    let _token = open(&mut f);
    assert!(observe(&mut f, global, EXPIRES).is_err());
    assert_eq!(rows(&f.connection, "ccp_invitation_closures"), 0);
    f.connection.execute_batch("CREATE TRIGGER stop_window_status BEFORE INSERT ON ccp_enrollment_events WHEN NEW.event='deadline_close' BEGIN SELECT RAISE(ABORT,'injected window failure'); END;").unwrap();
    assert!(observe(&mut f, owner, EXPIRES).is_err());
    assert_eq!(rows(&f.connection, "ccp_invitation_closures"), 0);
    assert_eq!(
        observe(&mut f, owner, NOW).unwrap().phase,
        InitialEnrollmentPhase::Open
    );
    f.connection
        .execute_batch("DROP TRIGGER stop_window_status")
        .unwrap();
    assert_eq!(
        observe(&mut f, owner, EXPIRES).unwrap().phase,
        InitialEnrollmentPhase::Closed
    );
}
