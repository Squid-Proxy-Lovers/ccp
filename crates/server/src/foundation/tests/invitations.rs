// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

fn initial(workspace_id: Uuid) -> InitialInvitationRequest {
    InitialInvitationRequest {
        workspace_id,
        role: DataRole::ReadWrite,
        max_clients: Some(2),
        expires_at: Some(EXPIRES),
    }
}

fn individual(workspace_id: Uuid) -> IndividualInvitationRequest {
    IndividualInvitationRequest {
        workspace_id,
        role: DataRole::Read,
        expires_at: EXPIRES,
    }
}

#[test]
fn initial_invitation_requires_a_real_positive_bound() {
    for (capacity, deadline, valid) in [
        (None, None, false),
        (Some(0), None, false),
        (None, Some(NOW), false),
        (Some(2), Some(-1), false),
        (Some(1), None, true),
        (None, Some(EXPIRES), true),
        (Some(3), Some(EXPIRES), true),
    ] {
        let mut f = fixture();
        let mut request = initial(f.workspace);
        request.max_clients = capacity;
        request.expires_at = deadline;
        let result = AccessStore::new(&mut f.connection)
            .unwrap()
            .issue_initial_invitation(Actor::Admin(f.admin), request, NOW);
        assert_eq!(result.is_ok(), valid);
        assert_eq!(rows(&f.connection, "ccp_invitations"), i64::from(valid));
        assert_eq!(
            rows(&f.connection, "ccp_initial_enrollment"),
            i64::from(valid)
        );
    }
}

#[test]
fn invitation_management_retains_admin_key_scope_and_never_stores_the_secret() {
    let mut f = fixture();
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    let key = store
        .issue_admin_key(
            Actor::Admin(f.owner),
            &AdminKeyRequest {
                principal_id: f.admin,
                scope: AdminKeyScope::Workspace(f.workspace),
                name: "admission key".to_string(),
                expires_at: None,
            },
            NOW,
        )
        .unwrap();
    let actor = store.authenticate_admin_key(key.secret(), NOW).unwrap();
    assert!(
        store
            .issue_initial_invitation(actor, initial(f.other_workspace), NOW)
            .is_err()
    );
    assert!(
        store
            .issue_initial_invitation(Actor::Admin(f.global), initial(f.workspace), NOW)
            .is_err()
    );
    let invite = store
        .issue_initial_invitation(actor, initial(f.workspace), NOW)
        .unwrap();
    let status = store
        .invitation_status(actor, f.workspace, invite.id, NOW)
        .unwrap();
    assert!(status.issuer_authorized);
    assert!(!status.initial_closed);
    assert_eq!(status.max_uses, Some(2));
    assert_eq!(status.uses, 0);
    assert_eq!(status.expires_at, Some(EXPIRES));
    assert!(
        store
            .invitation_status(actor, f.other_workspace, invite.id, NOW)
            .is_err()
    );
    assert!(
        store
            .revoke_invitation(actor, f.other_workspace, invite.id, NOW)
            .is_err()
    );
    let verifier: Vec<u8> = f
        .connection
        .query_row("SELECT token_hash FROM ccp_invitations", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(verifier.len(), 32);
    assert_ne!(verifier, invite.secret().as_bytes());
}

#[test]
fn initial_group_cannot_reopen_after_close_revoke_or_existing_admission() {
    let mut f = fixture();
    let request = initial(f.workspace);
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    let invite = store
        .issue_initial_invitation(Actor::Admin(f.admin), request, NOW)
        .unwrap();
    assert!(
        store
            .issue_initial_invitation(Actor::Admin(f.owner), request, NOW)
            .is_err()
    );
    store
        .revoke_invitation(Actor::Admin(f.owner), f.workspace, invite.id, NOW)
        .unwrap();
    let status = store
        .invitation_status(Actor::Admin(f.owner), f.workspace, invite.id, NOW)
        .unwrap();
    assert!(status.revoked && status.initial_closed);
    assert!(
        store
            .issue_initial_invitation(Actor::Admin(f.owner), request, NOW)
            .is_err()
    );
    let mut other = fixture();
    enroll(&mut other, DataRole::Read);
    assert!(
        AccessStore::new(&mut other.connection)
            .unwrap()
            .issue_initial_invitation(Actor::Admin(other.owner), initial(other.workspace), NOW)
            .is_err()
    );
}

#[test]
fn individual_admission_seals_unused_initial_phase_and_requires_explicit_group_close() {
    let mut f = fixture();
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    let group = store
        .issue_initial_invitation(Actor::Admin(f.admin), initial(f.workspace), NOW)
        .unwrap();
    assert!(
        store
            .issue_individual_invitation(Actor::Admin(f.admin), individual(f.workspace), NOW)
            .is_err()
    );
    store
        .close_initial_enrollment(Actor::Admin(f.admin), f.workspace, NOW)
        .unwrap();
    store
        .close_initial_enrollment(Actor::Admin(f.admin), f.workspace, NOW)
        .unwrap();
    let invite = store
        .issue_individual_invitation(Actor::Admin(f.admin), individual(f.workspace), NOW)
        .unwrap();
    let status = store
        .invitation_status(Actor::Admin(f.admin), f.workspace, invite.id, NOW)
        .unwrap();
    assert_eq!(status.max_uses, Some(1));
    assert!(
        store
            .invitation_status(Actor::Admin(f.admin), f.workspace, group.id, NOW)
            .unwrap()
            .initial_closed
    );
    assert!(
        store
            .issue_initial_invitation(Actor::Admin(f.admin), initial(f.workspace), NOW)
            .is_err()
    );
    assert_eq!(rows(&f.connection, "ccp_enrollment_events"), 2);
    let mut other = fixture();
    let mut store = AccessStore::new(&mut other.connection).unwrap();
    store
        .issue_individual_invitation(Actor::Admin(other.owner), individual(other.workspace), NOW)
        .unwrap();
    assert!(
        store
            .issue_initial_invitation(Actor::Admin(other.owner), initial(other.workspace), NOW)
            .is_err()
    );
}

#[test]
fn observed_deadline_closure_survives_clock_rollback_and_cannot_reopen() {
    let mut f = fixture();
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    let invite = store
        .issue_initial_invitation(Actor::Admin(f.admin), initial(f.workspace), NOW)
        .unwrap();
    assert!(
        !store
            .invitation_status(Actor::Admin(f.admin), f.workspace, invite.id, EXPIRES - 1)
            .unwrap()
            .initial_closed
    );
    assert!(
        store
            .invitation_status(Actor::Admin(f.admin), f.workspace, invite.id, EXPIRES)
            .unwrap()
            .initial_closed
    );
    assert!(
        store
            .invitation_status(Actor::Admin(f.admin), f.workspace, invite.id, NOW)
            .unwrap()
            .initial_closed
    );
    assert!(
        store
            .issue_initial_invitation(Actor::Admin(f.admin), initial(f.workspace), NOW)
            .is_err()
    );
    assert_eq!(rows(&f.connection, "ccp_enrollment_events"), 2);
    let event: String = f
        .connection
        .query_row(
            "SELECT event FROM ccp_enrollment_events ORDER BY id DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(event, "deadline_close");
}

#[test]
fn exhausted_persisted_capacity_closes_the_initial_window() {
    let mut f = fixture();
    let invite = AccessStore::new(&mut f.connection)
        .unwrap()
        .issue_initial_invitation(Actor::Admin(f.admin), initial(f.workspace), NOW)
        .unwrap();
    // This batch validates stored-policy transitions, not redemption/counter consumption.
    f.connection
        .execute(
            "UPDATE ccp_invitations SET uses = max_uses WHERE id = ?1",
            [invite.id.to_string()],
        )
        .unwrap();
    let status = AccessStore::new(&mut f.connection)
        .unwrap()
        .invitation_status(Actor::Admin(f.admin), f.workspace, invite.id, NOW)
        .unwrap();
    assert_eq!(status.uses, 2);
    assert!(status.initial_closed);
    let event: String = f
        .connection
        .query_row(
            "SELECT event FROM ccp_enrollment_events ORDER BY id DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(event, "capacity_close");
}

#[test]
fn expired_group_allows_individual_issuance_without_resetting_its_deadline() {
    let mut f = fixture();
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    let group = store
        .issue_initial_invitation(Actor::Admin(f.admin), initial(f.workspace), NOW)
        .unwrap();
    let mut request = individual(f.workspace);
    request.expires_at = EXPIRES + 100;
    store
        .issue_individual_invitation(Actor::Admin(f.admin), request, EXPIRES)
        .unwrap();
    let status = store
        .invitation_status(Actor::Admin(f.admin), f.workspace, group.id, NOW)
        .unwrap();
    assert!(status.initial_closed);
    assert_eq!(status.expires_at, Some(EXPIRES));
}

#[test]
fn archive_freezes_new_invites_but_preserves_scoped_security_controls() {
    let mut f = fixture();
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    let group = store
        .issue_initial_invitation(Actor::Admin(f.admin), initial(f.workspace), NOW)
        .unwrap();
    store
        .set_archived(Actor::Admin(f.owner), f.workspace, true, NOW)
        .unwrap();
    assert!(
        store
            .issue_individual_invitation(Actor::Admin(f.admin), individual(f.workspace), NOW)
            .is_err()
    );
    assert!(
        store
            .issue_initial_invitation(Actor::Admin(f.admin), initial(f.workspace), NOW)
            .is_err()
    );
    assert!(
        store
            .revoke_invitation(Actor::Admin(f.global), f.workspace, group.id, NOW)
            .is_err()
    );
    store
        .revoke_invitation(Actor::Admin(f.admin), f.workspace, group.id, NOW)
        .unwrap();
    assert!(
        !store
            .invitation_status(Actor::Admin(f.owner), f.workspace, group.id, NOW)
            .unwrap()
            .issuer_authorized
    );
    store
        .set_archived(Actor::Admin(f.owner), f.workspace, false, NOW)
        .unwrap();
    assert!(
        store
            .invitation_status(Actor::Admin(f.owner), f.workspace, group.id, NOW)
            .unwrap()
            .initial_closed
    );
}

#[test]
fn invitation_status_rechecks_current_issuer_role_and_expiring_key() {
    let mut f = fixture();
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    let key = store
        .issue_admin_key(
            Actor::Admin(f.owner),
            &AdminKeyRequest {
                principal_id: f.admin,
                scope: AdminKeyScope::Workspace(f.workspace),
                name: "temporary".to_string(),
                expires_at: Some(NOW + 10),
            },
            NOW,
        )
        .unwrap();
    let actor = store.authenticate_admin_key(key.secret(), NOW).unwrap();
    let invite = store
        .issue_initial_invitation(actor, initial(f.workspace), NOW)
        .unwrap();
    assert!(
        store
            .invitation_status(Actor::Admin(f.owner), f.workspace, invite.id, NOW)
            .unwrap()
            .issuer_authorized
    );
    assert!(
        !store
            .invitation_status(Actor::Admin(f.owner), f.workspace, invite.id, NOW + 10)
            .unwrap()
            .issuer_authorized
    );
    store
        .revoke_workspace_admin(Actor::Admin(f.owner), f.workspace, f.admin, NOW)
        .unwrap();
    assert!(
        !store
            .invitation_status(Actor::Admin(f.owner), f.workspace, invite.id, NOW)
            .unwrap()
            .issuer_authorized
    );
}

#[test]
fn invitation_and_window_changes_roll_back_on_audit_failure() {
    let mut f = fixture();
    f.connection.execute_batch("CREATE TRIGGER reject_invites BEFORE INSERT ON ccp_access_audit WHEN NEW.action = 'issue_initial_invitation' BEGIN SELECT RAISE(ABORT, 'audit down'); END;").unwrap();
    assert!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .issue_initial_invitation(Actor::Admin(f.admin), initial(f.workspace), NOW)
            .is_err()
    );
    for table in [
        "ccp_invitations",
        "ccp_initial_enrollment",
        "ccp_enrollment_events",
    ] {
        assert_eq!(rows(&f.connection, table), 0);
    }
    f.connection
        .execute_batch("DROP TRIGGER reject_invites")
        .unwrap();
    let invite = AccessStore::new(&mut f.connection)
        .unwrap()
        .issue_initial_invitation(Actor::Admin(f.admin), initial(f.workspace), NOW)
        .unwrap();
    f.connection.execute_batch("CREATE TRIGGER reject_close BEFORE INSERT ON ccp_access_audit WHEN NEW.action IN ('close_initial_enrollment','revoke_invitation') BEGIN SELECT RAISE(ABORT, 'audit down'); END;").unwrap();
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    assert!(
        store
            .close_initial_enrollment(Actor::Admin(f.admin), f.workspace, NOW)
            .is_err()
    );
    assert!(
        store
            .revoke_invitation(Actor::Admin(f.admin), f.workspace, invite.id, NOW)
            .is_err()
    );
    let status = store
        .invitation_status(Actor::Admin(f.owner), f.workspace, invite.id, NOW)
        .unwrap();
    assert!(!status.revoked && !status.initial_closed);
    assert_eq!(rows(&f.connection, "ccp_enrollment_events"), 1);
}

#[test]
fn invalid_individual_expiry_and_foreign_window_bindings_are_refused() {
    let mut f = fixture();
    let mut request = individual(f.workspace);
    request.expires_at = NOW;
    assert!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .issue_individual_invitation(Actor::Admin(f.owner), request, NOW)
            .is_err()
    );
    assert!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .issue_initial_invitation(Actor::Admin(f.owner), initial(f.workspace), -1)
            .is_err()
    );
    let invite = AccessStore::new(&mut f.connection)
        .unwrap()
        .issue_initial_invitation(Actor::Admin(f.owner), initial(f.workspace), NOW)
        .unwrap();
    assert!(
        f.connection
            .execute(
                "INSERT INTO ccp_initial_enrollment(workspace_id, invitation_id) VALUES (?1, ?2)",
                params![f.other_workspace.to_string(), invite.id.to_string()]
            )
            .is_err()
    );
    assert!(
        f.connection
            .execute(
                "UPDATE ccp_invitations SET uses = 3 WHERE id = ?1",
                [invite.id.to_string()]
            )
            .is_err()
    );
}

#[test]
fn schema6_upgrade_keeps_keys_and_initial_closure_survives_restart() {
    let path = std::env::temp_dir().join(format!("ccp-invitation-upgrade-{}.db", Uuid::new_v4()));
    crate::init::init_sqlite(&path).unwrap();
    let mut f = fixture_with_connection(Connection::open(&path).unwrap());
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    let key = store
        .issue_admin_key(
            Actor::Admin(f.owner),
            &AdminKeyRequest {
                principal_id: f.admin,
                scope: AdminKeyScope::Workspace(f.workspace),
                name: "existing key".to_string(),
                expires_at: None,
            },
            NOW,
        )
        .unwrap();
    // Enrollment additions are additive: removing their empty tables recreates schema6.
    f.connection
        .execute_batch(
            "DROP TRIGGER IF EXISTS ccp_no_incomplete_workspace_deletion; DROP TABLE ccp_invitation_receipts; DROP TABLE ccp_invitation_closures; DROP TABLE ccp_credential_certificates;
        DROP INDEX ccp_grant_workspace_identity;
        DROP TABLE ccp_enrollment_events; DROP TABLE ccp_initial_enrollment;
        DROP TABLE ccp_invitations; UPDATE schema_version SET version = 6;",
        )
        .unwrap();
    let workspace = f.workspace;
    drop(f);
    crate::init::init_sqlite(&path).unwrap();
    let mut connection = Connection::open(&path).unwrap();
    let mut store = AccessStore::new(&mut connection).unwrap();
    let actor = store.authenticate_admin_key(key.secret(), NOW).unwrap();
    let group = store
        .issue_initial_invitation(actor, initial(workspace), NOW)
        .unwrap();
    store
        .close_initial_enrollment(actor, workspace, NOW)
        .unwrap();
    drop(connection);
    crate::init::init_sqlite(&path).unwrap();
    let mut connection = Connection::open(&path).unwrap();
    let mut store = AccessStore::new(&mut connection).unwrap();
    let actor = store.authenticate_admin_key(key.secret(), NOW).unwrap();
    assert!(
        store
            .invitation_status(actor, workspace, group.id, NOW)
            .unwrap()
            .initial_closed
    );
    assert!(
        store
            .issue_initial_invitation(actor, initial(workspace), NOW)
            .is_err()
    );
    let invalid_fks: i64 = connection
        .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(invalid_fks, 0);
    drop(connection);
    fs::remove_file(path).unwrap();
}

#[test]
fn concurrent_opening_creates_only_one_initial_invitation() {
    use std::sync::{Arc, Barrier};

    let path = std::env::temp_dir().join(format!("ccp-invitation-race-{}.db", Uuid::new_v4()));
    crate::init::init_sqlite(&path).unwrap();
    let f = fixture_with_connection(Connection::open(&path).unwrap());
    let actor = Actor::Admin(f.admin);
    let workspace = f.workspace;
    drop(f);
    let barrier = Arc::new(Barrier::new(8));
    let workers: Vec<_> = (0..8)
        .map(|_| {
            let path = path.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                let mut connection = Connection::open(path).unwrap();
                connection
                    .busy_timeout(std::time::Duration::from_secs(5))
                    .unwrap();
                let mut store = AccessStore::new(&mut connection).unwrap();
                barrier.wait();
                store
                    .issue_initial_invitation(actor, initial(workspace), NOW)
                    .map(|_| ())
                    .map_err(|error| error.to_string())
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
            error.contains("initial enrollment has already started or closed"),
            "{error}"
        );
    }
    let connection = Connection::open(&path).unwrap();
    assert_eq!(rows(&connection, "ccp_invitations"), 1);
    assert_eq!(rows(&connection, "ccp_initial_enrollment"), 1);
    assert_eq!(rows(&connection, "ccp_enrollment_events"), 1);
    drop(connection);
    fs::remove_file(path).unwrap();
}
