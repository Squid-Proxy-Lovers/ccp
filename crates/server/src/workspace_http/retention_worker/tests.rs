// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use crate::foundation::{
    Actor, EntryChange, EntryContent, EntryMutation, EntryRetentionChange, NodeDraft,
};
use tokio::sync::oneshot;

fn setup() -> (PathBuf, Uuid) {
    let directory = std::env::temp_dir().join(format!("ccp-worker-drain-{}", Uuid::new_v4()));
    let setup = crate::local_admin::bootstrap(&directory, "operator", "first").unwrap();
    crate::instance_tls::initialize(&directory, &["127.0.0.1".into()]).unwrap();
    let mut db = Connection::open(directory.join("ccp.sqlite3")).unwrap();
    let mut store = AccessStore::new(&mut db).unwrap();
    let now = crate::instance_tls::now().unwrap() - 10;
    let actor = store
        .authenticate_admin_key(setup.owner_key.secret(), now)
        .unwrap();
    create_due(&mut store, actor, setup.workspace_id, now);
    (directory, setup.workspace_id)
}

fn create_due(store: &mut AccessStore<'_>, actor: Actor, workspace: Uuid, now: i64) {
    let node = NodeDraft {
        name: "notes".into(),
        description: String::new(),
    };
    let shelf = store.create_shelf(actor, workspace, &node, now).unwrap();
    let book = store
        .create_book(actor, workspace, shelf.id, &node, now)
        .unwrap();
    let request = EntryMutation {
        operation_id: Uuid::new_v4(),
        agent: None,
        change: EntryChange::Create {
            book_id: book.id,
            name: "entry".into(),
            content: EntryContent {
                text: "private".into(),
                description: String::new(),
                labels: Vec::new(),
            },
        },
    };
    let entry = store.mutate_entry(actor, workspace, &request, now).unwrap();
    let request = EntryMutation {
        operation_id: Uuid::new_v4(),
        agent: None,
        change: EntryChange::Archive {
            id: entry.entry_id,
            expected_revision: 1,
        },
    };
    store.mutate_entry(actor, workspace, &request, now).unwrap();
    store
        .change_entry_retention(
            actor,
            workspace,
            &EntryRetentionChange {
                operation_id: Uuid::new_v4(),
                expected_revision: 0,
                retention_seconds: Some(1),
                confirm_timed_purge: true,
            },
            now,
        )
        .unwrap();
}

async fn server(directory: &std::path::Path) -> WorkspaceServer {
    WorkspaceServer::bind(Config {
        data_dir: directory.to_owned(),
        https_listen: "127.0.0.1:0".parse().unwrap(),
        mtls_listen: "127.0.0.1:0".parse().unwrap(),
        activate_workspace: true,
    })
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_tracks_real_blocking_cleanup_and_retains_directory_exclusion() {
    let (directory, _) = setup();
    let server = server(&directory).await;
    let budget = server.cleanup_budget.clone();
    let lock = Connection::open(directory.join("ccp.sqlite3")).unwrap();
    lock.execute_batch("BEGIN IMMEDIATE").unwrap();
    let (stop, signal) = oneshot::channel();
    let task = tokio::spawn(server.run_until(async { signal.await.context("test stop dropped") }));
    timeout(Duration::from_secs(2), async {
        while budget.available_permits() != 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    stop.send(()).unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(!task.is_finished());
    assert!(DirectoryLease::maintenance(&directory).is_err());
    lock.execute_batch("ROLLBACK").unwrap();
    drop(lock);
    timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let lease = DirectoryLease::maintenance(&directory).unwrap();
    let db = Connection::open(directory.join("ccp.sqlite3")).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM ccp_entry_retention_purges", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        1
    );
    drop(db);
    drop(lease);
    std::fs::remove_dir_all(directory).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn worker_budget_waits_without_spawning_more_sql_and_stops_cleanly() {
    let (directory, _) = setup();
    let server = server(&directory).await;
    let budget = server.cleanup_budget.clone();
    let permit = budget.clone().acquire_owned().await.unwrap();
    let (stop, signal) = watch::channel(false);
    let task = tokio::spawn(run(server.state.clone(), signal, budget.clone()));
    tokio::time::sleep(Duration::from_millis(1100)).await;
    let db = Connection::open(directory.join("ccp.sqlite3")).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM ccp_content_entries", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        1
    );
    drop(permit);
    timeout(Duration::from_secs(4), async {
        loop {
            if db
                .query_row("SELECT count(*) FROM ccp_entry_retention_purges", [], |r| {
                    r.get::<_, u64>(0)
                })
                .unwrap()
                == 1
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    stop.send(true).unwrap();
    task.await.unwrap().unwrap();
    assert_eq!(budget.available_permits(), 1);
    drop(db);
    drop(server);
    std::fs::remove_dir_all(directory).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn one_workspace_fault_cannot_starve_other_scheduled_workspaces() {
    let (directory, first) = setup();
    let server = server(&directory).await;
    let mut db = Connection::open(directory.join("ccp.sqlite3")).unwrap();
    let (global,owner):(String,String)=db.query_row("SELECT s.admin_id,w.owner_admin_id FROM ccp_server_admins s,ccp_workspaces w WHERE w.id=?1",[first.to_string()],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    let mut store = AccessStore::new(&mut db).unwrap();
    let now = crate::instance_tls::now().unwrap() - 10;
    let second = store
        .create_workspace(
            Actor::Admin(global.parse().unwrap()),
            "second",
            owner.parse().unwrap(),
            now,
        )
        .unwrap();
    create_due(
        &mut store,
        Actor::Admin(owner.parse().unwrap()),
        second,
        now,
    );
    db.execute_batch(&format!("CREATE TRIGGER worker_target_fault BEFORE INSERT ON ccp_entry_retention_purges WHEN NEW.workspace_id='{first}' BEGIN SELECT RAISE(ABORT,'PRIVATE_BODY'); END;")).unwrap();
    let outcome = cycle(&server.state, None).unwrap();
    assert_eq!(outcome.failures, 1);
    assert!(outcome.after.is_none());
    assert_eq!(
        db.query_row(
            "SELECT workspace_id FROM ccp_entry_retention_purges",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        second.to_string()
    );
    assert_eq!(
        db.query_row("SELECT workspace_id FROM ccp_content_entries", [], |r| {
            r.get::<_, String>(0)
        })
        .unwrap(),
        first.to_string()
    );
    db.execute_batch("DROP TRIGGER worker_target_fault")
        .unwrap();
    assert_eq!(cycle(&server.state, None).unwrap().failures, 0);
    assert_eq!(
        db.query_row("SELECT count(*) FROM ccp_entry_retention_purges", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        2
    );
    drop(db);
    drop(server);
    std::fs::remove_dir_all(directory).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scheduling_cursor_bounds_each_cycle_and_reaches_the_next_page() {
    let (directory, _) = setup();
    let server = server(&directory).await;
    let mut db = Connection::open(directory.join("ccp.sqlite3")).unwrap();
    let (global, owner): (String, String) = db
        .query_row(
            "SELECT s.admin_id,w.owner_admin_id FROM ccp_server_admins s,ccp_workspaces w LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let now = crate::instance_tls::now().unwrap() - 10;
    let mut store = AccessStore::new(&mut db).unwrap();
    for index in 0..WORKSPACES {
        let workspace = store
            .create_workspace(
                Actor::Admin(global.parse().unwrap()),
                &format!("extra {index}"),
                owner.parse().unwrap(),
                now,
            )
            .unwrap();
        create_due(
            &mut store,
            Actor::Admin(owner.parse().unwrap()),
            workspace,
            now,
        );
    }
    let first = cycle(&server.state, None).unwrap();
    assert!(first.after.is_some());
    assert_eq!(first.failures, 0);
    assert_eq!(
        db.query_row("SELECT count(*) FROM ccp_entry_retention_purges", [], |r| r
            .get::<_, usize>(0))
            .unwrap(),
        WORKSPACES
    );
    let second = cycle(&server.state, first.after).unwrap();
    assert!(second.after.is_none());
    assert_eq!(second.failures, 0);
    assert_eq!(
        db.query_row("SELECT count(*) FROM ccp_entry_retention_purges", [], |r| r
            .get::<_, usize>(0))
            .unwrap(),
        WORKSPACES + 1
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM ccp_content_entries", [], |r| r
            .get::<_, usize>(0))
            .unwrap(),
        0
    );
    drop(db);
    drop(server);
    std::fs::remove_dir_all(directory).unwrap();
}
