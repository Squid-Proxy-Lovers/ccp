// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use sha2::{Digest, Sha256};
use std::time::Duration;

fn request(owner: InitialOwner) -> OwnerProvisioning {
    OwnerProvisioning {
        owner,
        key_expires_at: None,
    }
}
fn global_key(f: &mut Fixture) -> IssuedAdminKey {
    AccessStore::new(&mut f.connection)
        .unwrap()
        .issue_admin_key(
            Actor::Admin(f.global),
            &AdminKeyRequest {
                principal_id: f.global,
                scope: AdminKeyScope::Instance,
                name: "operator".into(),
                expires_at: None,
            },
            NOW,
        )
        .unwrap()
}
fn create(
    f: &mut Fixture,
    actor: Actor,
    name: &str,
    req: &OwnerProvisioning,
) -> anyhow::Result<ProvisionedWorkspace> {
    AccessStore::new(&mut f.connection)?.create_workspace_with_owner_key(actor, name, req, NOW)
}
fn assign(
    f: &mut Fixture,
    actor: Actor,
    workspace: Uuid,
    req: &OwnerProvisioning,
) -> anyhow::Result<ProvisionedWorkspace> {
    AccessStore::new(&mut f.connection)?.assign_initial_owner_with_key(actor, workspace, req, NOW)
}
fn counts(f: &Fixture) -> Vec<i64> {
    [
        "ccp_admin_principals",
        "ccp_workspaces",
        "ccp_admin_keys",
        "ccp_access_audit",
    ]
    .iter()
    .map(|table| rows(&f.connection, table))
    .collect()
}
fn unowned(f: &Fixture, state: &str) -> Uuid {
    let id = Uuid::new_v4();
    f.connection
        .execute(
            "INSERT INTO ccp_workspaces(id, name, state) VALUES (?1, ?2, ?3)",
            params![id.to_string(), id.to_string(), state],
        )
        .unwrap();
    id
}

#[test]
fn global_creation_delivers_one_new_owner_key_with_strict_scope_and_original_key_audit() {
    let mut f = fixture();
    let key = global_key(&mut f);
    let actor = Actor::AdminKey(key.id);
    let principal_count = rows(&f.connection, "ccp_admin_principals");
    let result = create(
        &mut f,
        actor,
        "  project  ",
        &request(InitialOwner::NewPrincipal("  new owner  ".into())),
    )
    .unwrap();
    let owner = result.owner_key.principal_id;
    assert_ne!(owner, f.owner);
    assert_ne!(owner, f.global);
    assert_eq!(
        rows(&f.connection, "ccp_admin_principals"),
        principal_count + 1
    );
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    let owner_actor = store
        .authenticate_admin_key(result.owner_key.secret(), NOW)
        .unwrap();
    assert_eq!(owner_actor, Actor::AdminKey(result.owner_key.id));
    assert!(
        authorize(
            &f.connection,
            owner_actor,
            Action::WriteContext,
            Some(result.workspace_id),
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
    assert!(
        authorize(
            &f.connection,
            owner_actor,
            Action::ReadContext,
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
            Some(result.workspace_id),
            NOW
        )
        .is_ok()
    );
    assert!(
        authorize(
            &f.connection,
            actor,
            Action::WriteContext,
            Some(result.workspace_id),
            NOW
        )
        .is_err()
    );
    let names: (String, String) = f.connection.query_row("SELECT w.name, p.name FROM ccp_workspaces w JOIN ccp_admin_principals p ON p.id = w.owner_admin_id WHERE w.id = ?1", [result.workspace_id.to_string()], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    assert_eq!(names, ("project".into(), "new owner".into()));
    assert_initial_records(&f, &result, &key);
}

fn assert_initial_records(f: &Fixture, result: &ProvisionedWorkspace, key: &IssuedAdminKey) {
    let row: (String, Vec<u8>, Option<i64>) = f
        .connection
        .query_row(
            "SELECT workspace_id, token_hash, expires_at FROM ccp_admin_keys WHERE id = ?1",
            [result.owner_key.id.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(row.0, result.workspace_id.to_string());
    assert_eq!(
        row.1,
        Sha256::digest(result.owner_key.secret().as_bytes()).to_vec()
    );
    assert_eq!(row.2, None);
    let audit: (String, String, String, String) = f.connection.query_row("SELECT actor_kind, actor_id, admin_key_id, workspace_id FROM ccp_access_audit WHERE action = 'issue_initial_owner_key' AND target_id = ?1", [result.owner_key.id.to_string()], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert_eq!(
        audit,
        (
            "admin".into(),
            f.global.to_string(),
            key.id.to_string(),
            result.workspace_id.to_string()
        )
    );
    let global_role: bool = f
        .connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM ccp_server_admins WHERE admin_id = ?1)",
            [result.owner_key.principal_id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!global_role);
}

#[test]
fn existing_global_principal_receives_only_workspace_scope_and_no_repeat_key_delivery() {
    let mut f = fixture();
    let key = global_key(&mut f);
    let actor = Actor::AdminKey(key.id);
    let global = f.global;
    let req = request(InitialOwner::Existing(global));
    let before = rows(&f.connection, "ccp_admin_principals");
    let result = create(&mut f, actor, "owned", &req).unwrap();
    assert_eq!(result.owner_key.principal_id, global);
    assert_eq!(rows(&f.connection, "ccp_admin_principals"), before);
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    let scoped = store
        .authenticate_admin_key(result.owner_key.secret(), NOW)
        .unwrap();
    assert!(
        store
            .issue_admin_key(
                actor,
                &AdminKeyRequest {
                    principal_id: global,
                    scope: AdminKeyScope::Workspace(result.workspace_id),
                    name: "escalated".into(),
                    expires_at: None
                },
                NOW
            )
            .is_err()
    );
    assert!(
        store
            .issue_admin_key(
                scoped,
                &AdminKeyRequest {
                    principal_id: global,
                    scope: AdminKeyScope::Workspace(result.workspace_id),
                    name: "owner replacement".into(),
                    expires_at: None
                },
                NOW
            )
            .is_ok()
    );
    assert!(authorize(&f.connection, scoped, Action::SaveGlobalBoard, None, NOW).is_err());
    assert!(
        authorize(
            &f.connection,
            scoped,
            Action::ReadContext,
            Some(f.workspace),
            NOW
        )
        .is_err()
    );
    let before = counts(&f);
    assert!(assign(&mut f, actor, result.workspace_id, &req).is_err());
    assert!(
        create(
            &mut f,
            actor,
            "owned",
            &request(InitialOwner::NewPrincipal("orphan".into()))
        )
        .is_err()
    );
    assert_eq!(counts(&f), before);
}

#[test]
fn only_current_global_creation_authority_can_provision_owners() {
    let mut f = fixture();
    let key = global_key(&mut f);
    let credential = enroll(&mut f, DataRole::ReadWrite).credential_id;
    let workspace = unowned(&f, "active");
    let owner = f.owner;
    let scoped_key = AccessStore::new(&mut f.connection)
        .unwrap()
        .issue_admin_key(
            Actor::Admin(owner),
            &AdminKeyRequest {
                principal_id: owner,
                scope: AdminKeyScope::Workspace(f.workspace),
                name: "owner".into(),
                expires_at: None,
            },
            NOW,
        )
        .unwrap();
    let req = request(InitialOwner::NewPrincipal("new".into()));
    let before = counts(&f);
    for actor in [
        Actor::Admin(f.owner),
        Actor::Admin(f.admin),
        Actor::AdminKey(scoped_key.id),
        Actor::Credential(credential),
        Actor::AdminKey(Uuid::new_v4()),
    ] {
        assert!(create(&mut f, actor, "denied", &req).is_err());
        assert!(assign(&mut f, actor, workspace, &req).is_err());
        assert_eq!(counts(&f), before);
    }
    for change in ["revoke", "disable", "remove_role", "expiry"] {
        match change {
            "revoke" => {
                f.connection
                    .execute(
                        "UPDATE ccp_admin_keys SET revoked_at = 1 WHERE id = ?1",
                        [key.id.to_string()],
                    )
                    .unwrap();
            }
            "disable" => {
                f.connection
                    .execute(
                        "UPDATE ccp_admin_principals SET enabled = 0 WHERE id = ?1",
                        [f.global.to_string()],
                    )
                    .unwrap();
            }
            "remove_role" => {
                f.connection
                    .execute(
                        "DELETE FROM ccp_server_admins WHERE admin_id = ?1",
                        [f.global.to_string()],
                    )
                    .unwrap();
            }
            _ => {
                f.connection
                    .execute(
                        "UPDATE ccp_admin_keys SET expires_at = ?1 WHERE id = ?2",
                        params![NOW, key.id.to_string()],
                    )
                    .unwrap();
            }
        }
        let issuer = Actor::AdminKey(key.id);
        assert!(create(&mut f, issuer, "denied issuer", &req).is_err());
        assert!(assign(&mut f, issuer, workspace, &req).is_err());
        assert_eq!(counts(&f), before);
        f.connection
            .execute(
                "UPDATE ccp_admin_keys SET revoked_at = NULL, expires_at = NULL WHERE id = ?1",
                [key.id.to_string()],
            )
            .unwrap();
        f.connection
            .execute(
                "UPDATE ccp_admin_principals SET enabled = 1 WHERE id = ?1",
                [f.global.to_string()],
            )
            .unwrap();
        f.connection
            .execute(
                "INSERT OR IGNORE INTO ccp_server_admins(admin_id) VALUES (?1)",
                [f.global.to_string()],
            )
            .unwrap();
    }
}

#[test]
fn initial_assignment_preserves_archived_state_closed_enrollment_and_legacy_display_owner() {
    let mut f = fixture();
    let key = global_key(&mut f);
    let actor = Actor::AdminKey(key.id);
    f.connection.execute("INSERT INTO sessions(id, name, owner, description) VALUES (7, 'legacy', 'owner', 'keep')", []).unwrap();
    let legacy = map_legacy_session(&f.connection, 7).unwrap();
    f.connection
        .execute(
            "UPDATE ccp_workspaces SET state = 'archived' WHERE id = ?1",
            [legacy.to_string()],
        )
        .unwrap();
    f.connection
        .execute(
            "INSERT INTO ccp_initial_enrollment(workspace_id, closed_at) VALUES (?1, 1)",
            [legacy.to_string()],
        )
        .unwrap();
    let result = assign(
        &mut f,
        actor,
        legacy,
        &request(InitialOwner::NewPrincipal("owner".into())),
    )
    .unwrap();
    assert_ne!(result.owner_key.principal_id, f.owner);
    let scoped = AccessStore::new(&mut f.connection)
        .unwrap()
        .authenticate_admin_key(result.owner_key.secret(), NOW)
        .unwrap();
    assert!(
        authorize(
            &f.connection,
            scoped,
            Action::ReadContext,
            Some(legacy),
            NOW
        )
        .is_ok()
    );
    assert!(
        authorize(
            &f.connection,
            scoped,
            Action::WriteContext,
            Some(legacy),
            NOW
        )
        .is_err()
    );
    assert!(
        authorize(
            &f.connection,
            scoped,
            Action::AdmitClient,
            Some(legacy),
            NOW
        )
        .is_err()
    );
    let state: String = f
        .connection
        .query_row(
            "SELECT state FROM ccp_workspaces WHERE id = ?1",
            [legacy.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "archived");
    let original: (String, String) = f
        .connection
        .query_row(
            "SELECT owner, description FROM sessions WHERE id = 7",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(original, ("owner".into(), "keep".into()));
    let closed: i64 = f
        .connection
        .query_row(
            "SELECT closed_at FROM ccp_initial_enrollment WHERE workspace_id = ?1",
            [legacy.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(closed, 1);
    let existing_owner = f.owner;
    assert!(
        assign(
            &mut f,
            actor,
            legacy,
            &request(InitialOwner::Existing(existing_owner))
        )
        .is_err()
    );
}

#[test]
fn missing_purged_owned_disabled_or_invalid_targets_never_leave_a_new_key_or_principal() {
    let mut f = fixture();
    let key = global_key(&mut f);
    let actor = Actor::AdminKey(key.id);
    let purged = unowned(&f, "purged");
    let active = unowned(&f, "active");
    let owned = f.workspace;
    let req = request(InitialOwner::NewPrincipal("no orphan".into()));
    let before = counts(&f);
    for target in [Uuid::new_v4(), purged, owned] {
        assert!(assign(&mut f, actor, target, &req).is_err());
        assert_eq!(counts(&f), before);
    }
    f.connection
        .execute(
            "UPDATE ccp_admin_principals SET enabled = 0 WHERE id = ?1",
            [f.other_owner.to_string()],
        )
        .unwrap();
    for owner in [f.other_owner, Uuid::new_v4()] {
        assert!(
            create(
                &mut f,
                actor,
                "denied",
                &request(InitialOwner::Existing(owner))
            )
            .is_err()
        );
        assert!(
            assign(
                &mut f,
                actor,
                active,
                &request(InitialOwner::Existing(owner))
            )
            .is_err()
        );
        assert_eq!(counts(&f), before);
    }
    for name in ["", "\n", &"x".repeat(257)] {
        assert!(create(&mut f, actor, name, &req).is_err());
        assert!(
            assign(
                &mut f,
                actor,
                active,
                &request(InitialOwner::NewPrincipal(name.into()))
            )
            .is_err()
        );
        assert_eq!(counts(&f), before);
    }
}

#[test]
fn owner_key_expiry_is_optional_and_never_exceeds_the_requested_lifetime() {
    let mut f = fixture();
    let key = global_key(&mut f);
    let actor = Actor::AdminKey(key.id);
    let owner = f.owner;
    for expiry in [Some(NOW), Some(NOW - 1)] {
        let before = counts(&f);
        let req = OwnerProvisioning {
            owner: InitialOwner::Existing(owner),
            key_expires_at: expiry,
        };
        assert!(create(&mut f, actor, "expired", &req).is_err());
        assert_eq!(counts(&f), before);
    }
    let req = OwnerProvisioning {
        owner: InitialOwner::Existing(owner),
        key_expires_at: Some(NOW + 1),
    };
    let result = create(&mut f, actor, "short", &req).unwrap();
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    assert!(
        store
            .authenticate_admin_key(result.owner_key.secret(), NOW)
            .is_ok()
    );
    assert!(
        store
            .authenticate_admin_key(result.owner_key.secret(), NOW + 1)
            .is_err()
    );
    assert!(
        store
            .create_workspace_with_owner_key(actor, "negative time", &req, -1)
            .is_err()
    );
}

#[test]
fn key_and_final_audit_failures_rollback_both_creation_and_first_appointment() {
    for failure in [
        "CREATE TRIGGER fail_key BEFORE INSERT ON ccp_admin_keys BEGIN SELECT RAISE(ABORT, 'key unavailable'); END;",
        "CREATE TRIGGER fail_audit BEFORE INSERT ON ccp_access_audit WHEN NEW.action = 'issue_initial_owner_key' BEGIN SELECT RAISE(ABORT, 'audit unavailable'); END;",
    ] {
        let mut f = fixture();
        let key = global_key(&mut f);
        let actor = Actor::AdminKey(key.id);
        let target = unowned(&f, "active");
        f.connection.execute_batch(failure).unwrap();
        let before = counts(&f);
        let req = request(InitialOwner::NewPrincipal("rollback".into()));
        assert!(create(&mut f, actor, "rollback", &req).is_err());
        assert_eq!(counts(&f), before);
        assert!(assign(&mut f, actor, target, &req).is_err());
        assert_eq!(counts(&f), before);
        let owner: Option<String> = f
            .connection
            .query_row(
                "SELECT owner_admin_id FROM ccp_workspaces WHERE id = ?1",
                [target.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(owner, None);
    }
}

struct DiskFixture {
    directory: std::path::PathBuf,
    path: std::path::PathBuf,
    key: IssuedAdminKey,
    workspace: Uuid,
}
impl DiskFixture {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!("ccp-initial-owner-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("ccp.sqlite3");
        crate::init::init_sqlite(&path).unwrap();
        let mut db = Connection::open(&path).unwrap();
        let key = AccessStore::new(&mut db)
            .unwrap()
            .bootstrap_admin_key("operator", NOW)
            .unwrap();
        db.execute(
            "INSERT INTO sessions(id, name, owner) VALUES (7, 'legacy', 'display only')",
            [],
        )
        .unwrap();
        let workspace = map_legacy_session(&db, 7).unwrap();
        Self {
            directory,
            path,
            key,
            workspace,
        }
    }
}
impl Drop for DiskFixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.directory).unwrap();
    }
}

#[test]
fn competing_first_appointments_commit_exactly_one_owner_key_and_no_orphan_principals() {
    use std::sync::{Arc, Barrier};
    let f = DiskFixture::new();
    let gate = Arc::new(Barrier::new(8));
    let actor = Actor::AdminKey(f.key.id);
    let workers: Vec<_> = (0..8)
        .map(|index| {
            let path = f.path.clone();
            let gate = gate.clone();
            let workspace = f.workspace;
            std::thread::spawn(move || {
                let mut db = Connection::open(path).unwrap();
                db.busy_timeout(Duration::from_secs(20)).unwrap();
                let mut store = AccessStore::new(&mut db).unwrap();
                gate.wait();
                store.assign_initial_owner_with_key(
                    actor,
                    workspace,
                    &request(InitialOwner::NewPrincipal(format!("owner {index}"))),
                    NOW,
                )
            })
        })
        .collect();
    let results: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    for error in results.iter().filter_map(|result| result.as_ref().err()) {
        assert!(
            error
                .to_string()
                .contains("target does not exist or state changed"),
            "{error:#}"
        );
    }
    let winner = results.into_iter().find_map(Result::ok).unwrap();
    let mut db = Connection::open(&f.path).unwrap();
    let actor = AccessStore::new(&mut db)
        .unwrap()
        .authenticate_admin_key(winner.owner_key.secret(), NOW)
        .unwrap();
    assert!(authorize(&db, actor, Action::WriteContext, Some(f.workspace), NOW).is_ok());
    assert_eq!(rows(&db, "ccp_admin_principals"), 2);
    assert_eq!(rows(&db, "ccp_admin_keys"), 2);
    let events: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM ccp_access_audit WHERE action = 'issue_initial_owner_key'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(events, 1);
    let owner: String = db
        .query_row(
            "SELECT owner_admin_id FROM ccp_workspaces WHERE id = ?1",
            [f.workspace.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(owner, winner.owner_key.principal_id.to_string());
    let display: String = db
        .query_row("SELECT owner FROM sessions WHERE id = 7", [], |r| r.get(0))
        .unwrap();
    assert_eq!(display, "display only");
}

#[test]
fn lost_first_key_delivery_requires_existing_owner_authority_or_physical_reset() {
    let f = DiskFixture::new();
    let actor = Actor::AdminKey(f.key.id);
    let req = request(InitialOwner::NewPrincipal("new owner".into()));
    let principal = {
        let mut db = Connection::open(&f.path).unwrap();
        let mut store = AccessStore::new(&mut db).unwrap();
        let delivered = store
            .assign_initial_owner_with_key(actor, f.workspace, &req, NOW)
            .unwrap();
        let principal = delivered.owner_key.principal_id;
        drop(delivered);
        assert!(
            store
                .assign_initial_owner_with_key(actor, f.workspace, &req, NOW)
                .is_err()
        );
        assert!(
            store
                .issue_admin_key(
                    actor,
                    &AdminKeyRequest {
                        principal_id: principal,
                        scope: AdminKeyScope::Workspace(f.workspace),
                        name: "remote reset denied".into(),
                        expires_at: None
                    },
                    NOW
                )
                .is_err()
        );
        principal
    };
    let info = crate::local_admin::info(&f.directory).unwrap();
    assert!(
        info.workspaces
            .iter()
            .any(|w| w.owner_id == Some(principal.to_string()))
    );
    let replacement = crate::local_admin::reset_key(
        &f.directory,
        LocalKeyTarget::WorkspaceOwner(f.workspace),
        None,
    )
    .unwrap();
    assert_eq!(replacement.key.principal_id, principal);
    assert_eq!(replacement.revoked_keys, 1);
    let mut db = Connection::open(&f.path).unwrap();
    let current = AccessStore::new(&mut db)
        .unwrap()
        .authenticate_admin_key(replacement.key.secret(), NOW)
        .unwrap();
    assert!(authorize(&db, current, Action::WriteContext, Some(f.workspace), NOW).is_ok());
    assert_eq!(rows(&db, "ccp_admin_principals"), 2);
    assert!(
        AccessStore::new(&mut db)
            .unwrap()
            .authenticate_admin_key(f.key.secret(), NOW)
            .is_ok()
    );
}
