// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

fn snapshot(f: &mut Fixture, credential: Uuid, workspace: Option<Uuid>) -> GuidanceRead {
    AccessStore::new(&mut f.connection)
        .unwrap()
        .client_guidance(Actor::Credential(credential), workspace, NOW)
        .unwrap()
}

#[test]
fn delivery_context_is_client_only_and_matches_the_guidance_read_snapshot() {
    let mut f = fixture();
    let client = enroll(&mut f, DataRole::Read);
    let workspace = f.workspace;
    let result = snapshot(&mut f, client.credential_id, Some(workspace));
    assert_eq!(result.context.client_id, client.client_id);
    assert_eq!(result.context.credential_id, client.credential_id);
    assert_eq!(result.context.workspace_grant_id, Some(client.grant_id));
    assert_eq!(
        result.context.instance_id,
        AccessStore::new(&mut f.connection)
            .unwrap()
            .instance_id()
            .unwrap()
    );
    assert_eq!(
        result.guidance,
        read(
            &mut f,
            Actor::Credential(client.credential_id),
            Some(workspace)
        )
    );
    let global = snapshot(&mut f, client.credential_id, None);
    assert_eq!(global.context.workspace_grant_id, None);
    assert!(global.guidance.workspace.is_none());
    let owner = Actor::Admin(f.owner);
    let error = AccessStore::new(&mut f.connection)
        .unwrap()
        .client_guidance(owner, Some(workspace), NOW)
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<ContentError>(),
        Some(&ContentError::Forbidden)
    );
    AccessStore::new(&mut f.connection)
        .unwrap()
        .set_archived(owner, workspace, true, NOW)
        .unwrap();
    assert_eq!(
        snapshot(&mut f, client.credential_id, Some(workspace)),
        result
    );
}

#[test]
fn re_admission_changes_delivery_grant_without_changing_credential_or_guidance_revision() {
    let mut f = fixture();
    let client = enroll(&mut f, DataRole::Read);
    let workspace = f.workspace;
    let old = snapshot(&mut f, client.credential_id, Some(workspace));
    let owner = Actor::Admin(f.owner);
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    store.revoke_grant(owner, client.grant_id, NOW).unwrap();
    assert!(
        store
            .client_guidance(
                Actor::Credential(client.credential_id),
                Some(workspace),
                NOW
            )
            .is_err()
    );
    assert!(
        store
            .client_guidance(Actor::Credential(client.credential_id), None, NOW)
            .unwrap()
            .context
            .workspace_grant_id
            .is_none()
    );
    let grant = store
        .admit_existing_client(
            owner,
            Admission {
                workspace_id: workspace,
                presented_credential: client.credential_id,
                role: DataRole::Read,
            },
            NOW,
        )
        .unwrap();
    let fresh = snapshot(&mut f, client.credential_id, Some(workspace));
    assert_ne!(grant, client.grant_id);
    assert_eq!(fresh.context.workspace_grant_id, Some(grant));
    assert_eq!(fresh.context.credential_id, old.context.credential_id);
    assert_eq!(fresh.guidance, old.guidance);
}

#[test]
fn delivery_context_never_ignores_binding_revocation_foreign_scope_expiry_or_disable() {
    let mut f = fixture();
    let client = enroll(&mut f, DataRole::Read);
    let actor = Actor::Credential(client.credential_id);
    assert!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .client_guidance(actor, Some(f.other_workspace), NOW)
            .is_err()
    );
    let token = AccessStore::new(&mut f.connection)
        .unwrap()
        .issue_recovery_invitation(
            Actor::Admin(f.owner),
            RecoveryInvitationRequest {
                client_id: client.client_id,
                scope: RecoveryScope::Workspace(f.workspace),
                expires_at: EXPIRES,
            },
            NOW,
        )
        .unwrap();
    f.connection.execute("INSERT INTO ccp_binding_revocations(credential_id,grant_id,revoked_at,recovery_invitation_id) VALUES(?1,?2,?3,?4)",params![client.credential_id.to_string(),client.grant_id.to_string(),NOW,token.id.to_string()]).unwrap();
    assert!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .client_guidance(actor, Some(f.workspace), NOW)
            .is_err()
    );
    assert!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .client_guidance(actor, None, EXPIRES + 1)
            .is_err()
    );
    f.connection
        .execute(
            "UPDATE ccp_clients SET disabled=1 WHERE id=?1",
            [client.client_id.to_string()],
        )
        .unwrap();
    assert!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .client_guidance(actor, None, NOW)
            .is_err()
    );
}
