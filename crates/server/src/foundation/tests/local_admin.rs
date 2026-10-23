// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

#[test]
fn local_bootstrap_atomically_provisions_one_global_and_first_owner_scope() {
    let mut connection = database();
    let result = AccessStore::new(&mut connection)
        .unwrap()
        .bootstrap_instance("deployer", "first", NOW)
        .unwrap();
    assert_eq!(
        result.instance_key.principal_id,
        result.owner_key.principal_id
    );
    assert_ne!(result.instance_key.id, result.owner_key.id);
    assert_ne!(result.instance_key.secret(), result.owner_key.secret());
    let mut store = AccessStore::new(&mut connection).unwrap();
    let global = store
        .authenticate_admin_key(result.instance_key.secret(), NOW)
        .unwrap();
    let owner = store
        .authenticate_admin_key(result.owner_key.secret(), NOW)
        .unwrap();
    assert!(authorize(&connection, global, Action::CreateWorkspace, None, NOW).is_ok());
    assert!(
        authorize(
            &connection,
            global,
            Action::WriteContext,
            Some(result.workspace_id),
            NOW
        )
        .is_err()
    );
    assert!(authorize(&connection, owner, Action::CreateWorkspace, None, NOW).is_err());
    assert!(
        authorize(
            &connection,
            owner,
            Action::WriteContext,
            Some(result.workspace_id),
            NOW
        )
        .is_ok()
    );
    assert_eq!(rows(&connection, "ccp_admin_principals"), 1);
    assert_eq!(rows(&connection, "ccp_server_admins"), 1);
    assert_eq!(rows(&connection, "ccp_workspaces"), 1);
    assert_eq!(rows(&connection, "ccp_admin_keys"), 2);
    let expiry: u32 = connection
        .query_row(
            "SELECT COUNT(*) FROM ccp_admin_keys WHERE expires_at IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(expiry, 0);
    let actors: u32 = connection.query_row("SELECT COUNT(*) FROM ccp_access_audit WHERE actor_kind != 'local_operator' OR actor_id != 'local' OR admin_key_id IS NOT NULL", [], |row| row.get(0)).unwrap();
    assert_eq!(actors, 0);
    let key_scopes: u32 = connection.query_row("SELECT COUNT(*) FROM ccp_access_audit WHERE action = 'issue_admin_key' AND workspace_id = ?1", [result.workspace_id.to_string()], |row| row.get(0)).unwrap();
    assert_eq!(key_scopes, 1);
    assert!(
        AccessStore::new(&mut connection)
            .unwrap()
            .bootstrap_instance("again", "other", NOW)
            .is_err()
    );
}

#[test]
fn bootstrap_failure_rolls_back_principal_roles_workspace_keys_and_audit() {
    let mut connection = database();
    connection.execute_batch("CREATE TRIGGER no_owner_key BEFORE INSERT ON ccp_admin_keys WHEN NEW.workspace_id IS NOT NULL BEGIN SELECT RAISE(ABORT, 'owner key down'); END;").unwrap();
    assert!(
        AccessStore::new(&mut connection)
            .unwrap()
            .bootstrap_instance("deployer", "first", NOW)
            .is_err()
    );
    for table in [
        "ccp_admin_principals",
        "ccp_server_admins",
        "ccp_workspaces",
        "ccp_admin_keys",
        "ccp_access_audit",
    ] {
        assert_eq!(rows(&connection, table), 0, "{table}");
    }
    connection.execute_batch("DROP TRIGGER no_owner_key; CREATE TRIGGER no_setup_audit BEFORE INSERT ON ccp_access_audit WHEN NEW.action = 'issue_admin_key' BEGIN SELECT RAISE(ABORT, 'audit down'); END;").unwrap();
    assert!(
        AccessStore::new(&mut connection)
            .unwrap()
            .bootstrap_instance("deployer", "first", NOW)
            .is_err()
    );
    assert_eq!(rows(&connection, "ccp_admin_principals"), 0);
    assert_eq!(rows(&connection, "ccp_workspaces"), 0);
    assert_eq!(rows(&connection, "ccp_admin_keys"), 0);
    assert_eq!(rows(&connection, "ccp_access_audit"), 0);
}

#[test]
fn disabled_existing_global_records_still_prevent_second_bootstrap() {
    let mut connection = database();
    let old = AccessStore::new(&mut connection)
        .unwrap()
        .bootstrap_admin_key("existing", NOW)
        .unwrap();
    connection
        .execute(
            "UPDATE ccp_admin_principals SET enabled = 0 WHERE id = ?1",
            [old.principal_id.to_string()],
        )
        .unwrap();
    assert!(
        AccessStore::new(&mut connection)
            .unwrap()
            .bootstrap_instance("replacement", "first", NOW)
            .is_err()
    );
    assert_eq!(rows(&connection, "ccp_admin_principals"), 1);
    assert_eq!(rows(&connection, "ccp_workspaces"), 0);
}

fn key(f: &mut Fixture, principal: Uuid, scope: AdminKeyScope) -> IssuedAdminKey {
    let actor = match scope {
        AdminKeyScope::Instance => f.global,
        AdminKeyScope::Workspace(id) if id == f.workspace => f.owner,
        AdminKeyScope::Workspace(_) => f.other_owner,
    };
    AccessStore::new(&mut f.connection)
        .unwrap()
        .issue_admin_key(
            Actor::Admin(actor),
            &AdminKeyRequest {
                principal_id: principal,
                scope,
                name: "existing".into(),
                expires_at: None,
            },
            NOW,
        )
        .unwrap()
}

#[test]
fn reset_instance_keys_leaves_owner_and_other_global_principals_untouched() {
    let mut f = fixture();
    let mine = AccessStore::new(&mut f.connection)
        .unwrap()
        .create_workspace(Actor::Admin(f.global), "mine", f.global, NOW)
        .unwrap();
    let global = f.global;
    let instance = key(&mut f, global, AdminKeyScope::Instance);
    let owner = AccessStore::new(&mut f.connection)
        .unwrap()
        .issue_admin_key(
            Actor::Admin(f.global),
            &AdminKeyRequest {
                principal_id: f.global,
                scope: AdminKeyScope::Workspace(mine),
                name: "owner".into(),
                expires_at: None,
            },
            NOW,
        )
        .unwrap();
    let other = f.owner;
    f.connection
        .execute(
            "INSERT INTO ccp_server_admins VALUES (?1)",
            [other.to_string()],
        )
        .unwrap();
    let other_global = key(&mut f, other, AdminKeyScope::Instance);
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    let reset = store
        .reset_local_admin_key(LocalKeyTarget::InstancePrincipal(f.global), None, NOW)
        .unwrap();
    assert_eq!(reset.revoked_keys, 1);
    assert!(
        store
            .authenticate_admin_key(instance.secret(), NOW)
            .is_err()
    );
    assert!(store.authenticate_admin_key(owner.secret(), NOW).is_ok());
    assert!(
        store
            .authenticate_admin_key(other_global.secret(), NOW)
            .is_ok()
    );
    assert!(
        store
            .authenticate_admin_key(reset.key.secret(), NOW)
            .is_ok()
    );
    assert_eq!(reset.scope, AdminKeyScope::Instance);
    assert_eq!(reset.key.principal_id, f.global);
}

#[test]
fn owner_reset_preserves_other_admin_keys_grants_archives_and_closed_enrollment() {
    let mut f = fixture();
    let old_client = enroll(&mut f, DataRole::ReadWrite);
    let owner_id = f.owner;
    let admin_id = f.admin;
    let workspace = f.workspace;
    let owner = key(&mut f, owner_id, AdminKeyScope::Workspace(workspace));
    let admin = key(&mut f, admin_id, AdminKeyScope::Workspace(workspace));
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    store
        .close_initial_enrollment(Actor::Admin(owner_id), workspace, NOW)
        .unwrap();
    store
        .set_archived(Actor::Admin(owner_id), workspace, true, NOW)
        .unwrap();
    let reset = store
        .reset_local_admin_key(
            LocalKeyTarget::WorkspaceOwner(workspace),
            Some(EXPIRES),
            NOW,
        )
        .unwrap();
    assert_eq!(reset.revoked_keys, 1);
    assert_eq!(reset.key.principal_id, owner_id);
    assert_eq!(reset.scope, AdminKeyScope::Workspace(workspace));
    assert!(store.authenticate_admin_key(owner.secret(), NOW).is_err());
    assert!(store.authenticate_admin_key(admin.secret(), NOW).is_ok());
    let actor = store
        .authenticate_admin_key(reset.key.secret(), NOW)
        .unwrap();
    assert!(
        authorize(
            &f.connection,
            actor,
            Action::ReadContext,
            Some(workspace),
            NOW
        )
        .is_ok()
    );
    assert!(
        authorize(
            &f.connection,
            actor,
            Action::WriteContext,
            Some(workspace),
            NOW
        )
        .is_err()
    );
    assert!(
        authorize(
            &f.connection,
            Actor::Credential(old_client.credential_id),
            Action::ReadContext,
            Some(workspace),
            NOW
        )
        .is_ok()
    );
    assert_eq!(rows(&f.connection, "ccp_grants"), 1);
    assert_eq!(rows(&f.connection, "ccp_workspace_admins"), 1);
    assert!(
        f.connection
            .query_row(
                "SELECT closed_at IS NOT NULL FROM ccp_initial_enrollment WHERE workspace_id = ?1",
                [workspace.to_string()],
                |row| row.get::<_, bool>(0)
            )
            .unwrap()
    );
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    assert!(
        store
            .authenticate_admin_key(reset.key.secret(), EXPIRES)
            .is_err()
    );
}

#[test]
fn reset_rejects_missing_disabled_purged_or_unassigned_role_without_changes() {
    for scenario in [
        "unknown_global",
        "not_global",
        "disabled",
        "disabled_owner",
        "unknown_workspace",
        "unowned",
        "purged",
    ] {
        let mut f = fixture();
        let mut target = LocalKeyTarget::InstancePrincipal(f.global);
        match scenario {
            "unknown_global" => target = LocalKeyTarget::InstancePrincipal(Uuid::new_v4()),
            "not_global" => target = LocalKeyTarget::InstancePrincipal(f.owner),
            "disabled" => {
                f.connection
                    .execute(
                        "UPDATE ccp_admin_principals SET enabled = 0 WHERE id = ?1",
                        [f.global.to_string()],
                    )
                    .unwrap();
            }
            "disabled_owner" => {
                f.connection
                    .execute(
                        "UPDATE ccp_admin_principals SET enabled = 0 WHERE id = ?1",
                        [f.owner.to_string()],
                    )
                    .unwrap();
                target = LocalKeyTarget::WorkspaceOwner(f.workspace);
            }
            "unknown_workspace" => target = LocalKeyTarget::WorkspaceOwner(Uuid::new_v4()),
            "unowned" => {
                f.connection
                    .execute(
                        "UPDATE ccp_workspaces SET owner_admin_id = NULL WHERE id = ?1",
                        [f.workspace.to_string()],
                    )
                    .unwrap();
                target = LocalKeyTarget::WorkspaceOwner(f.workspace);
            }
            "purged" => {
                f.connection
                    .execute(
                        "UPDATE ccp_workspaces SET state = 'purged' WHERE id = ?1",
                        [f.workspace.to_string()],
                    )
                    .unwrap();
                target = LocalKeyTarget::WorkspaceOwner(f.workspace);
            }
            _ => panic!("unknown scenario {scenario}"),
        }
        let audit = rows(&f.connection, "ccp_access_audit");
        assert!(
            AccessStore::new(&mut f.connection)
                .unwrap()
                .reset_local_admin_key(target, None, NOW)
                .is_err(),
            "{scenario}"
        );
        assert_eq!(rows(&f.connection, "ccp_admin_keys"), 0);
        assert_eq!(rows(&f.connection, "ccp_access_audit"), audit);
    }
}

#[test]
fn reset_audit_failure_restores_all_old_keys_and_drops_replacement() {
    let mut f = fixture();
    let owner_id = f.owner;
    let workspace = f.workspace;
    let first = key(&mut f, owner_id, AdminKeyScope::Workspace(workspace));
    let second = key(&mut f, owner_id, AdminKeyScope::Workspace(workspace));
    f.connection.execute_batch("CREATE TRIGGER no_reset BEFORE INSERT ON ccp_access_audit WHEN NEW.action = 'reset_admin_key' BEGIN SELECT RAISE(ABORT, 'audit down'); END;").unwrap();
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    assert!(
        store
            .reset_local_admin_key(LocalKeyTarget::WorkspaceOwner(workspace), None, NOW)
            .is_err()
    );
    assert!(store.authenticate_admin_key(first.secret(), NOW).is_ok());
    assert!(store.authenticate_admin_key(second.secret(), NOW).is_ok());
    assert_eq!(rows(&f.connection, "ccp_admin_keys"), 2);
    assert_eq!(
        f.connection
            .query_row(
                "SELECT COUNT(*) FROM ccp_admin_keys WHERE revoked_at IS NOT NULL",
                [],
                |row| row.get::<_, u32>(0)
            )
            .unwrap(),
        0
    );
}

#[test]
fn invalid_names_times_and_expiry_never_bootstrap_or_replace_keys() {
    let mut connection = database();
    for (operator, workspace, now) in [
        ("", "workspace", NOW),
        ("operator", "\n", NOW),
        ("operator", "workspace", -1),
    ] {
        assert!(
            AccessStore::new(&mut connection)
                .unwrap()
                .bootstrap_instance(operator, workspace, now)
                .is_err()
        );
    }
    let initial = AccessStore::new(&mut connection)
        .unwrap()
        .bootstrap_instance("operator", "first", NOW)
        .unwrap();
    for (expiry, now) in [(Some(NOW), NOW), (Some(NOW - 1), NOW), (None, -1)] {
        assert!(
            AccessStore::new(&mut connection)
                .unwrap()
                .reset_local_admin_key(
                    LocalKeyTarget::InstancePrincipal(initial.instance_key.principal_id),
                    expiry,
                    now
                )
                .is_err()
        );
    }
    assert_eq!(rows(&connection, "ccp_admin_keys"), 2);
    let mut store = AccessStore::new(&mut connection).unwrap();
    assert!(
        store
            .authenticate_admin_key(initial.instance_key.secret(), NOW)
            .is_ok()
    );
    assert!(
        store
            .authenticate_admin_key(initial.owner_key.secret(), NOW)
            .is_ok()
    );
}

#[test]
fn resetting_one_owner_scope_preserves_the_same_principals_other_workspace_key() {
    let mut f = fixture();
    let owner_id = f.owner;
    let workspace = f.workspace;
    let other_workspace = AccessStore::new(&mut f.connection)
        .unwrap()
        .create_workspace(Actor::Admin(f.global), "also mine", owner_id, NOW)
        .unwrap();
    let first = key(&mut f, owner_id, AdminKeyScope::Workspace(workspace));
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    let other = store
        .issue_admin_key(
            Actor::Admin(owner_id),
            &AdminKeyRequest {
                principal_id: owner_id,
                scope: AdminKeyScope::Workspace(other_workspace),
                name: "other owner".into(),
                expires_at: None,
            },
            NOW,
        )
        .unwrap();
    let reset = store
        .reset_local_admin_key(LocalKeyTarget::WorkspaceOwner(workspace), None, NOW)
        .unwrap();
    assert_eq!(reset.revoked_keys, 1);
    assert!(store.authenticate_admin_key(first.secret(), NOW).is_err());
    let other_actor = store.authenticate_admin_key(other.secret(), NOW).unwrap();
    assert!(
        authorize(
            &f.connection,
            other_actor,
            Action::WriteContext,
            Some(other_workspace),
            NOW
        )
        .is_ok()
    );
    assert!(
        authorize(
            &f.connection,
            other_actor,
            Action::ReadContext,
            Some(workspace),
            NOW
        )
        .is_err()
    );
}

#[test]
fn concurrent_initial_setup_creates_exactly_one_principal_workspace_and_pair_of_keys() {
    let path = std::env::temp_dir().join(format!("ccp-local-bootstrap-{}.db", Uuid::new_v4()));
    crate::init::init_sqlite(&path).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
    let mut workers = Vec::new();
    for _ in 0..8 {
        let path = path.clone();
        let barrier = barrier.clone();
        workers.push(std::thread::spawn(move || {
            let mut connection = Connection::open(path).unwrap();
            connection
                .busy_timeout(std::time::Duration::from_secs(20))
                .unwrap();
            barrier.wait();
            AccessStore::new(&mut connection)
                .unwrap()
                .bootstrap_instance("operator", "first", NOW)
        }));
    }
    let outcomes: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();
    assert_eq!(outcomes.iter().filter(|result| result.is_ok()).count(), 1);
    for error in outcomes.iter().filter_map(|result| result.as_ref().err()) {
        assert!(
            error
                .to_string()
                .contains("already has administrative bootstrap records"),
            "{error:#}"
        );
    }
    let connection = Connection::open(&path).unwrap();
    assert_eq!(rows(&connection, "ccp_admin_principals"), 1);
    assert_eq!(rows(&connection, "ccp_server_admins"), 1);
    assert_eq!(rows(&connection, "ccp_workspaces"), 1);
    assert_eq!(rows(&connection, "ccp_admin_keys"), 2);
    drop(connection);
    fs::remove_file(path).unwrap();
}
