// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use client::workspace::SelectionUpdate;

fn update(ids: &[String], default: Option<String>, revision: u64) -> SelectionUpdate {
    SelectionUpdate {
        workspace_ids: ids.to_vec(),
        default_workspace_id: default,
        expected_revision: revision,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn callers_keep_independent_choices_and_join_or_explicit_override_never_changes_them() {
    let f = Fixture::new().await;
    enroll_profile(&f).await;
    let a = f.root.join("caller-a");
    let b = f.root.join("caller-b");
    let first = f.setup.workspace_id.to_string();
    let mut p = WorkspaceProfile::open(&f.profile).unwrap();
    p.initialize_selection(&a).unwrap();
    p.initialize_selection(&b).unwrap();
    p.set_selection(&a, update(std::slice::from_ref(&first), None, 0))
        .await
        .unwrap();
    assert!(p.target(Some(&a), None).await.is_err());
    let (token, second) = second_invitation(&f);
    p.join(&token).await.unwrap();
    assert_eq!(p.selection(&a).unwrap().workspace_ids, vec![first.clone()]);
    assert!(p.selection(&b).unwrap().workspace_ids.is_empty());
    assert_eq!(p.selection(&a).unwrap().revision, 1);
    p.set_selection(
        &b,
        update(&[second.to_string()], Some(second.to_string()), 0),
    )
    .await
    .unwrap();
    assert_eq!(
        p.target(Some(&b), None).await.unwrap().workspace.id,
        second.to_string()
    );
    assert_eq!(
        p.target(Some(&a), Some(&second.to_string()))
            .await
            .unwrap()
            .workspace
            .id,
        second.to_string()
    );
    assert_eq!(p.selection(&a).unwrap().workspace_ids, vec![first.clone()]);
    assert!(p.selection(&a).unwrap().default_workspace_id.is_none());
    f.db()
        .execute(
            "UPDATE ccp_workspaces SET name='renamed' WHERE id=?1",
            [&first],
        )
        .unwrap();
    let selected = p.selected_workspaces(&a).await.unwrap();
    assert_eq!(selected[0].workspace.id, first);
    assert_eq!(selected[0].workspace.name, "renamed");
    drop(p);
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lost_target_fails_whole_selection_and_default_without_silent_fallback() {
    let f = Fixture::new().await;
    enroll_profile(&f).await;
    let (token, second) = second_invitation(&f);
    let first = f.setup.workspace_id.to_string();
    let state = f.root.join("caller");
    let mut p = WorkspaceProfile::open(&f.profile).unwrap();
    p.join(&token).await.unwrap();
    p.initialize_selection(&state).unwrap();
    p.set_selection(
        &state,
        update(&[first.clone(), second.to_string()], Some(first.clone()), 0),
    )
    .await
    .unwrap();
    f.db()
        .execute(
            "UPDATE ccp_grants SET revoked_at=?1 WHERE workspace_id=?2",
            rusqlite::params![clock(), first],
        )
        .unwrap();
    assert!(p.selected_workspaces(&state).await.is_err());
    assert!(p.target(Some(&state), None).await.is_err());
    assert_eq!(
        p.target(None, Some(&second.to_string()))
            .await
            .unwrap()
            .workspace
            .id,
        second.to_string()
    );
    let saved = p.selection(&state).unwrap();
    assert_eq!(saved.workspace_ids, vec![first.clone(), second.to_string()]);
    assert_eq!(saved.default_workspace_id, Some(first));
    assert_eq!(saved.revision, 1);
    f.db()
        .execute(
            "UPDATE ccp_workspaces SET state='archived' WHERE id=?1",
            [second.to_string()],
        )
        .unwrap();
    p.set_selection(
        &state,
        update(&[second.to_string()], Some(second.to_string()), 1),
    )
    .await
    .unwrap();
    assert!(
        p.target(Some(&state), None)
            .await
            .unwrap()
            .workspace
            .archived
    );
    drop(p);
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn disabled_client_can_inspect_or_clear_preferences_but_cannot_resolve_or_add_targets() {
    let f = Fixture::new().await;
    enroll_profile(&f).await;
    let first = f.setup.workspace_id.to_string();
    let state = f.root.join("caller");
    let mut p = WorkspaceProfile::open(&f.profile).unwrap();
    p.initialize_selection(&state).unwrap();
    p.set_selection(
        &state,
        update(std::slice::from_ref(&first), Some(first.clone()), 0),
    )
    .await
    .unwrap();
    f.db()
        .execute("UPDATE ccp_clients SET disabled=1", [])
        .unwrap();
    assert!(p.selected_workspaces(&state).await.is_err());
    assert!(p.target(Some(&state), None).await.is_err());
    assert!(
        p.set_selection(&state, update(&[first], None, 1))
            .await
            .is_err()
    );
    assert_eq!(p.selection(&state).unwrap().revision, 1);
    assert!(
        p.set_selection(&state, update(&[], None, 1))
            .await
            .unwrap()
            .workspace_ids
            .is_empty()
    );
    assert!(p.selected_workspaces(&state).await.is_err());
    drop(p);
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recovered_profile_cannot_treat_an_old_selection_as_unrelated_workspace_authority() {
    let f = Fixture::new().await;
    let client = enroll_profile(&f).await.client_id.unwrap();
    let (join, second) = second_invitation(&f);
    let state = f.root.join("caller");
    let mut p = WorkspaceProfile::open(&f.profile).unwrap();
    p.join(&join).await.unwrap();
    p.initialize_selection(&state).unwrap();
    p.set_selection(
        &state,
        update(&[second.to_string()], Some(second.to_string()), 0),
    )
    .await
    .unwrap();
    drop(p);
    let token = super::recovery::invitation(&f, &client, false);
    let dir = super::recovery::destination(&f).await;
    let mut recovered = WorkspaceProfile::open(&dir).unwrap();
    recovered.recover(&client, &token).await.unwrap();
    assert_eq!(
        recovered.selection(&state).unwrap().default_workspace_id,
        Some(second.to_string())
    );
    assert!(recovered.selected_workspaces(&state).await.is_err());
    assert!(recovered.target(Some(&state), None).await.is_err());
    assert_eq!(
        recovered
            .target(None, Some(&f.setup.workspace_id.to_string()))
            .await
            .unwrap()
            .workspace
            .id,
        f.setup.workspace_id.to_string()
    );
    drop(recovered);
    let mut old = WorkspaceProfile::open(&f.profile).unwrap();
    assert_eq!(
        old.target(Some(&state), None).await.unwrap().workspace.id,
        second.to_string()
    );
    drop(old);
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn actual_cli_selection_init_set_resolve_show_and_override_preserve_profile_identity() {
    let f = Fixture::new().await;
    let original = enroll_profile(&f).await;
    let first = f.setup.workspace_id.to_string();
    let state = f.root.join("caller");
    let prefix = [
        "workspace",
        "selection",
        "--profile",
        f.profile.to_str().unwrap(),
    ];
    let mut args = prefix.to_vec();
    args.extend(["init", "--selection-dir", state.to_str().unwrap()]);
    assert_eq!(value(f.cli(&args).await)["revision"], 0);
    let mut args = prefix.to_vec();
    args.extend([
        "set",
        "--selection-dir",
        state.to_str().unwrap(),
        "--expected-revision",
        "0",
        "--workspace-id",
        &first,
        "--default-workspace-id",
        &first,
    ]);
    let saved = value(f.cli(&args).await);
    assert_eq!(saved["revision"], 1);
    assert!(!f.cli(&args).await.status.success());
    for action in ["show", "resolve"] {
        let mut args = prefix.to_vec();
        args.extend([action, "--selection-dir", state.to_str().unwrap()]);
        let output = value(f.cli(&args).await);
        assert!(output.to_string().contains(&first));
        assert!(!output.to_string().contains("PRIVATE KEY"));
    }
    let mut args = prefix.to_vec();
    args.extend(["target", "--workspace-id", &first]);
    assert_eq!(value(f.cli(&args).await)["workspace"]["id"], first);
    let p = WorkspaceProfile::open(&f.profile).unwrap();
    assert_eq!(p.info().unwrap().credential_id, original.credential_id);
    assert_eq!(p.selection(&state).unwrap().revision, 1);
    drop(p);
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn competing_cli_selection_updates_accept_only_one_expected_revision() {
    let f = Fixture::new().await;
    enroll_profile(&f).await;
    let (token, second) = second_invitation(&f);
    let first = f.setup.workspace_id.to_string();
    let state = f.root.join("caller");
    let mut p = WorkspaceProfile::open(&f.profile).unwrap();
    p.join(&token).await.unwrap();
    p.initialize_selection(&state).unwrap();
    drop(p);
    let mut a = vec![
        "workspace",
        "selection",
        "--profile",
        f.profile.to_str().unwrap(),
        "set",
        "--selection-dir",
        state.to_str().unwrap(),
        "--expected-revision",
        "0",
        "--workspace-id",
        &first,
    ];
    let second = second.to_string();
    let mut b = a.clone();
    *b.last_mut().unwrap() = &second;
    a.extend(["--default-workspace-id", &first]);
    let (one, two) = tokio::join!(f.cli(&a), f.cli(&b));
    assert_ne!(one.status.success(), two.status.success());
    let p = WorkspaceProfile::open(&f.profile).unwrap();
    let selected = p.selection(&state).unwrap();
    assert_eq!(selected.revision, 1);
    assert_eq!(
        selected.workspace_ids,
        vec![if one.status.success() { first } else { second }]
    );
    drop(p);
    f.finish().await;
}

fn paged_workspaces(f: &Fixture, client: &str, credential: &str, count: u128) -> String {
    let mut db = f.db();
    let tx = db.transaction().unwrap();
    for number in 1..=count {
        let workspace = Uuid::from_u128(number).to_string();
        let grant = Uuid::new_v4().to_string();
        tx.execute(
            "INSERT INTO ccp_workspaces(id,name,owner_admin_id) VALUES (?1,?2,?3)",
            rusqlite::params![
                workspace,
                format!("paged-{number:05}"),
                f.setup.owner_key.principal_id.to_string()
            ],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO ccp_grants(id,workspace_id,client_id,role) VALUES (?1,?2,?3,'read')",
            rusqlite::params![grant, workspace, client],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO ccp_credential_grants(credential_id,grant_id,client_id) VALUES (?1,?2,?3)",
            rusqlite::params![credential, grant, client],
        )
        .unwrap();
    }
    tx.commit().unwrap();
    Uuid::from_u128(count).to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn separate_same_client_profiles_contend_on_selection_revision_without_a_shared_profile_lock()
{
    let f = Fixture::new().await;
    enroll_profile(&f).await;
    let (token, second) = second_invitation(&f);
    let state = f.root.join("caller");
    let mut first = WorkspaceProfile::open(&f.profile).unwrap();
    first.join(&token).await.unwrap();
    first.initialize_selection(&state).unwrap();
    drop(first);
    let other = f.root.join("profile-copy");
    std::fs::create_dir(&other).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&other, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    std::fs::copy(
        f.profile.join("workspace-profile.sqlite3"),
        other.join("workspace-profile.sqlite3"),
    )
    .unwrap();
    let mut first = WorkspaceProfile::open(&f.profile).unwrap();
    let mut next = WorkspaceProfile::open(&other).unwrap();
    let a = update(&[f.setup.workspace_id.to_string()], None, 0);
    let b = update(&[second.to_string()], None, 0);
    let (one, two) = tokio::join!(
        first.set_selection(&state, a),
        next.set_selection(&state, b)
    );
    assert_ne!(one.is_ok(), two.is_ok());
    let error = match (one, two) {
        (Err(error), Ok(_)) | (Ok(_), Err(error)) => error,
        _ => panic!("exactly one selection update must succeed"),
    };
    assert!(error.to_string().contains("selection changed"));
    assert_eq!(first.selection(&state).unwrap().revision, 1);
    drop(first);
    drop(next);
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn target_resolution_follows_real_pages_and_refuses_to_silently_exceed_its_bound() {
    for count in [501, 10_001] {
        let f = Fixture::new().await;
        let original = enroll_profile(&f).await;
        let target = paged_workspaces(
            &f,
            original.client_id.as_deref().unwrap(),
            original.credential_id.as_deref().unwrap(),
            count,
        );
        let mut p = WorkspaceProfile::open(&f.profile).unwrap();
        let result = p.target(None, Some(&target)).await;
        if count == 501 {
            assert_eq!(result.unwrap().workspace.id, target);
        } else {
            assert!(result.err().unwrap().to_string().contains("20-page"));
        }
        assert_eq!(p.info().unwrap().credential_id, original.credential_id);
        drop(p);
        f.finish().await;
    }
}
