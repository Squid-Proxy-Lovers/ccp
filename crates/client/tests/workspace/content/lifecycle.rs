// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scoped_recovery_profiles_read_only_their_own_current_content_grants() {
    let f = Fixture::new().await;
    let info = enroll_profile(&f).await;
    let r = records(&f, &info);
    let (join, second) = second_invitation(&f);
    WorkspaceProfile::open(&f.profile)
        .unwrap()
        .join(&join)
        .await
        .unwrap();
    let original_bytes = fs::read(f.profile.join("workspace-profile.sqlite3")).unwrap();
    let client = info.client_id.as_deref().unwrap();
    let token = recovery::invitation(&f, client, false);
    let destination = recovery::destination(&f).await;
    let mut recovered = WorkspaceProfile::open(&destination).unwrap();
    recovered.recover(client, &token).await.unwrap();
    assert_eq!(
        recovered
            .content_entry(&entry_request(&f, &r))
            .await
            .unwrap()
            .entry
            .owner_client_id
            .as_deref(),
        Some(client)
    );
    let request = api::ShelfPage {
        workspace_id: second.to_string(),
        after: None,
        limit: None,
    };
    assert!(
        recovered
            .content_shelves(&request)
            .await
            .err()
            .unwrap()
            .to_string()
            .contains("403")
    );
    drop(recovered);
    assert_eq!(
        fs::read(f.profile.join("workspace-profile.sqlite3")).unwrap(),
        original_bytes
    );
    let mut original = WorkspaceProfile::open(&f.profile).unwrap();
    assert!(
        original
            .content_entry(&entry_request(&f, &r))
            .await
            .err()
            .unwrap()
            .to_string()
            .contains("403")
    );
    assert!(
        original
            .content_shelves(&request)
            .await
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(original.info().unwrap().credential_id, info.credential_id);
    assert_eq!(f.count("ccp_clients"), 1);
    drop(original);
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn authorized_empty_content_page_renews_once_but_a_denied_target_never_starts_renewal() {
    for authorized in [true, false] {
        let f = Fixture::new().await;
        WorkspaceProfile::trust(&f.profile, f.config())
            .await
            .unwrap();
        short_credential(&f, false);
        let mut p = WorkspaceProfile::open(&f.profile).unwrap();
        let old = p.info().unwrap();
        assert!(old.renewal_due);
        let r = records(&f, &old);
        if !authorized {
            f.db()
                .execute("UPDATE ccp_grants SET revoked_at=?1", [clock()])
                .unwrap();
        }
        let request = api::ShelfPage {
            workspace_id: f.setup.workspace_id.to_string(),
            after: Some(r.shelf),
            limit: Some(1),
        };
        let result = p.content_shelves(&request).await;
        if authorized {
            assert!(result.unwrap().items.is_empty());
        } else {
            assert!(result.is_err());
        }
        let next = p.info().unwrap();
        assert_eq!(next.client_id, old.client_id);
        assert_eq!(next.credential_id == old.credential_id, !authorized);
        assert_eq!(f.count("ccp_credentials"), if authorized { 2 } else { 1 });
        assert!(next.pending_request_id.is_none());
        if authorized {
            p.content_shelves(&request).await.unwrap();
            assert_eq!(f.count("ccp_credentials"), 2);
        }
        drop(p);
        f.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn content_reads_resume_exact_committed_renewal_and_preserve_pending_admission() {
    let f = Fixture::new().await;
    let old = enroll_profile(&f).await;
    let r = records(&f, &old);
    fail_activation(&f);
    let mut p = WorkspaceProfile::open(&f.profile).unwrap();
    assert!(p.renew().await.is_err());
    assert_eq!(
        p.info().unwrap().pending_operation.as_deref(),
        Some("renewal")
    );
    assert_eq!(f.count("ccp_credentials"), 2);
    drop(p);
    allow_activation(&f);
    let mut p = WorkspaceProfile::open(&f.profile).unwrap();
    assert_eq!(
        p.content_entry(&entry_request(&f, &r))
            .await
            .unwrap()
            .content
            .text,
        "original+contribution"
    );
    let renewed = p.info().unwrap();
    assert_eq!(renewed.client_id, old.client_id);
    assert_ne!(renewed.credential_id, old.credential_id);
    assert!(renewed.pending_request_id.is_none());
    assert_eq!(f.count("ccp_credentials"), 2);
    drop(p);
    let (token, _) = second_invitation(&f);
    fail_activation(&f);
    let mut p = WorkspaceProfile::open(&f.profile).unwrap();
    assert!(p.join(&token).await.is_err());
    let pending = p.info().unwrap().pending_request_id;
    assert_eq!(
        p.content_entry(&entry_request(&f, &r))
            .await
            .unwrap()
            .content
            .text,
        "original+contribution"
    );
    assert_eq!(p.info().unwrap().pending_request_id, pending);
    assert_eq!(p.info().unwrap().credential_id, renewed.credential_id);
    drop(p);
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn content_reads_enforce_current_read_only_archive_revocation_and_disable_without_fallback() {
    for disable in [true, false] {
        let f = Fixture::new().await;
        let info = enroll_profile(&f).await;
        let r = records(&f, &info);
        f.db()
            .execute("UPDATE ccp_grants SET role='read'", [])
            .unwrap();
        f.db()
            .execute(
                "UPDATE ccp_workspaces SET state='archived' WHERE id=?1",
                [f.setup.workspace_id.to_string()],
            )
            .unwrap();
        let mut p = WorkspaceProfile::open(&f.profile).unwrap();
        assert_eq!(
            p.content_entry(&entry_request(&f, &r))
                .await
                .unwrap()
                .content
                .text,
            "original+contribution"
        );
        if disable {
            f.db()
                .execute("UPDATE ccp_clients SET disabled=1", [])
                .unwrap();
        } else {
            f.db()
                .execute("UPDATE ccp_grants SET revoked_at=?1", [clock()])
                .unwrap();
        }
        let error = p
            .content_entry(&entry_request(&f, &r))
            .await
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("403"));
        assert!(!error.contains("original"));
        assert_eq!(p.info().unwrap().credential_id, info.credential_id);
        assert_eq!(f.count("ccp_clients"), 1);
        assert_eq!(f.count("ccp_credentials"), 1);
        drop(p);
        f.finish().await;
    }
}
