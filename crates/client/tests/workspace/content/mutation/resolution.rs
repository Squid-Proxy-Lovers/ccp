// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use sha2::{Digest, Sha256};

pub(super) fn stage(f: &Fixture, request: &api::EntryMutation) {
    let info = WorkspaceProfile::open(&f.profile).unwrap().info().unwrap();
    let hash = format!(
        "{:x}",
        Sha256::digest(
            protocol::workspace::content::fingerprint_bytes(
                &request.change,
                request.agent.as_deref()
            )
            .unwrap()
        )
    );
    let value = json!({"binding":{"instance_id":info.instance_id,"client_id":info.client_id,"credential_id":info.credential_id,"workspace_id":request.workspace_id,"operation_id":request.operation_id,"request_hash":hash},"request":request});
    Connection::open(f.profile.join("workspace-profile.sqlite3"))
        .unwrap()
        .execute("UPDATE content_state SET pending=?1", [value.to_string()])
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pending_short_credential_can_read_without_renewal_and_resolution_cancels_late_replay() {
    let f = Fixture::new().await;
    WorkspaceProfile::trust(&f.profile, f.config())
        .await
        .unwrap();
    short_credential(&f, false);
    let info = WorkspaceProfile::open(&f.profile).unwrap().info().unwrap();
    let r = records(&f, &info);
    let request = append(&f, &r);
    stage(&f, &request);
    let mut p = WorkspaceProfile::open(&f.profile).unwrap();
    assert!(p.info().unwrap().renewal_due);
    assert_eq!(
        p.content_entry(&entry_request(&f, &r))
            .await
            .unwrap()
            .entry
            .revision,
        2
    );
    assert_eq!(p.workspaces(None, 10).await.unwrap().workspaces.len(), 1);
    assert!(p.renew().await.is_err());
    assert_eq!(f.count("ccp_credentials"), 1);
    f.db()
        .execute("UPDATE ccp_grants SET role='read'", [])
        .unwrap();
    f.db()
        .execute("UPDATE ccp_workspaces SET state='archived'", [])
        .unwrap();
    assert!(matches!(
        p.resolve_entry_operation().await.unwrap().outcome,
        api::EntryOperationOutcome::Cancelled
    ));
    assert!(p.pending_entry_operation().is_none());
    assert!(p.mutate_entry(copy(&request)).await.is_err());
    assert_eq!(f.count("ccp_content_operations"), 2);
    assert_eq!(f.count("ccp_content_cancellations"), 1);
    assert_eq!(p.info().unwrap().credential_id, info.credential_id);
    drop(p);
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn latest_receipt_retries_use_original_binding_after_rotation_and_older_nonce_conflicts_safely()
 {
    let f = Fixture::new().await;
    let info = enroll_profile(&f).await;
    let r = records(&f, &info);
    let request = append(&f, &r);
    let mut p = WorkspaceProfile::open(&f.profile).unwrap();
    assert_eq!(p.mutate_entry(copy(&request)).await.unwrap().revision, 3);
    p.renew().await.unwrap();
    assert_ne!(p.info().unwrap().credential_id, info.credential_id);
    assert_eq!(p.mutate_entry(copy(&request)).await.unwrap().revision, 3);
    assert_eq!(
        p.info().unwrap().last_content.unwrap().credential_id,
        info.credential_id.unwrap()
    );
    let mut changed = copy(&request);
    changed.agent = Some("changed".into());
    assert!(p.mutate_entry(changed).await.is_err());
    let second = self::request(
        &f,
        api::EntryChange::Append {
            entry_id: r.entry.clone(),
            expected_revision: 3,
            text: "+second".into(),
        },
    );
    assert_eq!(p.mutate_entry(second).await.unwrap().revision, 4);
    assert!(p.mutate_entry(copy(&request)).await.is_err());
    assert!(p.pending_entry_operation().is_none());
    assert_eq!(p.info().unwrap().last_content.unwrap().state, "conflict");
    assert_eq!(
        p.content_entry(&entry_request(&f, &r))
            .await
            .unwrap()
            .content
            .text,
        "original+contribution+PRIVATE_BODY+second"
    );
    assert_eq!(f.count("ccp_content_operations"), 4);
    drop(p);
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scoped_recovery_resolves_original_committed_intent_without_copying_or_unioning_authority()
{
    let f = Fixture::new().await;
    let info = enroll_profile(&f).await;
    let r = records(&f, &info);
    let (join, second) = second_invitation(&f);
    WorkspaceProfile::open(&f.profile)
        .unwrap()
        .join(&join)
        .await
        .unwrap();
    fail_finish(&f);
    let mut p = WorkspaceProfile::open(&f.profile).unwrap();
    assert!(p.mutate_entry(append(&f, &r)).await.is_err());
    drop(p);
    profile_sql(&f, "DROP TRIGGER fail_content_finish");
    let token = recovery::invitation(&f, info.client_id.as_deref().unwrap(), false);
    let dir = recovery::destination(&f).await;
    let mut recovered = WorkspaceProfile::open(&dir).unwrap();
    recovered
        .recover(info.client_id.as_deref().unwrap(), &token)
        .await
        .unwrap();
    let mut p = WorkspaceProfile::open(&f.profile).unwrap();
    assert!(p.resolve_entry_operation().await.is_err());
    assert!(p.pending_entry_operation().is_some());
    let result = p.resolve_entry_operation_via(&mut recovered).await.unwrap();
    assert!(matches!(
        result.outcome,
        api::EntryOperationOutcome::Committed { revision: 3, .. }
    ));
    assert_eq!(result.credential_id, info.credential_id.clone().unwrap());
    assert_eq!(p.info().unwrap().credential_id, info.credential_id);
    assert!(p.content_entry(&entry_request(&f, &r)).await.is_err());
    let page = api::ShelfPage {
        workspace_id: second.to_string(),
        after: None,
        limit: None,
    };
    assert!(p.content_shelves(&page).await.is_ok());
    assert!(recovered.content_shelves(&page).await.is_err());
    assert_eq!(f.count("ccp_clients"), 1);
    assert_eq!(f.count("ccp_content_operations"), 3);
    drop(p);
    drop(recovered);
    let cli_result = value(cli(&f, &["resolve", "--via-profile", dir.to_str().unwrap()]).await);
    assert_eq!(cli_result["credential_id"], info.credential_id.unwrap());
    assert_eq!(cli_result["outcome"]["revision"], 3);
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn expired_original_can_resolve_only_through_current_authorized_same_client_profile() {
    let f = Fixture::new().await;
    WorkspaceProfile::trust(&f.profile, f.config())
        .await
        .unwrap();
    short_credential(&f, true);
    let info = WorkspaceProfile::open(&f.profile).unwrap().info().unwrap();
    let r = Records {
        shelf: Uuid::new_v4().to_string(),
        book: Uuid::new_v4().to_string(),
        entry: Uuid::new_v4().to_string(),
    };
    let request = append(&f, &r);
    stage(&f, &request);
    let token = recovery::invitation(&f, info.client_id.as_deref().unwrap(), false);
    let dir = recovery::destination(&f).await;
    let mut recovered = WorkspaceProfile::open(&dir).unwrap();
    recovered
        .recover(info.client_id.as_deref().unwrap(), &token)
        .await
        .unwrap();
    let mut original = WorkspaceProfile::open(&f.profile).unwrap();
    assert!(original.resolve_entry_operation().await.is_err());
    assert!(original.pending_entry_operation().is_some());
    f.db()
        .execute("UPDATE ccp_grants SET revoked_at=?1", [clock()])
        .unwrap();
    assert!(
        original
            .resolve_entry_operation_via(&mut recovered)
            .await
            .is_err()
    );
    assert!(original.pending_entry_operation().is_some());
    f.db()
        .execute("UPDATE ccp_grants SET revoked_at=NULL", [])
        .unwrap();
    assert!(matches!(
        original
            .resolve_entry_operation_via(&mut recovered)
            .await
            .unwrap()
            .outcome,
        api::EntryOperationOutcome::Cancelled
    ));
    assert!(original.info().unwrap().expired);
    assert_eq!(original.info().unwrap().credential_id, info.credential_id);
    assert_eq!(f.count("ccp_content_operations"), 0);
    drop(original);
    drop(recovered);
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn private_cli_input_and_writer_ownership_conflicts_do_not_expose_bodies_or_choose_new_nonces()
 {
    let f = Fixture::new().await;
    let info = enroll_profile(&f).await;
    let r = records(&f, &info);
    let original = request(
        &f,
        api::EntryChange::Revise {
            entry_id: r.entry.clone(),
            expected_revision: 1,
            content: api::EntryContent {
                text: "PRIVATE_BODY".into(),
                description: String::new(),
                labels: vec![],
            },
        },
    );
    let file = private_invitation(&f, "stale.json", &serde_json::to_string(&original).unwrap());
    assert!(
        !cli(&f, &["mutate", "--request-file", file.to_str().unwrap()])
            .await
            .status
            .success()
    );
    assert!(value(cli(&f, &["pending"]).await).is_null());
    assert_eq!(f.count("ccp_content_operations"), 2);
    let bad = private_invitation(&f, "bad.json", r#"{"text":"PRIVATE_BODY"}"#);
    let result = cli(&f, &["mutate", "--request-file", bad.to_str().unwrap()]).await;
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
    assert!(!String::from_utf8_lossy(&result.stderr).contains("PRIVATE_BODY"));
    let large = private_invitation(&f, "oversize.json", &"x".repeat(2 * 1024 * 1024 + 1));
    let result = cli(&f, &["mutate", "--request-file", large.to_str().unwrap()]).await;
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
    assert_eq!(f.count("ccp_content_operations"), 2);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{PermissionsExt, symlink};
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(
            !cli(&f, &["mutate", "--request-file", file.to_str().unwrap()])
                .await
                .status
                .success()
        );
        let link = f.root.join("linked.json");
        symlink(&bad, &link).unwrap();
        assert!(
            !cli(&f, &["mutate", "--request-file", link.to_str().unwrap()])
                .await
                .status
                .success()
        );
    }
    let other = f.root.join("other-client");
    WorkspaceProfile::trust(&other, f.config()).await.unwrap();
    let mut p = WorkspaceProfile::open(&other).unwrap();
    let token = {
        let mut db = f.db();
        let mut store = AccessStore::new(&mut db).unwrap();
        let actor = store
            .authenticate_admin_key(f.setup.owner_key.secret(), clock())
            .unwrap();
        store
            .issue_individual_invitation(
                actor,
                server::foundation::IndividualInvitationRequest {
                    workspace_id: f.setup.workspace_id,
                    role: DataRole::ReadWrite,
                    expires_at: clock() + 3600,
                },
                clock(),
            )
            .unwrap()
            .secret()
            .to_owned()
    };
    p.enroll(&token).await.unwrap();
    let request = request(
        &f,
        api::EntryChange::Archive {
            entry_id: r.entry.clone(),
            expected_revision: 2,
        },
    );
    assert!(p.mutate_entry(request).await.is_err());
    assert!(p.pending_entry_operation().is_none());
    assert_eq!(p.mutate_entry(append(&f, &r)).await.unwrap().revision, 3);
    assert_eq!(
        p.content_entry(&entry_request(&f, &r))
            .await
            .unwrap()
            .entry
            .owner_client_id,
        info.client_id
    );
    drop(p);
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn terminal_retry_resumes_a_committed_pending_renewal_before_resolving_original_binding() {
    let f = Fixture::new().await;
    let info = enroll_profile(&f).await;
    let r = records(&f, &info);
    let request = append(&f, &r);
    let mut p = WorkspaceProfile::open(&f.profile).unwrap();
    assert_eq!(p.mutate_entry(copy(&request)).await.unwrap().revision, 3);
    drop(p);
    fail_activation(&f);
    let mut p = WorkspaceProfile::open(&f.profile).unwrap();
    assert!(p.renew().await.is_err());
    assert_eq!(
        p.info().unwrap().pending_operation.as_deref(),
        Some("renewal")
    );
    drop(p);
    allow_activation(&f);
    let mut p = WorkspaceProfile::open(&f.profile).unwrap();
    assert_eq!(p.mutate_entry(request).await.unwrap().revision, 3);
    assert!(p.info().unwrap().pending_operation.is_none());
    assert_ne!(p.info().unwrap().credential_id, info.credential_id);
    assert_eq!(
        p.info().unwrap().last_content.unwrap().credential_id,
        info.credential_id.unwrap()
    );
    assert_eq!(f.count("ccp_credentials"), 2);
    assert_eq!(f.count("ccp_content_operations"), 3);
    drop(p);
    f.finish().await;
}
