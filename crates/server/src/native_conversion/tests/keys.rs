// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;

#[test]
fn source_keys_are_globally_unique_nonnull_and_canonical_in_all_ten_domains() {
    for table in source::Table::ALL {
        let key = if table == source::Table::Archives {
            "entry_key"
        } else {
            "id"
        };
        for values in ["NULL", "1,1", "'alias'", "x'ff'"] {
            let db = Connection::open_in_memory().unwrap();
            for other in source::Table::ALL {
                let column = if other == source::Table::Archives {
                    "entry_key"
                } else {
                    "id"
                };
                db.execute_batch(&format!("CREATE TABLE {}({column})", other.name()))
                    .unwrap();
            }
            assert!(source::validate_keys(&db).is_ok());
            for value in values.split(',') {
                db.execute_batch(&format!(
                    "INSERT INTO {}({key}) VALUES({value})",
                    table.name()
                ))
                .unwrap();
            }
            // A text key is valid only for archives and must still be unique.
            if table == source::Table::Archives && values == "'alias'" {
                assert!(source::validate_keys(&db).is_ok());
                db.execute_batch("INSERT INTO deleted_message_packs(entry_key) VALUES('alias')")
                    .unwrap();
            }
            assert!(
                source::validate_keys(&db).is_err(),
                "{} {values}",
                table.name()
            );
        }
    }
}

#[test]
fn duplicate_append_ids_across_sessions_refuse_without_publishing_or_touching_source() {
    let f = Fixture::new();
    let db = f.db();
    db.execute_batch("INSERT INTO sessions(id,name) VALUES(8,'other');
        INSERT INTO message_packs(id,session_id,name,context) VALUES(43,8,'other entry','OTHER_BODY');
        CREATE TABLE history_copy AS SELECT * FROM message_history;
        DROP TABLE message_history;
        ALTER TABLE history_copy RENAME TO message_history;
        INSERT INTO message_history(id,message_pack_id,operation_id,client_common_name,appended_content,created_at)
            VALUES(-55,43,'other operation','other client','OTHER_APPEND','other time');").unwrap();
    drop(db);
    let before = f.files();
    error(convert(&f.options()), ConversionError::SourceRefused);
    assert_eq!(f.files(), before);
    assert!(!f.destination.exists());
    assert_eq!(fs::read_dir(&f.root).unwrap().count(), 1);
}

#[test]
fn duplicate_shelf_ids_across_sessions_refuse_before_projecting_wrong_names() {
    let f = Fixture::new();
    let db = f.db();
    db.execute_batch(
        "INSERT INTO sessions(id,name) VALUES(8,'other');
        CREATE TABLE shelves_copy AS SELECT * FROM shelves;
        DROP TABLE shelves;
        ALTER TABLE shelves_copy RENAME TO shelves;
        INSERT INTO shelves(id,session_id,shelf_name,description,created_at,updated_at)
            VALUES(11,8,'other shelf','OTHER_SHELF','other time','other time');",
    )
    .unwrap();
    drop(db);
    let before = f.files();
    error(convert(&f.options()), ConversionError::SourceRefused);
    assert_eq!(f.files(), before);
    assert!(!f.destination.exists());
    assert_eq!(fs::read_dir(&f.root).unwrap().count(), 1);
}
