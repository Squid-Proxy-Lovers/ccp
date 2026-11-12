// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

#[test]
fn current_read_write_scope_archive_disable_and_revocation_control_statuses() {
    let mut f = fixture();
    let a = enroll(&mut f, DataRole::ReadWrite);
    let reader = enroll(&mut f, DataRole::Read);
    let id = Uuid::new_v4();
    let request = post(id, 0, "retained");
    apply(&mut f, a.credential_id, &request, NOW).unwrap();
    let ws = f.workspace;
    let other = f.other_workspace;
    for actor in [
        Actor::Admin(f.global),
        Actor::Admin(f.owner),
        Actor::Admin(f.admin),
        Actor::Credential(reader.credential_id),
    ] {
        assert_eq!(
            AccessStore::new(&mut f.connection)
                .unwrap()
                .work_status(actor, ws, id, NOW)
                .unwrap()
                .message
                .as_deref(),
            Some("retained")
        );
    }
    error(
        apply(
            &mut f,
            reader.credential_id,
            &post(Uuid::new_v4(), 0, "no write"),
            NOW,
        ),
        ContentError::Forbidden,
    );
    for actor in [
        Actor::Admin(f.global),
        Actor::Admin(f.owner),
        Actor::Admin(f.admin),
    ] {
        error(
            AccessStore::new(&mut f.connection)
                .unwrap()
                .mutate_work_status(actor, ws, &request, NOW),
            ContentError::Forbidden,
        );
    }
    error(
        AccessStore::new(&mut f.connection).unwrap().work_status(
            Actor::Credential(a.credential_id),
            other,
            id,
            NOW,
        ),
        ContentError::Forbidden,
    );
    AccessStore::new(&mut f.connection)
        .unwrap()
        .set_archived(Actor::Admin(f.owner), ws, true, NOW + 1)
        .unwrap();
    assert_eq!(page(&mut f, None, 32, NOW + 1).items.len(), 1);
    error(
        apply(&mut f, a.credential_id, &request, NOW + 1),
        ContentError::Forbidden,
    );
    error(
        apply(&mut f, a.credential_id, &clear(id, 1), NOW + 1),
        ContentError::Forbidden,
    );
    AccessStore::new(&mut f.connection)
        .unwrap()
        .set_archived(Actor::Admin(f.owner), ws, false, NOW + 2)
        .unwrap();
    AccessStore::new(&mut f.connection)
        .unwrap()
        .disable_client(Actor::Admin(f.global), a.client_id, NOW + 3)
        .unwrap();
    error(
        apply(&mut f, a.credential_id, &request, NOW + 3),
        ContentError::Forbidden,
    );
    let value = read(&mut f, id, NOW + 300);
    assert!(value.client_disabled);
    assert!(value.stale);
    assert_eq!(value.message.as_deref(), Some("retained"));
    AccessStore::new(&mut f.connection)
        .unwrap()
        .revoke_grant(Actor::Admin(f.owner), reader.grant_id, NOW + 3)
        .unwrap();
    error(
        AccessStore::new(&mut f.connection).unwrap().work_status(
            Actor::Credential(reader.credential_id),
            ws,
            id,
            NOW + 3,
        ),
        ContentError::Forbidden,
    );
}

#[test]
fn status_byte_limits_ids_cas_and_missing_or_cleared_targets_are_explicit() {
    let mut f = fixture();
    let a = enroll(&mut f, DataRole::ReadWrite);
    let id = Uuid::new_v4();
    for text in ["".into(), " \n".into(), "\0".into(), "é".repeat(2049)] {
        error(
            apply(&mut f, a.credential_id, &post(id, 0, &text), NOW),
            ContentError::InvalidInput,
        );
    }
    for agent in ["".into(), "\n".into(), "\0".into(), "é".repeat(65)] {
        let mut request = post(id, 0, "okay");
        request.change = WorkStatusChange::Set {
            agent: Some(agent),
            message: "okay".into(),
        };
        error(
            apply(&mut f, a.credential_id, &request, NOW),
            ContentError::InvalidInput,
        );
    }
    let mut request = post(id, 0, "okay");
    request.operation_id = Uuid::nil();
    error(
        apply(&mut f, a.credential_id, &request, NOW),
        ContentError::InvalidInput,
    );
    error(
        apply(&mut f, a.credential_id, &post(Uuid::nil(), 0, "okay"), NOW),
        ContentError::InvalidInput,
    );
    error(
        apply(
            &mut f,
            a.credential_id,
            &post(id, i64::MAX.unsigned_abs(), "okay"),
            NOW,
        ),
        ContentError::InvalidInput,
    );
    error(
        apply(&mut f, a.credential_id, &post(id, 0, "okay"), -1),
        ContentError::InvalidInput,
    );
    error(
        apply(&mut f, a.credential_id, &clear(id, 0), NOW),
        ContentError::NotFound,
    );
    let mut request = post(id, 0, &"é".repeat(2048));
    request.change = WorkStatusChange::Set {
        agent: Some("é".repeat(64)),
        message: "é".repeat(2048),
    };
    apply(&mut f, a.credential_id, &request, NOW).unwrap();
    assert_eq!(read(&mut f, id, NOW).message.unwrap().len(), 4096);
    assert_eq!(read(&mut f, id, NOW).agent.unwrap().len(), 128);
    error(
        AccessStore::new(&mut f.connection).unwrap().work_status(
            Actor::Admin(f.owner),
            f.workspace,
            Uuid::new_v4(),
            NOW,
        ),
        ContentError::NotFound,
    );
}

#[test]
fn live_slot_limit_paging_and_clears_do_not_confuse_tombstones_with_empty_history() {
    let mut f = fixture();
    let a = enroll(&mut f, DataRole::ReadWrite);
    let mut ids = Vec::new();
    for _ in 0..64 {
        let id = Uuid::new_v4();
        apply(&mut f, a.credential_id, &post(id, 0, "live"), NOW).unwrap();
        ids.push(id);
    }
    let extra = Uuid::new_v4();
    error(
        apply(&mut f, a.credential_id, &post(extra, 0, "over quota"), NOW),
        ContentError::InvalidInput,
    );
    let first = page(&mut f, None, 32, NOW);
    let second = page(&mut f, first.next_cursor, 32, NOW);
    let mut actual = first
        .items
        .iter()
        .chain(&second.items)
        .map(|v| v.status_id)
        .collect::<Vec<_>>();
    let mut expected = ids.clone();
    actual.sort();
    expected.sort();
    assert_eq!(actual, expected);
    assert_eq!(first.next_cursor, first.items.last().map(|v| v.status_id));
    assert_eq!(second.next_cursor, None);
    apply(
        &mut f,
        a.credential_id,
        &post(ids[0], 1, "update while full"),
        NOW + 1,
    )
    .unwrap();
    apply(&mut f, a.credential_id, &clear(ids[0], 2), NOW + 2).unwrap();
    apply(
        &mut f,
        a.credential_id,
        &post(extra, 0, "replacement slot"),
        NOW + 3,
    )
    .unwrap();
    assert_eq!(rows(&f.connection, "ccp_work_status"), 65);
    assert_eq!(page(&mut f, None, 32, NOW).items.len(), 32);
    for limit in [0, 33, usize::MAX] {
        error(
            AccessStore::new(&mut f.connection).unwrap().work_statuses(
                Actor::Admin(f.owner),
                f.workspace,
                None,
                limit,
                NOW,
            ),
            ContentError::InvalidInput,
        );
    }
}

#[test]
fn client_scope_and_optional_agent_are_independent_of_workspace_and_legacy_display_names() {
    let mut f = fixture();
    let a = enroll(&mut f, DataRole::ReadWrite);
    let id = Uuid::new_v4();
    let mut request = post(id, 0, "no agent label");
    request.change = WorkStatusChange::Set {
        agent: None,
        message: "no agent label".into(),
    };
    apply(&mut f, a.credential_id, &request, NOW).unwrap();
    assert_eq!(read(&mut f, id, NOW).agent, None);
    let other = f.other_workspace;
    let b = AccessStore::new(&mut f.connection)
        .unwrap()
        .register_client(
            Actor::Admin(f.other_owner),
            &registration(other, DataRole::ReadWrite),
            NOW,
        )
        .unwrap();
    let other_receipt = AccessStore::new(&mut f.connection)
        .unwrap()
        .mutate_work_status(
            Actor::Credential(b.credential_id),
            other,
            &post(id, 0, "different workspace"),
            NOW,
        )
        .unwrap();
    assert_eq!(other_receipt.client_id, b.client_id);
    assert_eq!(read(&mut f, id, NOW).client_id, a.client_id);
    assert_eq!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .work_statuses(Actor::Admin(f.other_owner), other, None, 32, NOW)
            .unwrap()
            .items
            .len(),
        1
    );
    f.connection
        .execute_batch("INSERT INTO sessions(id,name) VALUES(88,'legacy');")
        .unwrap();
    f.connection
        .execute(
            "UPDATE ccp_workspaces SET legacy_session_id=88 WHERE id=?1",
            [f.workspace.to_string()],
        )
        .unwrap();
    error(
        apply(&mut f, a.credential_id, &post(id, 1, "legacy"), NOW),
        ContentError::LegacyConversionRequired,
    );
    error(
        AccessStore::new(&mut f.connection).unwrap().work_status(
            Actor::Admin(f.owner),
            f.workspace,
            id,
            NOW,
        ),
        ContentError::LegacyConversionRequired,
    );
}

#[test]
fn sqlite_maximum_revision_has_one_exact_terminal_increment_and_replays_stay_historical() {
    let mut f = fixture();
    let a = enroll(&mut f, DataRole::ReadWrite);
    let id = Uuid::new_v4();
    apply(&mut f, a.credential_id, &post(id, 0, "first"), NOW).unwrap();
    f.connection
        .execute(
            "UPDATE ccp_work_status SET revision=?2 WHERE id=?1",
            params![id.to_string(), i64::MAX - 1],
        )
        .unwrap();
    let request = post(
        id,
        i64::MAX.unsigned_abs() - 1,
        "last representable revision",
    );
    let receipt = apply(&mut f, a.credential_id, &request, NOW + 1).unwrap();
    assert_eq!(receipt.revision, i64::MAX.unsigned_abs());
    error(
        apply(
            &mut f,
            a.credential_id,
            &post(id, receipt.revision, "overflow"),
            NOW + 2,
        ),
        ContentError::InvalidInput,
    );
    assert_eq!(
        apply(&mut f, a.credential_id, &request, NOW + 2).unwrap(),
        receipt
    );
}
