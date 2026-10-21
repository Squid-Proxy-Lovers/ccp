// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;
use crate::foundation::{
    AccessStore, EntryChange, EntryMutation, EntryPurge, NativeHistoryRequest, NativeRow,
    NativeSqlValue, WorkspaceDeletionRequest,
};
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};

mod boundaries;
mod deletion;
mod keys;
mod publication;
mod relationships;

struct Fixture {
    root: PathBuf,
    source: PathBuf,
    destination: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("ccp-native-conversion-test-{}", Uuid::new_v4()));
        let source = root.join("source");
        let destination = root.join("converted");
        fs::create_dir_all(&source).unwrap();
        let db = Connection::open(source.join("ccp.sqlite3")).unwrap();
        db.execute_batch(include_str!("../init-db.sql")).unwrap();
        db.execute_batch("INSERT INTO schema_version(version) VALUES(2);
            INSERT INTO sessions(id,name,description,owner,labels,purpose) VALUES(7,'native','SOURCE_SESSION_META','untrusted owner',' raw , labels ','SOURCE_PURPOSE');
            INSERT INTO shelves(id,session_id,shelf_name,description) VALUES(11,7,'notes','SOURCE_SHELF');
            INSERT INTO shelves(id,session_id,shelf_name,description) VALUES(12,7,'empty','SOURCE_EMPTY_SHELF');
            INSERT INTO books(id,session_id,shelf_name,book_name,description) VALUES(19,7,'notes','records','SOURCE_BOOK');
            INSERT INTO books(id,session_id,shelf_name,book_name,description) VALUES(20,7,'empty','empty','SOURCE_EMPTY_BOOK');
            INSERT INTO message_packs(id,session_id,name,description,labels,context,shelf_name,book_name,created_at,updated_at) VALUES(42,7,'entry','SOURCE_META','alpha,beta','SOURCE_BODYAPPEND','notes','records','arbitrary creation time','1000');
            INSERT INTO message_history(id,message_pack_id,operation_id,client_common_name,agent_name,host_name,reason,appended_content,created_at) VALUES(-55,42,'SOURCE_OPERATION','SOURCE_CN','SOURCE_AGENT','SOURCE_HOST','SOURCE_REASON','APPEND','unparseable time');
            INSERT INTO deleted_message_packs(entry_key,session_id,name,description,labels,context,shelf_name,book_name,shelf_description,book_description,created_at,updated_at,deleted_at,deleted_by_client_common_name) VALUES('archived-key',7,'entry','SOURCE_ARCHIVE_META','a','SOURCE_ARCHIVED','notes','records','SOURCE_OLD_SHELF','SOURCE_OLD_BOOK','1000','1001','1000000','SOURCE_DELETER');
            INSERT INTO deleted_message_history(id,entry_key,operation_id,client_common_name,appended_content,created_at) VALUES(66,'archived-key','SOURCE_OLD_OPERATION','SOURCE_CN','SOURCE_OLD_APPEND','old time');
            INSERT INTO auth_tokens(id,session_id,token_nonce,token_value,token_hash,token_prefix,access_level) VALUES(77,7,'SOURCE_NONCE','SOURCE_TOKEN','SOURCE_HASH','SOURCE_PREFIX','admin');
            INSERT INTO issued_client_certs(id,session_id,common_name,access_level,cert_pem,expires_at,revoked_at) VALUES(88,7,'SOURCE_CERT_CN','admin','SOURCE_PEM','past time','revoked time');
            INSERT INTO transfer_log(id,session_id,direction,scope_json,bundle_sha256,outcome,entry_count,created_at,created_by) VALUES(99,7,'import','SOURCE_SCOPE','SOURCE_BUNDLE','kept',1,'1000','SOURCE_IMPORTER');").unwrap();
        drop(db);
        fs::write(source.join("runtime-journal.jsonl"), b"").unwrap();
        fs::write(source.join("old-key.pem"), b"SOURCE_PRIVATE_KEY").unwrap();
        Self {
            root,
            source,
            destination,
        }
    }
    fn options(&self) -> ConversionOptions {
        ConversionOptions {
            data_dir: self.source.clone(),
            destination: self.destination.clone(),
            expected_source_sha256: hash(&self.source.join("ccp.sqlite3")),
            operator: "new operator".into(),
            confirm_offline: true,
        }
    }
    fn db(&self) -> Connection {
        Connection::open(self.source.join("ccp.sqlite3")).unwrap()
    }
    fn files(&self) -> BTreeMap<String, String> {
        fs::read_dir(&self.source)
            .unwrap()
            .map(|e| {
                let e = e.unwrap();
                (e.file_name().to_str().unwrap().to_owned(), hash(&e.path()))
            })
            .collect()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}
fn hash(path: &Path) -> String {
    format!("{:x}", Sha256::digest(fs::read(path).unwrap()))
}
fn row(db: &Connection, table: &str, key: &dyn rusqlite::ToSql) -> NativeRow {
    let column = if table == "deleted_message_packs" {
        "entry_key"
    } else {
        "id"
    };
    db.query_row(
        &format!("SELECT * FROM {table} WHERE {column}=?1"),
        [key],
        NativeRow::capture,
    )
    .unwrap()
}
fn error<T>(result: Result<T>, expected: ConversionError) {
    match result {
        Ok(_) => panic!("expected conversion refusal"),
        Err(e) => assert_eq!(e, expected),
    }
}

#[test]
fn converts_every_native_domain_without_source_changes_or_inferred_authority() {
    let f = Fixture::new();
    let before = f.files();
    let result = convert(&f.options()).unwrap();
    assert_eq!(f.files(), before);
    assert_eq!(result.receipt.entry_count, 2);
    assert_eq!(result.receipt.workspace_count, 1);
    assert!(result.receipt.source_database_unchanged_at_verification);
    assert_eq!(result.receipt.source_sha256, before["ccp.sqlite3"]);
    let workspace = result.workspaces[0].id;
    let owner = result.instance_key.principal_id;
    let mut db = Connection::open(f.destination.join("ccp.sqlite3")).unwrap();
    let entries: Vec<(String, bool)> = db
        .prepare("SELECT id,archived FROM ccp_content_entries ORDER BY archived")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    let entry = Uuid::parse_str(&entries[0].0).unwrap();
    let mut store = AccessStore::new(&mut db).unwrap();
    let actor = store
        .authenticate_admin_key(
            result.workspaces[0].owner_key.secret(),
            result.receipt.converted_at,
        )
        .unwrap();
    let snapshot = store
        .get_entry(actor, workspace, entry, result.receipt.converted_at)
        .unwrap();
    assert_eq!(snapshot.content.text, "SOURCE_BODYAPPEND");
    assert_eq!(snapshot.entry.owner_client_id, None);
    assert_eq!(snapshot.revision.admin_principal_id, Some(owner));
    assert_eq!(
        snapshot.revision.agent.as_deref(),
        Some("native conversion")
    );
    assert_eq!(snapshot.entry.revision, 1);
    assert_eq!(snapshot.entry.created_at, result.receipt.converted_at);
    let history = store
        .native_entry_history(
            actor,
            workspace,
            &NativeHistoryRequest {
                entry_id: entry,
                after: None,
                limit: 100,
            },
            result.receipt.converted_at,
        )
        .unwrap()
        .unwrap();
    assert_eq!(history.items.len(), 2);
    assert_eq!(history.items[0].row, row(&f.db(), "message_packs", &42));
    assert_eq!(history.items[1].row, row(&f.db(), "message_history", &-55));
    for table in source::Table::ALL {
        let source = source::Source::open(&f.options()).unwrap();
        let mut ledger = source::Ledger::new();
        source.visit(table,None,&mut ledger,|record,_| {
            let actual:String=db.query_row("SELECT row_json FROM (SELECT source_table,source_key,row_json FROM ccp_native_workspace_rows UNION ALL SELECT source_table,source_key,row_json FROM ccp_native_entry_rows) WHERE source_table=?1 AND source_key=?2",params![table.name(),record.key.label()],|r|r.get(0))?;
            assert_eq!(serde_json::from_str::<NativeRow>(&actual).unwrap(),record.row);Ok(())
        }).unwrap();
    }
    for table in [
        "sessions",
        "message_packs",
        "message_history",
        "auth_tokens",
        "issued_client_certs",
        "ccp_clients",
        "ccp_grants",
        "ccp_credentials",
    ] {
        assert_eq!(
            db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                .get::<_, u64>(0))
                .unwrap(),
            0
        );
    }
    assert_eq!(
        db.query_row("SELECT count(*) FROM ccp_content_shelves", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM ccp_content_books", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        2
    );
    assert!(!f.destination.join("old-key.pem").exists());
    assert_eq!(fs::read_dir(&f.destination).unwrap().count(), 1);
    assert!(crate::directory_mode::DirectoryLease::native(&f.destination).is_err());
}
