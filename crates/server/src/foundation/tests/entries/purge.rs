// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

mod persistence;

fn intent(entry_id: Uuid, expected_revision: u64) -> EntryPurge {
    EntryPurge {
        operation_id: Uuid::new_v4(),
        entry_id,
        expected_revision,
        confirm_purge: true,
    }
}
fn archived(f: &mut Fixture, actor: Actor) -> (EntryMutation, EntryPurge) {
    let book = book(f);
    let create = create(book, "entry", "PRIVATE_BODY");
    let entry = apply(f, actor, &create);
    apply(
        f,
        actor,
        &command(EntryChange::Archive {
            id: entry.entry_id,
            expected_revision: 1,
        }),
    );
    (create, intent(entry.entry_id, 2))
}
fn purge(f: &mut Fixture, actor: Actor, request: &EntryPurge) -> EntryPurgeReceipt {
    AccessStore::new(&mut f.connection)
        .unwrap()
        .purge_entry(actor, f.workspace, request, NOW)
        .unwrap()
}
fn denied(f: &mut Fixture, actor: Actor, request: &EntryPurge, expected: ContentError) {
    error(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .purge_entry(actor, f.workspace, request, NOW),
        expected,
    )
}

#[test]
fn purge_removes_payload_and_history_preserving_write_proofs_resolution_and_siblings() {
    let mut f = fixture();
    let guidance = keep_guidance(&mut f);
    let writer = enroll(&mut f, DataRole::ReadWrite);
    let actor = Actor::Credential(writer.credential_id);
    let book = book(&mut f);
    let create = create(book, "entry", "PRIVATE_BODY");
    let original = apply(&mut f, actor, &create);
    let append = command(EntryChange::Append {
        id: original.entry_id,
        expected_revision: 1,
        text: "PRIVATE_APPEND".into(),
    });
    apply(&mut f, actor, &append);
    apply(
        &mut f,
        actor,
        &command(EntryChange::Archive {
            id: original.entry_id,
            expected_revision: 2,
        }),
    );
    let sibling = apply(&mut f, actor, &super::create(book, "sibling", "kept"));
    let target = binding(&f, create.operation_id, writer.credential_id);
    let cancellation = EntryOperationTarget {
        operation_id: Uuid::new_v4(),
        credential_id: writer.credential_id,
        request_hash: [2; 32],
    };
    AccessStore::new(&mut f.connection)
        .unwrap()
        .resolve_entry_operation(actor, f.workspace, &cancellation, NOW)
        .unwrap();
    let owner = Actor::Admin(f.owner);
    let request = intent(original.entry_id, 3);
    let proof = purge(&mut f, owner, &request);
    assert_eq!(proof.revision, 3);
    assert_eq!(proof.purged_at, NOW);
    assert_eq!(rows(&f.connection, "ccp_content_entries"), 1);
    assert_eq!(rows(&f.connection, "ccp_content_versions"), 1);
    assert_eq!(rows(&f.connection, "ccp_content_operations"), 4);
    assert_eq!(rows(&f.connection, "ccp_content_cancellations"), 1);
    assert_eq!(get(&mut f, sibling.entry_id).content.text, "kept");
    assert_eq!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .guidance(owner, Some(f.workspace), NOW)
            .unwrap(),
        guidance
    );
    assert_eq!(rows(&f.connection, "ccp_content_books"), 1);
    assert_eq!(rows(&f.connection, "ccp_content_shelves"), 1);
    assert_eq!(apply(&mut f, actor, &create), original);
    assert_eq!(apply(&mut f, actor, &append).revision, 2);
    resolved_after_purge(&mut f, actor, &target, &cancellation, original);
    error(
        AccessStore::new(&mut f.connection).unwrap().get_entry(
            owner,
            f.workspace,
            request.entry_id,
            NOW,
        ),
        ContentError::NotFound,
    );
    fail(
        &mut f,
        actor,
        &command(EntryChange::Restore {
            id: request.entry_id,
            expected_revision: 3,
        }),
        ContentError::NotFound,
    );
    assert_eq!(purge(&mut f, owner, &request), proof);
    assert_eq!(rows(&f.connection, "ccp_content_entries"), 1);
    let replacement = apply(&mut f, actor, &super::create(book, "entry", "fresh"));
    assert_ne!(replacement.entry_id, request.entry_id);
    assert!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .entries(owner, f.workspace, &page(book, true), NOW)
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(
        f.connection
            .query_row(
                "SELECT COUNT(*) FROM ccp_access_audit WHERE action='purge_entry'",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
        1
    );
}

#[test]
fn d064_scoped_admin_purge_is_allowed_in_archived_workspace_but_not_for_clients_or_global_role() {
    let mut f = fixture();
    let writer = enroll(&mut f, DataRole::ReadWrite);
    let reader = enroll(&mut f, DataRole::Read);
    let owner = Actor::Admin(f.owner);
    let (_, request) = archived(&mut f, owner);
    AccessStore::new(&mut f.connection)
        .unwrap()
        .set_archived(owner, f.workspace, true, NOW)
        .unwrap();
    for actor in [
        Actor::Credential(writer.credential_id),
        Actor::Credential(reader.credential_id),
        Actor::Admin(f.global),
        Actor::Admin(f.other_owner),
    ] {
        denied(&mut f, actor, &request, ContentError::Forbidden);
    }
    let admin = Actor::Admin(f.admin);
    let proof = purge(&mut f, admin, &request);
    assert_eq!(purge(&mut f, admin, &request), proof);
    denied(
        &mut f,
        owner,
        &intent(request.entry_id, 2),
        ContentError::NotFound,
    );
}

#[test]
fn only_individually_archived_matching_revisions_with_confirmation_can_be_purged() {
    let mut f = fixture();
    let owner = Actor::Admin(f.owner);
    let book = book(&mut f);
    let entry = apply(&mut f, owner, &create(book, "active", "kept"));
    denied(
        &mut f,
        owner,
        &intent(entry.entry_id, 1),
        ContentError::Conflict,
    );
    let shelf: String = f
        .connection
        .query_row(
            "SELECT shelf_id FROM ccp_content_books WHERE id=?1",
            [book.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    f.connection
        .execute(
            "UPDATE ccp_content_shelves SET archived=1 WHERE id=?1",
            [shelf],
        )
        .unwrap();
    denied(
        &mut f,
        owner,
        &intent(entry.entry_id, 1),
        ContentError::Conflict,
    );
    f.connection
        .execute("UPDATE ccp_content_shelves SET archived=0", [])
        .unwrap();
    apply(
        &mut f,
        owner,
        &command(EntryChange::Archive {
            id: entry.entry_id,
            expected_revision: 1,
        }),
    );
    let mut request = intent(entry.entry_id, 2);
    request.confirm_purge = false;
    denied(&mut f, owner, &request, ContentError::InvalidInput);
    request.confirm_purge = true;
    for revision in [0, u64::MAX] {
        let bad = intent(entry.entry_id, revision);
        denied(&mut f, owner, &bad, ContentError::InvalidInput);
    }
    denied(
        &mut f,
        owner,
        &intent(entry.entry_id, 1),
        ContentError::Conflict,
    );
    let mut bad = intent(entry.entry_id, 2);
    bad.operation_id = Uuid::nil();
    denied(&mut f, owner, &bad, ContentError::InvalidInput);
    denied(
        &mut f,
        owner,
        &intent(Uuid::nil(), 2),
        ContentError::InvalidInput,
    );
    // A separately archived entry remains eligible even within an archived shelf.
    f.connection
        .execute("UPDATE ccp_content_shelves SET archived=1", [])
        .unwrap();
    purge(&mut f, owner, &request);
    denied(
        &mut f,
        owner,
        &intent(request.entry_id, 2),
        ContentError::NotFound,
    );
    assert_eq!(rows(&f.connection, "ccp_entry_purges"), 1);
}

#[test]
fn purge_replay_binds_original_key_request_and_current_scoped_authority() {
    let mut f = fixture();
    let owner = Actor::Admin(f.owner);
    let (_, request) = archived(&mut f, owner);
    let mut actors = Vec::new();
    let mut keys = Vec::new();
    for _ in 0..2 {
        let mut store = AccessStore::new(&mut f.connection).unwrap();
        let key = store
            .issue_admin_key(
                owner,
                &AdminKeyRequest {
                    principal_id: f.owner,
                    scope: AdminKeyScope::Workspace(f.workspace),
                    name: "publisher".into(),
                    expires_at: None,
                },
                NOW,
            )
            .unwrap();
        actors.push(store.authenticate_admin_key(key.secret(), NOW).unwrap());
        keys.push(key);
    }
    let first = purge(&mut f, actors[0], &request);
    assert_eq!(purge(&mut f, actors[0], &request), first);
    denied(&mut f, actors[1], &request, ContentError::Conflict);
    denied(&mut f, owner, &request, ContentError::Conflict);
    let mut changed = intent(Uuid::new_v4(), 2);
    changed.operation_id = request.operation_id;
    denied(&mut f, actors[0], &changed, ContentError::Conflict);
    changed.entry_id = request.entry_id;
    changed.expected_revision = 1;
    denied(&mut f, actors[0], &changed, ContentError::Conflict);
    f.connection
        .execute(
            "UPDATE ccp_admin_keys SET revoked_at=?1 WHERE id=?2",
            params![NOW, keys[0].id.to_string()],
        )
        .unwrap();
    denied(&mut f, actors[0], &request, ContentError::Forbidden);
    let result = AccessStore::new(&mut f.connection).unwrap().purge_entry(
        actors[1],
        f.workspace,
        &intent(request.entry_id, 2),
        -1,
    );
    error(result, ContentError::Forbidden);
}

fn resolved_after_purge(
    f: &mut Fixture,
    actor: Actor,
    target: &EntryOperationTarget,
    cancellation: &EntryOperationTarget,
    original: EntryReceipt,
) {
    let outcome = AccessStore::new(&mut f.connection)
        .unwrap()
        .resolve_entry_operation(actor, f.workspace, target, NOW)
        .unwrap();
    assert_eq!(outcome, EntryOperationOutcome::Committed(original));
    assert_eq!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .resolve_entry_operation(actor, f.workspace, cancellation, NOW)
            .unwrap(),
        EntryOperationOutcome::Cancelled
    );
}

fn keep_guidance(f: &mut Fixture) -> GuidanceBundle {
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    for (actor, scope) in [
        (Actor::Admin(f.global), GuidanceScope::Global),
        (Actor::Admin(f.owner), GuidanceScope::Workspace(f.workspace)),
    ] {
        store
            .save_guidance(
                actor,
                scope,
                &GuidanceSave {
                    operation_id: Uuid::new_v4(),
                    expected_revision: 0,
                    text: "preserved guidance".into(),
                },
                NOW,
            )
            .unwrap();
    }
    store
        .guidance(Actor::Admin(f.owner), Some(f.workspace), NOW)
        .unwrap()
}

#[test]
fn instance_key_cannot_borrow_its_principal_workspace_purge_permission() {
    let mut f = fixture();
    let owner = Actor::Admin(f.owner);
    let (_, request) = archived(&mut f, owner);
    f.connection
        .execute(
            "UPDATE ccp_workspaces SET owner_admin_id=?1 WHERE id=?2",
            params![f.global.to_string(), f.workspace.to_string()],
        )
        .unwrap();
    let mut actors = Vec::new();
    for scope in [
        AdminKeyScope::Instance,
        AdminKeyScope::Workspace(f.workspace),
    ] {
        let mut store = AccessStore::new(&mut f.connection).unwrap();
        let key = store
            .issue_admin_key(
                Actor::Admin(f.global),
                &AdminKeyRequest {
                    principal_id: f.global,
                    scope,
                    name: "scoped purge".into(),
                    expires_at: None,
                },
                NOW,
            )
            .unwrap();
        actors.push(store.authenticate_admin_key(key.secret(), NOW).unwrap());
    }
    denied(&mut f, actors[0], &request, ContentError::Forbidden);
    purge(&mut f, actors[1], &request);
}

#[test]
fn purge_can_remove_a_terminal_maximum_revision_without_incrementing_or_overflow() {
    let mut f = fixture();
    let owner = Actor::Admin(f.owner);
    let (_, mut request) = archived(&mut f, owner);
    f.connection.execute("INSERT INTO ccp_content_versions(entry_id,workspace_id,revision,action,actor_kind,actor_id,client_id,admin_principal_id,agent,recorded_at,content_json,appended_text) SELECT entry_id,workspace_id,?1,action,actor_kind,actor_id,client_id,admin_principal_id,agent,recorded_at,content_json,appended_text FROM ccp_content_versions WHERE entry_id=?2 AND revision=2",params![i64::MAX,request.entry_id.to_string()]).unwrap();
    f.connection
        .execute(
            "UPDATE ccp_content_entries SET revision=?1 WHERE id=?2",
            params![i64::MAX, request.entry_id.to_string()],
        )
        .unwrap();
    request.expected_revision = i64::MAX.unsigned_abs();
    assert_eq!(
        purge(&mut f, owner, &request).revision,
        i64::MAX.unsigned_abs()
    );
    assert_eq!(rows(&f.connection, "ccp_content_versions"), 0);
}

#[test]
fn purge_does_not_bypass_legacy_conversion_or_current_admin_demotion() {
    let mut f = fixture();
    let owner = Actor::Admin(f.owner);
    let (_, request) = archived(&mut f, owner);
    f.connection
        .execute(
            "DELETE FROM ccp_workspace_admins WHERE admin_id=?1",
            [f.admin.to_string()],
        )
        .unwrap();
    let admin = Actor::Admin(f.admin);
    denied(&mut f, admin, &request, ContentError::Forbidden);
    f.connection
        .execute("INSERT INTO sessions(id,name) VALUES(99,'legacy')", [])
        .unwrap();
    f.connection
        .execute(
            "UPDATE ccp_workspaces SET legacy_session_id=99 WHERE id=?1",
            [f.workspace.to_string()],
        )
        .unwrap();
    denied(
        &mut f,
        owner,
        &request,
        ContentError::LegacyConversionRequired,
    );
    assert_eq!(rows(&f.connection, "ccp_content_versions"), 2);
}

fn binding(f: &Fixture, operation_id: Uuid, credential_id: Uuid) -> EntryOperationTarget {
    let hash: Vec<u8> = f
        .connection
        .query_row(
            "SELECT request_hash FROM ccp_content_operations WHERE operation_id=?1",
            [operation_id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    EntryOperationTarget {
        operation_id,
        credential_id,
        request_hash: hash.try_into().unwrap(),
    }
}

#[test]
fn a_confirmed_purge_cannot_act_after_restore_and_rearchive_changes_the_revision() {
    let mut f = fixture();
    let owner = Actor::Admin(f.owner);
    let (_, stale) = archived(&mut f, owner);
    apply(
        &mut f,
        owner,
        &command(EntryChange::Restore {
            id: stale.entry_id,
            expected_revision: 2,
        }),
    );
    apply(
        &mut f,
        owner,
        &command(EntryChange::Archive {
            id: stale.entry_id,
            expected_revision: 3,
        }),
    );
    denied(&mut f, owner, &stale, ContentError::Conflict);
    assert_eq!(rows(&f.connection, "ccp_content_versions"), 4);
    assert_eq!(rows(&f.connection, "ccp_entry_purges"), 0);
    purge(&mut f, owner, &intent(stale.entry_id, 4));
}
