// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;
use crate::foundation::workspace_deletion::PAYLOAD;

fn mutation(change: EntryChange) -> EntryMutation {
    EntryMutation {
        operation_id: Uuid::new_v4(),
        agent: Some("PRIVATE_AGENT".into()),
        change,
    }
}
fn seed(f: &mut Fixture, client: &Enrollment) -> EntryMutation {
    let owner = Actor::Admin(f.owner);
    let writer = Actor::Credential(client.credential_id);
    let mut s = AccessStore::new(&mut f.connection).unwrap();
    let node = NodeDraft {
        name: "PRIVATE_NODE".into(),
        description: "PRIVATE_DESCRIPTION".into(),
    };
    let shelf = s.create_shelf(owner, f.workspace, &node, NOW).unwrap();
    let book = s
        .create_book(owner, f.workspace, shelf.id, &node, NOW)
        .unwrap();
    let create = mutation(EntryChange::Create {
        book_id: book.id,
        name: "PRIVATE_NAME".into(),
        content: EntryContent {
            text: "PRIVATE_BODY".into(),
            description: "PRIVATE_META".into(),
            labels: vec!["PRIVATE_LABEL".into()],
        },
    });
    let e = s.mutate_entry(writer, f.workspace, &create, NOW).unwrap();
    s.mutate_entry(
        writer,
        f.workspace,
        &mutation(EntryChange::Append {
            id: e.entry_id,
            expected_revision: 1,
            text: "PRIVATE_APPEND".into(),
        }),
        NOW,
    )
    .unwrap();
    s.mutate_entry(
        writer,
        f.workspace,
        &mutation(EntryChange::Archive {
            id: e.entry_id,
            expected_revision: 2,
        }),
        NOW,
    )
    .unwrap();
    s.purge_entry(
        owner,
        f.workspace,
        &EntryPurge {
            operation_id: Uuid::new_v4(),
            entry_id: e.entry_id,
            expected_revision: 3,
            confirm_purge: true,
        },
        NOW,
    )
    .unwrap();
    seed_archives(&mut s, owner, f.workspace, writer, &create);
    for archived in [true, false] {
        s.transition_shelf(
            owner,
            f.workspace,
            &ShelfTransition {
                operation_id: Uuid::new_v4(),
                shelf_id: shelf.id,
                expected_revision: if archived { 1 } else { 2 },
                archived,
            },
            NOW + 10,
        )
        .unwrap();
    }
    s.resolve_entry_operation(
        writer,
        f.workspace,
        &EntryOperationTarget {
            operation_id: Uuid::new_v4(),
            credential_id: client.credential_id,
            request_hash: [7; 32],
        },
        NOW,
    )
    .unwrap();
    seed_boards(&mut s, owner, f.workspace, writer, f.global);
    seed_invites(&mut s, owner, f.workspace, client);
    create
}

#[test]
fn bounded_cleanup_removes_all_payload_and_history_preserving_shared_identity_and_other_workspace()
{
    let mut f = fixture();
    let client = enroll(&mut f, DataRole::ReadWrite);
    let owner = f.owner;
    let ws = f.workspace;
    let k = key(&mut f, owner, AdminKeyScope::Workspace(ws));
    let original = seed(&mut f, &client);
    for (table, scope) in PAYLOAD {
        let count: i64 = f
            .connection
            .query_row(
                &format!("SELECT count(*) FROM {table} WHERE {scope}=?1"),
                [ws.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        assert!(count > 0, "fixture must exercise {table}");
    }
    let before = shared_shelf(&mut f, &client);
    let identities = identity_rows(&f);
    archive(&mut f);
    let request = intent(
        ws,
        preview(&mut f, Actor::AdminKey(k.id)).lifecycle_revision,
    );
    begin(&mut f, Actor::AdminKey(k.id), &request);
    // A completion claim is forbidden while any payload remains.
    assert!(
        f.connection
            .execute("UPDATE ccp_workspace_deletions SET completed_at=?1", [NOW])
            .is_err()
    );
    let result = finish(&mut f, 1);
    assert_eq!(result.completed_at, Some(NOW + 1));
    for (table, scope) in PAYLOAD {
        let count: i64 = f
            .connection
            .query_row(
                &format!("SELECT count(*) FROM {table} WHERE {scope}=?1"),
                [ws.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0, "{table}");
    }
    let after = identity_rows(&f);
    assert_eq!(after, identities);
    retired(&f, &k);
    let mut s = AccessStore::new(&mut f.connection).unwrap();
    assert_eq!(
        s.get_shelf(
            Actor::Credential(client.credential_id),
            f.other_workspace,
            before.id,
            NOW
        )
        .unwrap(),
        before
    );
    assert_eq!(
        s.guidance(Actor::Admin(f.global), None, NOW)
            .unwrap()
            .global
            .text,
        "KEEP_GLOBAL_BOARD"
    );
    error(
        s.mutate_entry(Actor::Credential(client.credential_id), ws, &original, NOW),
        ContentError::Forbidden,
    );
    assert_eq!(
        s.advance_workspace_deletion(ws, 1, NOW + 100).unwrap(),
        result
    );
    assert!(s.pending_workspace_deletions(None, 128).unwrap().is_empty());
    let replacement = s
        .create_workspace(Actor::Admin(f.global), "alpha", owner, NOW)
        .unwrap();
    assert_ne!(replacement, ws);
    assert_eq!(f.connection.query_row("SELECT count(*) FROM ccp_access_audit WHERE target_id=?1 AND action IN('begin_workspace_deletion','complete_workspace_deletion') AND workspace_id IS NULL",[ws.to_string()],|r|r.get::<_,i64>(0)).unwrap(),2);
    let fk: Option<String> = f
        .connection
        .query_row("PRAGMA foreign_key_check", [], |r| r.get(0))
        .optional()
        .unwrap();
    assert!(fk.is_none());
}

fn seed_archives(
    s: &mut AccessStore<'_>,
    owner: Actor,
    workspace: Uuid,
    writer: Actor,
    create: &EntryMutation,
) {
    // A second archive is removed by timed retention; the third keeps payload and a paused clock.
    for index in 0..2 {
        let mut another = create.clone();
        another.operation_id = Uuid::new_v4();
        if let EntryChange::Create { name, .. } = &mut another.change {
            *name = format!("archive{index}");
        }
        let e = s.mutate_entry(writer, workspace, &another, NOW).unwrap();
        s.mutate_entry(
            writer,
            workspace,
            &mutation(EntryChange::Archive {
                id: e.entry_id,
                expected_revision: 1,
            }),
            NOW,
        )
        .unwrap();
        if index == 0 {
            s.change_entry_retention(
                owner,
                workspace,
                &EntryRetentionChange {
                    operation_id: Uuid::new_v4(),
                    expected_revision: 0,
                    retention_seconds: Some(10),
                    confirm_timed_purge: true,
                },
                NOW,
            )
            .unwrap();
            assert_eq!(
                s.purge_due_entry_archives(workspace, 8, NOW + 10)
                    .unwrap()
                    .len(),
                1
            );
        }
    }
}

fn seed_boards(
    s: &mut AccessStore<'_>,
    owner: Actor,
    workspace: Uuid,
    writer: Actor,
    global: Uuid,
) {
    for rev in 0..2 {
        s.save_guidance(
            owner,
            GuidanceScope::Workspace(workspace),
            &GuidanceSave {
                operation_id: Uuid::new_v4(),
                expected_revision: rev,
                text: format!("PRIVATE_BOARD{rev}"),
            },
            NOW,
        )
        .unwrap();
    }
    s.save_guidance(
        Actor::Admin(global),
        GuidanceScope::Global,
        &GuidanceSave {
            operation_id: Uuid::new_v4(),
            expected_revision: 0,
            text: "KEEP_GLOBAL_BOARD".into(),
        },
        NOW,
    )
    .unwrap();
    s.mutate_work_status(
        writer,
        workspace,
        &WorkStatusMutation {
            operation_id: Uuid::new_v4(),
            status_id: Uuid::new_v4(),
            expected_revision: 0,
            change: WorkStatusChange::Set {
                agent: Some("PRIVATE_AGENT".into()),
                message: "PRIVATE_STATUS".into(),
            },
        },
        NOW,
    )
    .unwrap();
}

fn shared_shelf(f: &mut Fixture, client: &Enrollment) -> WorkspaceShelf {
    // Bind this exact credential to B too, as individual admission would do.
    let bgrant = Uuid::new_v4();
    f.connection
        .execute(
            "INSERT INTO ccp_grants(id,workspace_id,client_id,role) VALUES(?1,?2,?3,'read_write')",
            params![
                bgrant.to_string(),
                f.other_workspace.to_string(),
                client.client_id.to_string()
            ],
        )
        .unwrap();
    f.connection
        .execute(
            "INSERT INTO ccp_credential_grants VALUES(?1,?2,?3)",
            params![
                client.credential_id.to_string(),
                bgrant.to_string(),
                client.client_id.to_string()
            ],
        )
        .unwrap();
    let mut s = AccessStore::new(&mut f.connection).unwrap();
    let b = s
        .create_shelf(
            Actor::Credential(client.credential_id),
            f.other_workspace,
            &NodeDraft {
                name: "KEEP_B".into(),
                description: "KEEP_B_META".into(),
            },
            NOW,
        )
        .unwrap();
    s.get_shelf(
        Actor::Credential(client.credential_id),
        f.other_workspace,
        b.id,
        NOW,
    )
    .unwrap()
}

fn retired(f: &Fixture, k: &IssuedAdminKey) {
    for table in [
        "ccp_admin_keys",
        "ccp_grants",
        "ccp_invitations",
        "ccp_recovery_invitations",
    ] {
        let count: i64 = f
            .connection
            .query_row(
                &format!(
                    "SELECT count(*) FROM {table} WHERE workspace_id=?1 AND revoked_at IS NULL"
                ),
                [f.workspace.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0, "{table}");
    }
    assert_eq!(
        f.connection
            .query_row(
                "SELECT name FROM ccp_admin_keys WHERE id=?1",
                [k.id.to_string()],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        ""
    );
    assert_eq!(
        f.connection
            .query_row(
                "SELECT count(*) FROM ccp_workspace_admins WHERE workspace_id=?1",
                [f.workspace.to_string()],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}

fn seed_invites(s: &mut AccessStore<'_>, owner: Actor, workspace: Uuid, client: &Enrollment) {
    s.issue_individual_invitation(
        owner,
        IndividualInvitationRequest {
            workspace_id: workspace,
            role: DataRole::Read,
            expires_at: EXPIRES,
        },
        NOW,
    )
    .unwrap();
    s.issue_recovery_invitation(
        owner,
        RecoveryInvitationRequest {
            client_id: client.client_id,
            scope: RecoveryScope::Workspace(workspace),
            expires_at: EXPIRES,
        },
        NOW,
    )
    .unwrap();
}

fn identity_rows(f: &Fixture) -> Vec<Vec<Vec<rusqlite::types::Value>>> {
    [
        "ccp_clients",
        "ccp_credentials",
        "ccp_admin_principals",
        "ccp_credential_grants",
    ]
    .iter()
    .map(|table| {
        let mut query = f
            .connection
            .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
            .unwrap();
        let columns = query.column_count();
        query
            .query_map([], |r| {
                (0..columns)
                    .map(|i| r.get(i))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    })
    .collect()
}
