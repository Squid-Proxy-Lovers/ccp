// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

fn flags(target: &Value) -> Vec<String> {
    [
        "workspace_id",
        "operation_id",
        "credential_id",
        "request_hash",
    ]
    .into_iter()
    .flat_map(|field| {
        [
            format!("--{}", field.replace('_', "-")),
            target[field].as_str().unwrap().to_owned(),
        ]
    })
    .collect()
}

async fn guarded(f: &Fixture, action: &str, target: &Value, via: Option<&str>) -> Output {
    let mut args = vec![
        "workspace".to_owned(),
        "content".to_owned(),
        "--profile".to_owned(),
        f.profile.to_str().unwrap().to_owned(),
        action.to_owned(),
    ];
    args.extend(flags(target));
    if let Some(path) = via {
        args.extend(["--via-profile".to_owned(), path.to_owned()]);
    }
    f.cli(&args.iter().map(String::as_str).collect::<Vec<_>>())
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guarded_cli_refuses_each_mismatched_field_before_network_or_other_profile_open() {
    let f = Fixture::new().await;
    let info = enroll_profile(&f).await;
    let r = records(&f, &info);
    fail_finish(&f);
    let mut p = WorkspaceProfile::open(&f.profile).unwrap();
    assert!(p.mutate_entry(append(&f, &r)).await.is_err());
    drop(p);
    profile_sql(&f, "DROP TRIGGER fail_content_finish");
    let saved = pending(&f);
    let target = saved["binding"].clone();
    for field in [
        "workspace_id",
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
        for action in ["resume", "resolve"] {
            let output = guarded(
                &f,
                action,
                &wrong,
                (action == "resolve").then_some("/missing/private-recovery"),
            )
            .await;
            assert!(!output.status.success());
            assert!(output.stdout.is_empty());
            let error = String::from_utf8_lossy(&output.stderr);
            assert!(error.contains("expected binding"), "{error}");
            assert!(!error.contains("private-recovery"));
            assert_eq!(pending(&f), saved);
            assert_eq!(f.count("ccp_content_operations"), 3);
        }
    }
    let resumed = value(guarded(&f, "resume", &target, None).await);
    assert_eq!(resumed["revision"], 3);
    assert_eq!(resumed["operation"]["request_hash"], target["request_hash"]);
    assert_eq!(f.count("ccp_content_operations"), 3);
    let resolved = value(guarded(&f, "resolve", &target, None).await);
    assert_eq!(resolved["outcome"]["state"], "committed");
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn partial_cli_guard_is_rejected_during_parsing_and_stale_guard_never_resumes_new_intent() {
    let f = Fixture::new().await;
    enroll_profile(&f).await;
    let output = f
        .cli(&[
            "workspace",
            "content",
            "--profile",
            "/missing/profile",
            "resume",
            "--operation-id",
            &Uuid::new_v4().to_string(),
        ])
        .await;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("required arguments"));
    let info = WorkspaceProfile::open(&f.profile).unwrap().info().unwrap();
    let r = records(&f, &info);
    let mut p = WorkspaceProfile::open(&f.profile).unwrap();
    p.mutate_entry(append(&f, &r)).await.unwrap();
    let old = serde_json::to_value(p.info().unwrap().last_content.unwrap()).unwrap();
    drop(p);
    fail_remote(&f);
    let request = request(
        &f,
        api::EntryChange::Append {
            entry_id: r.entry,
            expected_revision: 3,
            text: "+second".into(),
        },
    );
    let mut p = WorkspaceProfile::open(&f.profile).unwrap();
    assert!(p.mutate_entry(request).await.is_err());
    drop(p);
    let saved = pending(&f);
    f.db()
        .execute_batch("DROP TRIGGER fail_entry; DROP TRIGGER fail_resolution")
        .unwrap();
    assert!(!guarded(&f, "resume", &old, None).await.status.success());
    assert_eq!(pending(&f), saved);
    assert_eq!(f.count("ccp_content_operations"), 3);
    let done = value(guarded(&f, "resume", &saved["binding"], None).await);
    assert_eq!(done["revision"], 4);
    assert_eq!(f.count("ccp_content_operations"), 4);
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guarded_resolution_uses_recovery_for_expired_original_without_rekeying_its_saved_intent() {
    let f = Fixture::new().await;
    WorkspaceProfile::trust(&f.profile, f.config())
        .await
        .unwrap();
    short_credential(&f, true);
    let old = WorkspaceProfile::open(&f.profile).unwrap().info().unwrap();
    assert!(old.expired);
    let request = request(
        &f,
        api::EntryChange::Append {
            entry_id: Uuid::new_v4().to_string(),
            expected_revision: 1,
            text: "PRIVATE_BODY".into(),
        },
    );
    super::resolution::stage(&f, &request);
    let target = pending(&f)["binding"].clone();
    let token = recovery::invitation(&f, old.client_id.as_deref().unwrap(), false);
    let dir = recovery::destination(&f).await;
    WorkspaceProfile::open(&dir)
        .unwrap()
        .recover(old.client_id.as_deref().unwrap(), &token)
        .await
        .unwrap();
    assert!(!guarded(&f, "resolve", &target, None).await.status.success());
    assert_eq!(pending(&f)["binding"], target);
    let resolved = value(guarded(&f, "resolve", &target, Some(dir.to_str().unwrap())).await);
    assert_eq!(resolved["outcome"]["state"], "cancelled");
    assert_eq!(
        resolved["credential_id"],
        old.credential_id.clone().unwrap()
    );
    let original = WorkspaceProfile::open(&f.profile).unwrap();
    assert!(original.info().unwrap().expired);
    assert_eq!(original.info().unwrap().credential_id, old.credential_id);
    assert!(original.pending_entry_operation().is_none());
    assert_eq!(f.count("ccp_content_operations"), 0);
    assert_eq!(f.count("ccp_clients"), 1);
    drop(original);
    f.finish().await;
}
