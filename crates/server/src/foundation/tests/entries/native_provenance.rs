// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;
use rusqlite::OptionalExtension;

mod boundaries;
mod deletion;
mod persistence;

fn proof(f: &Fixture, workspace: Uuid) {
    f.connection.execute("INSERT INTO ccp_native_imports(workspace_id,source_sha256,source_schema,legacy_session_id,recorded_by,recorded_at) VALUES(?1,?2,2,-7,?3,?4)",params![workspace.to_string(),"a".repeat(64),f.global.to_string(),NOW]).unwrap();
}
fn raw(text: &str) -> String {
    serde_json::to_string(&NativeRow {
        columns: vec!["context".into(), "created_at".into(), "owner".into()],
        values: vec![
            NativeSqlValue::Text(text.into()),
            NativeSqlValue::Text("arbitrary time".into()),
            NativeSqlValue::Text("untrusted owner".into()),
        ],
    })
    .unwrap()
}
fn record(f: &Fixture, workspace: Uuid, entry: Uuid, ordinal: i64, text: &str) {
    f.connection.execute("INSERT INTO ccp_native_entry_rows(workspace_id,entry_id,ordinal,source_table,source_key,row_json) VALUES(?1,?2,?3,'message_history',?4,?5)",params![workspace.to_string(),entry.to_string(),ordinal,format!("{entry}-{ordinal}"),raw(text)]).unwrap();
}
fn seal(f: &Fixture, workspace: Uuid) {
    f.connection
        .execute(
            "UPDATE ccp_native_imports SET sealed=1 WHERE workspace_id=?1",
            [workspace.to_string()],
        )
        .unwrap();
}
fn request(entry_id: Uuid, after: Option<u64>, limit: usize) -> NativeHistoryRequest {
    NativeHistoryRequest {
        entry_id,
        after,
        limit,
    }
}
fn read(
    f: &mut Fixture,
    actor: Actor,
    r: &NativeHistoryRequest,
) -> anyhow::Result<Option<NativeEntryHistory>> {
    AccessStore::new(&mut f.connection)?.native_entry_history(actor, f.workspace, r, NOW)
}
fn entry(f: &mut Fixture, name: &str) -> Uuid {
    let existing: Option<String> = f
        .connection
        .query_row(
            "SELECT id FROM ccp_content_books WHERE workspace_id=?1 LIMIT 1",
            [f.workspace.to_string()],
            |r| r.get(0),
        )
        .optional()
        .unwrap();
    let book = existing
        .map(|id| Uuid::parse_str(&id).unwrap())
        .unwrap_or_else(|| book(f));
    apply(
        f,
        Actor::Admin(f.owner),
        &create(book, name, "CURRENT_BODY"),
    )
    .entry_id
}

#[test]
fn source_history_is_sealed_ordered_paged_and_never_claims_creation_or_authority() {
    let mut f = fixture();
    let e = entry(&mut f, "imported");
    let owner = Actor::Admin(f.owner);
    assert_eq!(read(&mut f, owner, &request(e, None, 2)).unwrap(), None);
    let identities = rows(&f.connection, "ccp_clients")
        + rows(&f.connection, "ccp_grants")
        + rows(&f.connection, "ccp_admin_keys");
    proof(&f, f.workspace);
    for ordinal in [3, 1, 2] {
        record(&f, f.workspace, e, ordinal, "PRIVATE_ORIGINAL_APPEND");
    }
    assert_eq!(read(&mut f, owner, &request(e, None, 2)).unwrap(), None);
    seal(&f, f.workspace);
    let first = read(&mut f, owner, &request(e, None, 2)).unwrap().unwrap();
    assert_eq!(
        first.items.iter().map(|r| r.ordinal).collect::<Vec<_>>(),
        [1, 2]
    );
    assert_eq!(first.next_cursor, Some(2));
    assert_eq!(first.source.recorded_by, f.global);
    assert_eq!(first.source.legacy_session_id, -7);
    assert_eq!(
        first.items[0].row.values[0],
        NativeSqlValue::Text("PRIVATE_ORIGINAL_APPEND".into())
    );
    let last = read(&mut f, owner, &request(e, first.next_cursor, 2))
        .unwrap()
        .unwrap();
    assert_eq!(last.items[0].ordinal, 3);
    assert_eq!(last.next_cursor, None);
    assert!(
        read(&mut f, owner, &request(e, Some(3), 2))
            .unwrap()
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(get(&mut f, e).entry.owner_client_id, None);
    assert_eq!(get(&mut f, e).content.text, "CURRENT_BODY");
    assert_eq!(
        rows(&f.connection, "ccp_clients")
            + rows(&f.connection, "ccp_grants")
            + rows(&f.connection, "ccp_admin_keys"),
        identities
    );
    assert_eq!(rows(&f.connection, "message_packs"), 0);
}
