// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

async fn context_cli(f: &Fixture, workspace: Option<&str>) -> Value {
    let mut args = vec![
        "workspace",
        "guidance",
        "--profile",
        f.profile.to_str().unwrap(),
        "--delivery-context",
    ];
    if let Some(workspace) = workspace {
        args.extend(["--workspace-id", workspace]);
    }
    value(f.cli(&args).await)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guidance_delivery_library_and_cli_are_bound_to_profile_without_changing_default_output() {
    let f = Fixture::new().await;
    let info = enroll_profile(&f).await;
    let ws = f.setup.workspace_id.to_string();
    publish(&f, Some(f.setup.workspace_id), 0, "current");
    let mut p = WorkspaceProfile::open(&f.profile).unwrap();
    let delivery = p.guidance_delivery(Some(&ws)).await.unwrap();
    assert_eq!(delivery.format, 1);
    assert_eq!(delivery.context.instance_id, f.trust.instance_id);
    assert_eq!(Some(delivery.context.client_id), info.client_id);
    assert_eq!(Some(delivery.context.credential_id), info.credential_id);
    assert!(delivery.context.workspace_grant_id.is_some());
    assert_eq!(delivery.guidance.workspace.unwrap().text, "current");
    drop(p);
    let before = fs::read(f.profile.join("workspace-profile.sqlite3")).unwrap();
    let wrapped = context_cli(&f, Some(&ws)).await;
    let plain = value(cli(&f, Some(&ws)).await);
    assert_eq!(wrapped["format"], 1);
    assert_eq!(wrapped["guidance"], plain);
    assert!(plain.get("context").is_none());
    assert!(context_cli(&f, None).await["context"]["workspace_grant_id"].is_null());
    assert_eq!(
        fs::read(f.profile.join("workspace-profile.sqlite3")).unwrap(),
        before
    );
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guidance_delivery_after_due_renewal_matches_the_new_credential_and_global_only_never_starts_one()
 {
    for global_only in [false, true] {
        let f = Fixture::new().await;
        WorkspaceProfile::trust(&f.profile, f.config())
            .await
            .unwrap();
        short_credential(&f, false);
        let mut p = WorkspaceProfile::open(&f.profile).unwrap();
        let before = p.info().unwrap();
        assert!(before.renewal_due);
        if global_only {
            f.db()
                .execute("UPDATE ccp_grants SET revoked_at=?1", [clock()])
                .unwrap();
        }
        let ws = f.setup.workspace_id.to_string();
        let target = if global_only { None } else { Some(ws.as_str()) };
        let value = p.guidance_delivery(target).await.unwrap();
        let after = p.info().unwrap();
        assert_eq!(Some(value.context.credential_id), after.credential_id);
        assert_eq!(after.credential_id == before.credential_id, global_only);
        assert_eq!(value.context.workspace_grant_id.is_none(), global_only);
        assert_eq!(f.count("ccp_credentials"), if global_only { 1 } else { 2 });
        p.guidance_delivery(target).await.unwrap();
        assert_eq!(f.count("ccp_credentials"), if global_only { 1 } else { 2 });
        drop(p);
        f.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guidance_delivery_resumes_pending_renewal_once_and_rejects_malformed_guidance_before_new_renewal()
 {
    let f = Fixture::new().await;
    let old = enroll_profile(&f).await;
    let ws = f.setup.workspace_id.to_string();
    fail_activation(&f);
    let mut p = WorkspaceProfile::open(&f.profile).unwrap();
    assert!(p.renew().await.is_err());
    drop(p);
    allow_activation(&f);
    let mut p = WorkspaceProfile::open(&f.profile).unwrap();
    let value = p.guidance_delivery(Some(&ws)).await.unwrap();
    assert_ne!(Some(value.context.credential_id.clone()), old.credential_id);
    assert_eq!(
        Some(value.context.credential_id),
        p.info().unwrap().credential_id
    );
    assert_eq!(f.count("ccp_credentials"), 2);
    drop(p);
    f.finish().await;

    let f = Fixture::new().await;
    WorkspaceProfile::trust(&f.profile, f.config())
        .await
        .unwrap();
    short_credential(&f, false);
    let db = f.db();
    db.execute("INSERT INTO ccp_guidance_versions(board_key,revision,operation_id,text,publisher_kind,publisher_id,principal_id,recorded_at) VALUES('global',1,?1,'PRIVATE_GUIDANCE','key',?2,?3,?4)",rusqlite::params![Uuid::new_v4().to_string(),Uuid::nil().to_string(),f.setup.owner_key.principal_id.to_string(),clock()]).unwrap();
    db.execute(
        "INSERT INTO ccp_guidance_heads(board_key,revision) VALUES('global',1)",
        [],
    )
    .unwrap();
    drop(db);
    let mut p = WorkspaceProfile::open(&f.profile).unwrap();
    let original = p.info().unwrap().credential_id;
    let error = p
        .guidance_delivery(Some(&f.setup.workspace_id.to_string()))
        .await
        .err()
        .unwrap()
        .to_string();
    assert!(!error.contains("PRIVATE_GUIDANCE"));
    assert_eq!(p.info().unwrap().credential_id, original);
    assert_eq!(f.count("ccp_credentials"), 1);
    drop(p);
    f.finish().await;
}
