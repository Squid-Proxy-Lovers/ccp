// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

fn intent(id: Uuid, revision: u64) -> WorkStatusClear {
    WorkStatusClear {
        operation_id: Uuid::new_v4(),
        status_id: id,
        expected_revision: revision,
    }
}
fn moderate(
    f: &mut Fixture,
    actor: Actor,
    request: &WorkStatusClear,
    now: i64,
) -> anyhow::Result<WorkStatusReceipt> {
    AccessStore::new(&mut f.connection)?.clear_client_work_status(actor, f.workspace, request, now)
}
fn key(f: &mut Fixture, principal: Uuid, scope: AdminKeyScope) -> IssuedAdminKey {
    AccessStore::new(&mut f.connection)
        .unwrap()
        .issue_admin_key(
            Actor::Admin(if scope == AdminKeyScope::Instance {
                f.global
            } else {
                f.owner
            }),
            &AdminKeyRequest {
                principal_id: principal,
                scope,
                name: "moderator".into(),
                expires_at: None,
            },
            NOW,
        )
        .unwrap()
}

#[test]
fn moderation_retains_client_ownership_and_original_proofs_without_resurrecting_cleared_text() {
    for admin in [false, true] {
        let mut f = fixture();
        let client = enroll(&mut f, DataRole::ReadWrite);
        let id = Uuid::new_v4();
        let set = post(id, 0, "PRIVATE_WORK");
        let original = apply(&mut f, client.credential_id, &set, NOW).unwrap();
        let actor = Actor::Admin(if admin { f.admin } else { f.owner });
        let clear = intent(id, 1);
        let receipt = moderate(&mut f, actor, &clear, NOW + 1).unwrap();
        assert_eq!(receipt.client_id, client.client_id);
        assert_eq!(receipt.revision, 2);
        assert!(receipt.cleared);
        let head = read(&mut f, id, NOW + 1);
        assert!(head.message.is_none() && head.agent.is_none() && head.stale);
        assert_eq!(head.client_id, client.client_id);
        assert_eq!(
            apply(&mut f, client.credential_id, &set, NOW + 2).unwrap(),
            original
        );
        assert!(read(&mut f, id, NOW + 2).message.is_none());
        assert_eq!(
            apply(
                &mut f,
                client.credential_id,
                &post(id, 2, "new report"),
                NOW + 2
            )
            .unwrap()
            .revision,
            3
        );
        assert_eq!(moderate(&mut f, actor, &clear, NOW + 3).unwrap(), receipt);
        assert_eq!(
            read(&mut f, id, NOW + 3).message.as_deref(),
            Some("new report")
        );
        let audit: (String,String,String) = f.connection.query_row("SELECT actor_kind,actor_id,target_id FROM ccp_access_audit WHERE action='clear_client_work_status'", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        assert_eq!(
            audit,
            (
                "admin".into(),
                if admin { f.admin } else { f.owner }.to_string(),
                clear.operation_id.to_string()
            )
        );
    }
}

#[test]
fn moderation_requires_current_scoped_admin_not_client_or_global_key_even_when_principal_also_owns()
{
    let mut f = fixture();
    let writer = enroll(&mut f, DataRole::ReadWrite);
    let reader = enroll(&mut f, DataRole::Read);
    let id = Uuid::new_v4();
    apply(&mut f, writer.credential_id, &post(id, 0, "kept"), NOW).unwrap();
    let request = intent(id, 1);
    for actor in [
        Actor::Credential(writer.credential_id),
        Actor::Credential(reader.credential_id),
        Actor::Admin(f.global),
        Actor::Admin(f.other_owner),
    ] {
        error(
            moderate(&mut f, actor, &request, NOW),
            ContentError::Forbidden,
        );
    }
    let global = f.global;
    f.connection
        .execute(
            "UPDATE ccp_workspaces SET owner_admin_id=?1 WHERE id=?2",
            params![global.to_string(), f.workspace.to_string()],
        )
        .unwrap();
    let instance = key(&mut f, global, AdminKeyScope::Instance);
    error(
        moderate(&mut f, Actor::AdminKey(instance.id), &request, NOW),
        ContentError::Forbidden,
    );
    let owner = f.owner;
    f.connection
        .execute(
            "UPDATE ccp_workspaces SET owner_admin_id=?1 WHERE id=?2",
            params![owner.to_string(), f.workspace.to_string()],
        )
        .unwrap();
    let workspace = f.workspace;
    let scoped = key(&mut f, owner, AdminKeyScope::Workspace(workspace));
    let actor = Actor::AdminKey(scoped.id);
    let receipt = moderate(&mut f, actor, &request, NOW).unwrap();
    assert_eq!(receipt.client_id, writer.client_id);
    f.connection
        .execute(
            "UPDATE ccp_admin_keys SET revoked_at=?1 WHERE id=?2",
            params![NOW, scoped.id.to_string()],
        )
        .unwrap();
    error(
        moderate(&mut f, actor, &request, NOW),
        ContentError::Forbidden,
    );
    assert_eq!(rows(&f.connection, "ccp_work_status_operations"), 2);
}

#[test]
fn moderation_replay_binds_key_and_original_cas_and_is_blocked_by_archive_or_role_loss() {
    let mut f = fixture();
    let client = enroll(&mut f, DataRole::ReadWrite);
    let id = Uuid::new_v4();
    apply(&mut f, client.credential_id, &post(id, 0, "kept"), NOW).unwrap();
    let owner = f.owner;
    let ws = f.workspace;
    let a = key(&mut f, owner, AdminKeyScope::Workspace(ws));
    let b = key(&mut f, owner, AdminKeyScope::Workspace(ws));
    let request = intent(id, 1);
    moderate(&mut f, Actor::AdminKey(a.id), &request, NOW).unwrap();
    error(
        moderate(&mut f, Actor::AdminKey(b.id), &request, NOW),
        ContentError::Conflict,
    );
    error(
        moderate(&mut f, Actor::Admin(owner), &request, NOW),
        ContentError::Conflict,
    );
    let changed = WorkStatusClear {
        expected_revision: 2,
        ..request
    };
    error(
        moderate(&mut f, Actor::AdminKey(a.id), &changed, NOW),
        ContentError::Conflict,
    );
    f.connection
        .execute(
            "UPDATE ccp_workspaces SET state='archived' WHERE id=?1",
            [ws.to_string()],
        )
        .unwrap();
    error(
        moderate(&mut f, Actor::AdminKey(a.id), &request, NOW),
        ContentError::Forbidden,
    );
    assert!(read(&mut f, id, NOW).message.is_none());
    f.connection
        .execute(
            "UPDATE ccp_workspaces SET state='active' WHERE id=?1",
            [ws.to_string()],
        )
        .unwrap();
    f.connection
        .execute(
            "UPDATE ccp_admin_principals SET enabled=0 WHERE id=?1",
            [owner.to_string()],
        )
        .unwrap();
    error(
        moderate(&mut f, Actor::AdminKey(a.id), &request, NOW),
        ContentError::Forbidden,
    );
}

#[test]
fn moderation_of_disabled_or_grantless_client_still_requires_current_cas_and_valid_target() {
    let mut f = fixture();
    let client = enroll(&mut f, DataRole::ReadWrite);
    let id = Uuid::new_v4();
    apply(&mut f, client.credential_id, &post(id, 0, "retained"), NOW).unwrap();
    let actor = Actor::Admin(f.admin);
    error(
        moderate(&mut f, actor, &intent(id, 0), NOW),
        ContentError::Conflict,
    );
    error(
        moderate(&mut f, actor, &intent(id, 1), NOW - 1),
        ContentError::Conflict,
    );
    error(
        moderate(&mut f, actor, &intent(Uuid::new_v4(), 1), NOW),
        ContentError::NotFound,
    );
    error(
        moderate(&mut f, actor, &intent(Uuid::nil(), 1), NOW),
        ContentError::InvalidInput,
    );
    error(
        moderate(&mut f, actor, &intent(id, i64::MAX.unsigned_abs()), NOW),
        ContentError::InvalidInput,
    );
    f.connection
        .execute_batch("UPDATE ccp_clients SET disabled=1; UPDATE ccp_grants SET revoked_at=1")
        .unwrap();
    assert!(
        moderate(&mut f, actor, &intent(id, 1), NOW)
            .unwrap()
            .cleared
    );
    error(
        moderate(&mut f, actor, &intent(id, 2), NOW),
        ContentError::Conflict,
    );
}

#[test]
fn ignored_or_aborted_moderation_head_receipt_and_audit_roll_back_together() {
    for table in [
        "ccp_work_status",
        "ccp_work_status_operations",
        "ccp_access_audit",
    ] {
        for fault in ["IGNORE", "ABORT,'PRIVATE_WORK'"] {
            let mut f = fixture();
            let client = enroll(&mut f, DataRole::ReadWrite);
            let id = Uuid::new_v4();
            apply(&mut f, client.credential_id, &post(id, 0, "retained"), NOW).unwrap();
            let actor = Actor::Admin(f.owner);
            let request = intent(id, 1);
            let audits = rows(&f.connection, "ccp_access_audit");
            let event = if table == "ccp_work_status" {
                "UPDATE"
            } else {
                "INSERT"
            };
            f.connection.execute_batch(&format!("CREATE TRIGGER moderation_fault BEFORE {event} ON {table} BEGIN SELECT RAISE({fault}); END;")).unwrap();
            assert!(moderate(&mut f, actor, &request, NOW).is_err());
            assert_eq!(read(&mut f, id, NOW).message.as_deref(), Some("retained"));
            assert_eq!(read(&mut f, id, NOW).revision, 1);
            assert_eq!(rows(&f.connection, "ccp_access_audit"), audits);
            assert_eq!(rows(&f.connection, "ccp_work_status_operations"), 1);
            f.connection
                .execute_batch("DROP TRIGGER moderation_fault")
                .unwrap();
            assert_eq!(moderate(&mut f, actor, &request, NOW).unwrap().revision, 2);
        }
    }
}
