// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;
use std::path::PathBuf;
const OLD: &str = include_str!("schema26.sql");
fn replace_with_old(f: &mut Fixture) {
    f.connection
        .pragma_update(None, "foreign_keys", false)
        .unwrap();
    let extras=f.connection.prepare("SELECT sql FROM sqlite_schema WHERE tbl_name='ccp_invitations' AND type IN ('index','trigger') AND sql IS NOT NULL ORDER BY type,name").unwrap()
        .query_map([],|r|r.get::<_,String>(0)).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
    let tx = f.connection.transaction().unwrap();
    tx.execute_batch(&OLD.replace(
        "CREATE TABLE IF NOT EXISTS ccp_invitations",
        "CREATE TABLE schema26_invitations",
    ))
    .unwrap();
    tx.execute_batch("INSERT INTO schema26_invitations SELECT * FROM ccp_invitations; DROP TABLE ccp_invitations; ALTER TABLE schema26_invitations RENAME TO ccp_invitations;").unwrap();
    for sql in extras {
        tx.execute_batch(&sql).unwrap();
    }
    tx.execute_batch("CREATE TABLE IF NOT EXISTS schema_version(version INTEGER NOT NULL,applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP); DELETE FROM schema_version;INSERT INTO schema_version(version) VALUES(26);").unwrap();
    tx.commit().unwrap();
    f.connection
        .pragma_update(None, "foreign_keys", true)
        .unwrap();
}
fn path() -> PathBuf {
    std::env::temp_dir().join(format!("ccp-link-migration-{}.sqlite3", Uuid::new_v4()))
}
fn saved(f: &Fixture, path: &std::path::Path) {
    f.connection
        .execute("VACUUM INTO ?1", [path.to_str().unwrap()])
        .unwrap();
}
fn schema(db: &Connection) -> String {
    db.query_row(
        "SELECT sql FROM sqlite_schema WHERE name='ccp_invitations'",
        [],
        |r| r.get(0),
    )
    .unwrap()
}
fn sql_objects(db: &Connection) -> Vec<(String, String)> {
    db.prepare("SELECT name,sql FROM sqlite_schema WHERE tbl_name='ccp_invitations' AND type IN('index','trigger') AND sql IS NOT NULL ORDER BY name").unwrap()
        .query_map([],|r|Ok((r.get(0)?,r.get(1)?))).unwrap().collect::<rusqlite::Result<_>>().unwrap()
}
#[test]
fn schema26_upgrade_preserves_receipts_hashes_fk_indices_triggers_and_exact_retry() {
    let mut f = fixture();
    let (ca, _) = authority();
    let proof = csr_request(&KeyPair::generate().unwrap());
    let invite = AccessStore::new(&mut f.connection)
        .unwrap()
        .issue_individual_invitation(
            Actor::Admin(f.admin),
            IndividualInvitationRequest {
                workspace_id: f.workspace,
                role: DataRole::ReadWrite,
                expires_at: CLOCK + 3600,
            },
            CLOCK,
        )
        .unwrap();
    let receipt = redeem(&mut f, &invite, &proof, &ca, CLOCK).unwrap();
    f.connection.execute_batch("CREATE INDEX custom_invitation_expiry ON ccp_invitations(expires_at);
        CREATE TRIGGER keep_invitation_issuer BEFORE UPDATE OF issuer_id ON ccp_invitations BEGIN SELECT RAISE(ABORT,'issuer is immutable');END;").unwrap();
    replace_with_old(&mut f);
    let objects = sql_objects(&f.connection);
    assert!(schema(&f.connection).contains("AND expires_at IS NOT NULL"));
    let hash: Vec<u8> = f
        .connection
        .query_row(
            "SELECT token_hash FROM ccp_invitations WHERE id=?1",
            [invite.id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    let counts = [
        "ccp_clients",
        "ccp_credentials",
        "ccp_grants",
        "ccp_invitation_receipts",
        "ccp_initial_enrollment",
        "ccp_access_audit",
    ]
    .map(|t| rows(&f.connection, t));
    let path = path();
    saved(&f, &path);
    drop(f.connection);
    crate::init::init_sqlite(&path).unwrap();
    f.connection = Connection::open(&path).unwrap();
    assert_eq!(sql_objects(&f.connection), objects);
    for (table, count) in [
        "ccp_clients",
        "ccp_credentials",
        "ccp_grants",
        "ccp_invitation_receipts",
        "ccp_initial_enrollment",
        "ccp_access_audit",
    ]
    .iter()
    .zip(counts)
    {
        assert_eq!(rows(&f.connection, table), count);
    }
    assert_eq!(
        f.connection
            .query_row(
                "SELECT token_hash FROM ccp_invitations WHERE id=?1",
                [invite.id.to_string()],
                |r| r.get::<_, Vec<u8>>(0)
            )
            .unwrap(),
        hash
    );
    assert_eq!(
        f.connection
            .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |r| r
                .get::<_, i64>(
                0
            ))
            .unwrap(),
        0
    );
    assert_eq!(
        redeem(&mut f, &invite, &proof, &ca, CLOCK + 3601).unwrap(),
        receipt
    );
    let mut r = request(f.workspace);
    r.lifetime = LinkLifetime::Never { confirmed: true };
    assert!(issue(&mut f, r).is_ok());
    assert!(
        f.connection
            .execute("UPDATE ccp_invitations SET issuer_id=issuer_id", [])
            .is_err()
    );
    assert_eq!(
        f.connection
            .query_row("SELECT MAX(version) FROM schema_version", [], |r| r
                .get::<_, u32>(0))
            .unwrap(),
        27
    );
    drop(f.connection);
    fs::remove_file(path).unwrap();
}
#[test]
fn failed_schema_marker_rolls_back_rebuild_and_preserves_old_constraint() {
    let mut f = fixture();
    let workspace = f.workspace;
    let invite = issue(&mut f, request(workspace)).unwrap();
    replace_with_old(&mut f);
    let original = schema(&f.connection);
    let objects = sql_objects(&f.connection);
    let audit = rows(&f.connection, "ccp_access_audit");
    f.connection.execute_batch("CREATE TRIGGER fail_schema27 BEFORE INSERT ON schema_version WHEN NEW.version=27 BEGIN SELECT RAISE(ABORT,'injected schema failure');END;").unwrap();
    let path = path();
    saved(&f, &path);
    drop(f.connection);
    assert!(crate::init::init_sqlite(&path).is_err());
    f.connection = Connection::open(&path).unwrap();
    assert_eq!(schema(&f.connection), original);
    assert_eq!(sql_objects(&f.connection), objects);
    assert_eq!(rows(&f.connection, "ccp_access_audit"), audit);
    assert_eq!(
        f.connection
            .query_row("SELECT MAX(version) FROM schema_version", [], |r| r
                .get::<_, u32>(0))
            .unwrap(),
        26
    );
    assert!(
        f.connection
            .execute(
                "UPDATE ccp_invitations SET expires_at=NULL WHERE id=?1",
                [invite.id.to_string()]
            )
            .is_err()
    );
    f.connection
        .execute_batch("DROP TRIGGER fail_schema27")
        .unwrap();
    drop(f.connection);
    crate::init::init_sqlite(&path).unwrap();
    let db = Connection::open(&path).unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |r| r
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        0
    );
    drop(db);
    fs::remove_file(path).unwrap();
}
#[test]
fn unrecognized_table_definition_is_refused_without_advancing_version() {
    let mut f = fixture();
    replace_with_old(&mut f);
    f.connection
        .execute_batch("ALTER TABLE ccp_invitations ADD COLUMN operator_data TEXT")
        .unwrap();
    let original = schema(&f.connection);
    let path = path();
    saved(&f, &path);
    drop(f.connection);
    assert!(crate::init::init_sqlite(&path).is_err());
    let db = Connection::open(&path).unwrap();
    assert_eq!(schema(&db), original);
    assert_eq!(
        db.query_row("SELECT MAX(version) FROM schema_version", [], |r| r
            .get::<_, u32>(0))
            .unwrap(),
        26
    );
    drop(db);
    fs::remove_file(path).unwrap();
}

#[test]
fn different_quoted_check_literals_are_not_normalized_into_a_known_definition() {
    let mut f = fixture();
    replace_with_old(&mut f);
    f.connection
        .pragma_update(None, "foreign_keys", false)
        .unwrap();
    f.connection
        .execute_batch("DROP TABLE ccp_invitations")
        .unwrap();
    f.connection
        .execute_batch(&OLD.replace("kind != 'individual'", "kind != 'individual '"))
        .unwrap();
    f.connection
        .pragma_update(None, "foreign_keys", true)
        .unwrap();
    let original = schema(&f.connection);
    let path = path();
    saved(&f, &path);
    drop(f.connection);
    assert!(crate::init::init_sqlite(&path).is_err());
    let db = Connection::open(&path).unwrap();
    assert_eq!(schema(&db), original);
    assert_eq!(
        db.query_row("SELECT MAX(version) FROM schema_version", [], |r| r
            .get::<_, u32>(0))
            .unwrap(),
        26
    );
    drop(db);
    fs::remove_file(path).unwrap();
}

#[test]
fn orphan_invitation_refuses_upgrade_and_rolls_back_version_and_definition() {
    let mut f = fixture();
    let workspace = f.workspace;
    let invite = issue(&mut f, request(workspace)).unwrap();
    replace_with_old(&mut f);
    f.connection
        .pragma_update(None, "foreign_keys", false)
        .unwrap();
    let orphan = Uuid::new_v4().to_string();
    f.connection
        .execute(
            "UPDATE ccp_invitations SET workspace_id=?1 WHERE id=?2",
            params![orphan, invite.id.to_string()],
        )
        .unwrap();
    let original = schema(&f.connection);
    let path = path();
    saved(&f, &path);
    drop(f.connection);
    assert!(crate::init::init_sqlite(&path).is_err());
    let db = Connection::open(&path).unwrap();
    assert_eq!(schema(&db), original);
    assert_eq!(
        db.query_row("SELECT MAX(version) FROM schema_version", [], |r| r
            .get::<_, u32>(0))
            .unwrap(),
        26
    );
    assert_eq!(
        db.query_row(
            "SELECT workspace_id FROM ccp_invitations WHERE id=?1",
            [invite.id.to_string()],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        orphan
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE name='ccp_invitations_upgrade'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    drop(db);
    fs::remove_file(path).unwrap();
}
#[test]
fn old_table_rebuild_refuses_a_transaction_with_foreign_keys_still_enabled() {
    let mut f = fixture();
    replace_with_old(&mut f);
    let original = schema(&f.connection);
    let tx = f.connection.transaction().unwrap();
    let error = crate::foundation::invitation_migration::migrate(&tx).unwrap_err();
    assert!(error.to_string().contains("private initialization"));
    tx.rollback().unwrap();
    assert_eq!(schema(&f.connection), original);
    assert!(
        f.connection
            .query_row("PRAGMA foreign_keys", [], |r| r.get::<_, bool>(0))
            .unwrap()
    );
}
