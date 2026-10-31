// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

#[test]
fn guidance_save_faults_roll_back_revision_live_pointer_receipt_and_audit_together() {
    let mut f = fixture();
    let actor = Actor::Admin(f.global);
    let scope = GuidanceScope::Global;
    save(&mut f, actor, scope, &request(0, "before"));
    let next = request(1, "after");
    let before = rows(&f.connection, "ccp_access_audit");
    for sql in [
        "CREATE TRIGGER guidance_fault BEFORE INSERT ON ccp_guidance_versions BEGIN SELECT RAISE(ABORT,'version fault'); END;",
        "CREATE TRIGGER guidance_fault BEFORE UPDATE ON ccp_guidance_heads BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER guidance_fault BEFORE INSERT ON ccp_guidance_operations BEGIN SELECT RAISE(ABORT,'receipt fault'); END;",
        "CREATE TRIGGER guidance_fault BEFORE INSERT ON ccp_access_audit WHEN NEW.action='save_global_guidance' BEGIN SELECT RAISE(ABORT,'audit fault'); END;",
    ] {
        f.connection.execute_batch(sql).unwrap();
        assert!(
            AccessStore::new(&mut f.connection)
                .unwrap()
                .save_guidance(actor, scope, &next, NOW)
                .is_err()
        );
        let current = read(&mut f, actor, None).global;
        assert_eq!(current.text, "before");
        assert_eq!(current.revision, 1);
        assert_eq!(rows(&f.connection, "ccp_guidance_versions"), 1);
        assert_eq!(rows(&f.connection, "ccp_guidance_operations"), 1);
        assert_eq!(rows(&f.connection, "ccp_access_audit"), before);
        f.connection
            .execute_batch("DROP TRIGGER guidance_fault")
            .unwrap();
    }
    assert_eq!(save(&mut f, actor, scope, &next).revision, 2);
}

#[test]
fn guidance_bounds_preserve_exact_text_empty_published_state_and_versioned_hash() {
    let mut f = fixture();
    let actor = Actor::Admin(f.global);
    let scope = GuidanceScope::Global;
    let original = request(0, "first\0é\n");
    save(&mut f, actor, scope, &original);
    let hash: String = f
        .connection
        .query_row(
            "SELECT lower(hex(request_hash)) FROM ccp_guidance_operations WHERE board_key='global'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        hash,
        "022da3e812d7396d7ad1c5085004fc9d38ce99ba29a842404edd452a82db2579"
    );
    let text = "\0".repeat(GUIDANCE_TEXT_BYTES);
    save(&mut f, actor, scope, &request(1, &text));
    assert_eq!(read(&mut f, actor, None).global.text, text);
    let owner = Actor::Admin(f.owner);
    let workspace = GuidanceScope::Workspace(f.workspace);
    let unicode = "é".repeat(GUIDANCE_TEXT_BYTES / 2);
    save(&mut f, owner, workspace, &request(0, &unicode));
    assert_eq!(
        read(&mut f, owner, workspace.workspace())
            .workspace
            .unwrap()
            .text,
        unicode
    );
    for invalid in [
        request(2, &"x".repeat(GUIDANCE_TEXT_BYTES + 1)),
        request(2, &"é".repeat(GUIDANCE_TEXT_BYTES / 2 + 1)),
        request(i64::MAX.unsigned_abs(), "x"),
        request(u64::MAX, "x"),
        GuidanceSave {
            operation_id: Uuid::nil(),
            expected_revision: 2,
            text: "x".into(),
        },
    ] {
        denied(&mut f, actor, scope, &invalid, ContentError::InvalidInput);
    }
    assert_eq!(read(&mut f, actor, None).global.revision, 2);
    save(&mut f, actor, scope, &request(2, ""));
    let empty = read(&mut f, actor, None).global;
    assert_eq!(empty.revision, 3);
    assert!(empty.text.is_empty());
    assert!(empty.publisher.is_some());
}

#[test]
fn guidance_proofs_and_history_are_immutable_and_missing_live_payload_is_not_false_empty() {
    let mut f = fixture();
    let actor = Actor::Admin(f.global);
    save(
        &mut f,
        actor,
        GuidanceScope::Global,
        &request(0, "PRIVATE_GUIDANCE"),
    );
    for sql in [
        "UPDATE ccp_guidance_versions SET text='changed'",
        "UPDATE ccp_guidance_operations SET revision=2",
        "DELETE FROM ccp_guidance_operations",
    ] {
        assert!(f.connection.execute(sql, []).is_err());
    }
    assert!(f.connection.execute("INSERT INTO ccp_guidance_operations(board_key,operation_id,publisher_kind,publisher_id,expected_revision,request_hash,revision) VALUES('global',?1,'admin',?2,1,?3,2)",params![Uuid::new_v4().to_string(),f.global.to_string(),[0_u8;32].as_slice()]).is_err());
    assert!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .guidance(actor, None, -1)
            .is_err()
    );
    let client = enroll(&mut f, DataRole::Read);
    assert!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .guidance(Actor::Credential(client.credential_id), None, EXPIRES + 1)
            .is_err()
    );
    f.connection
        .pragma_update(None, "foreign_keys", false)
        .unwrap();
    f.connection
        .execute(
            "DELETE FROM ccp_guidance_versions WHERE board_key='global'",
            [],
        )
        .unwrap();
    f.connection
        .pragma_update(None, "foreign_keys", true)
        .unwrap();
    let error = AccessStore::new(&mut f.connection)
        .unwrap()
        .guidance(actor, None, NOW)
        .unwrap_err()
        .to_string();
    assert!(error.contains("unavailable"));
    assert!(!error.contains("PRIVATE_GUIDANCE"));
}

#[test]
fn guidance_survives_reopen_and_independent_stale_publishers_cannot_overwrite() {
    let path = std::env::temp_dir().join(format!("ccp-guidance-{}.sqlite3", Uuid::new_v4()));
    crate::init::init_sqlite(&path).unwrap();
    let mut f = fixture_with_connection(Connection::open(&path).unwrap());
    let global = Actor::Admin(f.global);
    let owner = Actor::Admin(f.owner);
    let scope = GuidanceScope::Workspace(f.workspace);
    let first = request(0, "global");
    save(&mut f, global, GuidanceScope::Global, &first);
    save(&mut f, owner, scope, &request(0, "workspace"));
    let before = read(&mut f, owner, scope.workspace());
    drop(f.connection);
    crate::init::init_sqlite(&path).unwrap();
    f.connection = Connection::open(&path).unwrap();
    assert_eq!(read(&mut f, owner, scope.workspace()), before);
    assert_eq!(
        save(&mut f, global, GuidanceScope::Global, &first).revision,
        1
    );
    let mut second = Connection::open(&path).unwrap();
    let stale = AccessStore::new(&mut second)
        .unwrap()
        .guidance(owner, scope.workspace(), NOW)
        .unwrap()
        .workspace
        .unwrap()
        .revision;
    save(&mut f, owner, scope, &request(1, "new workspace"));
    let error = AccessStore::new(&mut second)
        .unwrap()
        .save_guidance(owner, scope, &request(stale, "stale"), NOW)
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<ContentError>(),
        Some(&ContentError::Conflict)
    );
    save(
        &mut f,
        global,
        GuidanceScope::Global,
        &request(1, "new global"),
    );
    let bundle = read(&mut f, owner, scope.workspace());
    assert_eq!(bundle.global.revision, 2);
    assert_eq!(bundle.workspace.unwrap().revision, 2);
    assert_eq!(rows(&f.connection, "ccp_guidance_versions"), 4);
    assert_eq!(rows(&f.connection, "ccp_guidance_operations"), 4);
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
    drop(second);
    drop(f.connection);
    fs::remove_file(path).unwrap();
}

#[test]
fn failed_schema_record_rolls_back_new_guidance_tables_and_preserves_schema19_data() {
    let path =
        std::env::temp_dir().join(format!("ccp-guidance-upgrade-{}.sqlite3", Uuid::new_v4()));
    crate::init::init_sqlite(&path).unwrap();
    let mut f = fixture_with_connection(Connection::open(&path).unwrap());
    let name: String = f
        .connection
        .query_row(
            "SELECT name FROM ccp_workspaces WHERE id=?1",
            [f.workspace.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    f.connection
        .execute_batch(
            "DROP TRIGGER IF EXISTS ccp_no_incomplete_workspace_deletion; DROP TABLE ccp_guidance_operations;
        DROP TABLE ccp_guidance_heads; DROP TABLE ccp_guidance_versions;
        DELETE FROM schema_version WHERE version>19;
        INSERT OR IGNORE INTO schema_version(version) VALUES(19);",
        )
        .unwrap();
    f.connection.execute_batch(&format!("CREATE TRIGGER guidance_upgrade_fault BEFORE INSERT ON schema_version WHEN NEW.version={} BEGIN SELECT RAISE(ABORT,'schema fault'); END;",crate::init::SCHEMA_VERSION)).unwrap();
    drop(f.connection);
    assert!(crate::init::init_sqlite(&path).is_err());
    f.connection = Connection::open(&path).unwrap();
    assert_eq!(
        f.connection
            .query_row("SELECT MAX(version) FROM schema_version", [], |r| r
                .get::<_, u32>(0))
            .unwrap(),
        19
    );
    assert_eq!(f.connection.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('ccp_guidance_versions','ccp_guidance_heads','ccp_guidance_operations')",[],|r|r.get::<_,u32>(0)).unwrap(),0);
    assert_eq!(
        f.connection
            .query_row(
                "SELECT name FROM ccp_workspaces WHERE id=?1",
                [f.workspace.to_string()],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        name
    );
    f.connection
        .execute_batch("DROP TRIGGER guidance_upgrade_fault")
        .unwrap();
    drop(f.connection);
    crate::init::init_sqlite(&path).unwrap();
    f.connection = Connection::open(&path).unwrap();
    let global = Actor::Admin(f.global);
    let bundle = read(&mut f, global, None);
    assert_eq!(bundle.global.revision, 0);
    assert!(bundle.global.publisher.is_none());
    assert_eq!(
        save(
            &mut f,
            global,
            GuidanceScope::Global,
            &request(0, "after retry")
        )
        .revision,
        1
    );
    drop(f.connection);
    fs::remove_file(path).unwrap();
}
