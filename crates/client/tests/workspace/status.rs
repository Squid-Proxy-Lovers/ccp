// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use client::workspace::work_status::{self as status, WorkStatusTarget};
mod controls;

fn request(f: &Fixture) -> status::Mutation {
    status::Mutation {
        workspace_id: f.setup.workspace_id.to_string(),
        status_id: Uuid::new_v4().to_string(),
        operation_id: Uuid::new_v4().to_string(),
        expected_revision: 0,
        change: status::Change::Set {
            agent: Some("\u{feff}".into()),
            message: "PRIVATE_STATUS é\u{feff}".into(),
        },
    }
}
fn copy(value: &status::Mutation) -> status::Mutation {
    serde_json::from_value(serde_json::to_value(value).unwrap()).unwrap()
}
async fn cli(f: &Fixture, args: &[&str]) -> Output {
    let mut command = vec![
        "workspace",
        "status",
        "--profile",
        f.profile.to_str().unwrap(),
    ];
    command.extend_from_slice(args);
    f.cli(&command).await
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
        .query_row("SELECT pending FROM status_state", [], |r| r.get(0))
        .unwrap();
    serde_json::from_str(&raw).unwrap()
}
fn fail_finish(f: &Fixture) {
    profile_sql(
        f,
        "CREATE TRIGGER fail_status_finish BEFORE UPDATE ON status_state WHEN NEW.pending IS NULL BEGIN SELECT RAISE(ABORT,'PRIVATE_STATUS fixture save'); END;",
    );
}
async fn submit(f: &Fixture, req: &status::Mutation, name: &str) -> Output {
    let path = private_invitation(f, name, &serde_json::to_string(req).unwrap());
    cli(f, &["mutate", "--request-file", path.to_str().unwrap()]).await
}
async fn guarded(f: &Fixture, action: &str, target: &Value, confirm: bool) -> Output {
    let mut args = vec![action.to_owned()];
    for field in [
        "workspace_id",
        "status_id",
        "operation_id",
        "credential_id",
        "request_hash",
    ] {
        args.extend([
            format!("--{}", field.replace('_', "-")),
            target[field].as_str().unwrap().to_owned(),
        ]);
    }
    if confirm {
        args.push("--confirm-local-forget".into());
    }
    cli(f, &args.iter().map(String::as_str).collect::<Vec<_>>()).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn actual_status_cli_preserves_unicode_stale_state_and_historical_retry_after_clear() {
    let f = Fixture::new().await;
    let info = enroll_profile(&f).await;
    let req = request(&f);
    let first = value(submit(&f, &req, "status.json").await);
    assert_eq!(first["revision"], 1);
    assert_eq!(
        first["operation"]["credential_id"],
        info.credential_id.unwrap()
    );
    assert!(!first.to_string().contains("PRIVATE_STATUS"));
    let mut p = WorkspaceProfile::open(&f.profile).unwrap();
    let get = status::Get {
        workspace_id: req.workspace_id.clone(),
        status_id: req.status_id.clone(),
    };
    let current = p.status_message(&get).await.unwrap();
    assert_eq!(current.message.as_deref(), Some("PRIVATE_STATUS é\u{feff}"));
    assert_eq!(current.agent.as_deref(), Some("\u{feff}"));
    assert!(!current.stale);
    f.db()
        .execute("UPDATE ccp_work_status SET updated_at=?1", [clock() - 301])
        .unwrap();
    let list = status::List {
        workspace_id: req.workspace_id.clone(),
        after: None,
        limit: Some(1),
    };
    let page = p.status_messages(&list).await.unwrap();
    assert_eq!(page.items.len(), 1);
    assert!(page.items[0].stale);
    assert_eq!(page.items[0].message, current.message);
    drop(p);
    let clear = status::Mutation {
        operation_id: Uuid::new_v4().to_string(),
        expected_revision: 1,
        change: status::Change::Clear {},
        ..copy(&req)
    };
    assert_eq!(value(submit(&f, &clear, "clear.json").await)["revision"], 2);
    // A historical set receipt is not current state and must never resurrect cleared text.
    let replay = value(
        cli(
            &f,
            &[
                "mutate",
                "--request-file",
                f.root.join("status.json").to_str().unwrap(),
            ],
        )
        .await,
    );
    assert_eq!(replay, first);
    let tombstone = value(
        cli(
            &f,
            &[
                "get",
                "--workspace-id",
                &req.workspace_id,
                "--status-id",
                &req.status_id,
            ],
        )
        .await,
    );
    assert!(tombstone["message"].is_null());
    assert_eq!(tombstone["revision"], 2);
    assert!(
        value(cli(&f, &["list", "--workspace-id", &req.workspace_id]).await)["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(f.count("ccp_work_status_operations"), 2);
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn committed_status_with_failed_local_save_survives_restart_guards_and_exact_resume() {
    let f = Fixture::new().await;
    enroll_profile(&f).await;
    let req = request(&f);
    fail_finish(&f);
    let failed = submit(&f, &req, "pending.json").await;
    assert!(!failed.status.success());
    assert!(!String::from_utf8_lossy(&failed.stderr).contains("PRIVATE_STATUS"));
    let original = pending(&f);
    assert_eq!(original["request"], serde_json::to_value(&req).unwrap());
    let target = original["binding"]["target"].clone();
    let metadata = value(cli(&f, &["pending"]).await);
    assert_eq!(metadata["pending"]["state"], "pending");
    assert!(!metadata.to_string().contains("PRIVATE_STATUS"));
    profile_sql(&f, "DROP TRIGGER fail_status_finish");
    for field in [
        "workspace_id",
        "status_id",
        "operation_id",
        "credential_id",
        "request_hash",
    ] {
        let mut wrong = target.clone();
        wrong[field] = json!(if field == "request_hash" {
            "f".repeat(64)
        } else {
            Uuid::new_v4().to_string()
        });
        for action in ["resume", "forget"] {
            assert!(
                !guarded(&f, action, &wrong, action == "forget")
                    .await
                    .status
                    .success()
            );
            assert_eq!(pending(&f), original);
            assert_eq!(f.count("ccp_work_status_operations"), 1);
        }
    }
    let mut p = WorkspaceProfile::open(&f.profile).unwrap();
    assert!(p.renew().await.is_err());
    assert!(p.mutate_status(request(&f)).await.is_err());
    assert_eq!(
        p.status_messages(&status::List {
            workspace_id: req.workspace_id.clone(),
            after: None,
            limit: None
        })
        .await
        .unwrap()
        .items
        .len(),
        1
    );
    assert_eq!(
        p.info().unwrap().pending_status.unwrap().target,
        serde_json::from_value::<WorkStatusTarget>(target.clone()).unwrap()
    );
    drop(p);
    let done = value(guarded(&f, "resume", &target, false).await);
    assert_eq!(done["revision"], 1);
    assert_eq!(done["operation"]["request_hash"], target["request_hash"]);
    assert_eq!(f.count("ccp_work_status_operations"), 1);
    assert!(value(cli(&f, &["pending"]).await)["pending"].is_null());
    assert!(!guarded(&f, "resume", &target, false).await.status.success());
    f.finish().await;
}
