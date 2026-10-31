// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

mod boundaries;
mod faults;
mod moderation;
mod persistence;

fn post(id: Uuid, revision: u64, text: &str) -> WorkStatusMutation {
    WorkStatusMutation {
        operation_id: Uuid::new_v4(),
        status_id: id,
        expected_revision: revision,
        change: WorkStatusChange::Set {
            agent: Some("agent label".into()),
            message: text.into(),
        },
    }
}
fn clear(id: Uuid, revision: u64) -> WorkStatusMutation {
    WorkStatusMutation {
        operation_id: Uuid::new_v4(),
        status_id: id,
        expected_revision: revision,
        change: WorkStatusChange::Clear,
    }
}
fn apply(
    f: &mut Fixture,
    credential: Uuid,
    request: &WorkStatusMutation,
    now: i64,
) -> anyhow::Result<WorkStatusReceipt> {
    AccessStore::new(&mut f.connection)?.mutate_work_status(
        Actor::Credential(credential),
        f.workspace,
        request,
        now,
    )
}
fn read(f: &mut Fixture, id: Uuid, now: i64) -> WorkStatus {
    AccessStore::new(&mut f.connection)
        .unwrap()
        .work_status(Actor::Admin(f.owner), f.workspace, id, now)
        .unwrap()
}
fn page(f: &mut Fixture, after: Option<Uuid>, limit: usize, now: i64) -> WorkStatusPage {
    AccessStore::new(&mut f.connection)
        .unwrap()
        .work_statuses(Actor::Admin(f.owner), f.workspace, after, limit, now)
        .unwrap()
}
fn error<T>(value: anyhow::Result<T>, expected: ContentError) {
    match value {
        Ok(_) => panic!("expected {expected:?}"),
        Err(e) => assert_eq!(e.downcast_ref::<ContentError>(), Some(&expected), "{e:#}"),
    }
}

#[test]
fn explicit_status_freshness_never_hides_or_refreshes_last_message() {
    let mut f = fixture();
    let client = enroll(&mut f, DataRole::ReadWrite);
    let id = Uuid::new_v4();
    let request = post(id, 0, "Investigating a retry");
    let receipt = apply(&mut f, client.credential_id, &request, NOW).unwrap();
    assert_eq!(receipt.client_id, client.client_id);
    assert_eq!(receipt.revision, 1);
    let audit = rows(&f.connection, "ccp_access_audit");
    for (time, stale) in [
        (NOW, false),
        (NOW + 299, false),
        (NOW + 300, true),
        (NOW + 100_000, true),
        (NOW - 1, true),
    ] {
        let value = read(&mut f, id, time);
        assert_eq!(value.stale, stale);
        assert_eq!(value.message.as_deref(), Some("Investigating a retry"));
        assert_eq!(value.updated_at, NOW);
        let result = page(&mut f, None, 32, time);
        assert_eq!(result.items, vec![value]);
        assert_eq!(result.observed_at, time);
        assert_eq!(result.stale_after_seconds, 300);
    }
    assert_eq!(rows(&f.connection, "ccp_access_audit"), audit);
    assert_eq!(rows(&f.connection, "ccp_work_status_operations"), 1);
}

#[test]
fn same_labels_are_attribution_while_distinct_slots_preserve_authenticated_client_ownership() {
    let mut f = fixture();
    let a = enroll(&mut f, DataRole::ReadWrite);
    let b = enroll(&mut f, DataRole::ReadWrite);
    let id = Uuid::new_v4();
    apply(&mut f, a.credential_id, &post(id, 0, "first"), NOW).unwrap();
    let other = Uuid::new_v4();
    apply(&mut f, b.credential_id, &post(other, 0, "second"), NOW).unwrap();
    let values = page(&mut f, None, 32, NOW).items;
    assert_eq!(values.len(), 2);
    assert_eq!(values[0].agent, values[1].agent);
    assert_ne!(values[0].client_id, values[1].client_id);
    error(
        apply(
            &mut f,
            b.credential_id,
            &post(id, 1, "impersonation"),
            NOW + 1,
        ),
        ContentError::Forbidden,
    );
    error(
        apply(&mut f, b.credential_id, &clear(id, 1), NOW + 1),
        ContentError::Forbidden,
    );
    assert_eq!(read(&mut f, id, NOW).client_id, a.client_id);
    assert_eq!(read(&mut f, id, NOW).message.as_deref(), Some("first"));
}

#[test]
fn exact_receipts_survive_later_update_and_clear_without_refresh_or_resurrection() {
    let mut f = fixture();
    let client = enroll(&mut f, DataRole::ReadWrite);
    let id = Uuid::new_v4();
    let original = post(id, 0, "old private message");
    let receipt = apply(&mut f, client.credential_id, &original, NOW).unwrap();
    let update = post(id, 1, "new private message");
    apply(&mut f, client.credential_id, &update, NOW + 1).unwrap();
    let cleared = clear(id, 2);
    let proof = apply(&mut f, client.credential_id, &cleared, NOW + 2).unwrap();
    assert_eq!(proof.revision, 3);
    assert!(proof.cleared);
    assert_eq!(
        apply(&mut f, client.credential_id, &original, NOW + 3).unwrap(),
        receipt
    );
    assert_eq!(
        apply(&mut f, client.credential_id, &cleared, NOW + 4).unwrap(),
        proof
    );
    let value = read(&mut f, id, NOW + 4);
    assert_eq!(value.revision, 3);
    assert_eq!(value.message, None);
    assert_eq!(value.agent, None);
    assert_eq!(value.updated_at, NOW + 2);
    assert!(page(&mut f, None, 32, NOW + 4).items.is_empty());
    let before = rows(&f.connection, "ccp_work_status_operations");
    error(
        apply(
            &mut f,
            client.credential_id,
            &post(id, 1, "stale edit"),
            NOW + 5,
        ),
        ContentError::Conflict,
    );
    error(
        apply(&mut f, client.credential_id, &clear(id, 3), NOW + 5),
        ContentError::Conflict,
    );
    assert_eq!(rows(&f.connection, "ccp_work_status_operations"), before);
    let republished = apply(
        &mut f,
        client.credential_id,
        &post(id, 3, "deliberate new message"),
        NOW + 6,
    )
    .unwrap();
    assert_eq!(republished.revision, 4);
    assert_eq!(read(&mut f, id, NOW + 6).client_id, client.client_id);
    let fields = f
        .connection
        .prepare("PRAGMA table_info(ccp_work_status_operations)")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(1))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert!(
        !fields
            .iter()
            .any(|name| matches!(name.as_str(), "message" | "agent" | "text"))
    );
    assert_eq!(rows(&f.connection, "ccp_work_status"), 1);
}

#[test]
fn operation_id_binds_actor_target_revision_and_exact_message_without_silent_replacement() {
    let mut f = fixture();
    let a = enroll(&mut f, DataRole::ReadWrite);
    let b = enroll(&mut f, DataRole::ReadWrite);
    let id = Uuid::new_v4();
    let request = post(id, 0, "message");
    apply(&mut f, a.credential_id, &request, NOW).unwrap();
    error(
        apply(&mut f, b.credential_id, &request, NOW + 1),
        ContentError::Conflict,
    );
    for change in [
        post(id, 0, "changed"),
        post(id, 1, "message"),
        post(Uuid::new_v4(), 0, "message"),
        clear(id, 1),
    ] {
        let altered = WorkStatusMutation {
            operation_id: request.operation_id,
            ..change
        };
        error(
            apply(&mut f, a.credential_id, &altered, NOW + 1),
            ContentError::Conflict,
        );
    }
    error(
        apply(&mut f, a.credential_id, &post(id, 1, "backwards"), NOW - 1),
        ContentError::Conflict,
    );
    assert_eq!(rows(&f.connection, "ccp_work_status_operations"), 1);
    assert_eq!(read(&mut f, id, NOW).message.as_deref(), Some("message"));
}

#[test]
fn ownership_survives_replacement_credential_but_old_authority_cannot_replay() {
    let mut f = fixture();
    let a = enroll(&mut f, DataRole::ReadWrite);
    let id = Uuid::new_v4();
    let request = post(id, 0, "original");
    apply(&mut f, a.credential_id, &request, NOW).unwrap();
    let new = Uuid::new_v4();
    f.connection
        .execute(
            "INSERT INTO ccp_credentials(id,client_id,expires_at) VALUES(?1,?2,?3)",
            params![new.to_string(), a.client_id.to_string(), EXPIRES],
        )
        .unwrap();
    f.connection.execute("INSERT INTO ccp_credential_grants(credential_id,grant_id,client_id) SELECT ?1,grant_id,client_id FROM ccp_credential_grants WHERE credential_id=?2",params![new.to_string(),a.credential_id.to_string()]).unwrap();
    f.connection
        .execute(
            "UPDATE ccp_credentials SET revoked_at=?2 WHERE id=?1",
            params![a.credential_id.to_string(), NOW + 1],
        )
        .unwrap();
    error(
        apply(&mut f, a.credential_id, &request, NOW + 2),
        ContentError::Forbidden,
    );
    error(
        apply(&mut f, new, &request, NOW + 2),
        ContentError::Conflict,
    );
    apply(&mut f, new, &post(id, 1, "same registered client"), NOW + 2).unwrap();
    assert_eq!(read(&mut f, id, NOW + 2).client_id, a.client_id);
}
