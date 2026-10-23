// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;
use rusqlite::params;

fn seed(f: &Fixture, r: &Records) {
    let db = f.db();
    let workspace = f.setup.workspace_id.to_string();
    db.execute("INSERT INTO ccp_native_imports(workspace_id,source_sha256,source_schema,legacy_session_id,recorded_by,recorded_at) VALUES(?1,?2,2,7,?3,?4)",params![workspace,"b".repeat(64),f.setup.owner_key.principal_id.to_string(),clock()]).unwrap();
    for n in 1..=4 {
        let raw = json!({"columns":["id","appended_content"],"values":[{"type":"integer","value":n.to_string()},{"type":"text","value":"x".repeat(1_200_000)}]});
        db.execute("INSERT INTO ccp_native_entry_rows(workspace_id,entry_id,ordinal,source_table,source_key,row_json) VALUES(?1,?2,?3,'message_history',?4,?5)",params![workspace,r.entry,n,n.to_string(),raw.to_string()]).unwrap();
    }
    db.execute(
        "UPDATE ccp_native_imports SET sealed=1 WHERE workspace_id=?1",
        [workspace],
    )
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_history_library_and_cli_page_large_raw_rows_without_mutating_profiles() {
    let f = Fixture::new().await;
    let info = enroll_profile(&f).await;
    let r = records(&f, &info);
    seed(&f, &r);
    let profile = f.profile.join("workspace-profile.sqlite3");
    let before = fs::read(&profile).unwrap();
    let request = api::HistoryPage {
        workspace_id: f.setup.workspace_id.to_string(),
        entry_id: r.entry.clone(),
        after: None,
        limit: Some(32),
    };
    let mut p = WorkspaceProfile::open(&f.profile).unwrap();
    let first = p.content_native_history(&request).await.unwrap();
    assert_eq!(first.items.len(), 3);
    assert_eq!(first.next_cursor, Some(3));
    assert_eq!(
        first.source.as_ref().unwrap().database_sha256,
        "b".repeat(64)
    );
    match &first.items[0].row.values[1] {
        api::NativeValue::Text(v) => assert_eq!(v.len(), 1_200_000),
        _ => panic!("source text type changed"),
    }
    let tail = p
        .content_native_history(&api::HistoryPage {
            after: first.next_cursor,
            ..request
        })
        .await
        .unwrap();
    assert_eq!(tail.items.len(), 1);
    assert_eq!(tail.items[0].ordinal, 4);
    assert!(tail.next_cursor.is_none());
    drop(p);
    assert_eq!(fs::read(&profile).unwrap(), before);
    let args = [
        "workspace",
        "content",
        "--profile",
        f.profile.to_str().unwrap(),
        "native-history",
        "--workspace-id",
        &f.setup.workspace_id.to_string(),
        "--entry-id",
        &r.entry,
    ];
    let output = value(f.cli(&args).await);
    assert_eq!(output["items"].as_array().unwrap().len(), 3);
    assert_eq!(output["next_cursor"], 3);
    assert_eq!(output["workspace_id"], f.setup.workspace_id.to_string());
    assert!(!output.to_string().contains("recorded_by"));
    assert!(!output.to_string().contains("legacy_session_id"));
    assert_eq!(fs::read(&profile).unwrap(), before);
    for extra in [
        ["--limit", "0"],
        ["--limit", "33"],
        ["--after", "18446744073709551615"],
    ] {
        let mut bad = args.to_vec();
        bad.extend(extra);
        let bad = f.cli(&bad).await;
        assert!(!bad.status.success());
        assert!(bad.stdout.is_empty());
    }
    assert_eq!(fs::read(&profile).unwrap(), before);
    f.finish().await;
}
