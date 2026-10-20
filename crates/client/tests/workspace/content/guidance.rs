// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use client::workspace::guidance as boards;

fn publish(f: &Fixture, workspace: Option<Uuid>, expected_revision: u64, text: &str) {
    use server::foundation::{GuidanceSave, GuidanceScope};
    let mut db = f.db();
    let mut store = AccessStore::new(&mut db).unwrap();
    let key = if workspace.is_some() {
        f.setup.owner_key.secret()
    } else {
        f.setup.instance_key.secret()
    };
    let actor = store.authenticate_admin_key(key, clock()).unwrap();
    let scope = workspace.map_or(GuidanceScope::Global, GuidanceScope::Workspace);
    store
        .save_guidance(
            actor,
            scope,
            &GuidanceSave {
                operation_id: Uuid::new_v4(),
                expected_revision,
                text: text.into(),
            },
            clock(),
        )
        .unwrap();
}
async fn cli(f: &Fixture, workspace: Option<&str>) -> Output {
    let mut args = vec![
        "workspace",
        "guidance",
        "--profile",
        f.profile.to_str().unwrap(),
    ];
    if let Some(workspace) = workspace {
        args.extend(["--workspace-id", workspace]);
    }
    f.cli(&args).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guidance_library_and_cli_preserve_exact_sources_profile_and_independent_selection() {
    let f = Fixture::new().await;
    enroll_profile(&f).await;
    let ws = f.setup.workspace_id.to_string();
    let mut p = WorkspaceProfile::open(&f.profile).unwrap();
    let dir = f.root.join("selection");
    let selection = p.initialize_selection(&dir).unwrap();
    let initial = p.guidance(Some(&ws)).await.unwrap();
    assert_eq!(initial.global.revision, 0);
    assert!(initial.global.publisher.is_none());
    assert_eq!(initial.workspace.unwrap().revision, 0);
    publish(&f, None, 0, "global");
    publish(&f, Some(f.setup.workspace_id), 0, &"\0".repeat(8192));
    let bundle = p.guidance(Some(&ws)).await.unwrap();
    assert!(matches!(bundle.precedence, boards::Precedence::GlobalFirst));
    assert_eq!(bundle.global.text, "global");
    let board = bundle.workspace.unwrap();
    assert_eq!(board.text, "\0".repeat(8192));
    let publisher = board.publisher.unwrap();
    assert!(matches!(publisher.actor_kind, boards::PublisherKind::Key));
    assert_eq!(publisher.actor_id, f.setup.owner_key.id.to_string());
    assert_eq!(
        publisher.principal_id,
        f.setup.owner_key.principal_id.to_string()
    );
    assert!(p.guidance(None).await.unwrap().workspace.is_none());
    drop(p);
    let before = fs::read(f.profile.join("workspace-profile.sqlite3")).unwrap();
    let choice = fs::read(dir.join("caller-selection.sqlite3")).unwrap();
    assert_eq!(
        value(cli(&f, Some(&ws)).await)["workspace"]["text"],
        "\0".repeat(8192)
    );
    assert!(value(cli(&f, None).await)["workspace"].is_null());
    publish(&f, Some(f.setup.workspace_id), 1, "");
    let cleared = value(cli(&f, Some(&ws)).await);
    assert_eq!(cleared["workspace"]["text"], "");
    assert_eq!(cleared["workspace"]["revision"], 2);
    assert!(!cleared["workspace"]["publisher"].is_null());
    f.db()
        .execute(
            "UPDATE ccp_workspaces SET state='archived' WHERE id=?1",
            [&ws],
        )
        .unwrap();
    assert_eq!(value(cli(&f, Some(&ws)).await)["workspace"]["revision"], 2);
    assert_eq!(
        fs::read(f.profile.join("workspace-profile.sqlite3")).unwrap(),
        before
    );
    assert_eq!(
        fs::read(dir.join("caller-selection.sqlite3")).unwrap(),
        choice
    );
    let p = WorkspaceProfile::open(&f.profile).unwrap();
    assert_eq!(p.selection(&dir).unwrap().revision, selection.revision);
    drop(p);
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guidance_renews_only_after_a_valid_authorized_workspace_read_not_grantless_global_read() {
    for mode in ["authorized", "denied", "global"] {
        let f = Fixture::new().await;
        WorkspaceProfile::trust(&f.profile, f.config())
            .await
            .unwrap();
        short_credential(&f, false);
        let mut p = WorkspaceProfile::open(&f.profile).unwrap();
        let old = p.info().unwrap();
        assert!(old.renewal_due);
        if mode != "authorized" {
            f.db()
                .execute("UPDATE ccp_grants SET revoked_at=?1", [clock()])
                .unwrap();
        }
        let ws = f.setup.workspace_id.to_string();
        let target = if mode == "global" {
            None
        } else {
            Some(ws.as_str())
        };
        let result = p.guidance(target).await;
        assert_eq!(result.is_ok(), mode != "denied");
        if mode == "denied" {
            assert!(result.err().unwrap().to_string().contains("403"));
        }
        let next = p.info().unwrap();
        assert_eq!(next.client_id, old.client_id);
        assert_eq!(
            next.credential_id == old.credential_id,
            mode != "authorized"
        );
        assert_eq!(
            f.count("ccp_credentials"),
            if mode == "authorized" { 2 } else { 1 }
        );
        assert!(next.pending_request_id.is_none());
        if mode != "denied" {
            p.guidance(target).await.unwrap();
        }
        assert_eq!(
            f.count("ccp_credentials"),
            if mode == "authorized" { 2 } else { 1 }
        );
        drop(p);
        f.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guidance_resumes_existing_renewal_and_preserves_pending_admission_and_entry_intent() {
    let f = Fixture::new().await;
    let old = enroll_profile(&f).await;
    let r = records(&f, &old);
    let ws = f.setup.workspace_id.to_string();
    fail_activation(&f);
    let mut p = WorkspaceProfile::open(&f.profile).unwrap();
    assert!(p.renew().await.is_err());
    drop(p);
    assert_eq!(f.count("ccp_credentials"), 2);
    allow_activation(&f);
    let mut p = WorkspaceProfile::open(&f.profile).unwrap();
    assert_eq!(
        p.guidance(Some(&ws))
            .await
            .unwrap()
            .workspace
            .unwrap()
            .revision,
        0
    );
    assert_eq!(f.count("ccp_credentials"), 2);
    assert!(p.info().unwrap().pending_request_id.is_none());
    drop(p);
    let (token, _) = second_invitation(&f);
    fail_activation(&f);
    let mut p = WorkspaceProfile::open(&f.profile).unwrap();
    assert!(p.join(&token).await.is_err());
    let pending = p.info().unwrap().pending_request_id;
    p.guidance(Some(&ws)).await.unwrap();
    assert_eq!(p.info().unwrap().pending_request_id, pending);
    drop(p);
    allow_activation(&f);
    let mut p = WorkspaceProfile::open(&f.profile).unwrap();
    p.join(&token).await.unwrap();
    drop(p);
    Connection::open(f.profile.join("workspace-profile.sqlite3")).unwrap().execute_batch("CREATE TRIGGER fail_guidance_finish BEFORE UPDATE ON content_state WHEN NEW.pending IS NULL BEGIN SELECT RAISE(ABORT,'fixture local save'); END;").unwrap();
    let mut p = WorkspaceProfile::open(&f.profile).unwrap();
    assert!(
        p.mutate_entry(api::EntryMutation {
            workspace_id: ws.clone(),
            operation_id: Uuid::new_v4().to_string(),
            agent: None,
            change: api::EntryChange::Append {
                entry_id: r.entry,
                expected_revision: 2,
                text: "+pending".into()
            }
        })
        .await
        .is_err()
    );
    let intent = serde_json::to_value(p.info().unwrap().pending_content).unwrap();
    let credential = p.info().unwrap().credential_id;
    p.guidance(Some(&ws)).await.unwrap();
    p.guidance(None).await.unwrap();
    assert_eq!(
        serde_json::to_value(p.info().unwrap().pending_content).unwrap(),
        intent
    );
    assert_eq!(p.info().unwrap().credential_id, credential);
    drop(p);
    assert_eq!(value(cli(&f, Some(&ws)).await)["workspace"]["revision"], 0);
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guidance_current_disable_foreign_scope_and_invalid_target_refuse_without_profile_changes()
{
    let f = Fixture::new().await;
    enroll_profile(&f).await;
    let foreign = Uuid::new_v4().to_string();
    let ws = f.setup.workspace_id.to_string();
    let before = fs::read(f.profile.join("workspace-profile.sqlite3")).unwrap();
    let mut p = WorkspaceProfile::open(&f.profile).unwrap();
    assert!(
        p.guidance(Some(&foreign))
            .await
            .err()
            .unwrap()
            .to_string()
            .contains("403")
    );
    assert!(p.guidance(Some("bad")).await.is_err());
    assert!(p.guidance(Some(&Uuid::nil().to_string())).await.is_err());
    f.db()
        .execute("UPDATE ccp_clients SET disabled=1", [])
        .unwrap();
    assert!(
        p.guidance(None)
            .await
            .err()
            .unwrap()
            .to_string()
            .contains("403")
    );
    assert!(p.guidance(Some(&ws)).await.is_err());
    drop(p);
    let failed = cli(&f, Some(&ws)).await;
    assert!(!failed.status.success());
    assert!(failed.stdout.is_empty());
    assert_eq!(
        fs::read(f.profile.join("workspace-profile.sqlite3")).unwrap(),
        before
    );
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guidance_scoped_recovery_reads_only_captured_grants_without_changing_original_profile() {
    use server::foundation::{GuidanceSave, GuidanceScope};
    let f = Fixture::new().await;
    let info = enroll_profile(&f).await;
    let (token, second) = second_invitation(&f);
    WorkspaceProfile::open(&f.profile)
        .unwrap()
        .join(&token)
        .await
        .unwrap();
    publish(&f, Some(f.setup.workspace_id), 0, "first workspace");
    {
        let mut db = f.db();
        let mut store = AccessStore::new(&mut db).unwrap();
        store
            .save_guidance(
                Actor::Admin(f.setup.owner_key.principal_id),
                GuidanceScope::Workspace(second),
                &GuidanceSave {
                    operation_id: Uuid::new_v4(),
                    expected_revision: 0,
                    text: "second workspace".into(),
                },
                clock(),
            )
            .unwrap();
    }
    let raw = fs::read(f.profile.join("workspace-profile.sqlite3")).unwrap();
    let client = info.client_id.as_deref().unwrap();
    let token = recovery::invitation(&f, client, false);
    let destination = recovery::destination(&f).await;
    let mut recovered = WorkspaceProfile::open(&destination).unwrap();
    recovered.recover(client, &token).await.unwrap();
    assert_eq!(
        recovered
            .guidance(Some(&f.setup.workspace_id.to_string()))
            .await
            .unwrap()
            .workspace
            .unwrap()
            .text,
        "first workspace"
    );
    assert!(
        recovered
            .guidance(Some(&second.to_string()))
            .await
            .err()
            .unwrap()
            .to_string()
            .contains("403")
    );
    assert!(recovered.guidance(None).await.unwrap().workspace.is_none());
    drop(recovered);
    assert_eq!(
        fs::read(f.profile.join("workspace-profile.sqlite3")).unwrap(),
        raw
    );
    let mut original = WorkspaceProfile::open(&f.profile).unwrap();
    assert!(
        original
            .guidance(Some(&f.setup.workspace_id.to_string()))
            .await
            .is_err()
    );
    let board = original
        .guidance(Some(&second.to_string()))
        .await
        .unwrap()
        .workspace
        .unwrap();
    assert_eq!(board.text, "second workspace");
    assert!(matches!(
        board.publisher.unwrap().actor_kind,
        boards::PublisherKind::Admin
    ));
    assert_eq!(original.info().unwrap().credential_id, info.credential_id);
    drop(original);
    assert_eq!(f.count("ccp_clients"), 1);
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn malformed_guidance_over_real_tls_cannot_trigger_renewal_or_echo_remote_text() {
    let f = Fixture::new().await;
    WorkspaceProfile::trust(&f.profile, f.config())
        .await
        .unwrap();
    short_credential(&f, false);
    // Simulate host-corrupted publication metadata without weakening production guards.
    let db = f.db();
    db.execute("INSERT INTO ccp_guidance_versions(board_key,revision,operation_id,text,publisher_kind,publisher_id,principal_id,recorded_at) VALUES('global',1,?1,'PRIVATE_GUIDANCE','key',?2,?3,?4)",rusqlite::params![Uuid::new_v4().to_string(),Uuid::nil().to_string(),f.setup.owner_key.principal_id.to_string(),clock()]).unwrap();
    db.execute(
        "INSERT INTO ccp_guidance_heads(board_key,revision) VALUES('global',1)",
        [],
    )
    .unwrap();
    drop(db);
    let raw = fs::read(f.profile.join("workspace-profile.sqlite3")).unwrap();
    let mut p = WorkspaceProfile::open(&f.profile).unwrap();
    let before = p.info().unwrap();
    assert!(before.renewal_due);
    let error = p
        .guidance(Some(&f.setup.workspace_id.to_string()))
        .await
        .err()
        .unwrap()
        .to_string();
    assert!(!error.contains("PRIVATE_GUIDANCE"));
    assert!(error.contains("guidance"));
    let after = p.info().unwrap();
    assert_eq!(after.credential_id, before.credential_id);
    assert!(after.pending_request_id.is_none());
    drop(p);
    let failed = cli(&f, Some(&f.setup.workspace_id.to_string())).await;
    assert!(!failed.status.success());
    assert!(failed.stdout.is_empty());
    assert!(!String::from_utf8_lossy(&failed.stderr).contains("PRIVATE_GUIDANCE"));
    assert_eq!(f.count("ccp_credentials"), 1);
    assert_eq!(
        fs::read(f.profile.join("workspace-profile.sqlite3")).unwrap(),
        raw
    );
    f.finish().await;
}
