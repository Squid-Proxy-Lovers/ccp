// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;
fn setup() -> (Fixture, Enrollment) {
    let mut f = fixture();
    let old = enroll(&mut f, DataRole::ReadWrite);
    AccessStore::new(&mut f.connection)
        .unwrap()
        .admit_existing_client(
            Actor::Admin(f.other_owner),
            Admission {
                workspace_id: f.other_workspace,
                presented_credential: old.credential_id,
                role: DataRole::Read,
            },
            NOW,
        )
        .unwrap();
    (f, old)
}
fn issue(f: &mut Fixture, client: Uuid, scope: RecoveryScope) -> IssuedRecoveryInvitation {
    let actor = match scope {
        RecoveryScope::Instance => Actor::Admin(f.global),
        RecoveryScope::Workspace(id) if id == f.workspace => Actor::Admin(f.owner),
        RecoveryScope::Workspace(_) => Actor::Admin(f.other_owner),
    };
    AccessStore::new(&mut f.connection)
        .unwrap()
        .issue_recovery_invitation(
            actor,
            RecoveryInvitationRequest {
                client_id: client,
                scope,
                expires_at: EXPIRES,
            },
            NOW,
        )
        .unwrap()
}
fn page(
    f: &mut Fixture,
    actor: Actor,
    client: Uuid,
    scope: RecoveryScope,
    after: Option<Uuid>,
    limit: usize,
    now: i64,
) -> anyhow::Result<RecoveryInvitationPage> {
    AccessStore::new(&mut f.connection)?.recovery_invitations_page(
        actor,
        RecoveryPageRequest {
            client_id: client,
            scope,
            after,
            limit,
        },
        now,
    )
}
#[test]
fn recovery_pages_exact_scope_and_client_never_union_global_and_workspace_records() {
    let (mut f, client) = setup();
    let scope = RecoveryScope::Workspace(f.workspace);
    let a = issue(&mut f, client.client_id, scope);
    let b = issue(&mut f, client.client_id, scope);
    let other_scope = RecoveryScope::Workspace(f.other_workspace);
    let foreign = issue(&mut f, client.client_id, other_scope);
    let global = issue(&mut f, client.client_id, RecoveryScope::Instance);
    let other_client = enroll(&mut f, DataRole::Read);
    let _other = issue(&mut f, other_client.client_id, RecoveryScope::Instance);
    let owner = Actor::Admin(f.owner);
    let mut expected = vec![a.id, b.id];
    expected.sort();
    let mut found = Vec::new();
    let mut after = None;
    loop {
        let rows = page(&mut f, owner, client.client_id, scope, after, 1, NOW).unwrap();
        assert_eq!(rows.invitations.len(), 1);
        let item = rows.invitations.first().unwrap();
        assert_ne!(item.id, foreign.id);
        assert_ne!(item.id, global.id);
        let status = AccessStore::new(&mut f.connection)
            .unwrap()
            .recovery_invitation_status(owner, item.id, NOW)
            .unwrap();
        assert_eq!(item.status, status);
        found.push(item.id);
        if rows.next_cursor.is_none() {
            break;
        }
        assert_eq!(rows.next_cursor, Some(item.id));
        after = rows.next_cursor;
    }
    assert_eq!(found, expected);
    let global_actor = Actor::Admin(f.global);
    let result = page(
        &mut f,
        global_actor,
        client.client_id,
        RecoveryScope::Instance,
        None,
        500,
        NOW,
    )
    .unwrap();
    assert_eq!(result.invitations.len(), 1);
    assert_eq!(result.invitations[0].id, global.id);
    let empty = page(&mut f, owner, Uuid::new_v4(), scope, None, 100, NOW).unwrap();
    assert!(empty.invitations.is_empty());
    assert!(
        page(
            &mut f,
            owner,
            client.client_id,
            scope,
            found.last().copied(),
            1,
            NOW
        )
        .unwrap()
        .invitations
        .is_empty()
    );
}
#[test]
fn recovery_pages_enforce_current_actor_key_scope_bounds_and_client_credential_denial() {
    let (mut f, client) = setup();
    let scope = RecoveryScope::Workspace(f.workspace);
    let foreign_scope = RecoveryScope::Workspace(f.other_workspace);
    let _token = issue(&mut f, client.client_id, scope);
    let owner = Actor::Admin(f.owner);
    let global = Actor::Admin(f.global);
    for limit in [0, 501, usize::MAX] {
        assert!(page(&mut f, owner, client.client_id, scope, None, limit, NOW).is_err());
    }
    assert!(page(&mut f, global, client.client_id, scope, None, 1, NOW).is_err());
    assert!(
        page(
            &mut f,
            owner,
            client.client_id,
            RecoveryScope::Instance,
            None,
            1,
            NOW
        )
        .is_err()
    );
    assert!(page(&mut f, owner, client.client_id, foreign_scope, None, 1, NOW).is_err());
    assert!(
        page(
            &mut f,
            Actor::Credential(client.credential_id),
            client.client_id,
            scope,
            None,
            1,
            NOW
        )
        .is_err()
    );
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    let key = store
        .issue_admin_key(
            owner,
            &AdminKeyRequest {
                principal_id: f.admin,
                scope: AdminKeyScope::Workspace(f.workspace),
                name: "page key".into(),
                expires_at: Some(NOW + 10),
            },
            NOW,
        )
        .unwrap();
    assert!(
        store
            .recovery_invitations_page(
                Actor::AdminKey(key.id),
                RecoveryPageRequest {
                    client_id: client.client_id,
                    scope,
                    after: None,
                    limit: 1
                },
                NOW
            )
            .is_ok()
    );
    assert!(
        store
            .recovery_invitations_page(
                Actor::AdminKey(key.id),
                RecoveryPageRequest {
                    client_id: client.client_id,
                    scope,
                    after: None,
                    limit: 1
                },
                NOW + 10
            )
            .is_err()
    );
    store
        .revoke_admin_key(owner, AdminKeyScope::Workspace(f.workspace), key.id, NOW)
        .unwrap();
    assert!(
        page(
            &mut f,
            Actor::AdminKey(key.id),
            client.client_id,
            scope,
            None,
            1,
            NOW
        )
        .is_err()
    );
}
#[test]
fn recovery_pages_only_observe_returned_expiry_and_roll_back_on_later_failure() {
    let (mut f, client) = setup();
    let scope = RecoveryScope::Workspace(f.workspace);
    let owner = Actor::Admin(f.owner);
    let global = Actor::Admin(f.global);
    let a = issue(&mut f, client.client_id, scope);
    let b = issue(&mut f, client.client_id, scope);
    assert!(page(&mut f, global, client.client_id, scope, None, 2, EXPIRES).is_err());
    let expired = |f: &Fixture| {
        f.connection
            .query_row(
                "SELECT count(*) FROM ccp_recovery_invitations WHERE expired_at IS NOT NULL",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
    };
    assert_eq!(expired(&f), 0);
    let last = a.id.max(b.id);
    f.connection.execute_batch(&format!("CREATE TRIGGER stop_recovery_page BEFORE UPDATE OF expired_at ON ccp_recovery_invitations WHEN NEW.id='{last}' BEGIN SELECT RAISE(ABORT,'injected recovery page failure'); END;")).unwrap();
    assert!(page(&mut f, owner, client.client_id, scope, None, 2, EXPIRES).is_err());
    assert_eq!(expired(&f), 0);
    f.connection
        .execute_batch("DROP TRIGGER stop_recovery_page")
        .unwrap();
    let first = page(&mut f, owner, client.client_id, scope, None, 1, EXPIRES).unwrap();
    assert!(first.invitations[0].status.expired);
    assert_eq!(expired(&f), 1);
    let rollback = page(&mut f, owner, client.client_id, scope, None, 2, NOW).unwrap();
    assert_eq!(
        rollback
            .invitations
            .iter()
            .filter(|i| i.status.expired)
            .count(),
        1
    );
}
#[test]
fn recovery_pages_keep_archived_disabled_and_revoked_history_without_claiming_new_access() {
    let (mut f, client) = setup();
    let scope = RecoveryScope::Workspace(f.workspace);
    let owner = Actor::Admin(f.owner);
    let token = issue(&mut f, client.client_id, scope);
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    store.set_archived(owner, f.workspace, true, NOW).unwrap();
    store
        .revoke_recovery_invitation(owner, token.id, NOW)
        .unwrap();
    f.connection
        .execute(
            "UPDATE ccp_clients SET disabled=1 WHERE id=?1",
            [client.client_id.to_string()],
        )
        .unwrap();
    let metadata = page(&mut f, owner, client.client_id, scope, None, 100, NOW).unwrap();
    assert_eq!(metadata.invitations.len(), 1);
    assert!(metadata.invitations[0].status.revoked);
    assert!(metadata.invitations[0].status.issuer_authorized);
    assert!(!metadata.invitations[0].status.redeemed);
    assert!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .issue_recovery_invitation(
                owner,
                RecoveryInvitationRequest {
                    client_id: client.client_id,
                    scope,
                    expires_at: EXPIRES
                },
                NOW
            )
            .is_err()
    );
}
