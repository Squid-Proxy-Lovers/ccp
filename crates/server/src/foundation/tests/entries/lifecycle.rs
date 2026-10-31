// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

#[test]
fn archive_hides_full_bodies_and_history_and_restore_never_overwrites_a_live_collision() {
    let mut f = fixture();
    let owner = enroll(&mut f, DataRole::ReadWrite);
    let writer = enroll(&mut f, DataRole::ReadWrite);
    let reader = enroll(&mut f, DataRole::Read);
    let book = book(&mut f);
    let actor = Actor::Credential(owner.credential_id);
    let id = apply(
        &mut f,
        actor,
        &create(book, "entry", "original secret body"),
    )
    .entry_id;
    let archive = command(EntryChange::Archive {
        id,
        expected_revision: 1,
    });
    apply(&mut f, actor, &archive);
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    assert_archive_reads(
        &mut store,
        f.workspace,
        book,
        id,
        owner.client_id,
        [
            actor,
            Actor::Credential(reader.credential_id),
            Actor::Admin(f.admin),
        ],
    );
    let restore = command(EntryChange::Restore {
        id,
        expected_revision: 2,
    });
    error(
        store.mutate_entry(
            Actor::Credential(writer.credential_id),
            f.workspace,
            &restore,
            NOW,
        ),
        ContentError::Forbidden,
    );
    let replacement = store
        .mutate_entry(
            actor,
            f.workspace,
            &create(book, "entry", "replacement"),
            NOW,
        )
        .unwrap();
    error(
        store.mutate_entry(actor, f.workspace, &restore, NOW),
        ContentError::Conflict,
    );
    assert_eq!(
        store
            .get_entry(actor, f.workspace, replacement.entry_id, NOW)
            .unwrap()
            .content
            .text,
        "replacement"
    );
    store
        .mutate_entry(
            actor,
            f.workspace,
            &command(EntryChange::Archive {
                id: replacement.entry_id,
                expected_revision: 1,
            }),
            NOW,
        )
        .unwrap();
    assert_eq!(
        store
            .mutate_entry(Actor::Admin(f.admin), f.workspace, &restore, NOW)
            .unwrap()
            .revision,
        3
    );
    assert_restored_history(
        &mut store,
        actor,
        f.workspace,
        id,
        owner.client_id,
        &archive,
    );
}

#[test]
fn receipts_bind_exact_actor_request_agent_and_current_authority_without_duplicate_appends() {
    let mut f = fixture();
    let owner = enroll(&mut f, DataRole::ReadWrite);
    let writer = enroll(&mut f, DataRole::ReadWrite);
    let book = book(&mut f);
    let actor = Actor::Credential(owner.credential_id);
    let create = create(book, "entry", "base");
    let first = apply(&mut f, actor, &create);
    assert_eq!(apply(&mut f, actor, &create), first);
    let id = first.entry_id;
    let append = command(EntryChange::Append {
        id,
        expected_revision: 1,
        text: "+once".into(),
    });
    let receipt = apply(&mut f, actor, &append);
    assert_eq!(apply(&mut f, actor, &append), receipt);
    assert_eq!(get(&mut f, id).content.text, "base+once");
    let mut changed = append.clone();
    changed.agent = Some("different agent".into());
    fail(&mut f, actor, &changed, ContentError::Conflict);
    changed = append.clone();
    changed.change = EntryChange::Append {
        id,
        expected_revision: 1,
        text: "+twice".into(),
    };
    fail(&mut f, actor, &changed, ContentError::Conflict);
    fail(
        &mut f,
        Actor::Credential(writer.credential_id),
        &append,
        ContentError::Conflict,
    );
    let stale = command(append.change.clone());
    fail(&mut f, actor, &stale, ContentError::Conflict);
    let stale_revise = command(EntryChange::Revise {
        id,
        expected_revision: 1,
        content: content("lost append"),
    });
    fail(&mut f, actor, &stale_revise, ContentError::Conflict);
    assert_eq!(get(&mut f, id).content.text, "base+once");
    assert_eq!(rows(&f.connection, "ccp_content_operations"), 2);
    assert_eq!(rows(&f.connection, "ccp_content_versions"), 2);
    AccessStore::new(&mut f.connection)
        .unwrap()
        .revoke_grant(Actor::Admin(f.admin), owner.grant_id, NOW)
        .unwrap();
    fail(&mut f, actor, &append, ContentError::Forbidden);
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    error(
        store.get_entry(actor, f.workspace, id, NOW),
        ContentError::Forbidden,
    );
}

#[test]
fn ownership_never_bypasses_read_only_disable_expiry_or_archived_workspace() {
    let mut f = fixture();
    let owner = enroll(&mut f, DataRole::ReadWrite);
    let book = book(&mut f);
    let actor = Actor::Credential(owner.credential_id);
    let create = create(book, "entry", "keep");
    let id = apply(&mut f, actor, &create).entry_id;
    f.connection
        .execute(
            "UPDATE ccp_grants SET role='read' WHERE id=?1",
            [owner.grant_id.to_string()],
        )
        .unwrap();
    let archive = command(EntryChange::Archive {
        id,
        expected_revision: 1,
    });
    fail(&mut f, actor, &archive, ContentError::Forbidden);
    assert_eq!(get(&mut f, id).content.text, "keep");
    f.connection
        .execute(
            "UPDATE ccp_grants SET role='read_write' WHERE id=?1",
            [owner.grant_id.to_string()],
        )
        .unwrap();
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    store
        .set_archived(Actor::Admin(f.owner), f.workspace, true, NOW)
        .unwrap();
    assert_eq!(
        store
            .get_entry(actor, f.workspace, id, NOW)
            .unwrap()
            .content
            .text,
        "keep"
    );
    error(
        store.mutate_entry(actor, f.workspace, &create, NOW),
        ContentError::Forbidden,
    );
    error(
        store.mutate_entry(Actor::Admin(f.admin), f.workspace, &archive, NOW),
        ContentError::Forbidden,
    );
    store
        .set_archived(Actor::Admin(f.owner), f.workspace, false, NOW)
        .unwrap();
    error(
        store.get_entry(actor, f.workspace, id, EXPIRES),
        ContentError::Forbidden,
    );
    error(
        store.mutate_entry(actor, f.workspace, &archive, EXPIRES),
        ContentError::Forbidden,
    );
    store
        .disable_client(Actor::Admin(f.global), owner.client_id, NOW)
        .unwrap();
    error(
        store.get_entry(actor, f.workspace, id, NOW),
        ContentError::Forbidden,
    );
    error(
        store.mutate_entry(actor, f.workspace, &archive, NOW),
        ContentError::Forbidden,
    );
    assert_eq!(
        store
            .get_entry(Actor::Admin(f.owner), f.workspace, id, NOW)
            .unwrap()
            .content
            .text,
        "keep"
    );
}

#[test]
fn archived_entries_refuse_append_revise_and_repeated_archive_but_require_current_owner_to_restore()
{
    let mut f = fixture();
    let owner = enroll(&mut f, DataRole::ReadWrite);
    let book = book(&mut f);
    let actor = Actor::Credential(owner.credential_id);
    let id = apply(&mut f, actor, &create(book, "entry", "keep")).entry_id;
    fail(
        &mut f,
        actor,
        &command(EntryChange::Restore {
            id,
            expected_revision: 1,
        }),
        ContentError::Conflict,
    );
    apply(
        &mut f,
        actor,
        &command(EntryChange::Archive {
            id,
            expected_revision: 1,
        }),
    );
    for change in [
        EntryChange::Append {
            id,
            expected_revision: 2,
            text: "no".into(),
        },
        EntryChange::Revise {
            id,
            expected_revision: 2,
            content: content("no"),
        },
        EntryChange::Archive {
            id,
            expected_revision: 2,
        },
        EntryChange::Restore {
            id,
            expected_revision: 1,
        },
    ] {
        fail(&mut f, actor, &command(change), ContentError::Conflict);
    }
    f.connection
        .execute(
            "UPDATE ccp_grants SET role='read' WHERE id=?1",
            [owner.grant_id.to_string()],
        )
        .unwrap();
    fail(
        &mut f,
        actor,
        &command(EntryChange::Restore {
            id,
            expected_revision: 2,
        }),
        ContentError::Forbidden,
    );
    assert_eq!(rows(&f.connection, "ccp_content_versions"), 2);
    let admin = Actor::Admin(f.admin);
    apply(
        &mut f,
        admin,
        &command(EntryChange::Restore {
            id,
            expected_revision: 2,
        }),
    );
    assert_eq!(get(&mut f, id).content.text, "keep");
}

fn assert_archive_reads(
    store: &mut AccessStore<'_>,
    workspace: Uuid,
    book: Uuid,
    id: Uuid,
    owner: Uuid,
    actors: [Actor; 3],
) {
    for actor in actors {
        error(
            store.get_entry(actor, workspace, id, NOW),
            ContentError::NotFound,
        );
        error(
            store.entry_version(actor, workspace, id, 1, NOW),
            ContentError::NotFound,
        );
        error(
            store.entry_history(
                actor,
                workspace,
                &RevisionPageRequest {
                    entry_id: id,
                    after: 0,
                    limit: 100,
                },
                NOW,
            ),
            ContentError::NotFound,
        );
        let archived = store
            .entries(actor, workspace, &page(book, true), NOW)
            .unwrap();
        assert_eq!(archived.items.len(), 1);
        assert_eq!(archived.items[0].owner_client_id, Some(owner));
        assert!(archived.items[0].archived);
        assert_eq!(archived.items[0].revision, 2);
        assert!(
            store
                .entries(actor, workspace, &page(book, false), NOW)
                .unwrap()
                .items
                .is_empty()
        );
    }
}

fn assert_restored_history(
    store: &mut AccessStore<'_>,
    actor: Actor,
    workspace: Uuid,
    id: Uuid,
    owner: Uuid,
    archive: &EntryMutation,
) {
    let snapshot = store.get_entry(actor, workspace, id, NOW).unwrap();
    assert_eq!(snapshot.content.text, "original secret body");
    assert_eq!(snapshot.entry.owner_client_id, Some(owner));
    assert_eq!(
        store
            .entry_version(actor, workspace, id, 2, NOW)
            .unwrap()
            .revision
            .action,
        "archive"
    );
    assert_eq!(
        store
            .entry_version(actor, workspace, id, 1, NOW)
            .unwrap()
            .content
            .text,
        "original secret body"
    );
    assert_eq!(
        store
            .mutate_entry(actor, workspace, archive, NOW)
            .unwrap()
            .revision,
        2
    );
    assert_eq!(
        store
            .get_entry(actor, workspace, id, NOW)
            .unwrap()
            .entry
            .revision,
        3
    );
}
