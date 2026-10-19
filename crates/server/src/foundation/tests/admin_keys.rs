// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

fn request(principal_id: Uuid, scope: AdminKeyScope) -> AdminKeyRequest {
    AdminKeyRequest {
        principal_id,
        scope,
        name: "personal key".to_string(),
        expires_at: None,
    }
}

fn workspace_key(f: &mut Fixture, principal: Uuid) -> IssuedAdminKey {
    AccessStore::new(&mut f.connection)
        .unwrap()
        .issue_admin_key(
            Actor::Admin(f.owner),
            &request(principal, AdminKeyScope::Workspace(f.workspace)),
            NOW,
        )
        .unwrap()
}

#[test]
fn local_key_bootstrap_is_atomic_and_never_stores_the_secret() {
    let mut connection = database();
    let mut store = AccessStore::new(&mut connection).unwrap();
    let key = store.bootstrap_admin_key("operator", NOW).unwrap();
    let actor = store.authenticate_admin_key(key.secret(), NOW).unwrap();
    assert_eq!(actor, Actor::AdminKey(key.id));
    assert!(store.bootstrap_admin_key("second", NOW).is_err());
    assert!(authorize(&connection, actor, Action::CreateWorkspace, None, NOW).is_ok());
    let stored: Vec<u8> = connection
        .query_row("SELECT token_hash FROM ccp_admin_keys", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(stored.len(), 32);
    assert_ne!(stored, key.secret().as_bytes());
    let audit: String = connection
        .query_row(
            "SELECT group_concat(actor_id || action || target_id) FROM ccp_access_audit",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!audit.contains(key.secret()));
    let bootstrap_actor: (String, String, String) = connection.query_row(
        "SELECT actor_kind, actor_id, target_id FROM ccp_access_audit WHERE action = 'issue_admin_key'",
        [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).unwrap();
    assert_eq!(
        bootstrap_actor,
        (
            "local_operator".to_string(),
            "local".to_string(),
            key.id.to_string()
        )
    );
    assert_eq!(rows(&connection, "ccp_server_admins"), 1);
    assert_eq!(rows(&connection, "ccp_admin_keys"), 1);
}

#[test]
fn bootstrap_key_audit_failure_preserves_the_ability_to_bootstrap() {
    let mut connection = database();
    connection
        .execute_batch(
            "CREATE TRIGGER reject_keys BEFORE INSERT ON ccp_access_audit
        WHEN NEW.action = 'issue_admin_key' BEGIN SELECT RAISE(ABORT, 'audit unavailable'); END;",
        )
        .unwrap();
    assert!(
        AccessStore::new(&mut connection)
            .unwrap()
            .bootstrap_admin_key("operator", NOW)
            .is_err()
    );
    for table in [
        "ccp_admin_principals",
        "ccp_server_admins",
        "ccp_admin_keys",
        "ccp_access_audit",
    ] {
        assert_eq!(rows(&connection, table), 0);
    }
    connection
        .execute_batch("DROP TRIGGER reject_keys")
        .unwrap();
    assert!(
        AccessStore::new(&mut connection)
            .unwrap()
            .bootstrap_admin_key("operator", NOW)
            .is_ok()
    );
}

#[test]
fn scoped_key_cannot_follow_a_principals_roles_into_another_workspace() {
    let mut f = fixture();
    AccessStore::new(&mut f.connection)
        .unwrap()
        .assign_initial_owner(Actor::Admin(f.global), f.workspace, f.owner, NOW)
        .unwrap_err();
    f.connection
        .execute(
            "UPDATE ccp_workspaces SET owner_admin_id = ?1 WHERE id = ?2",
            params![f.owner.to_string(), f.other_workspace.to_string()],
        )
        .unwrap();
    let owner = f.owner;
    let key = workspace_key(&mut f, owner);
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    let actor = store.authenticate_admin_key(key.secret(), NOW).unwrap();
    let visible = store.visible_workspaces(actor, NOW).unwrap();
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].id, f.workspace);
    assert!(
        store
            .create_workspace(actor, "escaped", owner, NOW)
            .is_err()
    );
    assert!(
        store
            .set_archived(actor, f.other_workspace, true, NOW)
            .is_err()
    );
    assert!(
        store
            .issue_admin_key(
                actor,
                &request(owner, AdminKeyScope::Workspace(f.other_workspace)),
                NOW
            )
            .is_err()
    );
    assert!(
        store
            .register_client(
                actor,
                &registration(f.other_workspace, DataRole::ReadWrite),
                NOW
            )
            .is_err()
    );
    let grant = store
        .register_client(actor, &registration(f.workspace, DataRole::Read), NOW)
        .unwrap();
    let (principal, used_key): (String, String) = f.connection.query_row(
        "SELECT actor_id, admin_key_id FROM ccp_access_audit WHERE action = 'register_client' AND target_id = ?1",
        [grant.client_id.to_string()], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!(principal, owner.to_string());
    assert_eq!(used_key, key.id.to_string());
}

#[test]
fn only_owners_issue_or_revoke_workspace_admin_keys() {
    let mut f = fixture();
    let scope = AdminKeyScope::Workspace(f.workspace);
    let req = request(f.admin, scope);
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    for actor in [
        Actor::Admin(f.global),
        Actor::Admin(f.admin),
        Actor::Admin(f.other_owner),
    ] {
        assert!(store.issue_admin_key(actor, &req, NOW).is_err());
    }
    for target in [f.other_owner, f.global, Uuid::new_v4()] {
        assert!(
            store
                .issue_admin_key(Actor::Admin(f.owner), &request(target, scope), NOW)
                .is_err()
        );
    }
    let key = store
        .issue_admin_key(Actor::Admin(f.owner), &req, NOW)
        .unwrap();
    let actor = store.authenticate_admin_key(key.secret(), NOW).unwrap();
    assert!(store.issue_admin_key(actor, &req, NOW).is_err());
    assert!(store.revoke_admin_key(actor, scope, key.id, NOW).is_err());
    assert!(
        store
            .revoke_admin_key(Actor::Admin(f.other_owner), scope, key.id, NOW)
            .is_err()
    );
    assert!(
        store
            .revoke_admin_key(
                Actor::Admin(f.owner),
                AdminKeyScope::Workspace(f.other_workspace),
                key.id,
                NOW
            )
            .is_err()
    );
    store
        .revoke_admin_key(Actor::Admin(f.owner), scope, key.id, NOW)
        .unwrap();
    assert!(store.authenticate_admin_key(key.secret(), NOW).is_err());
    assert!(store.visible_workspaces(actor, NOW).is_err());
    assert!(
        store
            .register_client(actor, &registration(f.workspace, DataRole::Read), NOW)
            .is_err()
    );
}

#[test]
fn instance_key_preserves_global_read_without_adding_scoped_write_powers() {
    let mut f = fixture();
    let req = request(f.global, AdminKeyScope::Instance);
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    assert!(
        store
            .issue_admin_key(Actor::Admin(f.owner), &req, NOW)
            .is_err()
    );
    assert!(
        store
            .issue_admin_key(
                Actor::Admin(f.global),
                &request(f.owner, AdminKeyScope::Instance),
                NOW
            )
            .is_err()
    );
    let key = store
        .issue_admin_key(Actor::Admin(f.global), &req, NOW)
        .unwrap();
    let actor = store.authenticate_admin_key(key.secret(), NOW).unwrap();
    assert_eq!(store.visible_workspaces(actor, NOW).unwrap().len(), 2);
    assert!(
        store
            .register_client(actor, &registration(f.workspace, DataRole::Read), NOW)
            .is_err()
    );
    assert!(
        store
            .issue_admin_key(
                actor,
                &request(f.admin, AdminKeyScope::Workspace(f.workspace)),
                NOW
            )
            .is_err()
    );
    let new = store.create_workspace(actor, "new", f.owner, NOW).unwrap();
    store.set_archived(actor, new, true, NOW).unwrap();
    // Revoking the authenticating key itself must still write its audit record.
    store
        .revoke_admin_key(actor, AdminKeyScope::Instance, key.id, NOW)
        .unwrap();
    assert!(store.authenticate_admin_key(key.secret(), NOW).is_err());
    assert!(authorize(&f.connection, actor, Action::CreateWorkspace, None, NOW).is_err());
    let audit_key: String = f
        .connection
        .query_row(
            "SELECT admin_key_id FROM ccp_access_audit WHERE action = 'revoke_admin_key'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(audit_key, key.id.to_string());
}

#[test]
fn expiry_role_loss_and_disabled_principals_invalidate_existing_key_actors() {
    let mut f = fixture();
    let mut req = request(f.admin, AdminKeyScope::Workspace(f.workspace));
    req.expires_at = Some(EXPIRES);
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    let key = store
        .issue_admin_key(Actor::Admin(f.owner), &req, NOW)
        .unwrap();
    let actor = store
        .authenticate_admin_key(key.secret(), EXPIRES - 1)
        .unwrap();
    assert!(store.authenticate_admin_key(key.secret(), EXPIRES).is_err());
    assert!(store.visible_workspaces(actor, EXPIRES).is_err());
    assert!(store.authenticate_admin_key(key.secret(), -1).is_err());
    store
        .revoke_workspace_admin(Actor::Admin(f.owner), f.workspace, f.admin, NOW)
        .unwrap();
    assert!(store.authenticate_admin_key(key.secret(), NOW).is_err());
    assert!(store.visible_workspaces(actor, NOW).is_err());
    // A global role must not revive a scoped key after scoped membership loss.
    f.connection
        .execute(
            "INSERT INTO ccp_server_admins VALUES (?1)",
            [f.admin.to_string()],
        )
        .unwrap();
    assert!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .authenticate_admin_key(key.secret(), NOW)
            .is_err()
    );
    f.connection
        .execute(
            "UPDATE ccp_admin_principals SET enabled = 0 WHERE id = ?1",
            [f.owner.to_string()],
        )
        .unwrap();
    assert!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .issue_admin_key(Actor::Admin(f.owner), &req, NOW)
            .is_err()
    );
}

#[test]
fn default_keys_remain_valid_until_revoked_and_rotations_are_explicit() {
    let mut f = fixture();
    let owner = f.owner;
    let a = workspace_key(&mut f, owner);
    let b = workspace_key(&mut f, owner);
    assert_ne!(a.secret(), b.secret());
    assert_ne!(a.id, b.id);
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    let actor = store.authenticate_admin_key(b.secret(), i64::MAX).unwrap();
    store
        .revoke_admin_key(actor, AdminKeyScope::Workspace(f.workspace), a.id, NOW)
        .unwrap();
    assert!(store.authenticate_admin_key(a.secret(), NOW).is_err());
    assert!(store.authenticate_admin_key(b.secret(), i64::MAX).is_ok());
    assert!(
        store
            .revoke_admin_key(
                actor,
                AdminKeyScope::Workspace(f.workspace),
                Uuid::new_v4(),
                NOW
            )
            .is_err()
    );
    assert!(
        store
            .revoke_admin_key(actor, AdminKeyScope::Workspace(f.workspace), a.id, NOW)
            .is_err()
    );
}

#[test]
fn archived_key_security_does_not_unfreeze_content_or_admission() {
    let mut f = fixture();
    let owner = f.owner;
    let key = workspace_key(&mut f, owner);
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    let actor = store.authenticate_admin_key(key.secret(), NOW).unwrap();
    store.set_archived(actor, f.workspace, true, NOW).unwrap();
    let replacement = store
        .issue_admin_key(
            actor,
            &request(f.admin, AdminKeyScope::Workspace(f.workspace)),
            NOW,
        )
        .unwrap();
    let admin = store
        .authenticate_admin_key(replacement.secret(), NOW)
        .unwrap();
    assert_eq!(store.visible_workspaces(admin, NOW).unwrap().len(), 1);
    assert!(
        store
            .register_client(admin, &registration(f.workspace, DataRole::Read), NOW)
            .is_err()
    );
    store
        .revoke_admin_key(
            actor,
            AdminKeyScope::Workspace(f.workspace),
            replacement.id,
            NOW,
        )
        .unwrap();
    assert!(
        store
            .authenticate_admin_key(replacement.secret(), NOW)
            .is_err()
    );
    f.connection
        .execute(
            "UPDATE ccp_workspaces SET state = 'purged' WHERE id = ?1",
            [f.workspace.to_string()],
        )
        .unwrap();
    assert!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .authenticate_admin_key(key.secret(), NOW)
            .is_err()
    );
}

#[test]
fn invalid_or_forged_keys_and_bad_issuance_inputs_create_no_authority() {
    let mut f = fixture();
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    for token in [
        "",
        "claimed-admin",
        "ccp_admin_1234",
        &format!("ccp_admin_{}", "g".repeat(64)),
        &format!("ccp_admin_{}", "0".repeat(64)),
    ] {
        assert!(store.authenticate_admin_key(token, NOW).is_err());
    }
    let mut req = request(f.admin, AdminKeyScope::Workspace(f.workspace));
    req.name = "bad\nlabel".to_string();
    assert!(
        store
            .issue_admin_key(Actor::Admin(f.owner), &req, NOW)
            .is_err()
    );
    req.name = "valid".to_string();
    for expiry in [NOW, NOW - 1, -1] {
        req.expires_at = Some(expiry);
        assert!(
            store
                .issue_admin_key(Actor::Admin(f.owner), &req, NOW)
                .is_err()
        );
    }
    req.expires_at = None;
    assert!(
        store
            .issue_admin_key(Actor::Admin(f.owner), &req, -1)
            .is_err()
    );
    assert!(
        store
            .visible_workspaces(Actor::AdminKey(Uuid::new_v4()), NOW)
            .is_err()
    );
    assert_eq!(rows(&f.connection, "ccp_admin_keys"), 0);
}

#[test]
fn key_issuance_and_revocation_roll_back_if_the_audit_cannot_commit() {
    let mut f = fixture();
    let owner = f.owner;
    let key = workspace_key(&mut f, owner);
    f.connection.execute_batch("CREATE TRIGGER reject_keys BEFORE INSERT ON ccp_access_audit
        WHEN NEW.action IN ('issue_admin_key', 'revoke_admin_key') BEGIN SELECT RAISE(ABORT, 'audit unavailable'); END;").unwrap();
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    assert!(
        store
            .issue_admin_key(
                Actor::Admin(owner),
                &request(f.admin, AdminKeyScope::Workspace(f.workspace)),
                NOW
            )
            .is_err()
    );
    assert!(
        store
            .revoke_admin_key(
                Actor::Admin(owner),
                AdminKeyScope::Workspace(f.workspace),
                key.id,
                NOW
            )
            .is_err()
    );
    assert!(store.authenticate_admin_key(key.secret(), NOW).is_ok());
    assert_eq!(rows(&f.connection, "ccp_admin_keys"), 1);
}

#[test]
fn schema5_upgrade_preserves_identity_and_audit_then_persists_key_revocation() {
    let path = std::env::temp_dir().join(format!("ccp-key-upgrade-{}.db", Uuid::new_v4()));
    let connection = legacy_database(&path);
    connection
        .execute_batch(include_str!("schema5.sql"))
        .unwrap();
    let instance = Uuid::new_v4();
    let admin = Uuid::new_v4();
    connection
        .execute(
            "INSERT INTO ccp_instance VALUES (1, ?1)",
            [instance.to_string()],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO ccp_admin_principals(id, name) VALUES (?1, 'operator')",
            [admin.to_string()],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO ccp_server_admins VALUES (?1)",
            [admin.to_string()],
        )
        .unwrap();
    connection.execute("INSERT INTO ccp_access_audit(actor_kind, actor_id, action, recorded_at, target_id) VALUES ('local_operator', 'local', 'old audit', 1, ?1)", [admin.to_string()]).unwrap();
    connection
        .execute("UPDATE schema_version SET version = 5", [])
        .unwrap();
    drop(connection);
    crate::init::init_sqlite(&path).unwrap();
    let mut connection = Connection::open(&path).unwrap();
    let mut store = AccessStore::new(&mut connection).unwrap();
    assert_eq!(store.instance_id().unwrap(), instance);
    let key = store
        .issue_admin_key(
            Actor::Admin(admin),
            &request(admin, AdminKeyScope::Instance),
            NOW,
        )
        .unwrap();
    drop(connection);
    crate::init::init_sqlite(&path).unwrap();
    let mut connection = Connection::open(&path).unwrap();
    let actor = AccessStore::new(&mut connection)
        .unwrap()
        .authenticate_admin_key(key.secret(), NOW)
        .unwrap();
    let mut other = Connection::open(&path).unwrap();
    AccessStore::new(&mut other)
        .unwrap()
        .revoke_admin_key(Actor::Admin(admin), AdminKeyScope::Instance, key.id, NOW)
        .unwrap();
    assert!(
        AccessStore::new(&mut connection)
            .unwrap()
            .visible_workspaces(actor, NOW)
            .is_err()
    );
    let old_audit: (String, Option<String>) = connection
        .query_row(
            "SELECT action, admin_key_id FROM ccp_access_audit WHERE action = 'old audit'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(old_audit, ("old audit".to_string(), None));
    assert_eq!(rows(&connection, "message_packs"), 1);
    let invalid_fks: i64 = connection
        .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(invalid_fks, 0);
    drop(other);
    drop(connection);
    fs::remove_file(path).unwrap();
}

#[test]
fn disabled_principals_and_removed_global_roles_invalidate_live_key_actors() {
    let mut f = fixture();
    let owner = f.owner;
    let scoped = workspace_key(&mut f, owner);
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    let global = store
        .issue_admin_key(
            Actor::Admin(f.global),
            &request(f.global, AdminKeyScope::Instance),
            NOW,
        )
        .unwrap();
    let global_actor = store.authenticate_admin_key(global.secret(), NOW).unwrap();
    let scoped_actor = store.authenticate_admin_key(scoped.secret(), NOW).unwrap();
    f.connection
        .execute(
            "DELETE FROM ccp_server_admins WHERE admin_id = ?1",
            [f.global.to_string()],
        )
        .unwrap();
    f.connection
        .execute(
            "UPDATE ccp_admin_principals SET enabled = 0 WHERE id = ?1",
            [owner.to_string()],
        )
        .unwrap();
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    assert!(store.authenticate_admin_key(global.secret(), NOW).is_err());
    assert!(store.authenticate_admin_key(scoped.secret(), NOW).is_err());
    assert!(store.visible_workspaces(global_actor, NOW).is_err());
    assert!(store.visible_workspaces(scoped_actor, NOW).is_err());
    assert!(
        store
            .issue_admin_key(
                scoped_actor,
                &request(f.admin, AdminKeyScope::Workspace(f.workspace)),
                NOW
            )
            .is_err()
    );
}

fn mixed_role_keys(f: &mut Fixture) -> (IssuedAdminKey, IssuedAdminKey, Uuid) {
    f.connection
        .execute(
            "INSERT INTO ccp_workspace_admins(workspace_id, admin_id) VALUES (?1, ?2)",
            params![f.workspace.to_string(), f.global.to_string()],
        )
        .unwrap();
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    let mine = store
        .create_workspace(Actor::Admin(f.global), "global's workspace", f.global, NOW)
        .unwrap();
    let instance = store
        .issue_admin_key(
            Actor::Admin(f.global),
            &request(f.global, AdminKeyScope::Instance),
            NOW,
        )
        .unwrap();
    let workspace = store
        .issue_admin_key(
            Actor::Admin(f.owner),
            &request(f.global, AdminKeyScope::Workspace(f.workspace)),
            NOW,
        )
        .unwrap();
    (instance, workspace, mine)
}

#[test]
fn workspace_key_does_not_inherit_a_principals_global_archive_power() {
    let mut f = fixture();
    let (_, key, _) = mixed_role_keys(&mut f);
    let actor = AccessStore::new(&mut f.connection)
        .unwrap()
        .authenticate_admin_key(key.secret(), NOW)
        .unwrap();
    assert!(
        authorize(
            &f.connection,
            actor,
            Action::WriteContext,
            Some(f.workspace),
            NOW
        )
        .is_ok()
    );
    assert!(
        authorize(
            &f.connection,
            actor,
            Action::ArchiveWorkspace,
            Some(f.workspace),
            NOW
        )
        .is_err()
    );
}

#[test]
fn workspace_key_does_not_inherit_global_purge_or_reactivate_power() {
    let mut f = fixture();
    let (_, key, _) = mixed_role_keys(&mut f);
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    let actor = store.authenticate_admin_key(key.secret(), NOW).unwrap();
    store
        .set_archived(Actor::Admin(f.global), f.workspace, true, NOW)
        .unwrap();
    for action in [Action::PurgeWorkspace, Action::ReactivateWorkspace] {
        assert!(authorize(&f.connection, actor, action, Some(f.workspace), NOW).is_err());
    }
}

#[test]
fn instance_key_does_not_inherit_a_principals_workspace_owner_powers() {
    let mut f = fixture();
    let (key, _, mine) = mixed_role_keys(&mut f);
    let actor = AccessStore::new(&mut f.connection)
        .unwrap()
        .authenticate_admin_key(key.secret(), NOW)
        .unwrap();
    assert!(authorize(&f.connection, actor, Action::ReadContext, Some(mine), NOW).is_ok());
    for action in [
        Action::WriteContext,
        Action::SaveWorkspaceBoard,
        Action::ManageWorkspaceKeys,
        Action::AppointAdmin,
        Action::RevokeAdmin,
        Action::TransferOwner,
        Action::AdmitClient,
        Action::RevokeGrant,
        Action::ManageArchives,
    ] {
        assert!(authorize(&f.connection, actor, action, Some(mine), NOW).is_err());
    }
}

#[test]
fn matching_key_roles_keep_their_own_capabilities_when_the_principal_is_global() {
    let mut f = fixture();
    let (global, _, mine) = mixed_role_keys(&mut f);
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    let owner = store
        .issue_admin_key(
            Actor::Admin(f.global),
            &request(f.global, AdminKeyScope::Workspace(mine)),
            NOW,
        )
        .unwrap();
    let owner_actor = store.authenticate_admin_key(owner.secret(), NOW).unwrap();
    let global_actor = store.authenticate_admin_key(global.secret(), NOW).unwrap();
    assert!(
        authorize(
            &f.connection,
            owner_actor,
            Action::ManageWorkspaceKeys,
            Some(mine),
            NOW
        )
        .is_ok()
    );
    assert!(
        authorize(
            &f.connection,
            owner_actor,
            Action::WriteContext,
            Some(mine),
            NOW
        )
        .is_ok()
    );
    assert!(
        authorize(
            &f.connection,
            global_actor,
            Action::ArchiveWorkspace,
            Some(mine),
            NOW
        )
        .is_ok()
    );
    assert!(
        authorize(
            &f.connection,
            global_actor,
            Action::CreateWorkspace,
            None,
            NOW
        )
        .is_ok()
    );
    assert!(
        authorize(
            &f.connection,
            owner_actor,
            Action::CreateWorkspace,
            None,
            NOW
        )
        .is_err()
    );
}
