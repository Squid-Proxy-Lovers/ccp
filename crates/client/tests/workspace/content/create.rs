// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
mod faults;

fn shelf(f: &Fixture, name: &str) -> api::CreateShelf {
    api::CreateShelf {
        workspace_id: f.setup.workspace_id.to_string(),
        name: name.into(),
        description: "exact\nmetadata\0é".into(),
    }
}
fn book(f: &Fixture, parent: &str) -> api::CreateBook {
    api::CreateBook {
        workspace_id: f.setup.workspace_id.to_string(),
        shelf_id: parent.into(),
        name: " book ".into(),
        description: "book metadata".into(),
    }
}
async fn cli(f: &Fixture, action: &str, file: &std::path::Path) -> Output {
    f.cli(&[
        "workspace",
        "content",
        "--profile",
        f.profile.to_str().unwrap(),
        action,
        "--request-file",
        file.to_str().unwrap(),
    ])
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn actual_cli_builds_a_workspace_hierarchy_and_entry_without_overwriting_duplicate_names() {
    let f = Fixture::new().await;
    let info = enroll_profile(&f).await;
    assert_eq!(f.count("ccp_content_shelves"), 0);
    let request = shelf(&f, " notes ");
    let file = private_invitation(&f, "shelf.json", &serde_json::to_string(&request).unwrap());
    let created = value(cli(&f, "create-shelf", &file).await);
    assert_eq!(created["name"], "notes");
    assert_eq!(created["description"], request.description);
    assert_eq!(created["workspace_id"], request.workspace_id);
    let duplicate = cli(&f, "create-shelf", &file).await;
    assert!(!duplicate.status.success());
    assert!(duplicate.stdout.is_empty());
    assert!(!String::from_utf8_lossy(&duplicate.stderr).contains("metadata"));
    let request = book(&f, created["id"].as_str().unwrap());
    let file = private_invitation(&f, "book.json", &serde_json::to_string(&request).unwrap());
    let created_book = value(cli(&f, "create-book", &file).await);
    assert_eq!(created_book["shelf_id"], created["id"]);
    assert_eq!(created_book["name"], "book");
    assert!(!cli(&f, "create-book", &file).await.status.success());
    let mut p = WorkspaceProfile::open(&f.profile).unwrap();
    let entry = p
        .mutate_entry(api::EntryMutation {
            workspace_id: f.setup.workspace_id.to_string(),
            operation_id: Uuid::new_v4().to_string(),
            agent: Some("agent".into()),
            change: api::EntryChange::Create {
                book_id: created_book["id"].as_str().unwrap().into(),
                name: "entry".into(),
                content: api::EntryContent {
                    text: "human-readable content".into(),
                    description: String::new(),
                    labels: vec![],
                },
            },
        })
        .await
        .unwrap();
    let value = p
        .content_entry(&api::EntryTarget {
            workspace_id: f.setup.workspace_id.to_string(),
            entry_id: entry.entry_id,
            revision: None,
        })
        .await
        .unwrap();
    assert_eq!(value.content.text, "human-readable content");
    assert_eq!(value.entry.owner_client_id, info.client_id);
    assert_eq!(p.info().unwrap().credential_id, info.credential_id);
    assert_eq!(f.count("ccp_content_shelves"), 1);
    assert_eq!(f.count("ccp_content_books"), 1);
    drop(p);
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn library_creation_renews_before_post_and_uses_the_exact_explicit_parent_and_scope() {
    let f = Fixture::new().await;
    WorkspaceProfile::trust(&f.profile, f.config())
        .await
        .unwrap();
    short_credential(&f, false);
    let mut p = WorkspaceProfile::open(&f.profile).unwrap();
    let info = p.info().unwrap();
    assert!(info.renewal_due);
    let created = p.content_create_shelf(&shelf(&f, "研究")).await.unwrap();
    assert_ne!(p.info().unwrap().credential_id, info.credential_id);
    let created_book = p.content_create_book(&book(&f, &created.id)).await.unwrap();
    assert_eq!(created_book.shelf_id, created.id);
    assert_eq!(created_book.description, "book metadata");
    let mut maximum = book(&f, &created.id);
    maximum.name = "é".repeat(128);
    maximum.description = "\0".repeat(4096);
    let maximum_book = p.content_create_book(&maximum).await.unwrap();
    assert_eq!(maximum_book.name, maximum.name);
    assert_eq!(maximum_book.description, maximum.description);
    let (token, second) = second_invitation(&f);
    p.join(&token).await.unwrap();
    let mut wrong = book(&f, &created.id);
    wrong.workspace_id = second.to_string();
    assert!(p.content_create_book(&wrong).await.is_err());
    assert_eq!(f.count("ccp_content_books"), 2);
    assert_eq!(f.count("ccp_credentials"), 2);
    assert!(p.info().unwrap().pending_content.is_none());
    drop(p);
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn denied_preflight_does_not_start_renewal_or_fallback_and_current_read_only_cannot_create() {
    for cause in ["revoked", "disabled", "read", "archived"] {
        let f = Fixture::new().await;
        WorkspaceProfile::trust(&f.profile, f.config())
            .await
            .unwrap();
        short_credential(&f, false);
        let mut p = WorkspaceProfile::open(&f.profile).unwrap();
        let original = p.info().unwrap();
        let sql = match cause {
            "revoked" => "UPDATE ccp_grants SET revoked_at=1",
            "disabled" => "UPDATE ccp_clients SET disabled=1",
            "read" => "UPDATE ccp_grants SET role='read'",
            _ => "UPDATE ccp_workspaces SET state='archived'",
        };
        f.db().execute_batch(sql).unwrap();
        assert!(p.content_create_shelf(&shelf(&f, "denied")).await.is_err());
        assert_eq!(f.count("ccp_content_shelves"), 0);
        if matches!(cause, "revoked" | "disabled") {
            assert_eq!(f.count("ccp_credentials"), 1);
            assert_eq!(p.info().unwrap().credential_id, original.credential_id);
        }
        assert!(p.info().unwrap().pending_content.is_none());
        drop(p);
        f.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pending_entry_and_admission_block_creation_but_committed_pending_renewal_resumes_once() {
    let f = Fixture::new().await;
    let info = enroll_profile(&f).await;
    let r = records(&f, &info);
    Connection::open(f.profile.join("workspace-profile.sqlite3")).unwrap().execute_batch("CREATE TRIGGER fail_content BEFORE UPDATE ON content_state WHEN NEW.pending IS NULL BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    let mut p = WorkspaceProfile::open(&f.profile).unwrap();
    assert!(
        p.mutate_entry(api::EntryMutation {
            workspace_id: f.setup.workspace_id.to_string(),
            operation_id: Uuid::new_v4().to_string(),
            agent: None,
            change: api::EntryChange::Append {
                entry_id: r.entry,
                expected_revision: 2,
                text: "pending content".into()
            }
        })
        .await
        .is_err()
    );
    let pending = p.info().unwrap().pending_content.unwrap().operation_id;
    assert!(p.content_create_shelf(&shelf(&f, "blocked")).await.is_err());
    assert!(p.content_create_book(&book(&f, &r.shelf)).await.is_err());
    assert_eq!(
        p.info().unwrap().pending_content.unwrap().operation_id,
        pending
    );
    assert_eq!(f.count("ccp_content_shelves"), 1);
    assert_eq!(f.count("ccp_content_books"), 1);
    drop(p);
    Connection::open(f.profile.join("workspace-profile.sqlite3"))
        .unwrap()
        .execute_batch("DROP TRIGGER fail_content")
        .unwrap();
    let mut p = WorkspaceProfile::open(&f.profile).unwrap();
    p.resolve_entry_operation().await.unwrap();
    drop(p);
    fail_activation(&f);
    let mut p = WorkspaceProfile::open(&f.profile).unwrap();
    assert!(p.renew().await.is_err());
    drop(p);
    allow_activation(&f);
    let mut p = WorkspaceProfile::open(&f.profile).unwrap();
    p.content_create_shelf(&shelf(&f, "renewed")).await.unwrap();
    assert!(p.info().unwrap().pending_operation.is_none());
    assert_eq!(f.count("ccp_credentials"), 2);
    drop(p);
    let (token, _) = second_invitation(&f);
    fail_activation(&f);
    let mut p = WorkspaceProfile::open(&f.profile).unwrap();
    assert!(p.join(&token).await.is_err());
    let pending = p.info().unwrap().pending_request_id;
    assert!(
        p.content_create_shelf(&shelf(&f, "admission-blocked"))
            .await
            .is_err()
    );
    assert!(p.content_create_book(&book(&f, &r.shelf)).await.is_err());
    assert_eq!(p.info().unwrap().pending_request_id, pending);
    assert_eq!(f.count("ccp_content_shelves"), 2);
    assert_eq!(f.count("ccp_content_books"), 1);
    drop(p);
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invalid_or_nonprivate_cli_input_does_not_post_or_start_a_due_renewal() {
    let f = Fixture::new().await;
    WorkspaceProfile::trust(&f.profile, f.config())
        .await
        .unwrap();
    short_credential(&f, false);
    let mut p = WorkspaceProfile::open(&f.profile).unwrap();
    let mut request = shelf(&f, "bad\nname");
    assert!(p.content_create_shelf(&request).await.is_err());
    request.name = "valid".into();
    request.description = "x".repeat(4097);
    assert!(p.content_create_shelf(&request).await.is_err());
    assert!(p.content_create_book(&book(&f, "invalid")).await.is_err());
    assert_eq!(f.count("ccp_credentials"), 1);
    drop(p);
    for (index, text) in [
        r#"{"admin_key":"PRIVATE_VALUE"}"#.to_owned(),
        "x".repeat(64 * 1024 + 1),
    ]
    .into_iter()
    .enumerate()
    {
        let file = private_invitation(&f, &format!("bad-{index}.json"), &text);
        let result = cli(&f, "create-shelf", &file).await;
        assert!(!result.status.success());
        assert!(result.stdout.is_empty());
        assert!(!String::from_utf8_lossy(&result.stderr).contains("PRIVATE_VALUE"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let file = private_invitation(
            &f,
            "private.json",
            &serde_json::to_string(&shelf(&f, "valid")).unwrap(),
        );
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(!cli(&f, "create-shelf", &file).await.status.success());
        let linked = f.root.join("linked.json");
        symlink(&file, &linked).unwrap();
        assert!(!cli(&f, "create-shelf", &linked).await.status.success());
    }
    assert_eq!(f.count("ccp_credentials"), 1);
    assert_eq!(f.count("ccp_content_shelves"), 0);
    f.finish().await;
}
