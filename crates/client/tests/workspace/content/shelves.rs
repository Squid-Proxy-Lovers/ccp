// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

fn target(f: &Fixture, r: &Records) -> api::NodeTarget {
    api::NodeTarget {
        workspace_id: f.setup.workspace_id.to_string(),
        node_id: r.shelf.clone(),
    }
}
fn page(f: &Fixture) -> api::ShelfPage {
    api::ShelfPage {
        workspace_id: f.setup.workspace_id.to_string(),
        after: None,
        limit: Some(1),
    }
}
fn archive(f: &Fixture, r: &Records, revision: u64, archived: bool) {
    let mut db = f.db();
    let mut store = AccessStore::new(&mut db).unwrap();
    let actor = store
        .authenticate_admin_key(f.setup.owner_key.secret(), clock())
        .unwrap();
    store
        .transition_shelf(
            actor,
            f.setup.workspace_id,
            &server::foundation::ShelfTransition {
                operation_id: Uuid::new_v4(),
                shelf_id: Uuid::parse_str(&r.shelf).unwrap(),
                expected_revision: revision,
                archived,
            },
            clock(),
        )
        .unwrap();
}
async fn cli(f: &Fixture, action: &str, extra: &[&str]) -> Value {
    let workspace = f.setup.workspace_id.to_string();
    let mut args = vec![
        "workspace",
        "content",
        "--profile",
        f.profile.to_str().unwrap(),
        action,
        "--workspace-id",
        &workspace,
    ];
    args.extend_from_slice(extra);
    value(f.cli(&args).await)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shelf_read_library_and_cli_observe_overlay_restore_and_preserve_profile_and_choices() {
    let f = Fixture::new().await;
    let info = enroll_profile(&f).await;
    let r = records(&f, &info);
    let mut p = WorkspaceProfile::open(&f.profile).unwrap();
    let selection = p.initialize_selection(&f.root.join("selection")).unwrap();
    assert!(
        !p.content_shelf_state(&target(&f, &r))
            .await
            .unwrap()
            .archived
    );
    assert!(
        p.content_archived_shelves(&page(&f))
            .await
            .unwrap()
            .items
            .is_empty()
    );
    let before = p.content_entry(&entry_request(&f, &r)).await.unwrap();
    drop(p);
    let raw = fs::read(f.profile.join("workspace-profile.sqlite3")).unwrap();
    assert_eq!(
        cli(&f, "shelf-state", &["--node-id", &r.shelf]).await["revision"],
        1
    );
    archive(&f, &r, 1, true);
    assert!(
        cli(&f, "shelves", &[]).await["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let state = cli(&f, "shelf-state", &["--node-id", &r.shelf]).await;
    assert_eq!(state["archived"], true);
    assert_eq!(state["revision"], 2);
    let summaries = cli(&f, "archived-shelves", &["--limit", "1"]).await;
    assert_eq!(summaries["items"][0]["id"], r.shelf);
    assert_eq!(summaries["items"][0].as_object().unwrap().len(), 5);
    assert!(!summaries.to_string().contains("metadata"));
    let mut p = WorkspaceProfile::open(&f.profile).unwrap();
    assert!(p.content_entry(&entry_request(&f, &r)).await.is_err());
    assert_eq!(
        p.content_archived_shelves(&page(&f)).await.unwrap().items[0].revision,
        2
    );
    assert_eq!(
        p.selection(&f.root.join("selection")).unwrap().revision,
        selection.revision
    );
    drop(p);
    archive(&f, &r, 2, false);
    let mut p = WorkspaceProfile::open(&f.profile).unwrap();
    assert_eq!(
        serde_json::to_value(p.content_entry(&entry_request(&f, &r)).await.unwrap()).unwrap(),
        serde_json::to_value(before).unwrap()
    );
    assert_eq!(
        p.content_shelf_state(&target(&f, &r))
            .await
            .unwrap()
            .revision,
        3
    );
    assert!(
        p.content_archived_shelves(&page(&f))
            .await
            .unwrap()
            .items
            .is_empty()
    );
    drop(p);
    assert_eq!(
        fs::read(f.profile.join("workspace-profile.sqlite3")).unwrap(),
        raw
    );
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shelf_metadata_renews_only_after_authorized_reads_and_never_on_denial() {
    for authorized in [true, false] {
        let f = Fixture::new().await;
        WorkspaceProfile::trust(&f.profile, f.config())
            .await
            .unwrap();
        short_credential(&f, false);
        let mut p = WorkspaceProfile::open(&f.profile).unwrap();
        let old = p.info().unwrap();
        let r = records(&f, &old);
        if !authorized {
            f.db()
                .execute("UPDATE ccp_grants SET revoked_at=?1", [clock()])
                .unwrap();
        }
        let result = p.content_archived_shelves(&page(&f)).await;
        if authorized {
            assert!(result.unwrap().items.is_empty());
            assert_eq!(
                p.content_shelf_state(&target(&f, &r))
                    .await
                    .unwrap()
                    .revision,
                1
            );
        } else {
            assert!(result.is_err());
            assert!(p.content_shelf_state(&target(&f, &r)).await.is_err());
        }
        let next = p.info().unwrap();
        assert_eq!(next.client_id, old.client_id);
        assert_eq!(next.credential_id == old.credential_id, !authorized);
        assert_eq!(f.count("ccp_credentials"), if authorized { 2 } else { 1 });
        drop(p);
        f.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shelf_metadata_reads_preserve_an_uncertain_entry_intent_until_explicit_resolution() {
    let f = Fixture::new().await;
    let info = enroll_profile(&f).await;
    let r = records(&f, &info);
    Connection::open(f.profile.join("workspace-profile.sqlite3")).unwrap().execute_batch("CREATE TRIGGER fail_finish BEFORE UPDATE ON content_state WHEN NEW.pending IS NULL BEGIN SELECT RAISE(ABORT,'fixture local save'); END;").unwrap();
    let mut p = WorkspaceProfile::open(&f.profile).unwrap();
    let request = api::EntryMutation {
        workspace_id: f.setup.workspace_id.to_string(),
        operation_id: Uuid::new_v4().to_string(),
        agent: None,
        change: api::EntryChange::Append {
            entry_id: r.entry.clone(),
            expected_revision: 2,
            text: "+pending".into(),
        },
    };
    assert!(p.mutate_entry(request).await.is_err());
    let pending = serde_json::to_value(p.info().unwrap().pending_content).unwrap();
    archive(&f, &r, 1, true);
    assert!(
        p.content_shelf_state(&target(&f, &r))
            .await
            .unwrap()
            .archived
    );
    assert_eq!(
        p.content_archived_shelves(&page(&f)).await.unwrap().items[0].id,
        r.shelf
    );
    assert_eq!(
        serde_json::to_value(p.info().unwrap().pending_content).unwrap(),
        pending
    );
    assert_eq!(p.info().unwrap().credential_id, info.credential_id);
    drop(p);
    Connection::open(f.profile.join("workspace-profile.sqlite3"))
        .unwrap()
        .execute_batch("DROP TRIGGER fail_finish")
        .unwrap();
    let mut p = WorkspaceProfile::open(&f.profile).unwrap();
    p.resolve_entry_operation().await.unwrap();
    assert!(p.pending_entry_operation().is_none());
    drop(p);
    f.finish().await;
}
