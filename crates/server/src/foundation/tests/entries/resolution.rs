// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use sha2::{Digest, Sha256};

fn append(id: Uuid, credential: Uuid) -> (EntryMutation, EntryOperationTarget) {
    let request = command(EntryChange::Append {
        id,
        expected_revision: 1,
        text: "+once".into(),
    });
    // Independent serialization of the published entry-mutation-1 contract.
    let bytes = serde_json::to_vec(&serde_json::json!([
        "entry-mutation-1",
        "append",
        "agent one",
        [id.to_string(), 1, "+once"]
    ]))
    .unwrap();
    let target = EntryOperationTarget {
        operation_id: request.operation_id,
        credential_id: credential,
        request_hash: Sha256::digest(bytes).into(),
    };
    (request, target)
}

fn resolve(
    f: &mut Fixture,
    actor: Actor,
    target: &EntryOperationTarget,
) -> anyhow::Result<EntryOperationOutcome> {
    AccessStore::new(&mut f.connection)?.resolve_entry_operation(actor, f.workspace, target, NOW)
}

#[test]
fn cancellation_is_terminal_for_exact_request_changed_input_and_new_same_client_credentials() {
    let mut f = fixture();
    let owner = enroll(&mut f, DataRole::ReadWrite);
    let book = book(&mut f);
    let actor = Actor::Credential(owner.credential_id);
    let id = apply(&mut f, actor, &create(book, "entry", "base")).entry_id;
    let (request, target) = append(id, owner.credential_id);
    assert_eq!(
        resolve(&mut f, actor, &target).unwrap(),
        EntryOperationOutcome::Cancelled
    );
    assert_eq!(
        resolve(&mut f, actor, &target).unwrap(),
        EntryOperationOutcome::Cancelled
    );
    fail(&mut f, actor, &request, ContentError::OperationCancelled);
    let mut changed = request.clone();
    changed.agent = None;
    fail(&mut f, actor, &changed, ContentError::Conflict);
    let replacement = Uuid::new_v4();
    f.connection
        .execute(
            "INSERT INTO ccp_credentials(id,client_id,expires_at) VALUES(?1,?2,?3)",
            params![
                replacement.to_string(),
                owner.client_id.to_string(),
                EXPIRES
            ],
        )
        .unwrap();
    f.connection
        .execute(
            "INSERT INTO ccp_credential_grants(credential_id,grant_id,client_id) VALUES(?1,?2,?3)",
            params![
                replacement.to_string(),
                owner.grant_id.to_string(),
                owner.client_id.to_string()
            ],
        )
        .unwrap();
    fail(
        &mut f,
        Actor::Credential(replacement),
        &request,
        ContentError::Conflict,
    );
    f.connection
        .execute(
            "UPDATE ccp_credentials SET expires_at=?1 WHERE id=?2",
            params![NOW, owner.credential_id.to_string()],
        )
        .unwrap();
    assert_eq!(
        resolve(&mut f, Actor::Credential(replacement), &target).unwrap(),
        EntryOperationOutcome::Cancelled
    );
    assert_eq!(get(&mut f, id).content.text, "base");
    assert_eq!(get(&mut f, id).entry.revision, 1);
    assert_eq!(rows(&f.connection, "ccp_content_cancellations"), 1);
    assert_eq!(rows(&f.connection, "ccp_content_operations"), 1);
    let audits: u64 = f
        .connection
        .query_row(
            "SELECT COUNT(*) FROM ccp_access_audit WHERE action='cancel_entry_operation'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(audits, 1);
}

#[test]
fn committed_resolution_returns_only_original_receipt_even_after_entry_archive_and_role_loss() {
    let mut f = fixture();
    let owner = enroll(&mut f, DataRole::ReadWrite);
    let book = book(&mut f);
    let actor = Actor::Credential(owner.credential_id);
    let id = apply(&mut f, actor, &create(book, "entry", "base")).entry_id;
    let (request, mut target) = append(id, owner.credential_id);
    let receipt = apply(&mut f, actor, &request);
    apply(
        &mut f,
        actor,
        &command(EntryChange::Archive {
            id,
            expected_revision: 2,
        }),
    );
    f.connection
        .execute(
            "UPDATE ccp_grants SET role='read' WHERE id=?1",
            [owner.grant_id.to_string()],
        )
        .unwrap();
    assert_eq!(
        resolve(&mut f, actor, &target).unwrap(),
        EntryOperationOutcome::Committed(receipt.clone())
    );
    target.request_hash[0] ^= 1;
    error(resolve(&mut f, actor, &target), ContentError::Conflict);
    target.request_hash[0] ^= 1;
    assert_eq!(
        resolve(&mut f, actor, &target).unwrap(),
        EntryOperationOutcome::Committed(receipt)
    );
    assert_eq!(rows(&f.connection, "ccp_content_cancellations"), 0);
    error(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .get_entry(actor, f.workspace, id, NOW),
        ContentError::NotFound,
    );
}

#[test]
fn read_only_clients_can_only_fence_their_own_namespace_without_blocking_foreign_writers() {
    let mut f = fixture();
    let owner = enroll(&mut f, DataRole::ReadWrite);
    let reader = enroll(&mut f, DataRole::Read);
    let book = book(&mut f);
    let writer = Actor::Credential(owner.credential_id);
    let actor = Actor::Credential(reader.credential_id);
    let id = apply(&mut f, writer, &create(book, "entry", "base")).entry_id;
    let (request, target) = append(id, owner.credential_id);
    error(resolve(&mut f, actor, &target), ContentError::Forbidden);
    let own = EntryOperationTarget {
        credential_id: reader.credential_id,
        ..target
    };
    assert_eq!(
        resolve(&mut f, actor, &own).unwrap(),
        EntryOperationOutcome::Cancelled
    );
    let receipt = apply(&mut f, writer, &request);
    assert_eq!(
        resolve(&mut f, writer, &target).unwrap(),
        EntryOperationOutcome::Committed(receipt)
    );
    assert_eq!(
        resolve(&mut f, actor, &own).unwrap(),
        EntryOperationOutcome::Cancelled
    );
    assert_eq!(get(&mut f, id).content.text, "base+once");
    assert_eq!(rows(&f.connection, "ccp_content_cancellations"), 1);
    for credential in [Uuid::new_v4(), Uuid::nil()] {
        let foreign = EntryOperationTarget {
            credential_id: credential,
            ..own
        };
        error(resolve(&mut f, actor, &foreign), ContentError::Forbidden);
    }
    let global = Actor::Admin(f.global);
    error(resolve(&mut f, global, &target), ContentError::Forbidden);
    let mut changed = own;
    changed.request_hash[0] ^= 1;
    error(resolve(&mut f, actor, &changed), ContentError::Conflict);
}

#[test]
fn current_read_scope_disable_and_workspace_lifecycle_still_apply_to_resolution() {
    let mut f = fixture();
    let owner = enroll(&mut f, DataRole::ReadWrite);
    let reader = enroll(&mut f, DataRole::Read);
    let book = book(&mut f);
    let actor = Actor::Credential(owner.credential_id);
    let id = apply(&mut f, actor, &create(book, "entry", "base")).entry_id;
    let (_, target) = append(id, owner.credential_id);
    AccessStore::new(&mut f.connection)
        .unwrap()
        .set_archived(Actor::Admin(f.owner), f.workspace, true, NOW)
        .unwrap();
    assert_eq!(
        resolve(&mut f, actor, &target).unwrap(),
        EntryOperationOutcome::Cancelled
    );
    error(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .resolve_entry_operation(actor, f.other_workspace, &target, NOW),
        ContentError::Forbidden,
    );
    AccessStore::new(&mut f.connection)
        .unwrap()
        .revoke_grant(Actor::Admin(f.admin), owner.grant_id, NOW)
        .unwrap();
    error(resolve(&mut f, actor, &target), ContentError::Forbidden);
    let mut invalid = target;
    invalid.operation_id = Uuid::nil();
    error(resolve(&mut f, actor, &invalid), ContentError::InvalidInput);
    assert_eq!(rows(&f.connection, "ccp_content_cancellations"), 1);
    let own = EntryOperationTarget {
        credential_id: reader.credential_id,
        operation_id: Uuid::new_v4(),
        ..invalid
    };
    let reader_actor = Actor::Credential(reader.credential_id);
    assert_eq!(
        resolve(&mut f, reader_actor, &own).unwrap(),
        EntryOperationOutcome::Cancelled
    );
    AccessStore::new(&mut f.connection)
        .unwrap()
        .disable_client(Actor::Admin(f.global), reader.client_id, NOW)
        .unwrap();
    error(resolve(&mut f, reader_actor, &own), ContentError::Forbidden);
    assert_eq!(rows(&f.connection, "ccp_content_cancellations"), 2);
}

#[test]
fn cancellation_audit_failure_rolls_back_and_sql_backstops_preserve_mutual_exclusion() {
    let mut f = fixture();
    let owner = enroll(&mut f, DataRole::ReadWrite);
    let book = book(&mut f);
    let actor = Actor::Credential(owner.credential_id);
    let id = apply(&mut f, actor, &create(book, "entry", "base")).entry_id;
    let (request, target) = append(id, owner.credential_id);
    f.connection.execute_batch("CREATE TRIGGER fail_cancellation BEFORE INSERT ON ccp_access_audit WHEN NEW.action='cancel_entry_operation' BEGIN SELECT RAISE(ABORT,'fixture cancellation fault'); END;").unwrap();
    assert!(
        resolve(&mut f, actor, &target)
            .unwrap_err()
            .to_string()
            .contains("fixture cancellation fault")
    );
    assert_eq!(rows(&f.connection, "ccp_content_cancellations"), 0);
    f.connection
        .execute_batch("DROP TRIGGER fail_cancellation")
        .unwrap();
    apply(&mut f, actor, &request);
    assert!(f.connection.execute("INSERT INTO ccp_content_cancellations(workspace_id,operation_id,client_id,credential_id,request_hash,cancelled_at) VALUES(?1,?2,?3,?4,?5,?6)",params![f.workspace.to_string(),target.operation_id.to_string(),owner.client_id.to_string(),owner.credential_id.to_string(),target.request_hash.as_slice(),NOW]).unwrap_err().to_string().contains("already committed"));
    let (uncommitted, target) = append(id, owner.credential_id);
    assert_eq!(
        resolve(&mut f, actor, &target).unwrap(),
        EntryOperationOutcome::Cancelled
    );
    assert!(
        f.connection
            .execute("UPDATE ccp_content_cancellations SET cancelled_at=999", [])
            .unwrap_err()
            .to_string()
            .contains("immutable")
    );
    let foreign = Uuid::new_v4().to_string();
    assert!(f.connection.execute("INSERT INTO ccp_content_cancellations(workspace_id,operation_id,client_id,credential_id,request_hash,cancelled_at) VALUES(?1,?2,?3,?4,?5,?6)",params![f.workspace.to_string(),foreign,Uuid::new_v4().to_string(),owner.credential_id.to_string(),target.request_hash.as_slice(),NOW]).is_err());
    let copied=f.connection.execute("INSERT INTO ccp_content_operations(workspace_id,operation_id,actor_kind,actor_id,request_hash,entry_id,revision) SELECT workspace_id,?1,actor_kind,actor_id,request_hash,entry_id,revision FROM ccp_content_operations WHERE operation_id=?2",params![uncommitted.operation_id.to_string(),request.operation_id.to_string()]);
    assert!(copied.unwrap_err().to_string().contains("cancelled"));
    assert_eq!(get(&mut f, id).content.text, "base+once");
}

#[test]
fn independent_connections_linearize_resolution_against_the_original_mutation() {
    for _ in 0..12 {
        let path =
            std::env::temp_dir().join(format!("ccp-resolution-race-{}.sqlite3", Uuid::new_v4()));
        crate::init::init_sqlite(&path).unwrap();
        let mut f = fixture_with_connection(Connection::open(&path).unwrap());
        let owner = enroll(&mut f, DataRole::ReadWrite);
        let book = book(&mut f);
        let actor = Actor::Credential(owner.credential_id);
        let id = apply(&mut f, actor, &create(book, "entry", "base")).entry_id;
        let (request, target) = append(id, owner.credential_id);
        let workspace = f.workspace;
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let writer = std::thread::spawn({
            let path = path.clone();
            let barrier = barrier.clone();
            move || {
                let mut db = Connection::open(path).unwrap();
                db.busy_timeout(std::time::Duration::from_secs(5)).unwrap();
                barrier.wait();
                AccessStore::new(&mut db)
                    .unwrap()
                    .mutate_entry(actor, workspace, &request, NOW)
            }
        });
        let resolver = std::thread::spawn({
            let path = path.clone();
            move || {
                let mut db = Connection::open(path).unwrap();
                db.busy_timeout(std::time::Duration::from_secs(5)).unwrap();
                barrier.wait();
                AccessStore::new(&mut db)
                    .unwrap()
                    .resolve_entry_operation(actor, workspace, &target, NOW)
            }
        });
        let resolution = resolver.join().unwrap().unwrap();
        let mutation = writer.join().unwrap();
        match resolution {
            EntryOperationOutcome::Committed(receipt) => {
                assert_eq!(mutation.unwrap(), receipt);
                assert_eq!(get(&mut f, id).content.text, "base+once");
                assert_eq!(rows(&f.connection, "ccp_content_cancellations"), 0);
            }
            EntryOperationOutcome::Cancelled => {
                error(mutation, ContentError::OperationCancelled);
                assert_eq!(get(&mut f, id).content.text, "base");
                assert_eq!(rows(&f.connection, "ccp_content_cancellations"), 1);
            }
        }
        drop(f.connection);
        fs::remove_file(path).unwrap();
    }
}

#[test]
fn schema16_upgrade_preserves_commits_and_cancelled_intents_survive_reopen() {
    let path =
        std::env::temp_dir().join(format!("ccp-resolution-upgrade-{}.sqlite3", Uuid::new_v4()));
    crate::init::init_sqlite(&path).unwrap();
    let mut f = fixture_with_connection(Connection::open(&path).unwrap());
    let owner = enroll(&mut f, DataRole::ReadWrite);
    let book = book(&mut f);
    let actor = Actor::Credential(owner.credential_id);
    let create = create(book, "entry", "persisted body");
    let receipt = apply(&mut f, actor, &create);
    let id = receipt.entry_id;
    let hash: Vec<u8> = f
        .connection
        .query_row(
            "SELECT request_hash FROM ccp_content_operations WHERE operation_id=?1",
            [create.operation_id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    let committed = EntryOperationTarget {
        operation_id: create.operation_id,
        credential_id: owner.credential_id,
        request_hash: hash.try_into().unwrap(),
    };
    let (request, cancel) = append(id, owner.credential_id);
    let workspace = f.workspace;
    f.connection.execute_batch("DROP TRIGGER IF EXISTS ccp_no_incomplete_workspace_deletion; DROP TRIGGER ccp_operation_excludes_own_cancellation; DROP TABLE ccp_content_cancellations; DELETE FROM schema_version WHERE version>16; INSERT OR IGNORE INTO schema_version(version) VALUES(16);").unwrap();
    drop(f.connection);
    crate::init::init_sqlite(&path).unwrap();
    let mut db = Connection::open(&path).unwrap();
    let mut store = AccessStore::new(&mut db).unwrap();
    assert_eq!(
        store
            .resolve_entry_operation(actor, workspace, &committed, NOW)
            .unwrap(),
        EntryOperationOutcome::Committed(receipt.clone())
    );
    assert_eq!(
        store
            .resolve_entry_operation(actor, workspace, &cancel, NOW)
            .unwrap(),
        EntryOperationOutcome::Cancelled
    );
    drop(db);
    crate::init::init_sqlite(&path).unwrap();
    let mut db = Connection::open(&path).unwrap();
    let mut store = AccessStore::new(&mut db).unwrap();
    assert_eq!(
        store
            .resolve_entry_operation(actor, workspace, &cancel, NOW)
            .unwrap(),
        EntryOperationOutcome::Cancelled
    );
    error(
        store.mutate_entry(actor, workspace, &request, NOW),
        ContentError::OperationCancelled,
    );
    assert_eq!(
        store
            .resolve_entry_operation(actor, workspace, &committed, NOW)
            .unwrap(),
        EntryOperationOutcome::Committed(receipt)
    );
    assert_eq!(
        store
            .get_entry(actor, workspace, id, NOW)
            .unwrap()
            .content
            .text,
        "persisted body"
    );
    let violations: u64 = db
        .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(violations, 0);
    drop(db);
    fs::remove_file(path).unwrap();
}
