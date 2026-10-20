// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

mod faults;
mod persistence;

fn change(revision: u64, seconds: Option<u64>) -> EntryRetentionChange {
    EntryRetentionChange {
        operation_id: Uuid::new_v4(),
        expected_revision: revision,
        retention_seconds: seconds,
        confirm_timed_purge: seconds.is_some(),
    }
}
fn configure(f: &mut Fixture, request: &EntryRetentionChange, now: i64) -> EntryRetentionPolicy {
    AccessStore::new(&mut f.connection)
        .unwrap()
        .change_entry_retention(Actor::Admin(f.owner), f.workspace, request, now)
        .unwrap()
}
fn cleanup(f: &mut Fixture, now: i64) -> Vec<TimedEntryPurge> {
    AccessStore::new(&mut f.connection)
        .unwrap()
        .purge_due_entry_archives(f.workspace, 32, now)
        .unwrap()
}
fn archive(f: &mut Fixture, id: Uuid, expected: u64, now: i64) -> EntryReceipt {
    AccessStore::new(&mut f.connection)
        .unwrap()
        .mutate_entry(
            Actor::Admin(f.owner),
            f.workspace,
            &command(EntryChange::Archive {
                id,
                expected_revision: expected,
            }),
            now,
        )
        .unwrap()
}
fn entry(f: &mut Fixture, name: &str) -> EntryReceipt {
    let book = book(f);
    apply(
        f,
        Actor::Admin(f.owner),
        &create(book, name, "PRIVATE_BODY"),
    )
}
fn clock(f: &Fixture, id: Uuid) -> (u64, u64, Option<i64>) {
    f.connection.query_row("SELECT archive_revision,remaining_seconds,active_since FROM ccp_entry_retention_clocks WHERE entry_id=?1",[id.to_string()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap()
}
fn lifecycle(f: &mut Fixture, archived: bool, now: i64) {
    AccessStore::new(&mut f.connection)
        .unwrap()
        .set_archived(Actor::Admin(f.owner), f.workspace, archived, now)
        .unwrap();
}

#[test]
fn manual_is_default_and_only_current_scoped_admins_can_enable_retention() {
    let mut f = fixture();
    let owner = Actor::Admin(f.owner);
    let writer = enroll(&mut f, DataRole::ReadWrite);
    let policy = AccessStore::new(&mut f.connection)
        .unwrap()
        .entry_retention(owner, f.workspace, NOW)
        .unwrap();
    assert_eq!(policy.revision, 0);
    assert_eq!(policy.retention_seconds, None);
    assert_eq!(policy.changed_at, None);
    assert_eq!(rows(&f.connection, "ccp_entry_retention_versions"), 0);
    let request = change(0, Some(100));
    for actor in [
        Actor::Credential(writer.credential_id),
        Actor::Admin(f.global),
        Actor::Admin(f.other_owner),
    ] {
        error(
            AccessStore::new(&mut f.connection)
                .unwrap()
                .change_entry_retention(actor, f.workspace, &request, NOW),
            ContentError::Forbidden,
        );
    }
    error(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .entry_retention(Actor::Credential(writer.credential_id), f.workspace, NOW),
        ContentError::Forbidden,
    );
    assert_eq!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .entry_retention(Actor::Admin(f.global), f.workspace, NOW)
            .unwrap(),
        policy
    );
    let admin = Actor::Admin(f.admin);
    assert_eq!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .change_entry_retention(admin, f.workspace, &request, NOW)
            .unwrap()
            .retention_seconds,
        Some(100)
    );
    f.connection
        .execute(
            "DELETE FROM ccp_workspace_admins WHERE admin_id=?1",
            [f.admin.to_string()],
        )
        .unwrap();
    error(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .change_entry_retention(admin, f.workspace, &request, NOW),
        ContentError::Forbidden,
    );
    assert!(cleanup(&mut f, NOW + 100).is_empty());
}

#[test]
fn existing_archives_receive_fresh_full_window_without_using_original_archive_age() {
    let mut f = fixture();
    let e = entry(&mut f, "old archive");
    archive(&mut f, e.entry_id, 1, NOW);
    assert!(cleanup(&mut f, NOW + 1000).is_empty());
    configure(&mut f, &change(0, Some(100)), NOW + 1000);
    assert_eq!(clock(&f, e.entry_id), (2, 100, Some(NOW + 1000)));
    assert!(cleanup(&mut f, NOW + 1099).is_empty());
    let proofs = cleanup(&mut f, NOW + 1100);
    assert_eq!(proofs.len(), 1);
    assert_eq!(proofs[0].entry_id, e.entry_id);
    assert_eq!(proofs[0].archive_revision, 2);
    assert_eq!(proofs[0].policy_revision, 1);
    assert_eq!(proofs[0].purged_at, NOW + 1100);
    assert!(cleanup(&mut f, NOW + 1200).is_empty());
    assert_eq!(rows(&f.connection, "ccp_content_versions"), 0);
    assert_eq!(rows(&f.connection, "ccp_entry_retention_clocks"), 0);
    assert_eq!(rows(&f.connection, "ccp_entry_retention_purges"), 1);
}

#[test]
fn restore_cancels_old_clock_and_rearchive_gets_a_new_full_window() {
    let mut f = fixture();
    let owner = Actor::Admin(f.owner);
    let e = entry(&mut f, "entry");
    configure(&mut f, &change(0, Some(100)), NOW);
    let archived = command(EntryChange::Archive {
        id: e.entry_id,
        expected_revision: 1,
    });
    AccessStore::new(&mut f.connection)
        .unwrap()
        .mutate_entry(owner, f.workspace, &archived, NOW + 10)
        .unwrap();
    assert_eq!(clock(&f, e.entry_id), (2, 100, Some(NOW + 10)));
    AccessStore::new(&mut f.connection)
        .unwrap()
        .mutate_entry(
            owner,
            f.workspace,
            &command(EntryChange::Restore {
                id: e.entry_id,
                expected_revision: 2,
            }),
            NOW + 50,
        )
        .unwrap();
    assert_eq!(rows(&f.connection, "ccp_entry_retention_clocks"), 0);
    assert_eq!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .mutate_entry(owner, f.workspace, &archived, NOW + 150)
            .unwrap()
            .revision,
        2
    );
    assert_eq!(rows(&f.connection, "ccp_entry_retention_clocks"), 0);
    assert!(cleanup(&mut f, NOW + 150).is_empty());
    archive(&mut f, e.entry_id, 3, NOW + 200);
    assert_eq!(clock(&f, e.entry_id), (4, 100, Some(NOW + 200)));
    assert!(cleanup(&mut f, NOW + 299).is_empty());
    assert_eq!(cleanup(&mut f, NOW + 300).len(), 1);
}

#[test]
fn workspace_archive_pauses_remaining_duration_and_reactivation_resumes_it() {
    let mut f = fixture();
    let e = entry(&mut f, "entry");
    configure(&mut f, &change(0, Some(100)), NOW);
    archive(&mut f, e.entry_id, 1, NOW);
    lifecycle(&mut f, true, NOW + 30);
    assert_eq!(clock(&f, e.entry_id), (2, 70, None));
    assert!(cleanup(&mut f, NOW + 5000).is_empty());
    let disabled = change(1, None);
    error(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .change_entry_retention(Actor::Admin(f.owner), f.workspace, &disabled, NOW + 5000),
        ContentError::Forbidden,
    );
    lifecycle(&mut f, false, NOW + 5000);
    assert_eq!(clock(&f, e.entry_id), (2, 70, Some(NOW + 5000)));
    assert!(cleanup(&mut f, NOW + 5069).is_empty());
    assert_eq!(cleanup(&mut f, NOW + 5070).len(), 1);
}

#[test]
fn policy_cas_exact_retry_disable_and_reenable_never_silently_reset_windows() {
    let mut f = fixture();
    let e = entry(&mut f, "entry");
    archive(&mut f, e.entry_id, 1, NOW);
    let enable = change(0, Some(100));
    let first = configure(&mut f, &enable, NOW + 10);
    assert_eq!(configure(&mut f, &enable, NOW + 50), first);
    assert_eq!(clock(&f, e.entry_id), (2, 100, Some(NOW + 10)));
    for invalid in [change(0, Some(100)), change(1, Some(50)), change(0, None)] {
        error(
            AccessStore::new(&mut f.connection)
                .unwrap()
                .change_entry_retention(Actor::Admin(f.owner), f.workspace, &invalid, NOW + 50),
            ContentError::Conflict,
        );
    }
    let manual = configure(&mut f, &change(1, None), NOW + 60);
    assert_eq!(manual.revision, 2);
    assert_eq!(rows(&f.connection, "ccp_entry_retention_clocks"), 0);
    assert_eq!(configure(&mut f, &enable, NOW + 500), first);
    assert!(cleanup(&mut f, NOW + 500).is_empty());
    let current = AccessStore::new(&mut f.connection)
        .unwrap()
        .entry_retention(Actor::Admin(f.owner), f.workspace, NOW + 500)
        .unwrap();
    assert_eq!(current, manual);
    configure(&mut f, &change(2, Some(80)), NOW + 500);
    assert_eq!(clock(&f, e.entry_id), (2, 80, Some(NOW + 500)));
    assert!(cleanup(&mut f, NOW + 579).is_empty());
    assert_eq!(cleanup(&mut f, NOW + 580).len(), 1);
}

#[test]
fn policy_inputs_bounds_actor_binding_and_expired_or_revoked_key_are_checked() {
    let mut f = fixture();
    let owner = Actor::Admin(f.owner);
    let key = AccessStore::new(&mut f.connection)
        .unwrap()
        .issue_admin_key(
            owner,
            &AdminKeyRequest {
                principal_id: f.owner,
                scope: AdminKeyScope::Workspace(f.workspace),
                name: "retention".into(),
                expires_at: Some(NOW + 10),
            },
            NOW,
        )
        .unwrap();
    let actor = Actor::AdminKey(key.id);
    let request = change(0, Some(100));
    AccessStore::new(&mut f.connection)
        .unwrap()
        .change_entry_retention(actor, f.workspace, &request, NOW)
        .unwrap();
    error(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .change_entry_retention(owner, f.workspace, &request, NOW),
        ContentError::Conflict,
    );
    error(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .change_entry_retention(actor, f.workspace, &request, NOW + 10),
        ContentError::Forbidden,
    );
    f.connection
        .execute(
            "UPDATE ccp_admin_keys SET revoked_at=?1 WHERE id=?2",
            params![NOW, key.id.to_string()],
        )
        .unwrap();
    error(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .change_entry_retention(actor, f.workspace, &request, NOW),
        ContentError::Forbidden,
    );
    let mut invalid = vec![
        change(u64::MAX, Some(1)),
        change(i64::MAX.unsigned_abs(), Some(1)),
        change(1, Some(0)),
        change(1, Some(u64::MAX)),
    ];
    let mut confirm = change(1, None);
    confirm.confirm_timed_purge = true;
    invalid.push(confirm);
    let mut unconfirmed = change(1, Some(10));
    unconfirmed.confirm_timed_purge = false;
    invalid.push(unconfirmed);
    let mut nil = change(1, None);
    nil.operation_id = Uuid::nil();
    invalid.push(nil);
    for request in invalid {
        error(
            AccessStore::new(&mut f.connection)
                .unwrap()
                .change_entry_retention(owner, f.workspace, &request, NOW),
            ContentError::InvalidInput,
        );
    }
}

#[test]
fn timed_cleanup_preserves_original_write_receipts_siblings_guidance_and_purge_audit() {
    let mut f = fixture();
    let writer = enroll(&mut f, DataRole::ReadWrite);
    let actor = Actor::Credential(writer.credential_id);
    let book = book(&mut f);
    let original = create(book, "entry", "PRIVATE_BODY");
    let created = apply(&mut f, actor, &original);
    let archive = command(EntryChange::Archive {
        id: created.entry_id,
        expected_revision: 1,
    });
    apply(&mut f, actor, &archive);
    let sibling = apply(&mut f, actor, &create(book, "sibling", "kept"));
    let guidance = AccessStore::new(&mut f.connection)
        .unwrap()
        .guidance(Actor::Admin(f.owner), Some(f.workspace), NOW)
        .unwrap();
    configure(&mut f, &change(0, Some(10)), NOW);
    assert_eq!(cleanup(&mut f, NOW + 10).len(), 1);
    assert_eq!(apply(&mut f, actor, &original), created);
    assert_eq!(apply(&mut f, actor, &archive).revision, 2);
    assert_eq!(get(&mut f, sibling.entry_id).content.text, "kept");
    assert_eq!(rows(&f.connection, "ccp_content_operations"), 3);
    assert_eq!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .guidance(Actor::Admin(f.owner), Some(f.workspace), NOW)
            .unwrap(),
        guidance
    );
    let columns = f
        .connection
        .prepare("PRAGMA table_info(ccp_entry_retention_purges)")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(1))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(
        columns,
        vec![
            "entry_id",
            "workspace_id",
            "archive_revision",
            "policy_revision",
            "purged_at"
        ]
    );
    assert!(f.connection.execute("INSERT INTO ccp_content_entries(id,workspace_id,book_id,name,revision,archived,created_at,updated_at) VALUES(?1,?2,?3,'again',1,0,1000,1000)",params![created.entry_id.to_string(),f.workspace.to_string(),book.to_string()]).unwrap_err().to_string().contains("timed-purged entry"));
    for sql in [
        "UPDATE ccp_entry_retention_purges SET purged_at=purged_at",
        "DELETE FROM ccp_entry_retention_purges",
        "UPDATE ccp_entry_retention_versions SET retention_seconds=1",
        "DELETE FROM ccp_entry_retention_versions",
    ] {
        assert!(f.connection.execute(sql, []).is_err());
    }
}

#[test]
fn bounded_cleanup_and_shelf_overlay_never_purge_individually_active_entries() {
    let mut f = fixture();
    let owner = Actor::Admin(f.owner);
    let book = book(&mut f);
    let active = apply(&mut f, owner, &create(book, "active", "keep"));
    for name in ["one", "two", "three"] {
        let e = apply(&mut f, owner, &create(book, name, "archive"));
        archive(&mut f, e.entry_id, 1, NOW);
    }
    f.connection
        .execute("UPDATE ccp_content_shelves SET archived=1", [])
        .unwrap();
    configure(&mut f, &change(0, Some(1)), NOW);
    assert_eq!(rows(&f.connection, "ccp_entry_retention_clocks"), 3);
    let proofs = AccessStore::new(&mut f.connection)
        .unwrap()
        .purge_due_entry_archives(f.workspace, 2, NOW + 1)
        .unwrap();
    assert_eq!(proofs.len(), 2);
    assert_eq!(cleanup(&mut f, NOW + 1).len(), 1);
    assert_eq!(rows(&f.connection, "ccp_content_entries"), 1);
    f.connection
        .execute("UPDATE ccp_content_shelves SET archived=0", [])
        .unwrap();
    assert_eq!(get(&mut f, active.entry_id).content.text, "keep");
    for limit in [0, 33, usize::MAX] {
        error(
            AccessStore::new(&mut f.connection)
                .unwrap()
                .purge_due_entry_archives(f.workspace, limit, NOW),
            ContentError::InvalidInput,
        );
    }
    error(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .purge_due_entry_archives(f.workspace, 1, -1),
        ContentError::InvalidInput,
    );
    error(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .purge_due_entry_archives(Uuid::new_v4(), 1, NOW),
        ContentError::NotFound,
    );
}

#[test]
fn backward_clock_changes_preserve_remaining_and_signed_max_duration_does_not_overflow() {
    let mut f = fixture();
    let e = entry(&mut f, "entry");
    configure(&mut f, &change(0, Some(i64::MAX.unsigned_abs())), NOW);
    archive(&mut f, e.entry_id, 1, NOW);
    assert!(cleanup(&mut f, i64::MAX).is_empty());
    lifecycle(&mut f, true, NOW - 1);
    assert_eq!(clock(&f, e.entry_id), (2, i64::MAX.unsigned_abs(), None));
    lifecycle(&mut f, false, NOW + 100);
    assert!(cleanup(&mut f, i64::MAX).is_empty());
    configure(&mut f, &change(1, None), NOW + 100);
    configure(&mut f, &change(2, Some(5)), NOW + 100);
    assert!(cleanup(&mut f, NOW).is_empty());
    lifecycle(&mut f, true, NOW + 110);
    assert_eq!(clock(&f, e.entry_id), (2, 0, None));
    assert!(cleanup(&mut f, NOW + 500).is_empty());
    lifecycle(&mut f, false, NOW + 500);
    assert_eq!(cleanup(&mut f, NOW + 500).len(), 1);
}

#[test]
fn instance_key_cannot_borrow_owner_role_and_policy_survives_issuer_key_revocation() {
    let mut f = fixture();
    let owner = Actor::Admin(f.owner);
    f.connection
        .execute(
            "INSERT INTO ccp_server_admins(admin_id) VALUES(?1)",
            [f.owner.to_string()],
        )
        .unwrap();
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    let global = store
        .issue_admin_key(
            owner,
            &AdminKeyRequest {
                principal_id: f.owner,
                scope: AdminKeyScope::Instance,
                name: "instance".into(),
                expires_at: None,
            },
            NOW,
        )
        .unwrap();
    error(
        store.change_entry_retention(
            Actor::AdminKey(global.id),
            f.workspace,
            &change(0, Some(10)),
            NOW,
        ),
        ContentError::Forbidden,
    );
    let scoped = store
        .issue_admin_key(
            owner,
            &AdminKeyRequest {
                principal_id: f.owner,
                scope: AdminKeyScope::Workspace(f.workspace),
                name: "scoped".into(),
                expires_at: Some(NOW + 2),
            },
            NOW,
        )
        .unwrap();
    store
        .change_entry_retention(
            Actor::AdminKey(scoped.id),
            f.workspace,
            &change(0, Some(10)),
            NOW,
        )
        .unwrap();
    let e = entry(&mut f, "entry");
    archive(&mut f, e.entry_id, 1, NOW);
    f.connection
        .execute(
            "UPDATE ccp_admin_keys SET revoked_at=?1 WHERE id=?2",
            params![NOW + 1, scoped.id.to_string()],
        )
        .unwrap();
    assert_eq!(cleanup(&mut f, NOW + 10).len(), 1);
    configure(&mut f, &change(1, None), NOW + 10);
    assert_eq!(rows(&f.connection, "ccp_entry_retention_versions"), 2);
}

#[test]
fn final_policy_revision_remains_available_for_disable_and_stale_clocks_cannot_purge() {
    let mut f = fixture();
    let owner = Actor::Admin(f.owner);
    let e = entry(&mut f, "entry");
    archive(&mut f, e.entry_id, 1, NOW);
    configure(&mut f, &change(0, Some(1)), NOW);
    f.connection
        .execute(
            "UPDATE ccp_entry_retention_clocks SET archive_revision=3",
            [],
        )
        .unwrap();
    assert!(cleanup(&mut f, NOW + 100).is_empty());
    assert!(
        f.connection
            .execute(
                "INSERT INTO ccp_entry_retention_purges VALUES(?1,?2,3,1,?3)",
                params![e.entry_id.to_string(), f.workspace.to_string(), NOW + 100]
            )
            .is_err()
    );
    f.connection
        .execute(
            "UPDATE ccp_entry_retention_clocks SET archive_revision=2",
            [],
        )
        .unwrap();
    configure(&mut f, &change(1, None), NOW);
    let invalid = change(i64::MAX.unsigned_abs() - 1, Some(1));
    error(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .change_entry_retention(owner, f.workspace, &invalid, NOW),
        ContentError::InvalidInput,
    );
    f.connection.execute("INSERT INTO ccp_entry_retention_versions(workspace_id,revision,operation_id,retention_seconds,actor_kind,actor_id,principal_id,changed_at) VALUES(?1,?2,?3,1,'admin',?4,?4,?5)",params![f.workspace.to_string(),i64::MAX-1,Uuid::new_v4().to_string(),f.owner.to_string(),NOW]).unwrap();
    f.connection
        .execute(
            "UPDATE ccp_entry_retention_heads SET revision=?1",
            [i64::MAX - 1],
        )
        .unwrap();
    let policy = configure(&mut f, &change(i64::MAX.unsigned_abs() - 1, None), NOW);
    assert_eq!(policy.revision, i64::MAX.unsigned_abs());
    assert_eq!(policy.retention_seconds, None);
    assert!(cleanup(&mut f, i64::MAX).is_empty());
}
