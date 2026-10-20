// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

mod migration;

fn target(f: &Fixture, client: &Enrollment, request: &EntryMutation) -> EntryOperationTarget {
    let hash:Vec<u8>=f.connection.query_row("SELECT request_hash FROM ccp_content_operations WHERE workspace_id=?1 AND operation_id=?2",params![f.workspace.to_string(),request.operation_id.to_string()],|r|r.get(0)).unwrap();
    EntryOperationTarget {
        operation_id: request.operation_id,
        credential_id: client.credential_id,
        request_hash: hash.try_into().unwrap(),
    }
}

#[test]
fn body_free_receipts_survive_fixture_payload_removal_without_recreating_old_creates() {
    let mut f = fixture();
    let writer = enroll(&mut f, DataRole::ReadWrite);
    let book = book(&mut f);
    let actor = Actor::Credential(writer.credential_id);
    let original = create(book, "entry", "retained until fixture purge");
    let receipt = apply(&mut f, actor, &original);
    apply(
        &mut f,
        actor,
        &command(EntryChange::Archive {
            id: receipt.entry_id,
            expected_revision: 1,
        }),
    );
    let proof = target(&f, &writer, &original);
    let before = rows(&f.connection, "ccp_content_operations");
    // Test-only removal models a future separately authorized purge. No
    // production purge API or permission is introduced by this schema change.
    f.connection
        .execute(
            "DELETE FROM ccp_content_versions WHERE entry_id=?1",
            [receipt.entry_id.to_string()],
        )
        .unwrap();
    f.connection
        .execute(
            "DELETE FROM ccp_content_entries WHERE id=?1",
            [receipt.entry_id.to_string()],
        )
        .unwrap();
    assert_eq!(rows(&f.connection, "ccp_content_versions"), 0);
    assert_eq!(rows(&f.connection, "ccp_content_entries"), 0);
    assert_eq!(apply(&mut f, actor, &original), receipt);
    assert_eq!(rows(&f.connection, "ccp_content_entries"), 0);
    assert_eq!(rows(&f.connection, "ccp_content_operations"), before);
    f.connection
        .execute(
            "UPDATE ccp_grants SET role='read' WHERE id=?1",
            [writer.grant_id.to_string()],
        )
        .unwrap();
    let result = AccessStore::new(&mut f.connection)
        .unwrap()
        .resolve_entry_operation(actor, f.workspace, &proof, NOW)
        .unwrap();
    assert_eq!(result, EntryOperationOutcome::Committed(receipt));
    fail(&mut f, actor, &original, ContentError::Forbidden);
    let mut wrong = proof;
    wrong.request_hash = [0; 32];
    error(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .resolve_entry_operation(actor, f.workspace, &wrong, NOW),
        ContentError::Conflict,
    );
    AccessStore::new(&mut f.connection)
        .unwrap()
        .revoke_grant(Actor::Admin(f.admin), writer.grant_id, NOW)
        .unwrap();
    error(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .resolve_entry_operation(actor, f.workspace, &wrong, NOW),
        ContentError::Forbidden,
    );
}

#[test]
fn retained_receipts_are_immutable_and_still_exclude_same_client_cancellation() {
    let mut f = fixture();
    let client = enroll(&mut f, DataRole::ReadWrite);
    let book = book(&mut f);
    let actor = Actor::Credential(client.credential_id);
    let request = create(book, "entry", "body");
    let receipt = apply(&mut f, actor, &request);
    let original = target(&f, &client, &request);
    for sql in [
        "UPDATE ccp_content_operations SET revision=2",
        "DELETE FROM ccp_content_operations",
    ] {
        assert!(f.connection.execute(sql, []).is_err());
    }
    assert!(f.connection.execute("INSERT INTO ccp_content_cancellations(workspace_id,operation_id,client_id,credential_id,request_hash,cancelled_at) VALUES(?1,?2,?3,?4,?5,?6)",params![f.workspace.to_string(),request.operation_id.to_string(),client.client_id.to_string(),client.credential_id.to_string(),original.request_hash.as_slice(),NOW]).unwrap_err().to_string().contains("already committed"));
    for (workspace, entry) in [
        (f.workspace, Uuid::new_v4()),
        (f.other_workspace, receipt.entry_id),
    ] {
        assert!(f.connection.execute("INSERT INTO ccp_content_operations(workspace_id,operation_id,actor_kind,actor_id,request_hash,entry_id,revision) VALUES(?1,?2,'credential',?3,?4,?5,1)",params![workspace.to_string(),Uuid::new_v4().to_string(),client.credential_id.to_string(),original.request_hash.as_slice(),entry.to_string()]).unwrap_err().to_string().contains("matching committed version"));
    }
    let foreign = enroll(&mut f, DataRole::Read);
    error(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .resolve_entry_operation(
                Actor::Credential(foreign.credential_id),
                f.workspace,
                &original,
                NOW,
            ),
        ContentError::Forbidden,
    );
    assert_eq!(rows(&f.connection, "ccp_content_operations"), 1);
    assert_eq!(get(&mut f, receipt.entry_id).content.text, "body");
    assert!(
        f.connection
            .prepare("PRAGMA foreign_key_check")
            .unwrap()
            .query([])
            .unwrap()
            .next()
            .unwrap()
            .is_none()
    );
}
