// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

mod boundaries;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "ccp-native-inventory-test-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir(&directory).unwrap();
        let fixture = Self(directory.clone());
        let db = Connection::open(directory.join("ccp.sqlite3")).unwrap();
        db.pragma_update(None, "journal_mode", "WAL").unwrap();
        db.execute_batch(include_str!("../init-db.sql")).unwrap();
        db.execute("INSERT INTO schema_version(version) VALUES(2)", [])
            .unwrap();
        db.execute_batch("INSERT INTO sessions(id,name,description,owner,labels) VALUES(7,'PRIVATE_NAME','PRIVATE_DESCRIPTION','PRIVATE_OWNER',' raw , labels ');
            INSERT INTO shelves(id,session_id,shelf_name,description) VALUES(11,7,' shelf ','PRIVATE_SHELF');
            INSERT INTO books(id,session_id,shelf_name,book_name,description) VALUES(19,7,' shelf ',' book ','PRIVATE_BOOK');
            INSERT INTO message_packs(id,session_id,name,description,labels,context,shelf_name,book_name,created_at,updated_at) VALUES(42,7,' name ','PRIVATE_META','x,,x','PRIVATE_BODY',' shelf ',' book ','arbitrary old time','1000');
            INSERT INTO message_history(id,message_pack_id,operation_id,client_common_name,agent_name,host_name,reason,appended_content,created_at) VALUES(55,42,'PRIVATE_OPERATION','PRIVATE_CN','PRIVATE_AGENT','PRIVATE_HOST','PRIVATE_REASON','PRIVATE_APPEND','old append time');
            INSERT INTO deleted_message_packs(entry_key,session_id,name,description,labels,context,shelf_name,book_name,shelf_description,book_description,created_at,updated_at,deleted_at,deleted_by_client_common_name) VALUES('old-archive',7,' name ','PRIVATE_ARCHIVE_META',' raw ','PRIVATE_ARCHIVE',' shelf ',' book ','OLD_SHELF','OLD_BOOK','1000','1001','1000000','PRIVATE_DELETER');
            INSERT INTO deleted_message_history(id,entry_key,operation_id,client_common_name,appended_content,created_at) VALUES(66,'old-archive','PRIVATE_OLD_OPERATION','PRIVATE_CN','PRIVATE_OLD_APPEND','old time');
            INSERT INTO auth_tokens(id,session_id,token_nonce,token_value,token_hash,token_prefix,access_level) VALUES(77,7,'PRIVATE_NONCE','PRIVATE_TOKEN','PRIVATE_HASH','PRIVATE_PREFIX','admin');
            INSERT INTO issued_client_certs(id,session_id,common_name,access_level,cert_pem,expires_at,revoked_at) VALUES(88,7,'PRIVATE_CERT_CN','admin','PRIVATE_PEM','past time','revoked time');
            INSERT INTO transfer_log(id,session_id,direction,scope_json,bundle_sha256,outcome,entry_count,created_at,created_by) VALUES(99,7,'import','PRIVATE_SCOPE','PRIVATE_BUNDLE','kept',1,'1000','PRIVATE_IMPORTER');").unwrap();
        drop(db);
        fs::write(directory.join("runtime-journal.jsonl"), b"").unwrap();
        fs::write(
            directory.join("private-identity.pem"),
            b"PRIVATE_KEY_MATERIAL",
        )
        .unwrap();
        fixture
    }
    fn db(&self) -> Connection {
        Connection::open(self.0.join("ccp.sqlite3")).unwrap()
    }
    fn files(&self) -> BTreeMap<String, String> {
        fs::read_dir(&self.0)
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                (
                    entry.file_name().to_str().unwrap().to_owned(),
                    format!("{:x}", Sha256::digest(fs::read(entry.path()).unwrap())),
                )
            })
            .collect()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        #[cfg(unix)]
        fs::set_permissions(&self.0, fs::Permissions::from_mode(0o700)).unwrap();
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn raw_native_inventory_preserves_every_source_byte_and_omits_all_payload_and_identity_labels() {
    let f = Fixture::new();
    let before = f.files();
    assert!(!f.0.join("ccp.sqlite3-wal").exists());
    let report = inspect(&f.0).unwrap();
    assert_eq!(report.source_schema, 2);
    assert_eq!(report.format_version, 1);
    assert_eq!(report.database_sha256, before["ccp.sqlite3"]);
    assert_eq!(
        report.database_bytes,
        fs::metadata(f.0.join("ccp.sqlite3")).unwrap().len()
    );
    assert_eq!(report.data_mode, "native");
    assert!(report.observation_only);
    assert!(!report.conversion_performed);
    assert_eq!(
        report.native,
        NativeCounts {
            sessions: 1,
            shelves: 1,
            books: 1,
            entries: 1,
            archives: 1,
            append_records: 1,
            archived_append_records: 1,
            certificates: 1,
            tokens: 1,
            transfer_records: 1
        }
    );
    assert_eq!(
        report.workspace,
        WorkspaceCounts {
            workspaces: 0,
            principals: 0,
            keys: 0,
            clients: 0,
            grants: 0,
            entries: 0
        }
    );
    let json = serde_json::to_string(&report).unwrap();
    assert!(!json.contains("PRIVATE"));
    assert!(!json.contains("old time"));
    assert!(!json.contains(" shelf "));
    assert_eq!(f.files(), before);
    assert!(!f.0.join("ccp-directory-mode.sqlite3").exists());
    let db = f.db();
    let ids:(i64,i64,String,String)=db.query_row("SELECT p.id,h.id,p.labels,p.created_at FROM message_packs p JOIN message_history h ON h.message_pack_id=p.id",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert_eq!(ids, (42, 55, "x,,x".into(), "arbitrary old time".into()));
    let workspace_tables: i64 = db
        .query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name='ccp_instance'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(workspace_tables, 0);
}

#[test]
fn existing_workspace_metadata_is_counted_separately_without_activation_or_grant_inference() {
    let f = Fixture::new();
    crate::init::init_sqlite(&f.0.join("ccp.sqlite3")).unwrap();
    let mut db = f.db();
    let mut store = crate::foundation::AccessStore::new(&mut db).unwrap();
    let setup = store.bootstrap_instance("operator", "fresh", 1000).unwrap();
    let actor = store
        .authenticate_admin_key(setup.owner_key.secret(), 1000)
        .unwrap();
    let shelf = store
        .create_shelf(
            actor,
            setup.workspace_id,
            &crate::foundation::NodeDraft {
                name: "PRIVATE_NEW".into(),
                description: String::new(),
            },
            1000,
        )
        .unwrap();
    assert!(!shelf.id.is_nil());
    db.execute("UPDATE ccp_runtime_mode SET mode='workspace'", [])
        .unwrap();
    drop(db);
    let before = f.files();
    let report = inspect(&f.0).unwrap();
    assert_eq!(report.source_schema, crate::init::SCHEMA_VERSION);
    assert_eq!(report.workspace.workspaces, 2);
    assert_eq!(report.workspace.principals, 1);
    assert_eq!(report.workspace.keys, 2);
    assert_eq!(report.workspace.clients, 0);
    assert_eq!(report.workspace.grants, 0);
    assert_eq!(report.data_mode, "workspace");
    assert_eq!(report.native.entries, 1);
    assert_eq!(f.files(), before);
}

#[test]
fn private_capture_is_removed_and_a_changed_source_cannot_be_acknowledged() {
    let f = Fixture::new();
    let snapshot = snapshot::Snapshot::capture(&f.0).unwrap();
    let scratch = snapshot.database().parent().unwrap().to_owned();
    assert!(scratch.exists());
    #[cfg(unix)]
    {
        assert_eq!(
            fs::metadata(&scratch).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(snapshot.database())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    f.db()
        .execute(
            "UPDATE message_packs SET context='new source' WHERE id=42",
            [],
        )
        .unwrap();
    assert_eq!(
        snapshot.verify().unwrap_err(),
        InspectionError::SourceChanged
    );
    drop(snapshot);
    assert!(!scratch.exists());
    let snapshot = snapshot::Snapshot::capture(&f.0).unwrap();
    fs::write(f.0.join("runtime-journal.jsonl"), b"pending work").unwrap();
    assert_eq!(
        snapshot.verify().unwrap_err(),
        InspectionError::PendingJournal
    );
    drop(snapshot);
}

#[test]
fn metadata_refusal_never_echoes_database_errors_or_source_values() {
    let f = Fixture::new();
    f.db().execute("DROP TABLE books", []).unwrap();
    let before = f.files();
    let error = inspect(&f.0).unwrap_err();
    assert_eq!(error, InspectionError::InvalidDatabase);
    assert!(!error.to_string().contains("PRIVATE"));
    assert_eq!(f.files(), before);
    let db = f.db();
    db.execute(
        "UPDATE schema_version SET version=?1",
        params![crate::init::SCHEMA_VERSION + 1],
    )
    .unwrap();
    drop(db);
    assert_eq!(
        inspect(&f.0).unwrap_err(),
        InspectionError::UnsupportedSchema
    );
}
