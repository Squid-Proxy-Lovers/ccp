// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

#[test]
fn audit_or_receipt_failure_rolls_back_head_snapshot_and_operation_together() {
    let mut f = fixture();
    let book = book(&mut f);
    let actor = Actor::Admin(f.admin);
    let create = create(book, "entry", "base");
    f.connection.execute_batch("CREATE TRIGGER fail_entry_audit BEFORE INSERT ON ccp_access_audit WHEN NEW.action='create_entry' BEGIN SELECT RAISE(ABORT,'fixture audit fault'); END;").unwrap();
    let result =
        AccessStore::new(&mut f.connection)
            .unwrap()
            .mutate_entry(actor, f.workspace, &create, NOW);
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("fixture audit fault")
    );
    for table in [
        "ccp_content_entries",
        "ccp_content_versions",
        "ccp_content_operations",
    ] {
        assert_eq!(rows(&f.connection, table), 0);
    }
    f.connection
        .execute_batch("DROP TRIGGER fail_entry_audit")
        .unwrap();
    let id = apply(&mut f, actor, &create).entry_id;
    let append = command(EntryChange::Append {
        id,
        expected_revision: 1,
        text: "+once".into(),
    });
    f.connection.execute_batch("CREATE TRIGGER fail_receipt BEFORE INSERT ON ccp_content_operations BEGIN SELECT RAISE(ABORT,'fixture receipt fault'); END;").unwrap();
    let result =
        AccessStore::new(&mut f.connection)
            .unwrap()
            .mutate_entry(actor, f.workspace, &append, NOW);
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("fixture receipt fault")
    );
    assert_eq!(get(&mut f, id).content.text, "base");
    assert_eq!(get(&mut f, id).entry.revision, 1);
    assert_eq!(rows(&f.connection, "ccp_content_versions"), 1);
    assert_eq!(rows(&f.connection, "ccp_content_operations"), 1);
    f.connection
        .execute_batch("DROP TRIGGER fail_receipt")
        .unwrap();
    apply(&mut f, actor, &append);
    assert_eq!(get(&mut f, id).content.text, "base+once");
    assert!(
        f.connection
            .execute("UPDATE ccp_content_versions SET content_json='{}'", [])
            .unwrap_err()
            .to_string()
            .contains("immutable")
    );
    assert!(
        f.connection
            .execute("UPDATE ccp_content_entries SET owner_client_id=NULL", [])
            .unwrap_err()
            .to_string()
            .contains("immutable")
    );
    let bad = Uuid::new_v4().to_string();
    assert!(f.connection.execute("INSERT INTO ccp_content_entries(id,workspace_id,book_id,name,revision,archived,created_at,updated_at) VALUES (?1,?2,?3,'foreign',1,0,0,0)",params![bad,f.other_workspace.to_string(),book.to_string()]).is_err());
    assert_eq!(rows(&f.connection, "ccp_content_entries"), 1);
}

#[test]
fn independent_database_writer_cannot_erase_an_append_from_a_stale_read() {
    let path = std::env::temp_dir().join(format!("ccp-entry-race-{}.sqlite3", Uuid::new_v4()));
    crate::init::init_sqlite(&path).unwrap();
    let mut f = fixture_with_connection(Connection::open(&path).unwrap());
    let book = book(&mut f);
    let actor = Actor::Admin(f.admin);
    let id = apply(&mut f, actor, &create(book, "entry", "base")).entry_id;
    let mut second_db = Connection::open(&path).unwrap();
    let mut second = AccessStore::new(&mut second_db).unwrap();
    let stale = second.get_entry(actor, f.workspace, id, NOW).unwrap();
    assert_eq!(stale.entry.revision, 1);
    apply(
        &mut f,
        actor,
        &command(EntryChange::Append {
            id,
            expected_revision: 1,
            text: "+committed".into(),
        }),
    );
    let revision = command(EntryChange::Revise {
        id,
        expected_revision: stale.entry.revision,
        content: content("stale replacement"),
    });
    error(
        second.mutate_entry(actor, f.workspace, &revision, NOW),
        ContentError::Conflict,
    );
    assert_eq!(
        second
            .get_entry(actor, f.workspace, id, NOW)
            .unwrap()
            .content
            .text,
        "base+committed"
    );
    assert_eq!(rows(&f.connection, "ccp_content_versions"), 2);
    drop(second_db);
    drop(f.connection);
    fs::remove_file(path).unwrap();
}

#[test]
fn scoped_recovery_retains_immutable_client_ownership_and_ends_old_binding_authority() {
    use super::super::redemption::{authority, csr_request};
    let mut f = fixture();
    let owner = enroll(&mut f, DataRole::ReadWrite);
    let book = book(&mut f);
    let actor = Actor::Credential(owner.credential_id);
    let original = create(book, "entry", "owned before recovery");
    let id = apply(&mut f, actor, &original).entry_id;
    let clock = 1_700_000_000;
    let (ca, _) = authority();
    let csr = csr_request(&rcgen::KeyPair::generate().unwrap());
    let mut store = AccessStore::new(&mut f.connection).unwrap();
    let invitation = store
        .issue_recovery_invitation(
            Actor::Admin(f.admin),
            RecoveryInvitationRequest {
                client_id: owner.client_id,
                scope: RecoveryScope::Workspace(f.workspace),
                expires_at: clock + 3600,
            },
            clock,
        )
        .unwrap();
    let recovered = store
        .redeem_recovery_invitation(
            invitation.secret(),
            &RecoveryRedemptionRequest {
                request_id: csr.request_id,
                csr_pem: csr.csr_pem,
            },
            &ca,
            clock,
        )
        .unwrap();
    assert_eq!(recovered.client_id, owner.client_id);
    assert_ne!(recovered.credential_id, owner.credential_id);
    let new = Actor::Credential(recovered.credential_id);
    error(
        store.mutate_entry(actor, f.workspace, &original, clock),
        ContentError::Forbidden,
    );
    error(
        store.mutate_entry(new, f.workspace, &original, clock),
        ContentError::Conflict,
    );
    store
        .mutate_entry(
            new,
            f.workspace,
            &command(EntryChange::Revise {
                id,
                expected_revision: 1,
                content: content("after recovery"),
            }),
            clock,
        )
        .unwrap();
    let snapshot = store.get_entry(new, f.workspace, id, clock).unwrap();
    assert_eq!(snapshot.entry.owner_client_id, Some(owner.client_id));
    assert_eq!(snapshot.revision.client_id, Some(owner.client_id));
    assert_eq!(snapshot.revision.actor, new);
    assert_eq!(snapshot.content.text, "after recovery");
    assert_eq!(
        store
            .entry_version(new, f.workspace, id, 1, clock)
            .unwrap()
            .revision
            .actor,
        actor
    );
}

#[test]
fn schema15_upgrade_preserves_hierarchy_identity_and_entry_history_survives_reopen() {
    let path = std::env::temp_dir().join(format!("ccp-entry-upgrade-{}.sqlite3", Uuid::new_v4()));
    crate::init::init_sqlite(&path).unwrap();
    let mut f = fixture_with_connection(Connection::open(&path).unwrap());
    let owner = enroll(&mut f, DataRole::ReadWrite);
    let book = book(&mut f);
    let workspace = f.workspace;
    let actor = Actor::Credential(owner.credential_id);
    let instance = AccessStore::new(&mut f.connection)
        .unwrap()
        .instance_id()
        .unwrap();
    f.connection.execute_batch("DROP TABLE ccp_content_operations; DROP TABLE ccp_content_versions; DROP TABLE ccp_content_entries; DELETE FROM schema_version WHERE version>15; INSERT OR IGNORE INTO schema_version(version) VALUES(15); INSERT INTO sessions(id,name) VALUES(99,'old'); INSERT INTO message_packs(session_id,name,context) VALUES(99,'entry','retained native');").unwrap();
    drop(f.connection);
    crate::init::init_sqlite(&path).unwrap();
    let mut db = Connection::open(&path).unwrap();
    let mut store = AccessStore::new(&mut db).unwrap();
    assert_eq!(store.instance_id().unwrap(), instance);
    assert_eq!(
        store.get_book(actor, workspace, book, NOW).unwrap().id,
        book
    );
    let create = create(book, "entry", "persistent");
    let initial = store.mutate_entry(actor, workspace, &create, NOW).unwrap();
    let append = command(EntryChange::Append {
        id: initial.entry_id,
        expected_revision: 1,
        text: "+history".into(),
    });
    let receipt = store.mutate_entry(actor, workspace, &append, NOW).unwrap();
    drop(db);
    crate::init::init_sqlite(&path).unwrap();
    let mut db = Connection::open(&path).unwrap();
    let mut store = AccessStore::new(&mut db).unwrap();
    assert_eq!(
        store.mutate_entry(actor, workspace, &append, NOW).unwrap(),
        receipt
    );
    assert_eq!(
        store
            .get_entry(actor, workspace, receipt.entry_id, NOW)
            .unwrap()
            .content
            .text,
        "persistent+history"
    );
    assert_eq!(
        store
            .entry_version(actor, workspace, receipt.entry_id, 1, NOW)
            .unwrap()
            .content
            .text,
        "persistent"
    );
    let version: u32 = db
        .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, crate::init::SCHEMA_VERSION);
    let text: String = db
        .query_row(
            "SELECT context FROM message_packs WHERE session_id=99",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(text, "retained native");
    let violations: u64 = db
        .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(violations, 0);
    drop(db);
    fs::remove_file(path).unwrap();
}
