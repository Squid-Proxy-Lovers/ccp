// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

#[test]
fn clients_share_append_but_only_owner_revises_with_all_snapshots_and_attribution_retained() {
    let mut f = fixture();
    let owner = enroll(&mut f, DataRole::ReadWrite);
    let writer = enroll(&mut f, DataRole::ReadWrite);
    let reader = enroll(&mut f, DataRole::Read);
    let book = book(&mut f);
    let actor = Actor::Credential(owner.credential_id);
    let other = Actor::Credential(writer.credential_id);
    let receipt = apply(&mut f, actor, &create(book, " entry ", "first"));
    let id = receipt.entry_id;
    assert_eq!(receipt.revision, 1);
    assert_eq!(get(&mut f, id).entry.name, "entry");
    let append = command(EntryChange::Append {
        id,
        expected_revision: 1,
        text: "\nother contribution".into(),
    });
    assert_eq!(apply(&mut f, other, &append).revision, 2);
    assert_eq!(
        get(&mut f, id).content,
        content("first\nother contribution")
    );
    assert_eq!(get(&mut f, id).entry.owner_client_id, Some(owner.client_id));
    for change in [
        EntryChange::Revise {
            id,
            expected_revision: 2,
            content: content("forged"),
        },
        EntryChange::Archive {
            id,
            expected_revision: 2,
        },
    ] {
        fail(&mut f, other, &command(change), ContentError::Forbidden);
    }
    let revise = command(EntryChange::Revise {
        id,
        expected_revision: 2,
        content: content("revised"),
    });
    assert_eq!(apply(&mut f, actor, &revise).revision, 3);
    fail(
        &mut f,
        Actor::Credential(reader.credential_id),
        &command(EntryChange::Append {
            id,
            expected_revision: 3,
            text: "denied".into(),
        }),
        ContentError::Forbidden,
    );
    assert_snapshots(&mut f, owner, writer, reader, id);
}

#[test]
fn scoped_keys_override_preserve_owner_and_global_keys_cannot_borrow_write_authority() {
    let mut f = fixture();
    let owner = enroll(&mut f, DataRole::ReadWrite);
    let book = book(&mut f);
    let id = apply(
        &mut f,
        Actor::Credential(owner.credential_id),
        &create(book, "entry", "owned"),
    )
    .entry_id;
    f.connection
        .execute(
            "UPDATE ccp_workspaces SET owner_admin_id=?1 WHERE id=?2",
            params![f.global.to_string(), f.workspace.to_string()],
        )
        .unwrap();
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    let request = |scope| AdminKeyRequest {
        principal_id: f.global,
        scope,
        name: "key".into(),
        expires_at: None,
    };
    let global = store
        .issue_admin_key(
            Actor::Admin(f.global),
            &request(AdminKeyScope::Instance),
            NOW,
        )
        .unwrap();
    let scoped = store
        .issue_admin_key(
            Actor::Admin(f.global),
            &request(AdminKeyScope::Workspace(f.workspace)),
            NOW,
        )
        .unwrap();
    let global = store.authenticate_admin_key(global.secret(), NOW).unwrap();
    let scoped = store.authenticate_admin_key(scoped.secret(), NOW).unwrap();
    let change = command(EntryChange::Revise {
        id,
        expected_revision: 1,
        content: content("override"),
    });
    error(
        store.mutate_entry(global, f.workspace, &change, NOW),
        ContentError::Forbidden,
    );
    store
        .mutate_entry(scoped, f.workspace, &change, NOW)
        .unwrap();
    let snapshot = store.get_entry(global, f.workspace, id, NOW).unwrap();
    assert_eq!(snapshot.entry.owner_client_id, Some(owner.client_id));
    assert_eq!(snapshot.revision.actor, scoped);
    assert_eq!(snapshot.revision.admin_principal_id, Some(f.global));
    error(
        store.mutate_entry(
            scoped,
            f.other_workspace,
            &create(book, "foreign", "x"),
            NOW,
        ),
        ContentError::Forbidden,
    );
    error(
        store.mutate_entry(
            Actor::Admin(f.other_owner),
            f.other_workspace,
            &create(book, "foreign", "x"),
            NOW,
        ),
        ContentError::NotFound,
    );
    let audit:(String,String)=f.connection.query_row("SELECT actor_id,admin_key_id FROM ccp_access_audit WHERE target_id=?1 AND action='revise_entry'",[id.to_string()],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(audit.0, f.global.to_string());
    let Actor::AdminKey(key) = scoped else {
        panic!("expected key")
    };
    assert_eq!(audit.1, key.to_string());
}

#[test]
fn admin_creation_has_explicit_admin_provenance_and_no_claimable_client_owner() {
    let mut f = fixture();
    let writer = enroll(&mut f, DataRole::ReadWrite);
    let book = book(&mut f);
    let admin = Actor::Admin(f.admin);
    let id = apply(&mut f, admin, &create(book, "human", "admin text")).entry_id;
    let snapshot = get(&mut f, id);
    assert_eq!(snapshot.entry.owner_client_id, None);
    assert_eq!(snapshot.revision.admin_principal_id, Some(f.admin));
    assert_eq!(snapshot.revision.client_id, None);
    let actor = Actor::Credential(writer.credential_id);
    apply(
        &mut f,
        actor,
        &command(EntryChange::Append {
            id,
            expected_revision: 1,
            text: "+writer".into(),
        }),
    );
    fail(
        &mut f,
        actor,
        &command(EntryChange::Revise {
            id,
            expected_revision: 2,
            content: content("claim"),
        }),
        ContentError::Forbidden,
    );
    apply(
        &mut f,
        admin,
        &command(EntryChange::Revise {
            id,
            expected_revision: 2,
            content: content("admin edit"),
        }),
    );
    assert_eq!(get(&mut f, id).entry.owner_client_id, None);
    assert_eq!(get(&mut f, id).content.text, "admin edit");
}

fn assert_snapshots(
    f: &mut Fixture,
    owner: Enrollment,
    writer: Enrollment,
    reader: Enrollment,
    id: Uuid,
) {
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    let snapshot = store
        .entry_version(
            Actor::Credential(reader.credential_id),
            f.workspace,
            id,
            2,
            NOW,
        )
        .unwrap();
    assert_eq!(snapshot.content, content("first\nother contribution"));
    assert_eq!(
        snapshot.appended_text.as_deref(),
        Some("\nother contribution")
    );
    assert_eq!(
        snapshot.revision.actor,
        Actor::Credential(writer.credential_id)
    );
    assert_eq!(snapshot.revision.client_id, Some(writer.client_id));
    assert_eq!(snapshot.revision.agent.as_deref(), Some("agent one"));
    assert_eq!(snapshot.revision.admin_principal_id, None);
    assert_eq!(snapshot.entry.owner_client_id, Some(owner.client_id));
    assert_eq!(
        store
            .entry_version(
                Actor::Credential(owner.credential_id),
                f.workspace,
                id,
                1,
                NOW
            )
            .unwrap()
            .content,
        content("first")
    );
    assert_eq!(
        store
            .get_entry(Actor::Credential(owner.credential_id), f.workspace, id, NOW)
            .unwrap()
            .content,
        content("revised")
    );
    let first = store
        .entry_history(
            Actor::Credential(owner.credential_id),
            f.workspace,
            &RevisionPageRequest {
                entry_id: id,
                after: 0,
                limit: 2,
            },
            NOW,
        )
        .unwrap();
    assert_eq!(
        first
            .items
            .iter()
            .map(|v| v.action.as_str())
            .collect::<Vec<_>>(),
        vec!["create", "append"]
    );
    assert_eq!(first.next_cursor, Some(2));
    let last = store
        .entry_history(
            Actor::Credential(owner.credential_id),
            f.workspace,
            &RevisionPageRequest {
                entry_id: id,
                after: 2,
                limit: 2,
            },
            NOW,
        )
        .unwrap();
    assert_eq!(last.items.len(), 1);
    assert_eq!(last.items[0].action, "revise");
    assert_eq!(last.next_cursor, None);
}
