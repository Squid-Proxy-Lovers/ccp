// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use server::foundation::{RecoveryInvitationRequest, RecoveryScope};
use std::path::Path;

pub(super) fn invitation(f: &Fixture, client: &str, global: bool) -> String {
    let mut db = f.db();
    let mut store = AccessStore::new(&mut db).unwrap();
    let actor = store
        .authenticate_admin_key(
            if global {
                f.setup.instance_key.secret()
            } else {
                f.setup.owner_key.secret()
            },
            clock(),
        )
        .unwrap();
    store
        .issue_recovery_invitation(
            actor,
            RecoveryInvitationRequest {
                client_id: Uuid::parse_str(client).unwrap(),
                scope: if global {
                    RecoveryScope::Instance
                } else {
                    RecoveryScope::Workspace(f.setup.workspace_id)
                },
                expires_at: clock() + 3600,
            },
            clock(),
        )
        .unwrap()
        .secret()
        .to_owned()
}

pub(super) async fn destination(f: &Fixture) -> PathBuf {
    let dir = f.root.join("recovered");
    WorkspaceProfile::trust(&dir, f.config()).await.unwrap();
    dir
}

fn pending(dir: &Path) -> Value {
    let raw: String = Connection::open(dir.join("workspace-profile.sqlite3"))
        .unwrap()
        .query_row("SELECT pending FROM profile", [], |r| r.get(0))
        .unwrap();
    serde_json::from_str(&raw).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scoped_recovery_preserves_original_profile_and_its_unrelated_workspace_access() {
    let f = Fixture::new().await;
    let original = enroll_profile(&f).await;
    let client = original.client_id.unwrap();
    let (join, second) = second_invitation(&f);
    WorkspaceProfile::open(&f.profile)
        .unwrap()
        .join(&join)
        .await
        .unwrap();
    let bytes = fs::read(f.profile.join("workspace-profile.sqlite3")).unwrap();
    let token = invitation(&f, &client, false);
    let dir = destination(&f).await;
    let mut recovered = WorkspaceProfile::open(&dir).unwrap();
    let info = recovered.recover(&client, &token).await.unwrap();
    assert_eq!(info.client_id.as_deref(), Some(client.as_str()));
    assert_ne!(info.credential_id, original.credential_id);
    let page = recovered.workspaces(None, 10).await.unwrap();
    assert_eq!(page.workspaces.len(), 1);
    assert_eq!(page.workspaces[0].id, f.setup.workspace_id.to_string());
    assert!(recovered.recover(&client, &token).await.is_err());
    assert!(recovered.info().unwrap().pending_operation.is_none());
    drop(recovered);
    assert_eq!(
        fs::read(f.profile.join("workspace-profile.sqlite3")).unwrap(),
        bytes
    );
    let mut old = WorkspaceProfile::open(&f.profile).unwrap();
    let page = old.workspaces(None, 10).await.unwrap();
    assert_eq!(page.workspaces.len(), 1);
    assert_eq!(page.workspaces[0].id, second.to_string());
    assert!(old.recover(&client, &token).await.is_err());
    assert_eq!(old.info().unwrap().credential_id, original.credential_id);
    assert_eq!(f.count("ccp_clients"), 1);
    assert_eq!(f.count("ccp_credentials"), 2);
    drop(old);
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn global_recovery_uses_only_captured_live_grants_and_preserves_later_admission() {
    let f = Fixture::new().await;
    let original = enroll_profile(&f).await;
    let client = original.client_id.unwrap();
    let token = invitation(&f, &client, true);
    let (join, second) = second_invitation(&f);
    WorkspaceProfile::open(&f.profile)
        .unwrap()
        .join(&join)
        .await
        .unwrap();
    let dir = destination(&f).await;
    let mut recovered = WorkspaceProfile::open(&dir).unwrap();
    recovered.recover(&client, &token).await.unwrap();
    let page = recovered.workspaces(None, 10).await.unwrap();
    assert_eq!(page.workspaces.len(), 1);
    assert_eq!(page.workspaces[0].id, f.setup.workspace_id.to_string());
    drop(recovered);
    let mut old = WorkspaceProfile::open(&f.profile).unwrap();
    assert_eq!(
        old.workspaces(None, 10).await.unwrap().workspaces[0].id,
        second.to_string()
    );
    drop(old);
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn expired_original_recovers_without_old_peer_and_retains_its_metadata_and_key() {
    let f = Fixture::new().await;
    WorkspaceProfile::trust(&f.profile, f.config())
        .await
        .unwrap();
    short_credential(&f, true);
    let original = WorkspaceProfile::open(&f.profile).unwrap().info().unwrap();
    assert!(original.expired);
    let client = original.client_id.unwrap();
    let token = invitation(&f, &client, false);
    let dir = destination(&f).await;
    let mut recovered = WorkspaceProfile::open(&dir).unwrap();
    assert!(!recovered.recover(&client, &token).await.unwrap().expired);
    assert_eq!(
        recovered
            .workspaces(None, 10)
            .await
            .unwrap()
            .workspaces
            .len(),
        1
    );
    drop(recovered);
    let old = WorkspaceProfile::open(&f.profile).unwrap();
    assert_eq!(old.info().unwrap().credential_id, original.credential_id);
    assert!(old.info().unwrap().expired);
    assert_eq!(f.count("ccp_clients"), 1);
    drop(old);
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn committed_recovery_and_failed_local_activation_resume_exactly_after_cli_restart() {
    let f = Fixture::new().await;
    let client = enroll_profile(&f).await.client_id.unwrap();
    let token = invitation(&f, &client, false);
    let dir = destination(&f).await;
    let file = private_invitation(&f, "recover.txt", &token);
    Connection::open(dir.join("workspace-profile.sqlite3")).unwrap().execute_batch("CREATE TRIGGER fail_recovery BEFORE UPDATE OF credential ON profile BEGIN SELECT RAISE(ABORT,'fixture activation'); END;").unwrap();
    let args = [
        "workspace",
        "recover",
        "--profile",
        dir.to_str().unwrap(),
        "--expected-client-id",
        &client,
        "--invitation-file",
        file.to_str().unwrap(),
    ];
    assert!(!f.cli(&args).await.status.success());
    let saved = pending(&dir);
    assert_eq!(saved["operation"], "recovery");
    assert_eq!(saved["expected_client_id"], client);
    assert_eq!(f.count("ccp_credentials"), 2);
    Connection::open(dir.join("workspace-profile.sqlite3"))
        .unwrap()
        .execute_batch("DROP TRIGGER fail_recovery")
        .unwrap();
    let done = value(f.cli(&args).await);
    assert_eq!(done["client_id"], client);
    assert_eq!(done["pending_operation"], Value::Null);
    assert_eq!(f.count("ccp_clients"), 1);
    assert_eq!(f.count("ccp_credentials"), 2);
    let recorded: String = f
        .db()
        .query_row("SELECT request_id FROM ccp_recovery_receipts", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(recorded, saved["request_id"].as_str().unwrap());
    assert!(!done.to_string().contains(&token));
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recovery_target_mismatch_never_activates_and_cannot_replace_pending_intent() {
    let f = Fixture::new().await;
    let client = enroll_profile(&f).await.client_id.unwrap();
    let token = invitation(&f, &client, false);
    let wrong = Uuid::new_v4().to_string();
    let dir = destination(&f).await;
    let mut recovered = WorkspaceProfile::open(&dir).unwrap();
    assert!(recovered.recover(&wrong, &token).await.is_err());
    let info = recovered.info().unwrap();
    assert!(info.client_id.is_none());
    assert_eq!(info.pending_client_id.as_deref(), Some(wrong.as_str()));
    assert!(recovered.recover(&client, &token).await.is_err());
    assert!(
        recovered
            .recover(&wrong, &format!("ccp_recover_{}", "f".repeat(64)))
            .await
            .is_err()
    );
    assert!(
        recovered
            .enroll(&format!("ccp_join_{}", "d".repeat(64)))
            .await
            .is_err()
    );
    assert_eq!(
        recovered.info().unwrap().pending_request_id,
        info.pending_request_id
    );
    assert_eq!(f.count("ccp_credentials"), 2);
    drop(recovered);
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn current_disable_issuer_loss_or_revoked_grants_deny_recovery_without_registration() {
    for cause in ["disabled", "issuer", "grant"] {
        let f = Fixture::new().await;
        let client = enroll_profile(&f).await.client_id.unwrap();
        let token = invitation(&f, &client, false);
        let dir = destination(&f).await;
        let db = f.db();
        match cause {
            "disabled" => db.execute("UPDATE ccp_clients SET disabled=1", []).unwrap(),
            "issuer" => db
                .execute("UPDATE ccp_admin_keys SET revoked_at=?1", [clock()])
                .unwrap(),
            _ => db
                .execute("UPDATE ccp_grants SET revoked_at=?1", [clock()])
                .unwrap(),
        };
        drop(db);
        let mut recovered = WorkspaceProfile::open(&dir).unwrap();
        assert!(recovered.recover(&client, &token).await.is_err());
        assert!(recovered.info().unwrap().client_id.is_none());
        assert_eq!(
            recovered.info().unwrap().pending_operation.as_deref(),
            Some("recovery")
        );
        assert_eq!(f.count("ccp_clients"), 1);
        assert_eq!(f.count("ccp_credentials"), 1);
        drop(recovered);
        f.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn global_recovery_restores_all_captured_workspaces_without_a_new_client() {
    let f = Fixture::new().await;
    let client = enroll_profile(&f).await.client_id.unwrap();
    let (join, _) = second_invitation(&f);
    WorkspaceProfile::open(&f.profile)
        .unwrap()
        .join(&join)
        .await
        .unwrap();
    let token = invitation(&f, &client, true);
    let dir = destination(&f).await;
    let mut recovered = WorkspaceProfile::open(&dir).unwrap();
    recovered.recover(&client, &token).await.unwrap();
    assert_eq!(
        recovered
            .workspaces(None, 10)
            .await
            .unwrap()
            .workspaces
            .len(),
        2
    );
    drop(recovered);
    let mut old = WorkspaceProfile::open(&f.profile).unwrap();
    assert!(
        old.workspaces(None, 10)
            .await
            .unwrap()
            .workspaces
            .is_empty()
    );
    assert_eq!(f.count("ccp_clients"), 1);
    assert_eq!(f.count("ccp_credentials"), 2);
    drop(old);
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recovery_rechecks_pinned_instance_before_sending_its_fresh_invitation() {
    let f = Fixture::new().await;
    let client = enroll_profile(&f).await.client_id.unwrap();
    let token = invitation(&f, &client, false);
    let dir = destination(&f).await;
    let db = Connection::open(dir.join("workspace-profile.sqlite3")).unwrap();
    let raw: String = db
        .query_row("SELECT trust FROM profile", [], |r| r.get(0))
        .unwrap();
    let mut trust: Value = serde_json::from_str(&raw).unwrap();
    trust["instance_id"] = json!(Uuid::new_v4().to_string());
    db.execute("UPDATE profile SET trust=?1", [trust.to_string()])
        .unwrap();
    drop(db);
    let mut recovered = WorkspaceProfile::open(&dir).unwrap();
    assert!(
        recovered
            .recover(&client, &token)
            .await
            .err()
            .unwrap()
            .to_string()
            .contains("instance")
    );
    assert_eq!(f.count("ccp_recovery_receipts"), 0);
    assert_eq!(f.count("ccp_credentials"), 1);
    assert!(recovered.info().unwrap().client_id.is_none());
    drop(recovered);
    f.finish().await;
}
