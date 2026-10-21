// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::fs;

use rusqlite::{Connection, params};
use uuid::Uuid;

use super::*;

const NOW: i64 = 1_000;
const EXPIRES: i64 = 2_000;

struct Fixture {
    connection: Connection,
    global: Uuid,
    owner: Uuid,
    admin: Uuid,
    workspace: Uuid,
    other_owner: Uuid,
    other_workspace: Uuid,
}

fn database() -> Connection {
    let mut connection = Connection::open_in_memory().unwrap();
    connection
        .pragma_update(None, "foreign_keys", true)
        .unwrap();
    connection
        .execute_batch(include_str!("../init-db.sql"))
        .unwrap();
    let transaction = connection.transaction().unwrap();
    migrate(&transaction).unwrap();
    transaction.commit().unwrap();
    connection
}

fn fixture() -> Fixture {
    fixture_with_connection(database())
}

fn fixture_with_connection(mut connection: Connection) -> Fixture {
    let mut store = AccessStore::new(&mut connection).unwrap();
    let global = store.bootstrap_admin("operator", NOW).unwrap();
    let owner = store
        .create_principal(Actor::Admin(global), "owner", NOW)
        .unwrap();
    let workspace = store
        .create_workspace(Actor::Admin(global), "alpha", owner, NOW)
        .unwrap();
    let admin = store
        .appoint_workspace_admin(Actor::Admin(owner), workspace, "admin", NOW)
        .unwrap();
    let other_owner = store
        .create_principal(Actor::Admin(global), "other owner", NOW)
        .unwrap();
    let other_workspace = store
        .create_workspace(Actor::Admin(global), "beta", other_owner, NOW)
        .unwrap();
    Fixture {
        connection,
        global,
        owner,
        admin,
        workspace,
        other_owner,
        other_workspace,
    }
}

fn registration(workspace_id: Uuid, role: DataRole) -> ClientRegistration {
    ClientRegistration {
        workspace_id,
        role,
        expires_at: EXPIRES,
    }
}

fn enroll(f: &mut Fixture, role: DataRole) -> Enrollment {
    AccessStore::new(&mut f.connection)
        .unwrap()
        .register_client(Actor::Admin(f.admin), &registration(f.workspace, role), NOW)
        .unwrap()
}

fn rows(connection: &Connection, table: &str) -> i64 {
    connection
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
}

#[test]
fn role_matrix_keeps_global_read_separate_from_workspace_authority() {
    let f = fixture();
    let global = [Action::ReadContext, Action::ArchiveWorkspace];
    let admin = [
        Action::ReadContext,
        Action::WriteContext,
        Action::SaveWorkspaceBoard,
        Action::AdmitClient,
        Action::RevokeGrant,
        Action::RecoverWorkspaceClient,
        Action::ManageArchives,
    ];
    let owner_only = [
        Action::AppointAdmin,
        Action::RevokeAdmin,
        Action::TransferOwner,
        Action::ArchiveWorkspace,
        Action::ManageWorkspaceKeys,
    ];
    let actions = [
        Action::ReadContext,
        Action::WriteContext,
        Action::SaveWorkspaceBoard,
        Action::AdmitClient,
        Action::RevokeGrant,
        Action::RecoverWorkspaceClient,
        Action::AppointAdmin,
        Action::RevokeAdmin,
        Action::TransferOwner,
        Action::ArchiveWorkspace,
        Action::ReactivateWorkspace,
        Action::PurgeWorkspace,
        Action::ManageArchives,
        Action::ManageWorkspaceKeys,
    ];
    for (principal, allowed) in [
        (f.global, global.to_vec()),
        (f.admin, admin.to_vec()),
        (
            f.owner,
            admin.into_iter().chain(owner_only).collect::<Vec<_>>(),
        ),
    ] {
        for action in actions {
            assert_eq!(
                authorize(
                    &f.connection,
                    Actor::Admin(principal),
                    action,
                    Some(f.workspace),
                    NOW
                )
                .is_ok(),
                allowed.contains(&action),
                "principal={principal}, action={action:?}"
            );
        }
    }
    for action in actions {
        assert!(
            authorize(
                &f.connection,
                Actor::Admin(f.admin),
                action,
                Some(f.other_workspace),
                NOW
            )
            .is_err()
        );
        assert!(
            authorize(
                &f.connection,
                Actor::Admin(f.other_owner),
                action,
                Some(f.workspace),
                NOW
            )
            .is_err()
        );
    }
}

#[test]
fn instance_capabilities_require_global_admin_and_exact_scope() {
    let f = fixture();
    for action in [
        Action::CreateWorkspace,
        Action::SaveGlobalBoard,
        Action::DisableClient,
        Action::RecoverGlobalClient,
        Action::ManageInstanceKeys,
    ] {
        assert!(authorize(&f.connection, Actor::Admin(f.global), action, None, NOW).is_ok());
        assert!(authorize(&f.connection, Actor::Admin(f.owner), action, None, NOW).is_err());
        assert!(authorize(&f.connection, Actor::Admin(f.admin), action, None, NOW).is_err());
        assert!(
            authorize(
                &f.connection,
                Actor::Admin(f.global),
                action,
                Some(f.workspace),
                NOW
            )
            .is_err()
        );
    }
    assert!(
        authorize(
            &f.connection,
            Actor::Admin(f.global),
            Action::ReadContext,
            None,
            NOW
        )
        .is_err()
    );
    assert!(
        authorize(
            &f.connection,
            Actor::Admin(f.global),
            Action::ReadContext,
            Some(Uuid::new_v4()),
            NOW
        )
        .is_err()
    );
    assert!(
        authorize(
            &f.connection,
            Actor::Admin(Uuid::new_v4()),
            Action::ReadContext,
            Some(f.workspace),
            NOW
        )
        .is_err()
    );
    assert!(
        authorize(
            &f.connection,
            Actor::Admin(f.global),
            Action::ReadContext,
            Some(f.workspace),
            -1
        )
        .is_err()
    );
}

#[test]
fn clients_have_only_bound_workspace_data_roles() {
    let mut f = fixture();
    let reader = enroll(&mut f, DataRole::Read);
    let writer = enroll(&mut f, DataRole::ReadWrite);
    for (client, can_write) in [(reader, false), (writer, true)] {
        let actor = Actor::Credential(client.credential_id);
        assert!(
            authorize(
                &f.connection,
                actor,
                Action::ReadContext,
                Some(f.workspace),
                NOW
            )
            .is_ok()
        );
        assert_eq!(
            authorize(
                &f.connection,
                actor,
                Action::WriteContext,
                Some(f.workspace),
                NOW
            )
            .is_ok(),
            can_write
        );
        assert!(
            authorize(
                &f.connection,
                actor,
                Action::ReadContext,
                Some(f.other_workspace),
                NOW
            )
            .is_err()
        );
        for action in [
            Action::SaveWorkspaceBoard,
            Action::AdmitClient,
            Action::RevokeGrant,
            Action::RecoverWorkspaceClient,
            Action::AppointAdmin,
            Action::ArchiveWorkspace,
        ] {
            assert!(authorize(&f.connection, actor, action, Some(f.workspace), NOW).is_err());
        }
        assert!(authorize(&f.connection, actor, Action::DisableClient, None, NOW).is_err());
    }
}

#[test]
fn existing_identity_joins_another_workspace_and_revoke_does_not_cross_scopes() {
    let mut f = fixture();
    let client = enroll(&mut f, DataRole::ReadWrite);
    let grant_b = AccessStore::new(&mut f.connection)
        .unwrap()
        .admit_existing_client(
            Actor::Admin(f.other_owner),
            Admission {
                workspace_id: f.other_workspace,
                presented_credential: client.credential_id,
                role: DataRole::Read,
            },
            NOW,
        )
        .unwrap();
    assert_eq!(rows(&f.connection, "ccp_clients"), 1);
    assert_eq!(rows(&f.connection, "ccp_credentials"), 1);
    let actor = Actor::Credential(client.credential_id);
    assert!(
        authorize(
            &f.connection,
            actor,
            Action::ReadContext,
            Some(f.other_workspace),
            NOW
        )
        .is_ok()
    );
    assert!(
        authorize(
            &f.connection,
            actor,
            Action::WriteContext,
            Some(f.other_workspace),
            NOW
        )
        .is_err()
    );
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    assert!(
        store
            .revoke_grant(Actor::Admin(f.admin), grant_b, NOW)
            .is_err()
    );
    assert!(
        store
            .revoke_grant(Actor::Admin(f.global), grant_b, NOW)
            .is_err()
    );
    store
        .revoke_grant(Actor::Admin(f.other_owner), grant_b, NOW)
        .unwrap();
    assert!(
        authorize(
            &f.connection,
            actor,
            Action::ReadContext,
            Some(f.other_workspace),
            NOW
        )
        .is_err()
    );
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
    let new_grant = AccessStore::new(&mut f.connection)
        .unwrap()
        .admit_existing_client(
            Actor::Admin(f.other_owner),
            Admission {
                workspace_id: f.other_workspace,
                presented_credential: client.credential_id,
                role: DataRole::ReadWrite,
            },
            NOW,
        )
        .unwrap();
    assert_ne!(new_grant, grant_b);
    assert!(
        authorize(
            &f.connection,
            actor,
            Action::WriteContext,
            Some(f.other_workspace),
            NOW
        )
        .is_ok()
    );
}

#[test]
fn credential_ceiling_never_expands_from_identity_wide_grants_or_reactivation() {
    let mut f = fixture();
    let client = enroll(&mut f, DataRole::ReadWrite);
    let restricted = Uuid::new_v4();
    f.connection
        .execute(
            "INSERT INTO ccp_credentials(id, client_id, expires_at) VALUES (?1, ?2, ?3)",
            params![
                restricted.to_string(),
                client.client_id.to_string(),
                EXPIRES
            ],
        )
        .unwrap();
    f.connection
        .execute(
            "INSERT INTO ccp_credential_grants VALUES (?1, ?2, ?3)",
            params![
                restricted.to_string(),
                client.grant_id.to_string(),
                client.client_id.to_string()
            ],
        )
        .unwrap();
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    store
        .admit_existing_client(
            Actor::Admin(f.other_owner),
            Admission {
                workspace_id: f.other_workspace,
                presented_credential: client.credential_id,
                role: DataRole::ReadWrite,
            },
            NOW,
        )
        .unwrap();
    store
        .set_archived(Actor::Admin(f.global), f.other_workspace, true, NOW)
        .unwrap();
    store
        .set_archived(Actor::Admin(f.global), f.other_workspace, false, NOW)
        .unwrap();
    assert!(
        authorize(
            &f.connection,
            Actor::Credential(restricted),
            Action::ReadContext,
            Some(f.other_workspace),
            NOW
        )
        .is_err()
    );
    let visible = AccessStore::new(&mut f.connection)
        .unwrap()
        .visible_workspaces(Actor::Credential(restricted), NOW)
        .unwrap();
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].id, f.workspace);
}

#[test]
fn database_rejects_bindings_between_different_clients() {
    let mut f = fixture();
    let a = enroll(&mut f, DataRole::Read);
    let b = enroll(&mut f, DataRole::ReadWrite);
    for client_id in [a.client_id, b.client_id] {
        assert!(
            f.connection
                .execute(
                    "INSERT INTO ccp_credential_grants VALUES (?1, ?2, ?3)",
                    params![
                        a.credential_id.to_string(),
                        b.grant_id.to_string(),
                        client_id.to_string()
                    ]
                )
                .is_err()
        );
    }
    assert_eq!(rows(&f.connection, "ccp_credential_grants"), 2);
}

#[test]
fn global_disable_blocks_valid_credentials_admission_and_discovery_without_data_loss() {
    let mut f = fixture();
    let client = enroll(&mut f, DataRole::ReadWrite);
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    assert!(
        store
            .disable_client(Actor::Admin(f.admin), client.client_id, NOW)
            .is_err()
    );
    store
        .disable_client(Actor::Admin(f.global), client.client_id, NOW)
        .unwrap();
    assert!(
        store
            .visible_workspaces(Actor::Credential(client.credential_id), NOW)
            .is_err()
    );
    assert!(
        store
            .admit_existing_client(
                Actor::Admin(f.other_owner),
                Admission {
                    workspace_id: f.other_workspace,
                    presented_credential: client.credential_id,
                    role: DataRole::Read
                },
                NOW
            )
            .is_err()
    );
    assert!(
        authorize(
            &f.connection,
            Actor::Credential(client.credential_id),
            Action::ReadContext,
            Some(f.workspace),
            NOW
        )
        .is_err()
    );
    assert_eq!(rows(&f.connection, "ccp_clients"), 1);
    assert_eq!(rows(&f.connection, "ccp_credentials"), 1);
    assert_eq!(rows(&f.connection, "ccp_grants"), 1);
    assert_eq!(rows(&f.connection, "ccp_credential_grants"), 1);
}

#[test]
fn expiration_revocation_and_role_changes_are_checked_on_each_call() {
    let mut f = fixture();
    let client = enroll(&mut f, DataRole::ReadWrite);
    let actor = Actor::Credential(client.credential_id);
    assert!(
        authorize(
            &f.connection,
            actor,
            Action::ReadContext,
            Some(f.workspace),
            EXPIRES
        )
        .is_err()
    );
    assert!(
        authorize(
            &f.connection,
            actor,
            Action::ReadContext,
            Some(f.workspace),
            EXPIRES - 1
        )
        .is_ok()
    );
    f.connection
        .execute(
            "UPDATE ccp_grants SET role = 'read' WHERE id = ?1",
            [client.grant_id.to_string()],
        )
        .unwrap();
    assert!(
        authorize(
            &f.connection,
            actor,
            Action::WriteContext,
            Some(f.workspace),
            NOW
        )
        .is_err()
    );
    assert!(
        authorize(
            &f.connection,
            actor,
            Action::ReadContext,
            Some(f.workspace),
            NOW
        )
        .is_ok()
    );
    f.connection
        .execute(
            "UPDATE ccp_credentials SET revoked_at = ?1 WHERE id = ?2",
            params![NOW, client.credential_id.to_string()],
        )
        .unwrap();
    assert!(
        authorize(
            &f.connection,
            actor,
            Action::ReadContext,
            Some(f.workspace),
            NOW
        )
        .is_err()
    );
    assert!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .visible_workspaces(actor, NOW)
            .is_err()
    );
}

#[test]
fn archived_workspace_is_readable_but_content_board_and_admission_are_frozen() {
    let mut f = fixture();
    let client = enroll(&mut f, DataRole::ReadWrite);
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    assert!(
        store
            .set_archived(Actor::Admin(f.admin), f.workspace, true, NOW)
            .is_err()
    );
    store
        .set_archived(Actor::Admin(f.global), f.workspace, true, NOW)
        .unwrap();
    assert!(
        store
            .register_client(
                Actor::Admin(f.admin),
                &registration(f.workspace, DataRole::Read),
                NOW
            )
            .is_err()
    );
    let visible = store
        .visible_workspaces(Actor::Credential(client.credential_id), NOW)
        .unwrap();
    assert!(visible[0].archived);
    for actor in [
        Actor::Admin(f.owner),
        Actor::Admin(f.admin),
        Actor::Credential(client.credential_id),
    ] {
        assert!(
            authorize(
                &f.connection,
                actor,
                Action::ReadContext,
                Some(f.workspace),
                NOW
            )
            .is_ok()
        );
        assert!(
            authorize(
                &f.connection,
                actor,
                Action::WriteContext,
                Some(f.workspace),
                NOW
            )
            .is_err()
        );
        assert!(
            authorize(
                &f.connection,
                actor,
                Action::SaveWorkspaceBoard,
                Some(f.workspace),
                NOW
            )
            .is_err()
        );
    }
    assert!(
        authorize(
            &f.connection,
            Actor::Admin(f.admin),
            Action::RecoverWorkspaceClient,
            Some(f.workspace),
            NOW
        )
        .is_ok()
    );
    assert!(
        authorize(
            &f.connection,
            Actor::Admin(f.global),
            Action::PurgeWorkspace,
            Some(f.workspace),
            NOW
        )
        .is_ok()
    );
    AccessStore::new(&mut f.connection)
        .unwrap()
        .revoke_grant(Actor::Admin(f.admin), client.grant_id, NOW)
        .unwrap();
    AccessStore::new(&mut f.connection)
        .unwrap()
        .set_archived(Actor::Admin(f.owner), f.workspace, false, NOW)
        .unwrap();
    assert!(
        authorize(
            &f.connection,
            Actor::Credential(client.credential_id),
            Action::ReadContext,
            Some(f.workspace),
            NOW
        )
        .is_err()
    );
}

#[test]
fn scoped_discovery_never_advertises_ungranted_or_purged_workspaces() {
    let mut f = fixture();
    let client = enroll(&mut f, DataRole::Read);
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    assert_eq!(
        store
            .visible_workspaces(Actor::Admin(f.global), NOW)
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        store
            .visible_workspaces(Actor::Admin(f.admin), NOW)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        store
            .visible_workspaces(Actor::Credential(client.credential_id), NOW)
            .unwrap()
            .len(),
        1
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
            .visible_workspaces(Actor::Credential(client.credential_id), NOW)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .visible_workspaces(Actor::Admin(f.global), NOW)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn demotion_and_disabled_admin_are_effective_without_a_cached_role() {
    let mut f = fixture();
    AccessStore::new(&mut f.connection)
        .unwrap()
        .revoke_workspace_admin(Actor::Admin(f.owner), f.workspace, f.admin, NOW)
        .unwrap();
    assert!(
        authorize(
            &f.connection,
            Actor::Admin(f.admin),
            Action::AdmitClient,
            Some(f.workspace),
            NOW
        )
        .is_err()
    );
    assert!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .register_client(
                Actor::Admin(f.admin),
                &registration(f.workspace, DataRole::Read),
                NOW
            )
            .is_err()
    );
    f.connection
        .execute(
            "UPDATE ccp_admin_principals SET enabled = 0 WHERE id = ?1",
            [f.global.to_string()],
        )
        .unwrap();
    assert!(
        authorize(
            &f.connection,
            Actor::Admin(f.global),
            Action::CreateWorkspace,
            None,
            NOW
        )
        .is_err()
    );
    assert!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .visible_workspaces(Actor::Admin(f.global), NOW)
            .is_err()
    );
    assert!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .bootstrap_admin("second operator", NOW)
            .is_err()
    );
}

#[test]
fn first_owner_assignment_does_not_become_ownership_reassignment() {
    let mut f = fixture();
    f.connection
        .execute(
            "INSERT INTO sessions(id, name, owner) VALUES (81, 'legacy', 'untrusted owner')",
            [],
        )
        .unwrap();
    let transaction = f.connection.transaction().unwrap();
    let legacy = map_legacy_session(&transaction, 81).unwrap();
    transaction.commit().unwrap();
    assert!(
        authorize(
            &f.connection,
            Actor::Admin(f.owner),
            Action::WriteContext,
            Some(legacy),
            NOW
        )
        .is_err()
    );
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    assert!(
        store
            .assign_initial_owner(Actor::Admin(f.admin), legacy, f.owner, NOW)
            .is_err()
    );
    store
        .assign_initial_owner(Actor::Admin(f.global), legacy, f.owner, NOW)
        .unwrap();
    assert!(
        store
            .assign_initial_owner(Actor::Admin(f.global), legacy, f.other_owner, NOW)
            .is_err()
    );
    assert!(
        store
            .assign_initial_owner(Actor::Admin(f.global), Uuid::new_v4(), f.owner, NOW)
            .is_err()
    );
    assert!(
        authorize(
            &f.connection,
            Actor::Admin(f.owner),
            Action::WriteContext,
            Some(legacy),
            NOW
        )
        .is_ok()
    );
}

#[test]
fn audit_failure_rolls_back_every_registration_record() {
    let mut f = fixture();
    let before = rows(&f.connection, "ccp_access_audit");
    f.connection
        .execute_batch(
            "CREATE TRIGGER reject_registration BEFORE INSERT ON ccp_access_audit
        WHEN NEW.action = 'register_client' BEGIN SELECT RAISE(ABORT, 'audit unavailable'); END;",
        )
        .unwrap();
    assert!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .register_client(
                Actor::Admin(f.admin),
                &registration(f.workspace, DataRole::Read),
                NOW
            )
            .is_err()
    );
    for table in [
        "ccp_clients",
        "ccp_credentials",
        "ccp_grants",
        "ccp_credential_grants",
    ] {
        assert_eq!(
            rows(&f.connection, table),
            0,
            "partial registration left in {table}"
        );
    }
    assert_eq!(rows(&f.connection, "ccp_access_audit"), before);
}

#[test]
fn constraints_and_input_failures_leave_no_partial_authority() {
    let mut f = fixture();
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    for name in ["", "   ", "bad\nname", &"x".repeat(257)] {
        assert!(
            store
                .create_workspace(Actor::Admin(f.global), name, f.owner, NOW)
                .is_err()
        );
    }
    assert!(
        store
            .create_workspace(Actor::Admin(f.global), "alpha", f.owner, NOW)
            .is_err()
    );
    assert!(
        store
            .create_workspace(Actor::Admin(f.owner), "new", f.owner, NOW)
            .is_err()
    );
    assert!(
        store
            .create_workspace(Actor::Admin(f.global), "new", Uuid::new_v4(), NOW)
            .is_err()
    );
    assert!(
        store
            .appoint_workspace_admin(
                Actor::Admin(f.global),
                f.workspace,
                "unexpected global override",
                NOW
            )
            .is_err()
    );
    let mut invalid = registration(f.workspace, DataRole::Read);
    invalid.expires_at = NOW;
    assert!(
        store
            .register_client(Actor::Admin(f.admin), &invalid, NOW)
            .is_err()
    );
    assert!(
        store
            .register_client(Actor::Admin(f.admin), &invalid, -1)
            .is_err()
    );
    assert!(
        store
            .disable_client(Actor::Admin(f.global), Uuid::new_v4(), NOW)
            .is_err()
    );
    assert!(
        store
            .revoke_workspace_admin(Actor::Admin(f.owner), f.workspace, Uuid::new_v4(), NOW)
            .is_err()
    );
    assert_eq!(rows(&f.connection, "ccp_workspaces"), 2);
    assert_eq!(rows(&f.connection, "ccp_admin_principals"), 4);
    assert_eq!(rows(&f.connection, "ccp_clients"), 0);
}

#[test]
fn negative_bootstrap_time_is_atomic_and_foreign_keys_are_mandatory() {
    let mut connection = database();
    let mut store = AccessStore::new(&mut connection).unwrap();
    assert!(store.bootstrap_admin("operator", -1).is_err());
    assert_eq!(rows(&connection, "ccp_admin_principals"), 0);
    assert_eq!(rows(&connection, "ccp_server_admins"), 0);
    connection
        .pragma_update(None, "foreign_keys", false)
        .unwrap();
    let transaction = connection.transaction().unwrap();
    assert!(
        transaction
            .pragma_query_value::<bool, _>(None, "foreign_keys", |row| row.get(0))
            .is_ok_and(|enabled| !enabled)
    );
    // SQLite cannot turn foreign keys on in an already-running transaction.
    drop(transaction);
    connection.execute_batch("BEGIN").unwrap();
    assert!(AccessStore::new(&mut connection).is_err());
    connection.execute_batch("ROLLBACK").unwrap();
}

fn legacy_database(path: &std::path::Path) -> Connection {
    let connection = Connection::open(path).unwrap();
    connection
        .execute_batch(include_str!("../init-db.sql"))
        .unwrap();
    connection.execute_batch("INSERT INTO schema_version(version) VALUES (2);
        INSERT INTO sessions(id, name, owner) VALUES (41, 'legacy alpha', 'not-an-admin');
        INSERT INTO message_packs(id, session_id, name, context) VALUES (8, 41, 'keep', 'original payload');").unwrap();
    connection
}

#[test]
fn schema_upgrade_preserves_legacy_data_and_ids_without_granting_display_name_authority() {
    let path = std::env::temp_dir().join(format!("ccp-foundation-migrate-{}.db", Uuid::new_v4()));
    drop(legacy_database(&path));
    crate::init::init_sqlite(&path).unwrap();
    let mut connection = Connection::open(&path).unwrap();
    let instance = AccessStore::new(&mut connection)
        .unwrap()
        .instance_id()
        .unwrap();
    let (id, owner): (String, Option<String>) = connection
        .query_row(
            "SELECT id, owner_admin_id FROM ccp_workspaces WHERE legacy_session_id = 41",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(owner, None);
    let payload: String = connection
        .query_row(
            "SELECT context FROM message_packs WHERE id = 8",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(payload, "original payload");
    assert_eq!(rows(&connection, "ccp_admin_principals"), 0);
    assert_eq!(rows(&connection, "ccp_grants"), 0);
    connection
        .execute(
            "INSERT INTO sessions(id, name) VALUES (42, 'legacy beta')",
            [],
        )
        .unwrap();
    drop(connection);
    crate::init::init_sqlite(&path).unwrap();
    let mut connection = Connection::open(&path).unwrap();
    assert_eq!(
        AccessStore::new(&mut connection)
            .unwrap()
            .instance_id()
            .unwrap(),
        instance
    );
    let repeated: String = connection
        .query_row(
            "SELECT id FROM ccp_workspaces WHERE legacy_session_id = 41",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(repeated, id);
    assert_eq!(rows(&connection, "ccp_workspaces"), 2);
    assert!(
        connection
            .execute("DELETE FROM sessions WHERE id = 41", [])
            .is_err()
    );
    drop(connection);
    fs::remove_file(path).unwrap();
}

#[test]
fn failed_foundation_upgrade_does_not_record_a_new_version_or_partial_tables() {
    let path = std::env::temp_dir().join(format!("ccp-foundation-rollback-{}.db", Uuid::new_v4()));
    let connection = legacy_database(&path);
    connection
        .execute_batch(
            "CREATE TABLE ccp_instance(singleton INTEGER PRIMARY KEY,
        instance_id TEXT NOT NULL CHECK(instance_id = 'never-generated'));",
        )
        .unwrap();
    drop(connection);
    assert!(crate::init::init_sqlite(&path).is_err());
    let connection = Connection::open(&path).unwrap();
    let version: u32 = connection
        .query_row("SELECT MAX(version) FROM schema_version", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(version, 2);
    let exists: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name = 'ccp_admin_principals')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!exists);
    assert_eq!(rows(&connection, "message_packs"), 1);
    drop(connection);
    fs::remove_file(path).unwrap();
}

#[test]
fn newer_schema_is_refused_before_any_migration_or_foundation_creation() {
    let path = std::env::temp_dir().join(format!("ccp-foundation-future-{}.db", Uuid::new_v4()));
    let connection = legacy_database(&path);
    connection
        .execute(
            "UPDATE schema_version SET version = ?1",
            [crate::init::SCHEMA_VERSION + 1],
        )
        .unwrap();
    drop(connection);
    assert!(
        crate::init::init_sqlite(&path)
            .unwrap_err()
            .to_string()
            .contains("newer than supported")
    );
    let connection = Connection::open(&path).unwrap();
    let exists: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name = 'ccp_instance')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!exists);
    assert_eq!(rows(&connection, "message_packs"), 1);
    drop(connection);
    fs::remove_file(path).unwrap();
}

#[test]
fn corrupted_persisted_identity_and_bootstrap_fail_closed() {
    let mut connection = database();
    connection
        .execute("UPDATE ccp_instance SET instance_id = 'corrupt'", [])
        .unwrap();
    assert!(
        AccessStore::new(&mut connection)
            .unwrap()
            .instance_id()
            .is_err()
    );
    let mut store = AccessStore::new(&mut connection).unwrap();
    assert!(store.bootstrap_admin("\t", NOW).is_err());
    store.bootstrap_admin("operator", NOW).unwrap();
    assert!(store.bootstrap_admin("second", NOW).is_err());
}

#[test]
fn persistent_identity_and_grants_reopen_and_observe_another_connections_disable() {
    let path = std::env::temp_dir().join(format!("ccp-foundation-reopen-{}.db", Uuid::new_v4()));
    crate::init::init_sqlite(&path).unwrap();
    let mut f = fixture_with_connection(Connection::open(&path).unwrap());
    let client = enroll(&mut f, DataRole::ReadWrite);
    let instance = AccessStore::new(&mut f.connection)
        .unwrap()
        .instance_id()
        .unwrap();
    let (global, workspace) = (f.global, f.workspace);
    drop(f.connection);

    let mut reopened = Connection::open(&path).unwrap();
    let actor = Actor::Credential(client.credential_id);
    let mut store = AccessStore::new(&mut reopened).unwrap();
    assert_eq!(store.instance_id().unwrap(), instance);
    assert_eq!(
        store.visible_workspaces(actor, NOW).unwrap()[0].id,
        workspace
    );
    assert!(authorize(&reopened, actor, Action::WriteContext, Some(workspace), NOW).is_ok());

    let mut other_connection = Connection::open(&path).unwrap();
    AccessStore::new(&mut other_connection)
        .unwrap()
        .disable_client(Actor::Admin(global), client.client_id, NOW)
        .unwrap();
    assert!(authorize(&reopened, actor, Action::ReadContext, Some(workspace), NOW).is_err());
    assert_eq!(rows(&reopened, "ccp_clients"), 1);
    assert_eq!(rows(&reopened, "ccp_grants"), 1);
    drop(other_connection);
    drop(reopened);
    fs::remove_file(path).unwrap();
}

#[test]
fn legacy_name_collisions_preserve_both_workspace_ids_in_either_creation_order() {
    for legacy_first in [false, true] {
        let mut f = fixture();
        let create_legacy = |connection: &mut Connection| {
            let transaction = connection.transaction().unwrap();
            transaction
                .execute(
                    "INSERT INTO sessions(id, name) VALUES (99, 'collision')",
                    [],
                )
                .unwrap();
            let id = map_legacy_session(&transaction, 99).unwrap();
            transaction.commit().unwrap();
            id
        };
        let legacy = legacy_first.then(|| create_legacy(&mut f.connection));
        let new = AccessStore::new(&mut f.connection)
            .unwrap()
            .create_workspace(Actor::Admin(f.global), "collision", f.owner, NOW)
            .unwrap();
        let legacy = legacy.unwrap_or_else(|| create_legacy(&mut f.connection));
        assert_ne!(legacy, new);
        let transaction = f.connection.transaction().unwrap();
        migrate(&transaction).unwrap();
        assert_eq!(map_legacy_session(&transaction, 99).unwrap(), legacy);
        transaction.commit().unwrap();
        assert_eq!(rows(&f.connection, "ccp_workspaces"), 4);
        assert!(
            AccessStore::new(&mut f.connection)
                .unwrap()
                .create_workspace(Actor::Admin(f.global), "collision", f.owner, NOW)
                .is_err()
        );
    }
}

#[test]
fn migration_maps_a_colliding_session_created_by_an_older_binary() {
    let path = std::env::temp_dir().join(format!("ccp-name-collision-{}.db", Uuid::new_v4()));
    crate::init::init_sqlite(&path).unwrap();
    let f = fixture_with_connection(Connection::open(&path).unwrap());
    // An older binary writes only the native sessions table.
    f.connection
        .execute("INSERT INTO sessions(id, name) VALUES (99, 'alpha')", [])
        .unwrap();
    drop(f);
    crate::init::init_sqlite(&path).unwrap();
    let connection = Connection::open(&path).unwrap();
    assert_eq!(rows(&connection, "ccp_workspaces"), 3);
    let mapped: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM ccp_workspaces WHERE name = 'alpha'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(mapped, 2);
    drop(connection);
    fs::remove_file(path).unwrap();
}

#[test]
fn historical_http_schemas_are_refused_without_mutating_their_data() {
    for version in [3, 4] {
        let path = std::env::temp_dir().join(format!("ccp-http-draft-{}.db", Uuid::new_v4()));
        let connection = legacy_database(&path);
        connection
            .execute("UPDATE schema_version SET version = ?1", [version])
            .unwrap();
        connection.execute_batch("CREATE TABLE journal_checkpoint(id INTEGER PRIMARY KEY, generation TEXT, byte_offset INTEGER);
            INSERT INTO journal_checkpoint VALUES (1, 'keep-generation', 123);
            CREATE TABLE agent_statuses(status TEXT);
            INSERT INTO agent_statuses VALUES ('keep report');").unwrap();
        drop(connection);
        let error = crate::init::init_sqlite(&path).unwrap_err().to_string();
        assert!(error.contains("requires offline conversion"));
        let connection = Connection::open(&path).unwrap();
        let recorded: i64 = connection
            .query_row("SELECT MAX(version) FROM schema_version", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(recorded, version);
        let checkpoint: (String, i64) = connection
            .query_row(
                "SELECT generation, byte_offset FROM journal_checkpoint",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(checkpoint, ("keep-generation".to_string(), 123));
        let report: String = connection
            .query_row("SELECT status FROM agent_statuses", [], |row| row.get(0))
            .unwrap();
        assert_eq!(report, "keep report");
        assert_eq!(rows(&connection, "message_packs"), 1);
        let foundations: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name = 'ccp_instance')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!foundations);
        drop(connection);
        fs::remove_file(path).unwrap();
    }
}

mod admin_keys;
mod initial_window;
mod invitation_pages;
mod invitations;
mod redemption;

mod renewal;

mod recovery;
mod recovery_pages;

mod readmission;

mod local_admin;

mod provisioning;

mod discovery;

mod management;

mod content;
mod team;

mod entries;

mod shelves;

mod guidance;
