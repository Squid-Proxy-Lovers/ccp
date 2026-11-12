// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

fn key(f: &mut Fixture, principal: Uuid, scope: AdminKeyScope) -> IssuedAdminKey {
    let actor = match scope {
        AdminKeyScope::Instance => Actor::Admin(f.global),
        AdminKeyScope::Workspace(id) if id == f.workspace => Actor::Admin(f.owner),
        AdminKeyScope::Workspace(_) => Actor::Admin(f.other_owner),
    };
    AccessStore::new(&mut f.connection)
        .unwrap()
        .issue_admin_key(
            actor,
            &AdminKeyRequest {
                principal_id: principal,
                scope,
                name: "team key".into(),
                expires_at: None,
            },
            NOW,
        )
        .unwrap()
}
fn appoint(
    f: &mut Fixture,
    actor: Actor,
    principal: PrincipalSelection,
) -> anyhow::Result<IssuedAdminKey> {
    AccessStore::new(&mut f.connection)?.appoint_workspace_admin_with_key(
        actor,
        f.workspace,
        &AdminAppointment {
            principal,
            key_expires_at: None,
        },
        NOW,
    )
}
fn counts(f: &Fixture) -> Vec<i64> {
    [
        "ccp_admin_principals",
        "ccp_workspace_admins",
        "ccp_admin_keys",
        "ccp_access_audit",
    ]
    .iter()
    .map(|t| rows(&f.connection, t))
    .collect()
}
fn transfer(f: &mut Fixture, actor: Actor, target: Uuid) -> anyhow::Result<()> {
    AccessStore::new(&mut f.connection)?.transfer_workspace_owner(actor, f.workspace, target, NOW)
}

#[test]
fn appointment_delivers_atomic_scoped_key_and_never_borrows_roles() {
    let mut f = fixture();
    let owner = f.owner;
    let global = f.global;
    let workspace = f.workspace;
    let admin = f.admin;
    let ownerkey = key(&mut f, owner, AdminKeyScope::Workspace(workspace));
    let globalkey = key(&mut f, global, AdminKeyScope::Instance);
    let adminkey = key(&mut f, admin, AdminKeyScope::Workspace(workspace));
    for actor in [Actor::AdminKey(globalkey.id), Actor::AdminKey(adminkey.id)] {
        assert!(appoint(&mut f, actor, InitialOwner::NewPrincipal("no".into())).is_err());
    }
    let before = counts(&f);
    let issued = appoint(
        &mut f,
        Actor::AdminKey(ownerkey.id),
        InitialOwner::NewPrincipal(" newcomer ".into()),
    )
    .unwrap();
    let after = counts(&f);
    assert_eq!(after[0], before[0] + 1);
    assert_eq!(after[1], before[1] + 1);
    assert_eq!(after[2], before[2] + 1);
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    assert_eq!(
        store.authenticate_admin_key(issued.secret(), NOW).unwrap(),
        Actor::AdminKey(issued.id)
    );
    let before = counts(&f);
    assert!(
        appoint(
            &mut f,
            Actor::AdminKey(ownerkey.id),
            InitialOwner::Existing(issued.principal_id)
        )
        .is_err()
    );
    assert_eq!(counts(&f), before);
    assert!(
        authorize(
            &f.connection,
            Actor::AdminKey(issued.id),
            Action::AdmitClient,
            Some(workspace),
            NOW
        )
        .is_ok()
    );
    assert!(
        authorize(
            &f.connection,
            Actor::AdminKey(issued.id),
            Action::ManageWorkspaceKeys,
            Some(workspace),
            NOW
        )
        .is_err()
    );
    let audit_key:String=f.connection.query_row("SELECT admin_key_id FROM ccp_access_audit WHERE action='appoint_admin' AND target_id=?1",[issued.principal_id.to_string()],|r|r.get(0)).unwrap();
    assert_eq!(audit_key, ownerkey.id.to_string());
    let verifier: Vec<u8> = f
        .connection
        .query_row(
            "SELECT token_hash FROM ccp_admin_keys WHERE id=?1",
            [issued.id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(verifier.len(), 32);
    assert_ne!(verifier, issued.secret().as_bytes());
}

#[test]
fn revocation_and_reappointment_cannot_resurrect_keys_or_their_invitations() {
    let mut f = fixture();
    let admin = f.admin;
    let owner = f.owner;
    let workspace = f.workspace;
    let other = f.other_workspace;
    let old = key(&mut f, admin, AdminKeyScope::Workspace(workspace));
    let invitation = AccessStore::new(&mut f.connection)
        .unwrap()
        .issue_individual_invitation(
            Actor::AdminKey(old.id),
            IndividualInvitationRequest {
                workspace_id: workspace,
                role: DataRole::Read,
                expires_at: NOW + 100,
            },
            NOW,
        )
        .unwrap();
    let bkey = AccessStore::new(&mut f.connection)
        .unwrap()
        .appoint_workspace_admin_with_key(
            Actor::Admin(f.other_owner),
            other,
            &AdminAppointment {
                principal: InitialOwner::Existing(admin),
                key_expires_at: None,
            },
            NOW,
        )
        .unwrap();
    AccessStore::new(&mut f.connection)
        .unwrap()
        .revoke_workspace_admin(Actor::Admin(owner), workspace, admin, NOW)
        .unwrap();
    let initially_revoked: Option<i64> = f
        .connection
        .query_row(
            "SELECT revoked_at FROM ccp_admin_keys WHERE id=?1",
            [old.id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(initially_revoked, Some(NOW));
    // Simulate a retained pre-terminal-revocation key whose role was removed.
    f.connection
        .execute(
            "UPDATE ccp_admin_keys SET revoked_at=NULL WHERE id=?1",
            [old.id.to_string()],
        )
        .unwrap();
    assert!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .authenticate_admin_key(old.secret(), NOW)
            .is_err()
    );
    let fresh = appoint(&mut f, Actor::Admin(owner), InitialOwner::Existing(admin)).unwrap();
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    assert!(store.authenticate_admin_key(old.secret(), NOW).is_err());
    assert!(store.authenticate_admin_key(fresh.secret(), NOW).is_ok());
    assert!(store.authenticate_admin_key(bkey.secret(), NOW).is_ok());
    assert!(
        !store
            .invitation_status(Actor::Admin(owner), workspace, invitation.id, NOW)
            .unwrap()
            .issuer_authorized
    );
    let revoked: Option<i64> = f
        .connection
        .query_row(
            "SELECT revoked_at FROM ccp_admin_keys WHERE id=?1",
            [old.id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(revoked, Some(NOW));
}

#[test]
fn appointment_and_revoke_audit_failures_roll_back_every_role_and_key() {
    let mut f = fixture();
    let owner = f.owner;
    let admin = f.admin;
    let workspace = f.workspace;
    let old = key(&mut f, admin, AdminKeyScope::Workspace(workspace));
    let before = counts(&f);
    f.connection.execute_batch("CREATE TRIGGER fail_team BEFORE INSERT ON ccp_access_audit WHEN NEW.action='issue_admin_key' BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    assert!(
        appoint(
            &mut f,
            Actor::Admin(owner),
            InitialOwner::NewPrincipal("rollback".into())
        )
        .is_err()
    );
    assert_eq!(counts(&f), before);
    f.connection.execute_batch("DROP TRIGGER fail_team; CREATE TRIGGER fail_team BEFORE INSERT ON ccp_access_audit WHEN NEW.action='revoke_admin' BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    assert!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .revoke_workspace_admin(Actor::Admin(owner), workspace, admin, NOW)
            .is_err()
    );
    assert_eq!(counts(&f), before);
    assert!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .authenticate_admin_key(old.secret(), NOW)
            .is_ok()
    );
}

#[test]
fn transfer_requires_live_recipient_and_revokes_only_former_owner_workspace_authority() {
    let mut f = fixture();
    let owner = f.owner;
    let admin = f.admin;
    let workspace = f.workspace;
    let other = f.other_workspace;
    let ownerkey = key(&mut f, owner, AdminKeyScope::Workspace(workspace));
    assert!(transfer(&mut f, Actor::AdminKey(ownerkey.id), admin).is_err());
    let recipient = key(&mut f, admin, AdminKeyScope::Workspace(workspace));
    assert!(transfer(&mut f, Actor::AdminKey(recipient.id), owner).is_err());
    let unrelated = AccessStore::new(&mut f.connection)
        .unwrap()
        .appoint_workspace_admin_with_key(
            Actor::Admin(f.other_owner),
            other,
            &AdminAppointment {
                principal: InitialOwner::Existing(owner),
                key_expires_at: None,
            },
            NOW,
        )
        .unwrap();
    f.connection
        .execute(
            "INSERT INTO ccp_server_admins(admin_id) VALUES (?1)",
            [owner.to_string()],
        )
        .unwrap();
    let instance = key(&mut f, owner, AdminKeyScope::Instance);
    f.connection
        .execute(
            "INSERT INTO ccp_workspace_admins(workspace_id,admin_id) VALUES (?1,?2)",
            params![workspace.to_string(), owner.to_string()],
        )
        .unwrap();
    assert!(transfer(&mut f, Actor::AdminKey(instance.id), admin).is_err());
    transfer(&mut f, Actor::AdminKey(ownerkey.id), admin).unwrap();
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    assert!(
        store
            .authenticate_admin_key(ownerkey.secret(), NOW)
            .is_err()
    );
    assert!(
        store
            .authenticate_admin_key(recipient.secret(), NOW)
            .is_ok()
    );
    assert!(store.authenticate_admin_key(instance.secret(), NOW).is_ok());
    assert!(
        store
            .authenticate_admin_key(unrelated.secret(), NOW)
            .is_ok()
    );
    assert_eq!(
        store
            .workspace_administration(Actor::AdminKey(recipient.id), workspace, NOW)
            .unwrap()
            .owner_id,
        Some(admin)
    );
    assert!(
        authorize(
            &f.connection,
            Actor::AdminKey(recipient.id),
            Action::ManageWorkspaceKeys,
            Some(workspace),
            NOW
        )
        .is_ok()
    );
    assert_eq!(f.connection.query_row("SELECT COUNT(*) FROM ccp_workspace_admins WHERE workspace_id=?1 AND admin_id IN (?2,?3)",params![workspace.to_string(),owner.to_string(),admin.to_string()],|r|r.get::<_,i64>(0)).unwrap(),0);
    let actor_key: String = f
        .connection
        .query_row(
            "SELECT admin_key_id FROM ccp_access_audit WHERE action='transfer_owner'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(actor_key, ownerkey.id.to_string());
    assert!(transfer(&mut f, Actor::AdminKey(ownerkey.id), owner).is_err());
}

#[test]
fn transfer_refuses_disabled_expired_revoked_archived_or_same_owner_and_rolls_back_on_audit_failure()
 {
    let mut f = fixture();
    let owner = f.owner;
    let admin = f.admin;
    let workspace = f.workspace;
    let recipient = key(&mut f, admin, AdminKeyScope::Workspace(workspace));
    assert!(transfer(&mut f, Actor::Admin(owner), owner).is_err());
    for sql in [
        "UPDATE ccp_admin_principals SET enabled=0 WHERE id=?1",
        "UPDATE ccp_admin_keys SET expires_at=1000 WHERE admin_id=?1",
        "UPDATE ccp_admin_keys SET revoked_at=1000 WHERE admin_id=?1",
    ] {
        f.connection.execute(sql, [admin.to_string()]).unwrap();
        assert!(transfer(&mut f, Actor::Admin(owner), admin).is_err());
        f.connection
            .execute(
                "UPDATE ccp_admin_principals SET enabled=1 WHERE id=?1",
                [admin.to_string()],
            )
            .unwrap();
        f.connection
            .execute(
                "UPDATE ccp_admin_keys SET expires_at=NULL,revoked_at=NULL WHERE admin_id=?1",
                [admin.to_string()],
            )
            .unwrap();
    }
    AccessStore::new(&mut f.connection)
        .unwrap()
        .set_archived(Actor::Admin(owner), workspace, true, NOW)
        .unwrap();
    assert!(transfer(&mut f, Actor::Admin(owner), admin).is_err());
    assert!(
        appoint(
            &mut f,
            Actor::Admin(owner),
            InitialOwner::NewPrincipal("archived".into())
        )
        .is_err()
    );
    AccessStore::new(&mut f.connection)
        .unwrap()
        .set_archived(Actor::Admin(owner), workspace, false, NOW)
        .unwrap();
    let before = counts(&f);
    f.connection.execute_batch("CREATE TRIGGER fail_transfer BEFORE INSERT ON ccp_access_audit WHEN NEW.action='transfer_owner' BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    assert!(transfer(&mut f, Actor::Admin(owner), admin).is_err());
    assert_eq!(counts(&f), before);
    assert_eq!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .workspace_administration(Actor::Admin(owner), workspace, NOW)
            .unwrap()
            .owner_id,
        Some(owner)
    );
    assert!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .authenticate_admin_key(recipient.secret(), NOW)
            .is_ok()
    );
}

#[test]
fn owner_only_metadata_pages_are_bounded_secret_free_and_keep_history() {
    let mut f = fixture();
    let owner = f.owner;
    let workspace = f.workspace;
    let admin = f.admin;
    let global = f.global;
    let ownerkey = key(&mut f, owner, AdminKeyScope::Workspace(workspace));
    let adminkey = key(&mut f, admin, AdminKeyScope::Workspace(workspace));
    let globalkey = key(&mut f, global, AdminKeyScope::Instance);
    for _ in 0..3 {
        appoint(
            &mut f,
            Actor::AdminKey(ownerkey.id),
            InitialOwner::NewPrincipal("member".into()),
        )
        .unwrap();
    }
    let client = enroll(&mut f, DataRole::ReadWrite);
    {
        let mut store = AccessStore::new(&mut f.connection).unwrap();
        for actor in [
            Actor::AdminKey(adminkey.id),
            Actor::AdminKey(globalkey.id),
            Actor::Credential(client.credential_id),
        ] {
            assert!(
                store
                    .workspace_admins_page(actor, workspace, None, 10, NOW)
                    .is_err()
            );
            assert!(
                store
                    .workspace_admin_keys_page(actor, workspace, None, 10, NOW)
                    .is_err()
            );
        }
        for limit in [0, 501, usize::MAX] {
            assert!(
                store
                    .workspace_admins_page(Actor::Admin(owner), workspace, None, limit, NOW)
                    .is_err()
            );
            assert!(
                store
                    .workspace_admin_keys_page(Actor::Admin(owner), workspace, None, limit, NOW)
                    .is_err()
            );
        }
    }
    let mut after = None;
    let mut admins = Vec::new();
    loop {
        let page = AccessStore::new(&mut f.connection)
            .unwrap()
            .workspace_admins_page(Actor::Admin(owner), workspace, after, 2, NOW)
            .unwrap();
        assert!(page.admins.len() <= 2);
        admins.extend(page.admins);
        match page.next_cursor {
            Some(id) => after = Some(id),
            None => break,
        }
    }
    assert_eq!(admins.len(), 5);
    assert_eq!(admins.iter().filter(|a| a.is_owner).count(), 1);
    let ids: Vec<_> = admins.iter().map(|a| a.id).collect();
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(ids, sorted);
    sorted.dedup();
    assert_eq!(sorted.len(), 5);
    AccessStore::new(&mut f.connection)
        .unwrap()
        .revoke_workspace_admin(Actor::Admin(owner), workspace, admin, NOW)
        .unwrap();
    let page = AccessStore::new(&mut f.connection)
        .unwrap()
        .workspace_admin_keys_page(Actor::Admin(owner), workspace, None, 10, NOW)
        .unwrap();
    let revoked = page.keys.iter().find(|k| k.id == adminkey.id).unwrap();
    assert_eq!(revoked.revoked_at, Some(NOW));
    assert!(!revoked.role_active);
    assert!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .workspace_admins_page(Actor::Admin(owner), f.other_workspace, None, 10, NOW)
            .is_err()
    );
}

#[test]
fn owner_cannot_be_revoked_as_admin_and_invalid_appointments_leave_no_state() {
    let mut f = fixture();
    let owner = f.owner;
    let workspace = f.workspace;
    let other = f.other_owner;
    let ownerkey = key(&mut f, owner, AdminKeyScope::Workspace(workspace));
    f.connection
        .execute(
            "INSERT INTO ccp_workspace_admins(workspace_id,admin_id) VALUES (?1,?2)",
            params![workspace.to_string(), owner.to_string()],
        )
        .unwrap();
    let before = counts(&f);
    assert!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .revoke_workspace_admin(Actor::AdminKey(ownerkey.id), workspace, owner, NOW)
            .is_err()
    );
    assert_eq!(counts(&f), before);
    f.connection
        .execute(
            "UPDATE ccp_admin_principals SET enabled=0 WHERE id=?1",
            [other.to_string()],
        )
        .unwrap();
    assert!(appoint(&mut f, Actor::Admin(owner), InitialOwner::Existing(other)).is_err());
    assert_eq!(counts(&f), before);
    for expires in [Some(NOW), Some(-1)] {
        assert!(
            AccessStore::new(&mut f.connection)
                .unwrap()
                .appoint_workspace_admin_with_key(
                    Actor::Admin(owner),
                    workspace,
                    &AdminAppointment {
                        principal: InitialOwner::NewPrincipal("invalid".into()),
                        key_expires_at: expires
                    },
                    NOW
                )
                .is_err()
        );
        assert_eq!(counts(&f), before);
    }
    assert!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .authenticate_admin_key(ownerkey.secret(), NOW)
            .is_ok()
    );
}
