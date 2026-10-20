// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

fn managed_key(f: &mut Fixture, principal: Uuid, scope: AdminKeyScope) -> IssuedAdminKey {
    AccessStore::new(&mut f.connection)
        .unwrap()
        .issue_admin_key(
            Actor::Admin(principal),
            &AdminKeyRequest {
                principal_id: principal,
                scope,
                name: "metadata key".into(),
                expires_at: None,
            },
            NOW,
        )
        .unwrap()
}

#[test]
fn administrative_metadata_never_uses_client_grants_or_another_key_scope() {
    let mut f = fixture();
    let owner = f.owner;
    let workspace = f.workspace;
    let global = f.global;
    let scoped = managed_key(&mut f, owner, AdminKeyScope::Workspace(workspace));
    let instance = managed_key(&mut f, global, AdminKeyScope::Instance);
    let member = enroll(&mut f, DataRole::ReadWrite);
    {
        let mut store = AccessStore::new(&mut f.connection).unwrap();
        for actor in [
            Actor::Credential(member.credential_id),
            Actor::Admin(f.other_owner),
        ] {
            assert!(
                store
                    .workspace_administration(actor, workspace, NOW)
                    .is_err()
            );
            assert!(
                store
                    .workspace_grants_page(actor, workspace, None, 10, NOW)
                    .is_err()
            );
        }
        let info = store
            .workspace_administration(Actor::AdminKey(scoped.id), workspace, NOW)
            .unwrap();
        assert_eq!(info.owner_id, Some(owner));
        assert!(!info.workspace.archived);
        assert!(
            store
                .workspace_administration(Actor::AdminKey(scoped.id), f.other_workspace, NOW)
                .is_err()
        );
        let other = store
            .workspace_administration(Actor::AdminKey(instance.id), f.other_workspace, NOW)
            .unwrap();
        assert_eq!(other.owner_id, Some(f.other_owner));
        store
            .set_archived(Actor::AdminKey(scoped.id), workspace, true, NOW)
            .unwrap();
        assert!(
            store
                .workspace_administration(Actor::AdminKey(scoped.id), workspace, NOW)
                .unwrap()
                .workspace
                .archived
        );
        assert_eq!(
            store
                .workspace_grants_page(Actor::AdminKey(instance.id), workspace, None, 10, NOW)
                .unwrap()
                .grants
                .len(),
            1
        );
        assert!(
            store
                .workspace_administration(Actor::AdminKey(scoped.id), workspace, -1)
                .is_err()
        );
    }
    f.connection
        .execute(
            "UPDATE ccp_admin_keys SET revoked_at=?1 WHERE id=?2",
            params![NOW, scoped.id.to_string()],
        )
        .unwrap();
    assert!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .workspace_grants_page(Actor::AdminKey(scoped.id), workspace, None, 10, NOW)
            .is_err()
    );
    f.connection
        .execute(
            "DELETE FROM ccp_server_admins WHERE admin_id=?1",
            [global.to_string()],
        )
        .unwrap();
    assert!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .workspace_administration(Actor::AdminKey(instance.id), workspace, NOW)
            .is_err()
    );
    f.connection
        .execute(
            "UPDATE ccp_workspaces SET state='purged' WHERE id=?1",
            [workspace.to_string()],
        )
        .unwrap();
    assert!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .workspace_administration(Actor::Admin(owner), workspace, NOW)
            .is_err()
    );
}

#[test]
fn grant_pages_are_bounded_stable_and_retain_revoked_and_disabled_metadata() {
    let mut f = fixture();
    let owner = f.owner;
    let workspace = f.workspace;
    let key = managed_key(&mut f, owner, AdminKeyScope::Workspace(workspace));
    let actor = Actor::AdminKey(key.id);
    let mut enrolled = Vec::new();
    for _ in 0..7 {
        enrolled.push(enroll(&mut f, DataRole::Read));
    }
    {
        let mut store = AccessStore::new(&mut f.connection).unwrap();
        store
            .revoke_grant(actor, enrolled[0].grant_id, NOW)
            .unwrap();
        store
            .disable_client(Actor::Admin(f.global), enrolled[1].client_id, NOW)
            .unwrap();
    }
    let mut grants = Vec::new();
    let mut after = None;
    loop {
        let page = AccessStore::new(&mut f.connection)
            .unwrap()
            .workspace_grants_page(actor, workspace, after, 2, NOW)
            .unwrap();
        assert!(page.grants.len() <= 2);
        grants.extend(page.grants);
        match page.next_cursor {
            Some(id) => after = Some(id),
            None => break,
        }
    }
    let ids: Vec<_> = grants.iter().map(|g| g.id).collect();
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(ids, sorted);
    sorted.dedup();
    assert_eq!(sorted.len(), 7);
    assert_eq!(
        grants
            .iter()
            .find(|g| g.id == enrolled[0].grant_id)
            .unwrap()
            .revoked_at,
        Some(NOW)
    );
    assert!(
        grants
            .iter()
            .find(|g| g.id == enrolled[1].grant_id)
            .unwrap()
            .client_disabled
    );
    assert!(grants.iter().all(|g| g.role == DataRole::Read));
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    for limit in [0, 501, usize::MAX] {
        assert!(
            store
                .workspace_grants_page(actor, workspace, None, limit, NOW)
                .is_err()
        );
    }
    assert!(
        store
            .workspace_grants_page(actor, f.other_workspace, Some(Uuid::nil()), 1, NOW)
            .is_err()
    );
    assert!(
        store
            .workspace_grants_page(actor, workspace, Some(Uuid::max()), 1, NOW)
            .unwrap()
            .grants
            .is_empty()
    );
}
