// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;

fn collated_source(collation: &str, other_key: &str, orphan: bool) -> Fixture {
    let f = Fixture::new();
    let db = f.db();
    db.execute_batch("INSERT INTO sessions(id,name) VALUES(8,'other')")
        .unwrap();
    db.execute("INSERT INTO deleted_message_packs(entry_key,session_id,name,context,created_at,updated_at,deleted_by_client_common_name) VALUES(?1,8,'other archive','OTHER_ARCHIVE','old','old','other')",[other_key]).unwrap();
    db.execute_batch(&format!("CREATE TABLE history_copy AS SELECT * FROM deleted_message_history;
        DROP TABLE deleted_message_history;
        CREATE TABLE deleted_message_history(id INTEGER PRIMARY KEY, entry_key TEXT NOT NULL COLLATE {collation}, operation_id TEXT NOT NULL, client_common_name TEXT NOT NULL, agent_name TEXT, host_name TEXT, reason TEXT, appended_content TEXT NOT NULL, created_at TEXT NOT NULL);
        INSERT INTO deleted_message_history SELECT * FROM history_copy;
        DROP TABLE history_copy;")).unwrap();
    if orphan {
        db.execute_batch("INSERT INTO deleted_message_history(id,entry_key,operation_id,client_common_name,appended_content,created_at) VALUES(67,'orphan-key','other operation','other client','ORPHAN_APPEND','old')").unwrap();
    }
    drop(db);
    f
}

#[test]
fn collated_child_filters_with_orphans_cannot_balance_duplicate_cross_session_visits() {
    for (collation, other) in [("NOCASE", "ARCHIVED-KEY"), ("RTRIM", "archived-key ")] {
        let f = collated_source(collation, other, true);
        let before = f.files();
        error(convert(&f.options()), ConversionError::SourceRefused);
        assert_eq!(f.files(), before);
        assert!(!f.destination.exists());
        assert_eq!(fs::read_dir(&f.root).unwrap().count(), 1);
    }
}

#[test]
fn collated_child_filters_without_orphans_preserve_exact_parent_assignment() {
    for (collation, other) in [("NOCASE", "ARCHIVED-KEY"), ("RTRIM", "archived-key ")] {
        let f = collated_source(collation, other, false);
        let before = f.files();
        let result = convert(&f.options()).unwrap();
        assert_eq!(f.files(), before);
        let db = Connection::open(f.destination.join("ccp.sqlite3")).unwrap();
        let original = result
            .workspaces
            .iter()
            .find(|w| w.name == "native")
            .unwrap();
        let other = result
            .workspaces
            .iter()
            .find(|w| w.name == "other")
            .unwrap();
        assert_eq!(db.query_row("SELECT count(*) FROM ccp_native_entry_rows WHERE workspace_id=?1 AND source_table='deleted_message_history'",[original.id.to_string()],|r|r.get::<_,u64>(0)).unwrap(),1);
        assert_eq!(db.query_row("SELECT count(*) FROM ccp_native_entry_rows WHERE workspace_id=?1 AND source_table='deleted_message_history'",[other.id.to_string()],|r|r.get::<_,u64>(0)).unwrap(),0);
        assert_eq!(db.query_row("SELECT count(*) FROM ccp_native_entry_rows WHERE source_table='deleted_message_history'",[],|r|r.get::<_,u64>(0)).unwrap(),1);
    }
}

#[test]
fn all_child_domains_require_canonical_parent_types_and_existing_binary_parent() {
    let relations = [
        ("shelves", "session_id", "7"),
        ("books", "session_id", "7"),
        ("message_packs", "session_id", "7"),
        ("deleted_message_packs", "session_id", "7"),
        ("issued_client_certs", "session_id", "7"),
        ("auth_tokens", "session_id", "7"),
        ("transfer_log", "session_id", "7"),
        ("message_history", "message_pack_id", "42"),
        ("deleted_message_history", "entry_key", "'archived-key'"),
    ];
    for (table, column, valid) in relations {
        for bad in ["NULL", "x'ff'", "'missing'", "999999", "7.5"] {
            let f = Fixture::new();
            let db = f.db();
            assert!(source::validate_relationships(&db).is_ok());
            // Rebuild without affinity/FK constraints to exercise application validation itself.
            let fields = db
                .prepare(&format!("PRAGMA table_info({table})"))
                .unwrap()
                .query_map([], |r| r.get::<_, String>(1))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            let names = fields
                .iter()
                .map(|n| format!("\"{n}\""))
                .collect::<Vec<_>>()
                .join(",");
            db.execute_batch(&format!("CREATE TABLE relation_copy({names}); INSERT INTO relation_copy SELECT * FROM {table}; DROP TABLE {table}; ALTER TABLE relation_copy RENAME TO {table}; UPDATE {table} SET {column}={bad}")).unwrap();
            assert!(
                source::validate_relationships(&db).is_err(),
                "{table} {bad}"
            );
            db.execute_batch(&format!("UPDATE {table} SET {column}={valid}"))
                .unwrap();
            assert!(source::validate_relationships(&db).is_ok(), "{table}");
        }
    }
}
