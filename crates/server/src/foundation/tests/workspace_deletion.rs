// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
fn error<T>(result: anyhow::Result<T>, expected: ContentError) {
    match result {
        Ok(_) => panic!("expected {expected:?}"),
        Err(e) => assert_eq!(e.downcast_ref::<ContentError>(), Some(&expected), "{e:#}"),
    }
}

mod boundaries;
mod cleanup;
mod persistence;

fn intent(workspace: Uuid, revision: u64) -> WorkspaceDeletionRequest {
    WorkspaceDeletionRequest {
        workspace_id: workspace,
        operation_id: Uuid::new_v4(),
        expected_revision: revision,
        confirm_workspace_id: workspace,
    }
}
fn preview(f: &mut Fixture, actor: Actor) -> WorkspaceDeletionPreview {
    AccessStore::new(&mut f.connection)
        .unwrap()
        .workspace_deletion_preview(actor, f.workspace, NOW)
        .unwrap()
}
fn archive(f: &mut Fixture) {
    AccessStore::new(&mut f.connection)
        .unwrap()
        .set_archived(Actor::Admin(f.owner), f.workspace, true, NOW)
        .unwrap();
}
fn begin(f: &mut Fixture, actor: Actor, request: &WorkspaceDeletionRequest) -> WorkspaceDeletion {
    AccessStore::new(&mut f.connection)
        .unwrap()
        .begin_workspace_deletion(actor, request, NOW)
        .unwrap()
}
fn denied(
    f: &mut Fixture,
    actor: Actor,
    request: &WorkspaceDeletionRequest,
    expected: ContentError,
) {
    error(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .begin_workspace_deletion(actor, request, NOW),
        expected,
    );
}
fn finish(f: &mut Fixture, limit: usize) -> WorkspaceDeletion {
    for _ in 0..500 {
        let result = AccessStore::new(&mut f.connection)
            .unwrap()
            .advance_workspace_deletion(f.workspace, limit, NOW + 1)
            .unwrap();
        if result.completed_at.is_some() {
            return result;
        }
    }
    panic!("deletion failed to complete");
}
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
                name: "private key label".into(),
                expires_at: None,
            },
            NOW,
        )
        .unwrap()
}

#[test]
fn deletion_requires_current_owner_or_instance_admin_and_archived_exact_confirmation() {
    let mut f = fixture();
    let reader = enroll(&mut f, DataRole::Read);
    let writer = enroll(&mut f, DataRole::ReadWrite);
    let owner = Actor::Admin(f.owner);
    denied(
        &mut f,
        owner,
        &intent(writer.grant_id, 1),
        ContentError::Forbidden,
    );
    let workspace = f.workspace;
    denied(
        &mut f,
        owner,
        &intent(workspace, 1),
        ContentError::Forbidden,
    );
    archive(&mut f);
    let global = f.global;
    let admin = f.admin;
    let other = f.other_owner;
    let global_key = key(&mut f, global, AdminKeyScope::Instance);
    let admin_key = key(&mut f, admin, AdminKeyScope::Workspace(workspace));
    let other_workspace = f.other_workspace;
    let other_key = key(&mut f, other, AdminKeyScope::Workspace(other_workspace));
    let request = intent(workspace, preview(&mut f, owner).lifecycle_revision);
    for actor in [
        Actor::Admin(admin),
        Actor::Admin(other),
        Actor::AdminKey(admin_key.id),
        Actor::AdminKey(other_key.id),
        Actor::Credential(reader.credential_id),
        Actor::Credential(writer.credential_id),
    ] {
        error(
            AccessStore::new(&mut f.connection)
                .unwrap()
                .workspace_deletion_preview(actor, workspace, NOW),
            ContentError::Forbidden,
        );
        denied(&mut f, actor, &request, ContentError::Forbidden);
    }
    let result = begin(&mut f, Actor::AdminKey(global_key.id), &request);
    assert_eq!(result.lifecycle_revision, request.expected_revision + 1);
    assert_eq!(result.completed_at, None);
    assert_eq!(rows(&f.connection, "ccp_workspace_deletions"), 1);
    for actor in [
        owner,
        Actor::Admin(global),
        Actor::Credential(writer.credential_id),
    ] {
        assert!(
            authorize(
                &f.connection,
                actor,
                Action::ReadContext,
                Some(workspace),
                NOW
            )
            .is_err()
        );
        assert!(
            AccessStore::new(&mut f.connection)
                .unwrap()
                .visible_workspaces(actor, NOW)
                .unwrap()
                .iter()
                .all(|w| w.id != workspace)
        );
    }
    assert!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .set_archived(Actor::Admin(global), workspace, false, NOW)
            .is_err()
    );
}

#[test]
fn reactivation_owner_change_and_wrong_confirmation_cannot_reuse_old_intent() {
    let mut f = fixture();
    let owner = Actor::Admin(f.owner);
    archive(&mut f);
    let p = preview(&mut f, owner);
    assert_eq!(p.name, "alpha");
    assert_eq!(p.lifecycle_revision, 2);
    let mut request = intent(f.workspace, p.lifecycle_revision);
    for revision in [0, u64::MAX, i64::MAX.unsigned_abs()] {
        request.expected_revision = revision;
        denied(&mut f, owner, &request, ContentError::InvalidInput);
    }
    request.expected_revision = p.lifecycle_revision;
    request.confirm_workspace_id = Uuid::new_v4();
    denied(&mut f, owner, &request, ContentError::InvalidInput);
    request.confirm_workspace_id = f.workspace;
    request.operation_id = Uuid::nil();
    denied(&mut f, owner, &request, ContentError::InvalidInput);
    request.operation_id = Uuid::new_v4();
    {
        let mut s = AccessStore::new(&mut f.connection).unwrap();
        s.set_archived(owner, f.workspace, false, NOW).unwrap();
        s.set_archived(owner, f.workspace, true, NOW).unwrap();
    }
    denied(&mut f, owner, &request, ContentError::Conflict);
    request.expected_revision = preview(&mut f, owner).lifecycle_revision;
    f.connection
        .execute(
            "UPDATE ccp_workspaces SET owner_admin_id=?1 WHERE id=?2",
            params![f.admin.to_string(), f.workspace.to_string()],
        )
        .unwrap();
    denied(&mut f, owner, &request, ContentError::Forbidden);
    let new_owner = Actor::Admin(f.admin);
    denied(&mut f, new_owner, &request, ContentError::Conflict);
    request.expected_revision = preview(&mut f, new_owner).lifecycle_revision;
    begin(&mut f, new_owner, &request);
}
