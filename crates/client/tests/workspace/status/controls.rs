// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn local_forget_after_remote_rollback_does_not_cancel_or_fence_the_original_request() {
    let f = Fixture::new().await;
    enroll_profile(&f).await;
    let req = request(&f);
    f.db().execute_batch("CREATE TRIGGER fail_status_write BEFORE INSERT ON ccp_work_status BEGIN SELECT RAISE(ABORT,'PRIVATE_STATUS unavailable'); END;").unwrap();
    assert!(!submit(&f, &req, "status.json").await.status.success());
    let original = pending(&f);
    assert_eq!(f.count("ccp_work_status_operations"), 0);
    let target = &original["binding"]["target"];
    assert!(!guarded(&f, "forget", target, false).await.status.success());
    assert_eq!(pending(&f), original);
    // No server permission is needed for a local discard.
    f.db()
        .execute_batch("UPDATE ccp_clients SET disabled=1")
        .unwrap();
    let forgotten = value(guarded(&f, "forget", target, true).await);
    assert_eq!(forgotten["cancelled"], false);
    assert_eq!(forgotten["operation"]["state"], "locally_forgotten");
    assert!(!forgotten.to_string().contains("PRIVATE_STATUS"));
    assert_eq!(f.count("ccp_work_status_operations"), 0);
    f.db()
        .execute_batch("UPDATE ccp_clients SET disabled=0; DROP TRIGGER fail_status_write")
        .unwrap();
    let done = value(
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
    assert_eq!(done["revision"], 1);
    assert_eq!(f.count("ccp_work_status_operations"), 1);
    assert_eq!(done["operation"]["request_hash"], target["request_hash"]);
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn archived_or_revoked_authority_retains_original_pending_and_never_rekeys_it() {
    for revoked in [false, true] {
        let f = Fixture::new().await;
        enroll_profile(&f).await;
        let req = request(&f);
        fail_finish(&f);
        assert!(!submit(&f, &req, "status.json").await.status.success());
        let original = pending(&f);
        profile_sql(&f, "DROP TRIGGER fail_status_finish");
        if revoked {
            f.db()
                .execute_batch("UPDATE ccp_credentials SET revoked_at=1")
                .unwrap();
        } else {
            f.db()
                .execute_batch("UPDATE ccp_workspaces SET state='archived'")
                .unwrap();
            let list = value(cli(&f, &["list", "--workspace-id", &req.workspace_id]).await);
            assert_eq!(list["items"].as_array().unwrap().len(), 1);
        }
        let target = &original["binding"]["target"];
        assert!(!guarded(&f, "resume", target, false).await.status.success());
        assert_eq!(pending(&f), original);
        assert_eq!(f.count("ccp_credentials"), 1);
        assert_eq!(f.count("ccp_work_status_operations"), 1);
        assert_eq!(
            value(guarded(&f, "forget", target, true).await)["cancelled"],
            false
        );
        f.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn status_preflight_renews_before_capture_but_pending_reads_never_rotate() {
    let f = Fixture::new().await;
    value(f.trust_cli().await);
    short_credential(&f, false);
    let before = WorkspaceProfile::open(&f.profile).unwrap().info().unwrap();
    assert!(before.renewal_due);
    fail_finish(&f);
    let req = request(&f);
    assert!(!submit(&f, &req, "status.json").await.status.success());
    let original = pending(&f);
    let current = WorkspaceProfile::open(&f.profile).unwrap().info().unwrap();
    assert_ne!(before.credential_id, current.credential_id);
    assert_eq!(
        original["binding"]["target"]["credential_id"],
        current.credential_id.unwrap()
    );
    assert_eq!(f.count("ccp_credentials"), 2);
    value(cli(&f, &["list", "--workspace-id", &req.workspace_id]).await);
    assert_eq!(pending(&f), original);
    assert_eq!(f.count("ccp_credentials"), 2);
    profile_sql(&f, "DROP TRIGGER fail_status_finish");
    value(guarded(&f, "resume", &original["binding"]["target"], false).await);
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn status_cli_refuses_private_file_and_partial_guard_errors_without_work_text() {
    let f = Fixture::new().await;
    enroll_profile(&f).await;
    let req = request(&f);
    let file = private_invitation(&f, "request.json", &serde_json::to_string(&req).unwrap());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
        let failed = cli(&f, &["mutate", "--request-file", file.to_str().unwrap()]).await;
        assert!(!failed.status.success());
        assert!(!String::from_utf8_lossy(&failed.stderr).contains("PRIVATE_STATUS"));
        fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
        let link = f.root.join("request-link.json");
        std::os::unix::fs::symlink(&file, &link).unwrap();
        assert!(
            !cli(&f, &["mutate", "--request-file", link.to_str().unwrap()])
                .await
                .status
                .success()
        );
    }
    let partial = cli(&f, &["resume", "--operation-id", &req.operation_id]).await;
    assert!(!partial.status.success());
    assert!(String::from_utf8_lossy(&partial.stderr).contains("required arguments"));
    let invalid = private_invitation(
        &f,
        "invalid.json",
        &format!("{{\"PRIVATE_STATUS\":{}}}", "x".repeat(65536)),
    );
    assert!(
        !cli(&f, &["mutate", "--request-file", invalid.to_str().unwrap()])
            .await
            .status
            .success()
    );
    assert_eq!(f.count("ccp_work_status_operations"), 0);
    assert!(value(cli(&f, &["pending"]).await)["pending"].is_null());
    f.finish().await;
}
