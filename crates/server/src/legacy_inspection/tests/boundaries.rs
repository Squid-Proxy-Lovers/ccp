// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;

#[test]
fn journal_sidecars_bad_formats_and_incomplete_layout_are_refused_without_source_changes() {
    let f = Fixture::new();
    for name in ["ccp.sqlite3-wal", "ccp.sqlite3-shm", "ccp.sqlite3-journal"] {
        fs::write(f.0.join(name), b"").unwrap();
        let before = f.files();
        assert_eq!(inspect(&f.0).unwrap_err(), InspectionError::SidecarsPresent);
        assert_eq!(f.files(), before);
        fs::remove_file(f.0.join(name)).unwrap();
    }
    fs::write(f.0.join("runtime-journal.jsonl"), b"PRIVATE_PENDING_BODY").unwrap();
    let before = f.files();
    assert_eq!(inspect(&f.0).unwrap_err(), InspectionError::PendingJournal);
    assert_eq!(f.files(), before);
    fs::write(f.0.join("runtime-journal.jsonl"), b"").unwrap();
    for version in [3, 4, crate::init::SCHEMA_VERSION + 1] {
        f.db()
            .execute("UPDATE schema_version SET version=?1", [version])
            .unwrap();
        let before = f.files();
        assert_eq!(
            inspect(&f.0).unwrap_err(),
            InspectionError::UnsupportedSchema
        );
        assert_eq!(f.files(), before);
    }
    fs::write(f.0.join("ccp.sqlite3"), b"NOT_SQLITE_PRIVATE_BODY").unwrap();
    let before = f.files();
    assert_eq!(inspect(&f.0).unwrap_err(), InspectionError::InvalidDatabase);
    assert_eq!(f.files(), before);
}

#[test]
fn missing_relative_nonregular_and_oversized_sources_are_not_created_or_read_as_databases() {
    let f = Fixture::new();
    let absent = f.0.join("absent");
    assert_eq!(
        inspect(&absent).unwrap_err(),
        InspectionError::SourceUnavailable
    );
    assert!(!absent.exists());
    assert_eq!(
        inspect(Path::new(".")).unwrap_err(),
        InspectionError::SourceUnavailable
    );
    let db = f.0.join("ccp.sqlite3");
    fs::remove_file(&db).unwrap();
    fs::create_dir(&db).unwrap();
    assert_eq!(
        inspect(&f.0).unwrap_err(),
        InspectionError::SourceUnavailable
    );
    fs::remove_dir(&db).unwrap();
    fs::write(&db, b"SQLite format 3\0").unwrap();
    fs::OpenOptions::new()
        .write(true)
        .open(&db)
        .unwrap()
        .set_len(snapshot::MAX_BYTES + 1)
        .unwrap();
    assert_eq!(inspect(&f.0).unwrap_err(), InspectionError::TooLarge);
    assert_eq!(fs::metadata(db).unwrap().len(), snapshot::MAX_BYTES + 1);
}

#[test]
fn a_new_sidecar_after_capture_is_refused_even_if_source_main_bytes_are_identical() {
    let f = Fixture::new();
    let snapshot = snapshot::Snapshot::capture(&f.0).unwrap();
    fs::write(f.0.join("ccp.sqlite3-wal"), b"new work").unwrap();
    assert_eq!(
        snapshot.verify().unwrap_err(),
        InspectionError::SidecarsPresent
    );
}

#[test]
fn sqlite_integrity_and_foreign_keys_are_verified_on_the_copy() {
    let f = Fixture::new();
    let db = f.db();
    db.pragma_update(None, "foreign_keys", false).unwrap();
    db.execute("INSERT INTO message_history(message_pack_id,operation_id,client_common_name,appended_content) VALUES(9999,'PRIVATE','PRIVATE','PRIVATE')",[]).unwrap();
    drop(db);
    let before = f.files();
    assert_eq!(inspect(&f.0).unwrap_err(), InspectionError::InvalidDatabase);
    assert_eq!(f.files(), before);
}

#[cfg(unix)]
#[test]
fn readonly_source_and_symlink_refusals_do_not_change_source_or_alias_targets() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let db = f.0.join("ccp.sqlite3");
    fs::set_permissions(&db, fs::Permissions::from_mode(0o400)).unwrap();
    fs::set_permissions(&f.0, fs::Permissions::from_mode(0o500)).unwrap();
    let before = f.files();
    assert_eq!(inspect(&f.0).unwrap().native.entries, 1);
    assert_eq!(f.files(), before);
    assert_eq!(
        fs::metadata(&db).unwrap().permissions().mode() & 0o777,
        0o400
    );
    fs::set_permissions(&f.0, fs::Permissions::from_mode(0o700)).unwrap();
    let real = f.0.join("actual.sqlite3");
    fs::rename(&db, &real).unwrap();
    symlink(&real, &db).unwrap();
    assert_eq!(
        inspect(&f.0).unwrap_err(),
        InspectionError::SourceUnavailable
    );
    assert_eq!(
        format!("{:x}", Sha256::digest(fs::read(real).unwrap())),
        before["ccp.sqlite3"]
    );
    let alias = f.0.join("directory-alias");
    symlink(&f.0, &alias).unwrap();
    assert_eq!(
        inspect(&alias).unwrap_err(),
        InspectionError::SourceUnavailable
    );
}

#[test]
fn an_absent_schema_ledger_is_observed_without_recreating_it_but_a_view_cannot_masquerade_as_one() {
    let f = Fixture::new();
    f.db().execute("DROP TABLE schema_version", []).unwrap();
    let before = f.files();
    let report = inspect(&f.0).unwrap();
    assert_eq!(report.source_schema, 0);
    assert_eq!(report.native.entries, 1);
    assert_eq!(f.files(), before);
    f.db()
        .execute("CREATE VIEW schema_version AS SELECT 3 AS version", [])
        .unwrap();
    let before = f.files();
    assert_eq!(inspect(&f.0).unwrap_err(), InspectionError::InvalidDatabase);
    assert_eq!(f.files(), before);
}

#[test]
fn schema_version_refusal_honors_case_insensitive_sqlite_identifiers() {
    let f = Fixture::new();
    f.db()
        .execute_batch("ALTER TABLE schema_version RENAME TO temporary_version; ALTER TABLE temporary_version RENAME TO SCHEMA_VERSION;")
        .unwrap();
    for version in [3, 4, crate::init::SCHEMA_VERSION + 1] {
        f.db()
            .execute("UPDATE SCHEMA_VERSION SET version=?1", [version])
            .unwrap();
        let before = f.files();
        assert_eq!(
            inspect(&f.0).unwrap_err(),
            InspectionError::UnsupportedSchema
        );
        assert_eq!(f.files(), before);
    }
}

#[test]
fn uppercase_views_cannot_hide_as_absent_metadata_or_native_tables() {
    for table in [
        "schema_version",
        "ccp_workspaces",
        "ccp_admin_principals",
        "ccp_admin_keys",
        "ccp_clients",
        "ccp_grants",
        "ccp_content_entries",
        "ccp_runtime_mode",
        "sessions",
    ] {
        let f = Fixture::new();
        let db = f.db();
        if matches!(table, "schema_version" | "sessions") {
            db.execute(&format!("ALTER TABLE {table} RENAME TO hidden_{table}"), [])
                .unwrap();
        }
        db.execute(
            &format!(
                "CREATE VIEW {} AS SELECT 4 AS version",
                table.to_uppercase()
            ),
            [],
        )
        .unwrap();
        drop(db);
        let before = f.files();
        assert_eq!(
            inspect(&f.0).unwrap_err(),
            InspectionError::InvalidDatabase,
            "{table}"
        );
        assert_eq!(f.files(), before);
    }
}

#[test]
fn ambiguous_schema_names_are_refused_instead_of_using_the_first_match() {
    for name in ["schema_version", "SCHEMA_VERSION", "sessions", "CCP_GRANTS"] {
        let f = Fixture::new();
        let db = f.db();
        if name == "CCP_GRANTS" {
            db.execute("CREATE TABLE ccp_grants(id INTEGER)", [])
                .unwrap();
        }
        db.execute(
            &format!("CREATE TRIGGER {name} AFTER INSERT ON message_packs BEGIN SELECT 1; END"),
            [],
        )
        .unwrap();
        drop(db);
        let before = f.files();
        assert_eq!(
            inspect(&f.0).unwrap_err(),
            InspectionError::InvalidDatabase,
            "{name}"
        );
        assert_eq!(f.files(), before);
    }
}

#[test]
fn valid_mixed_case_table_names_keep_native_counts_and_source_bytes() {
    let f = Fixture::new();
    f.db()
        .execute_batch(
            "ALTER TABLE schema_version RENAME TO temporary_version;
        ALTER TABLE temporary_version RENAME TO ScHeMa_VeRsIoN;
        ALTER TABLE shelves RENAME TO temporary_shelves;
        ALTER TABLE temporary_shelves RENAME TO ShElVeS;
        CREATE TABLE CcP_GrAnTs(id INTEGER); INSERT INTO CcP_GrAnTs VALUES(1);",
        )
        .unwrap();
    let before = f.files();
    let report = inspect(&f.0).unwrap();
    assert_eq!(report.source_schema, 2);
    assert_eq!(report.native.shelves, 1);
    assert_eq!(report.native.entries, 1);
    assert_eq!(report.workspace.grants, 1);
    assert_eq!(f.files(), before);
}
