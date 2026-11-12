// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;

fn issue(f: &mut Fixture, actor: Actor) -> IssuedInvitation {
    AccessStore::new(&mut f.connection)
        .unwrap()
        .issue_individual_invitation(
            actor,
            IndividualInvitationRequest {
                workspace_id: f.workspace,
                role: DataRole::Read,
                expires_at: EXPIRES,
            },
            NOW,
        )
        .unwrap()
}
fn page(
    f: &mut Fixture,
    actor: Actor,
    after: Option<Uuid>,
    limit: usize,
    now: i64,
) -> anyhow::Result<InvitationPage> {
    AccessStore::new(&mut f.connection)?.workspace_invitations_page(
        actor,
        f.workspace,
        after,
        limit,
        now,
    )
}

#[test]
fn invitation_pages_order_uuid_and_match_status_without_cross_workspace_rows() {
    let mut f = fixture();
    let actor = Actor::Admin(f.admin);
    let group = AccessStore::new(&mut f.connection)
        .unwrap()
        .issue_initial_invitation(
            actor,
            InitialInvitationRequest {
                workspace_id: f.workspace,
                role: DataRole::ReadWrite,
                max_clients: Some(2),
                expires_at: Some(EXPIRES),
            },
            NOW,
        )
        .unwrap();
    AccessStore::new(&mut f.connection)
        .unwrap()
        .close_initial_enrollment(actor, f.workspace, NOW)
        .unwrap();
    let a = issue(&mut f, actor);
    let b = issue(&mut f, actor);
    let foreign = AccessStore::new(&mut f.connection)
        .unwrap()
        .issue_individual_invitation(
            Actor::Admin(f.other_owner),
            IndividualInvitationRequest {
                workspace_id: f.other_workspace,
                role: DataRole::Read,
                expires_at: EXPIRES,
            },
            NOW,
        )
        .unwrap();
    let mut expected = vec![group.id, a.id, b.id];
    expected.sort();
    let mut after = None;
    let mut found = Vec::new();
    loop {
        let p = page(&mut f, actor, after, 1, NOW).unwrap();
        assert_eq!(p.invitations.len(), 1);
        let item = p.invitations.first().unwrap();
        assert_ne!(item.status.id, foreign.id);
        let individual = AccessStore::new(&mut f.connection)
            .unwrap()
            .invitation_status(actor, f.workspace, item.status.id, NOW)
            .unwrap();
        assert_eq!(item.status, individual);
        assert_eq!(
            item.kind,
            if item.status.id == group.id {
                InvitationKind::InitialGroup
            } else {
                InvitationKind::Individual
            }
        );
        assert_eq!(
            item.role,
            if item.status.id == group.id {
                DataRole::ReadWrite
            } else {
                DataRole::Read
            }
        );
        found.push(item.status.id);
        if let Some(cursor) = p.next_cursor {
            assert_eq!(cursor, item.status.id);
            after = Some(cursor);
        } else {
            break;
        }
    }
    assert_eq!(found, expected);
    let empty = page(&mut f, actor, found.last().copied(), 500, NOW).unwrap();
    assert!(empty.invitations.is_empty());
    assert!(empty.next_cursor.is_none());
}

#[test]
fn invitation_pages_recheck_scope_bounds_and_current_key_authority() {
    let mut f = fixture();
    let actor = Actor::Admin(f.owner);
    assert!(
        page(&mut f, actor, None, 100, NOW)
            .unwrap()
            .invitations
            .is_empty()
    );
    for limit in [0, 501, usize::MAX] {
        assert!(page(&mut f, actor, None, limit, NOW).is_err());
    }
    let token = issue(&mut f, actor);
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    let key = store
        .issue_admin_key(
            actor,
            &AdminKeyRequest {
                principal_id: f.admin,
                scope: AdminKeyScope::Workspace(f.workspace),
                name: "page key".into(),
                expires_at: Some(NOW + 10),
            },
            NOW,
        )
        .unwrap();
    let key_actor = Actor::AdminKey(key.id);
    assert_eq!(
        store
            .workspace_invitations_page(key_actor, f.workspace, None, 1, NOW)
            .unwrap()
            .invitations[0]
            .status
            .id,
        token.id
    );
    assert!(
        store
            .workspace_invitations_page(key_actor, f.other_workspace, None, 1, NOW)
            .is_err()
    );
    assert!(
        store
            .workspace_invitations_page(Actor::Admin(f.global), f.workspace, None, 1, NOW)
            .is_err()
    );
    assert!(
        store
            .workspace_invitations_page(key_actor, f.workspace, None, 1, NOW + 10)
            .is_err()
    );
    store
        .revoke_admin_key(actor, AdminKeyScope::Workspace(f.workspace), key.id, NOW)
        .unwrap();
    assert!(
        store
            .workspace_invitations_page(key_actor, f.workspace, None, 1, NOW)
            .is_err()
    );
    let enrolled = enroll(&mut f, DataRole::Read);
    assert!(
        page(
            &mut f,
            Actor::Credential(enrolled.credential_id),
            None,
            1,
            NOW
        )
        .is_err()
    );
}

#[test]
fn invitation_pages_only_observe_returned_expiry_and_keep_it_terminal_on_clock_rollback() {
    let mut f = fixture();
    let actor = Actor::Admin(f.admin);
    let global = Actor::Admin(f.global);
    let a = issue(&mut f, actor);
    let b = issue(&mut f, actor);
    assert!(page(&mut f, global, None, 1, EXPIRES).is_err());
    assert_eq!(rows(&f.connection, "ccp_invitation_closures"), 0);
    let first = page(&mut f, actor, None, 1, EXPIRES).unwrap();
    assert!(first.invitations[0].status.expired);
    assert_eq!(rows(&f.connection, "ccp_invitation_closures"), 1);
    let lookahead = if first.invitations[0].status.id == a.id {
        b.id
    } else {
        a.id
    };
    let untouched: bool = f
        .connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM ccp_invitation_closures WHERE invitation_id=?1)",
            [lookahead.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!untouched);
    let rollback = page(&mut f, actor, None, 2, NOW).unwrap();
    assert_eq!(
        rollback
            .invitations
            .iter()
            .filter(|i| i.status.expired)
            .count(),
        1
    );
    let all = page(&mut f, actor, None, 2, EXPIRES).unwrap();
    assert!(all.invitations.iter().all(|i| i.status.expired));
    assert_eq!(rows(&f.connection, "ccp_invitation_closures"), 2);
}

#[test]
fn invitation_pages_roll_back_all_observed_closures_if_later_row_fails() {
    let mut f = fixture();
    let actor = Actor::Admin(f.admin);
    let a = issue(&mut f, actor);
    let b = issue(&mut f, actor);
    let last = a.id.max(b.id);
    f.connection.execute_batch(&format!("CREATE TRIGGER stop_page_expiry BEFORE INSERT ON ccp_invitation_closures WHEN NEW.invitation_id='{last}' BEGIN SELECT RAISE(ABORT,'injected page failure'); END;")).unwrap();
    assert!(page(&mut f, actor, None, 2, EXPIRES).is_err());
    assert_eq!(rows(&f.connection, "ccp_invitation_closures"), 0);
    f.connection
        .execute_batch("DROP TRIGGER stop_page_expiry")
        .unwrap();
    assert!(
        page(&mut f, actor, None, 2, EXPIRES)
            .unwrap()
            .invitations
            .iter()
            .all(|i| i.status.expired)
    );
}

#[test]
fn invitation_pages_track_revocation_readmission_and_current_issuer_state_while_archived() {
    let mut f = fixture();
    let owner = Actor::Admin(f.owner);
    let admin = Actor::Admin(f.admin);
    let admitted = enroll(&mut f, DataRole::Read);
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    store.revoke_grant(admin, admitted.grant_id, NOW).unwrap();
    let readmission = store
        .issue_readmission_invitation(
            admin,
            ReadmissionInvitationRequest {
                workspace_id: f.workspace,
                client_id: admitted.client_id,
                role: DataRole::ReadWrite,
                expires_at: EXPIRES,
            },
            NOW,
        )
        .unwrap();
    let key = store
        .issue_admin_key(
            owner,
            &AdminKeyRequest {
                principal_id: f.admin,
                scope: AdminKeyScope::Workspace(f.workspace),
                name: "issuer".into(),
                expires_at: Some(NOW + 10),
            },
            NOW,
        )
        .unwrap();
    let token = issue(&mut f, Actor::AdminKey(key.id));
    let p = page(&mut f, owner, None, 100, NOW).unwrap();
    let target = p
        .invitations
        .iter()
        .find(|i| i.status.id == readmission.id)
        .unwrap();
    assert_eq!(target.status.target_client_id, Some(admitted.client_id));
    assert_eq!(target.kind, InvitationKind::Individual);
    assert_eq!(target.role, DataRole::ReadWrite);
    assert!(
        p.invitations
            .iter()
            .find(|i| i.status.id == token.id)
            .unwrap()
            .status
            .issuer_authorized
    );
    assert!(
        !page(&mut f, owner, None, 100, NOW + 10)
            .unwrap()
            .invitations
            .iter()
            .find(|i| i.status.id == token.id)
            .unwrap()
            .status
            .issuer_authorized
    );
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    store
        .revoke_invitation(owner, f.workspace, token.id, NOW)
        .unwrap();
    store.set_archived(owner, f.workspace, true, NOW).unwrap();
    let archived = page(&mut f, owner, None, 100, NOW).unwrap();
    assert!(
        archived
            .invitations
            .iter()
            .find(|i| i.status.id == token.id)
            .unwrap()
            .status
            .revoked
    );
    assert!(
        archived
            .invitations
            .iter()
            .all(|i| !i.status.issuer_authorized)
    );
}
