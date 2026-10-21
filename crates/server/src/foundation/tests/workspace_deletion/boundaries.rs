// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;

#[test]
fn retries_and_body_free_status_bind_exact_actor_and_live_key_without_revocation_exception() {
    let mut f = fixture();
    let owner = f.owner;
    let ws = f.workspace;
    let k = key(&mut f, owner, AdminKeyScope::Workspace(ws));
    let other = key(&mut f, owner, AdminKeyScope::Workspace(ws));
    let actor = Actor::AdminKey(k.id);
    archive(&mut f);
    let mut request = intent(ws, preview(&mut f, actor).lifecycle_revision);
    let receipt = begin(&mut f, actor, &request);
    denied(&mut f, actor, &request, ContentError::Forbidden);
    denied(
        &mut f,
        Actor::AdminKey(other.id),
        &request,
        ContentError::Forbidden,
    );
    denied(
        &mut f,
        Actor::Admin(owner),
        &request,
        ContentError::Forbidden,
    );
    let global = Actor::Admin(f.global);
    denied(&mut f, global, &request, ContentError::Conflict);
    request.expected_revision -= 1;
    denied(&mut f, actor, &request, ContentError::Forbidden);
    request.expected_revision += 1;
    let operation = request.operation_id;
    request.operation_id = Uuid::new_v4();
    denied(&mut f, actor, &request, ContentError::Forbidden);
    request.operation_id = operation;
    {
        let mut s = AccessStore::new(&mut f.connection).unwrap();
        error(
            s.workspace_deletion_status(actor, ws, operation, NOW),
            ContentError::Forbidden,
        );
        error(
            s.workspace_deletion_status(actor, ws, Uuid::new_v4(), NOW),
            ContentError::Forbidden,
        );
        error(
            s.workspace_deletion_status(actor, ws, operation, -1),
            ContentError::Forbidden,
        );
        assert_eq!(
            s.workspace_deletion_status(global, ws, operation, NOW)
                .unwrap(),
            receipt
        );
        assert!(s.begin_workspace_deletion(actor, &request, -1).is_err());
    }
    // Retirement is bounded but never extends the scoped key's validity.
    AccessStore::new(&mut f.connection)
        .unwrap()
        .advance_workspace_deletion(ws, 128, NOW)
        .unwrap();
    denied(&mut f, actor, &request, ContentError::Forbidden);
    let complete = finish(&mut f, 128);
    let mut s = AccessStore::new(&mut f.connection).unwrap();
    assert!(s.authenticate_admin_key(k.secret(), NOW).is_err());
    error(
        s.workspace_deletion_status(actor, ws, operation, NOW),
        ContentError::Forbidden,
    );
    assert_eq!(
        s.workspace_deletion_status(global, ws, operation, NOW)
            .unwrap(),
        complete
    );
}

#[test]
fn legacy_content_missing_targets_disabled_principals_and_expired_keys_never_start_deletion() {
    let mut f = fixture();
    let owner = f.owner;
    let ws = f.workspace;
    let k = key(&mut f, owner, AdminKeyScope::Workspace(ws));
    archive(&mut f);
    let request = intent(ws, preview(&mut f, Actor::Admin(owner)).lifecycle_revision);
    f.connection
        .execute(
            "UPDATE ccp_admin_keys SET expires_at=?1 WHERE id=?2",
            params![NOW, k.id.to_string()],
        )
        .unwrap();
    denied(
        &mut f,
        Actor::AdminKey(k.id),
        &request,
        ContentError::Forbidden,
    );
    f.connection
        .execute(
            "UPDATE ccp_admin_principals SET enabled=0 WHERE id=?1",
            [owner.to_string()],
        )
        .unwrap();
    denied(
        &mut f,
        Actor::Admin(owner),
        &request,
        ContentError::Forbidden,
    );
    f.connection
        .execute(
            "UPDATE ccp_admin_principals SET enabled=1 WHERE id=?1",
            [owner.to_string()],
        )
        .unwrap();
    f.connection
        .execute("INSERT INTO sessions(id,name) VALUES(99,'legacy')", [])
        .unwrap();
    f.connection
        .execute(
            "UPDATE ccp_workspaces SET legacy_session_id=99 WHERE id=?1",
            [ws.to_string()],
        )
        .unwrap();
    denied(
        &mut f,
        Actor::Admin(owner),
        &request,
        ContentError::LegacyConversionRequired,
    );
    let mut s = AccessStore::new(&mut f.connection).unwrap();
    error(
        s.workspace_deletion_status(
            Actor::Admin(owner),
            Uuid::new_v4(),
            request.operation_id,
            NOW,
        ),
        ContentError::Forbidden,
    );
    error(
        s.workspace_deletion_status(
            Actor::Admin(f.global),
            Uuid::new_v4(),
            request.operation_id,
            NOW,
        ),
        ContentError::NotFound,
    );
    error(
        s.workspace_deletion_status(Actor::Admin(owner), ws, request.operation_id, NOW),
        ContentError::NotFound,
    );
    assert_eq!(rows(&f.connection, "ccp_workspace_deletions"), 0);
}

#[test]
fn operation_uuid_is_unique_and_global_receipt_access_ends_on_demotion() {
    let mut f = fixture();
    archive(&mut f);
    let global = f.global;
    let k = key(&mut f, global, AdminKeyScope::Instance);
    let request = intent(f.workspace, 2);
    let result = begin(&mut f, Actor::AdminKey(k.id), &request);
    let mut s = AccessStore::new(&mut f.connection).unwrap();
    s.set_archived(Actor::Admin(global), f.other_workspace, true, NOW)
        .unwrap();
    let mut second = intent(f.other_workspace, 2);
    second.operation_id = request.operation_id;
    error(
        s.begin_workspace_deletion(Actor::Admin(global), &second, NOW),
        ContentError::Conflict,
    );
    assert!(
        s.pending_workspace_deletions(None, 1)
            .unwrap()
            .contains(&f.workspace)
    );
    assert!(
        s.pending_workspace_deletions(Some(Uuid::max()), 1)
            .unwrap()
            .is_empty()
    );
    for bound in [0, 129, usize::MAX] {
        assert!(s.pending_workspace_deletions(None, bound).is_err());
        assert!(
            s.advance_workspace_deletion(f.workspace, bound, NOW)
                .is_err()
        );
    }
    error(
        s.advance_workspace_deletion(f.workspace, 1, NOW - 1),
        ContentError::InvalidInput,
    );
    error(
        s.advance_workspace_deletion(f.other_workspace, 1, NOW),
        ContentError::NotFound,
    );
    assert_eq!(
        s.workspace_deletion_status(
            Actor::AdminKey(k.id),
            f.workspace,
            request.operation_id,
            NOW
        )
        .unwrap(),
        result
    );
    f.connection
        .execute(
            "DELETE FROM ccp_server_admins WHERE admin_id=?1",
            [global.to_string()],
        )
        .unwrap();
    error(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .workspace_deletion_status(
                Actor::AdminKey(k.id),
                f.workspace,
                request.operation_id,
                NOW,
            ),
        ContentError::Forbidden,
    );
}
