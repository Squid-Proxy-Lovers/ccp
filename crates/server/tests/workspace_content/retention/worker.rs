// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

async fn archived(f: &Fixture, user: &User) -> (Value, Value, Value) {
    let (_, book) = hierarchy(f, user).await;
    let original = create_entry(f, &book["id"], "entry", "PRIVATE_BODY");
    let receipt = call(f, user, "entries/mutate", &original, 200).await["data"].clone();
    let archive = mutation(
        f,
        json!({"action":"archive","entry_id":receipt["entry_id"],"expected_revision":1}),
    );
    call(f, user, "entries/mutate", &archive, 200).await;
    (receipt, original, archive)
}
async fn enabled(f: &Fixture, revision: u64, seconds: u64) -> Value {
    request(
        f,
        f.setup.owner_key.secret(),
        CHANGE,
        &change(f, revision, Some(seconds)),
        200,
    )
    .await["data"]
        .clone()
}
async fn purged(f: &Fixture) {
    tokio::time::timeout(Duration::from_secs(6), async {
        loop {
            if count(f, "ccp_entry_retention_purges") == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(count(f, "ccp_content_entries"), 0);
    assert_eq!(count(f, "ccp_content_versions"), 0);
    assert_eq!(count(f, "ccp_entry_retention_clocks"), 0);
}
async fn restart(f: Fixture) -> Fixture {
    let Fixture {
        directory,
        setup,
        trust,
        stop,
        task,
        ..
    } = f;
    stop.send(()).unwrap();
    task.await.unwrap().unwrap();
    let server = WorkspaceServer::bind(Config {
        data_dir: directory.clone(),
        https_listen: "127.0.0.1:0".parse().unwrap(),
        mtls_listen: "127.0.0.1:0".parse().unwrap(),
        activate_workspace: false,
    })
    .await
    .unwrap();
    let startup = server.startup().unwrap();
    let (stop, signal) = oneshot::channel();
    let task = tokio::spawn(
        server.run_until(async { signal.await.context("test restart sender dropped") }),
    );
    Fixture {
        directory,
        setup,
        trust,
        startup,
        stop,
        task,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_worker_keeps_manual_default_and_only_purges_after_explicit_fresh_window() {
    let f = Fixture::new().await;
    let user = enrolled(&f, "read_write").await;
    let (entry, original, archive) = archived(&f, &user).await;
    f.db()
        .execute(
            "UPDATE ccp_content_entries SET updated_at=?1",
            [now() - 10000],
        )
        .unwrap();
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert_eq!(count(&f, "ccp_content_entries"), 1);
    assert_eq!(count(&f, "ccp_entry_retention_clocks"), 0);
    enabled(&f, 0, 2).await;
    let (seconds, start): (u64, i64) = f
        .db()
        .query_row(
            "SELECT remaining_seconds,active_since FROM ccp_entry_retention_clocks",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(seconds, 2);
    assert!(start >= now() - 1);
    assert_eq!(count(&f, "ccp_content_entries"), 1);
    purged(&f).await;
    assert_eq!(
        call(&f, &user, "entries/mutate", &original, 200).await["data"],
        entry
    );
    assert_eq!(
        call(&f, &user, "entries/mutate", &archive, 200).await["data"]["revision"],
        2
    );
    code(
        &call(
            &f,
            &user,
            "entries/get",
            &entry_target(&f, &entry["entry_id"]),
            404,
        )
        .await,
        "not_found",
    );
    assert_eq!(count(&f, "ccp_entry_retention_purges"), 1);
    assert_eq!(count(&f, "ccp_content_operations"), 2);
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_worker_disable_exact_old_retry_and_reenable_preserve_manual_control() {
    let f = Fixture::new().await;
    let user = enrolled(&f, "read_write").await;
    archived(&f, &user).await;
    let enable = change(&f, 0, Some(2));
    let original = request(&f, f.setup.owner_key.secret(), CHANGE, &enable, 200).await;
    request(
        &f,
        f.setup.owner_key.secret(),
        CHANGE,
        &change(&f, 1, None),
        200,
    )
    .await;
    assert_eq!(
        request(&f, f.setup.owner_key.secret(), CHANGE, &enable, 200).await,
        original
    );
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(count(&f, "ccp_content_entries"), 1);
    assert_eq!(count(&f, "ccp_entry_retention_clocks"), 0);
    assert!(policy(&f).await["retention_seconds"].is_null());
    enabled(&f, 2, 1).await;
    purged(&f).await;
    assert_eq!(
        f.db()
            .query_row(
                "SELECT policy_revision FROM ccp_entry_retention_purges",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
        3
    );
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_worker_workspace_archive_pauses_across_restart_and_reactivation_resumes() {
    let f = Fixture::new().await;
    let user = enrolled(&f, "read_write").await;
    archived(&f, &user).await;
    enabled(&f, 0, 3).await;
    request(
        &f,
        f.setup.owner_key.secret(),
        "/api/v1/admin/workspaces/archive",
        &json!({"workspace_id":f.setup.workspace_id.to_string()}),
        200,
    )
    .await;
    let remaining: u64 = f
        .db()
        .query_row(
            "SELECT remaining_seconds FROM ccp_entry_retention_clocks WHERE active_since IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!((1..=3).contains(&remaining));
    let f = restart(f).await;
    tokio::time::sleep(Duration::from_secs(4)).await;
    assert_eq!(count(&f, "ccp_content_entries"), 1);
    assert_eq!(count(&f, "ccp_entry_retention_purges"), 0);
    assert_eq!(f.db().query_row("SELECT remaining_seconds FROM ccp_entry_retention_clocks WHERE active_since IS NULL",[],|r|r.get::<_,u64>(0)).unwrap(),remaining);
    request(
        &f,
        f.setup.owner_key.secret(),
        "/api/v1/admin/workspaces/reactivate",
        &json!({"workspace_id":f.setup.workspace_id.to_string()}),
        200,
    )
    .await;
    purged(&f).await;
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_worker_restore_cancels_then_rearchive_and_fault_retry_remain_atomic() {
    let f = Fixture::new().await;
    let user = enrolled(&f, "read_write").await;
    let (entry, _, _) = archived(&f, &user).await;
    enabled(&f, 0, 2).await;
    call(
        &f,
        &user,
        "entries/mutate",
        &mutation(
            &f,
            json!({"action":"restore","entry_id":entry["entry_id"],"expected_revision":2}),
        ),
        200,
    )
    .await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(count(&f, "ccp_content_entries"), 1);
    assert_eq!(count(&f, "ccp_entry_retention_clocks"), 0);
    f.db().execute_batch("CREATE TRIGGER timed_worker_fault BEFORE INSERT ON ccp_entry_retention_purges BEGIN SELECT RAISE(ABORT,'PRIVATE_BODY'); END;").unwrap();
    call(
        &f,
        &user,
        "entries/mutate",
        &mutation(
            &f,
            json!({"action":"archive","entry_id":entry["entry_id"],"expected_revision":3}),
        ),
        200,
    )
    .await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(count(&f, "ccp_content_entries"), 1);
    assert_eq!(count(&f, "ccp_content_versions"), 4);
    assert_eq!(count(&f, "ccp_entry_retention_clocks"), 1);
    assert_eq!(count(&f, "ccp_entry_retention_purges"), 0);
    f.db()
        .execute_batch("DROP TRIGGER timed_worker_fault")
        .unwrap();
    purged(&f).await;
    assert_eq!(
        f.db()
            .query_row(
                "SELECT archive_revision FROM ccp_entry_retention_purges",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
        4
    );
    f.finish().await;
}
