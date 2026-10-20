// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
mod resolution;

fn request(f: &Fixture, change: api::EntryChange) -> api::EntryMutation {
    api::EntryMutation {
        workspace_id: f.setup.workspace_id.to_string(),
        operation_id: Uuid::new_v4().to_string(),
        agent: Some("human-readable agent".into()),
        change,
    }
}
fn copy(request: &api::EntryMutation) -> api::EntryMutation {
    serde_json::from_value(serde_json::to_value(request).unwrap()).unwrap()
}
fn append(f: &Fixture, r: &Records) -> api::EntryMutation {
    request(
        f,
        api::EntryChange::Append {
            entry_id: r.entry.clone(),
            expected_revision: 2,
            text: "+PRIVATE_BODY".into(),
        },
    )
}
fn profile_sql(f: &Fixture, sql: &str) {
    Connection::open(f.profile.join("workspace-profile.sqlite3"))
        .unwrap()
        .execute_batch(sql)
        .unwrap();
}
fn pending(f: &Fixture) -> Value {
    let raw: String = Connection::open(f.profile.join("workspace-profile.sqlite3"))
        .unwrap()
        .query_row("SELECT pending FROM content_state", [], |r| r.get(0))
        .unwrap();
    serde_json::from_str(&raw).unwrap()
}
async fn cli(f: &Fixture, args: &[&str]) -> Output {
    let mut command = vec![
        "workspace",
        "content",
        "--profile",
        f.profile.to_str().unwrap(),
    ];
    command.extend_from_slice(args);
    f.cli(&command).await
}
fn fail_finish(f: &Fixture) {
    profile_sql(
        f,
        "CREATE TRIGGER fail_content_finish BEFORE UPDATE ON content_state WHEN NEW.pending IS NULL BEGIN SELECT RAISE(ABORT,'private fixture failure'); END;",
    );
}
fn fail_remote(f: &Fixture) {
    f.db().execute_batch("CREATE TRIGGER fail_entry BEFORE UPDATE ON ccp_content_entries BEGIN SELECT RAISE(ABORT,'fixture unavailable'); END; CREATE TRIGGER fail_resolution BEFORE INSERT ON ccp_content_cancellations BEGIN SELECT RAISE(ABORT,'fixture unavailable'); END;").unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn actual_cli_five_actions_keep_exact_content_and_return_safe_original_binding_metadata() {
    let f = Fixture::new().await;
    let info = enroll_profile(&f).await;
    let r = records(&f, &info);
    let create = request(
        &f,
        api::EntryChange::Create {
            book_id: r.book.clone(),
            name: "new entry".into(),
            content: api::EntryContent {
                text: "PRIVATE_BODY\0é".into(),
                description: "exact\nmetadata".into(),
                labels: vec!["z".into(), "a".into()],
            },
        },
    );
    let file = private_invitation(&f, "create.json", &serde_json::to_string(&create).unwrap());
    let first = value(cli(&f, &["mutate", "--request-file", file.to_str().unwrap()]).await);
    assert_eq!(first["revision"], 1);
    assert_eq!(first["operation"]["operation_id"], create.operation_id);
    assert_eq!(
        first["operation"]["credential_id"],
        info.credential_id.unwrap()
    );
    assert!(!first.to_string().contains("PRIVATE_BODY"));
    let id = first["entry_id"].as_str().unwrap();
    for (index, change) in [
        api::EntryChange::Append {
            entry_id: id.into(),
            expected_revision: 1,
            text: "+tail".into(),
        },
        api::EntryChange::Revise {
            entry_id: id.into(),
            expected_revision: 2,
            content: api::EntryContent {
                text: "revised".into(),
                description: "d".into(),
                labels: vec![],
            },
        },
        api::EntryChange::Archive {
            entry_id: id.into(),
            expected_revision: 3,
        },
        api::EntryChange::Restore {
            entry_id: id.into(),
            expected_revision: 4,
        },
    ]
    .into_iter()
    .enumerate()
    {
        let request = request(&f, change);
        let file = private_invitation(
            &f,
            &format!("mutation-{index}.json"),
            &serde_json::to_string(&request).unwrap(),
        );
        let args = ["mutate", "--request-file", file.to_str().unwrap()];
        let receipt = value(cli(&f, &args).await);
        assert_eq!(receipt["entry_id"], id);
        assert_eq!(receipt["revision"], u64::try_from(index).unwrap() + 2);
        assert_eq!(value(cli(&f, &args).await), receipt);
    }
    assert!(value(cli(&f, &["pending"]).await).is_null());
    assert_eq!(value(cli(&f, &["resume"]).await)["revision"], 5);
    assert_eq!(
        value(cli(&f, &["resolve"]).await)["outcome"]["state"],
        "committed"
    );
    let mut p = WorkspaceProfile::open(&f.profile).unwrap();
    let snapshot = p
        .content_entry(&api::EntryTarget {
            workspace_id: f.setup.workspace_id.to_string(),
            entry_id: id.into(),
            revision: None,
        })
        .await
        .unwrap();
    assert_eq!(snapshot.content.text, "revised");
    assert_eq!(snapshot.entry.revision, 5);
    assert_eq!(f.count("ccp_content_operations"), 7);
    drop(p);
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_local_receipt_save_and_cli_restart_resume_same_append_exactly_once() {
    let f = Fixture::new().await;
    let info = enroll_profile(&f).await;
    let r = records(&f, &info);
    let request = append(&f, &r);
    let file = private_invitation(&f, "append.json", &serde_json::to_string(&request).unwrap());
    fail_finish(&f);
    let failed = cli(&f, &["mutate", "--request-file", file.to_str().unwrap()]).await;
    assert!(!failed.status.success());
    assert!(failed.stdout.is_empty());
    assert!(!String::from_utf8_lossy(&failed.stderr).contains("PRIVATE_BODY"));
    let saved = pending(&f);
    assert_eq!(saved["request"], serde_json::to_value(&request).unwrap());
    assert_eq!(
        saved["binding"]["credential_id"],
        info.credential_id.unwrap()
    );
    assert_eq!(f.count("ccp_content_operations"), 3);
    let metadata = value(cli(&f, &["pending"]).await);
    assert_eq!(metadata["operation_id"], request.operation_id);
    assert!(!metadata.to_string().contains("PRIVATE_BODY"));
    profile_sql(&f, "DROP TRIGGER fail_content_finish");
    let done = value(cli(&f, &["resume"]).await);
    assert_eq!(done["revision"], 3);
    assert_eq!(done["operation"]["operation_id"], request.operation_id);
    assert_eq!(f.count("ccp_content_operations"), 3);
    let mut p = WorkspaceProfile::open(&f.profile).unwrap();
    assert_eq!(
        p.content_entry(&entry_request(&f, &r))
            .await
            .unwrap()
            .content
            .text,
        "original+contribution+PRIVATE_BODY"
    );
    drop(p);
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prepare_failure_never_posts_and_unknown_remote_outcome_retains_intent_and_blocks_rotation()
{
    let f = Fixture::new().await;
    WorkspaceProfile::trust(&f.profile, f.config())
        .await
        .unwrap();
    short_credential(&f, false);
    let info = WorkspaceProfile::open(&f.profile).unwrap().info().unwrap();
    let r = records(&f, &info);
    profile_sql(
        &f,
        "CREATE TRIGGER fail_prepare BEFORE UPDATE ON content_state BEGIN SELECT RAISE(ABORT,'fixture prepare failure'); END;",
    );
    let mut p = WorkspaceProfile::open(&f.profile).unwrap();
    assert!(p.mutate_entry(append(&f, &r)).await.is_err());
    assert!(p.pending_entry_operation().is_none());
    assert_eq!(f.count("ccp_content_operations"), 2);
    // Preflight performs the due renewal before capturing the pending binding.
    assert_ne!(p.info().unwrap().credential_id, info.credential_id);
    drop(p);
    profile_sql(&f, "DROP TRIGGER fail_prepare");
    fail_remote(&f);
    let mut p = WorkspaceProfile::open(&f.profile).unwrap();
    assert!(p.mutate_entry(append(&f, &r)).await.is_err());
    let original = p.info().unwrap().pending_content.unwrap();
    assert!(p.renew().await.is_err());
    assert!(
        p.join(&format!("ccp_join_{}", "a".repeat(64)))
            .await
            .is_err()
    );
    assert!(p.mutate_entry(append(&f, &r)).await.is_err());
    assert_eq!(
        p.info().unwrap().pending_content.unwrap().operation_id,
        original.operation_id
    );
    assert_eq!(
        p.content_entry(&entry_request(&f, &r))
            .await
            .unwrap()
            .entry
            .revision,
        2
    );
    assert_eq!(p.workspaces(None, 10).await.unwrap().workspaces.len(), 1);
    assert_eq!(
        p.info().unwrap().credential_id.as_deref(),
        Some(original.credential_id.as_str())
    );
    drop(p);
    let saved = pending(&f);
    f.db()
        .execute_batch("DROP TRIGGER fail_entry; DROP TRIGGER fail_resolution")
        .unwrap();
    let mut p = WorkspaceProfile::open(&f.profile).unwrap();
    assert_eq!(p.resume_entry_operation().await.unwrap().revision, 3);
    assert_eq!(
        p.info().unwrap().last_content.unwrap().operation_id,
        saved["binding"]["operation_id"].as_str().unwrap()
    );
    assert_eq!(f.count("ccp_credentials"), 2);
    drop(p);
    f.finish().await;
}
