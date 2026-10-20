// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

mod boundaries;

const TRANSITION: &str = "/api/v1/admin/content/shelves/transition";

fn request(f: &Fixture, shelf: &Value, revision: u64, archived: bool) -> Value {
    json!({"workspace_id":f.setup.workspace_id.to_string(),"shelf_id":shelf,"operation_id":Uuid::new_v4().to_string(),"expected_revision":revision,"archived":archived})
}

fn target(f: &Fixture, shelf: &Value) -> Value {
    json!({"workspace_id":f.setup.workspace_id.to_string(),"node_id":shelf})
}

async fn change(client: &Client, f: &Fixture, key: &str, body: &Value, status: u16) -> Value {
    json_reply(
        client
            .post(f.url(TRANSITION))
            .bearer_auth(key)
            .json(body)
            .send()
            .await
            .unwrap(),
        status,
    )
    .await
}

fn receipts(f: &Fixture) -> u64 {
    f.db()
        .query_row("SELECT COUNT(*) FROM ccp_shelf_operations", [], |row| {
            row.get(0)
        })
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shelf_http_archive_restore_keeps_descendants_and_exact_replay_is_historical() {
    let f = Fixture::new().await;
    let writer = enrolled(&f, "read_write").await;
    let reader = enrolled(&f, "read").await;
    let (shelf, book) = hierarchy(&f, &writer).await;
    let entry = call(
        &f,
        &writer,
        "entries/mutate",
        &create_entry(&f, &book["id"], "active", "PRIVATE_BODY"),
        200,
    )
    .await["data"]["entry_id"]
        .clone();
    let before = snapshot(&f, &reader, &entry, None).await;
    let other = call(
        &f,
        &writer,
        "entries/mutate",
        &create_entry(&f, &book["id"], "individual", "other"),
        200,
    )
    .await["data"]["entry_id"]
        .clone();
    call(
        &f,
        &writer,
        "entries/mutate",
        &mutation(
            &f,
            json!({"action":"archive","entry_id":other,"expected_revision":1}),
        ),
        200,
    )
    .await;
    let http = f.public_client();
    let archive = request(&f, &shelf["id"], 1, true);
    let saved = change(&http, &f, f.setup.owner_key.secret(), &archive, 200).await;
    assert_eq!(saved["data"].as_object().unwrap().len(), 2);
    assert_eq!(saved["data"]["operation_id"], archive["operation_id"]);
    assert_eq!(
        saved["data"]["result"],
        json!({"workspace_id":f.setup.workspace_id.to_string(),"shelf_id":shelf["id"],"revision":2,"archived":true})
    );
    for user in [&writer, &reader] {
        hidden(&f, user, &shelf["id"], &book["id"], &entry).await;
    }
    code(&call(&f,&writer,"books/create",&json!({"workspace_id":f.setup.workspace_id.to_string(),"shelf_id":shelf["id"],"name":"denied","description":""}),404).await,"not_found");
    let listed = call(
        &f,
        &reader,
        "shelves/archived",
        &json!({"workspace_id":f.setup.workspace_id.to_string()}),
        200,
    )
    .await;
    let summary = &listed["data"]["items"][0];
    assert_eq!(summary.as_object().unwrap().len(), 5);
    assert_eq!(summary["id"], shelf["id"]);
    assert_eq!(summary["name"], shelf["name"]);
    assert_eq!(summary["revision"], 2);
    assert!(!listed.to_string().contains("metadata"));
    assert!(!listed.to_string().contains("PRIVATE_BODY"));
    change(
        &http,
        &f,
        f.setup.owner_key.secret(),
        &request(&f, &shelf["id"], 2, false),
        200,
    )
    .await;
    assert_eq!(snapshot(&f, &reader, &entry, None).await, before);
    code(
        &call(&f, &reader, "entries/get", &entry_target(&f, &other), 404).await,
        "not_found",
    );
    assert_eq!(
        change(&http, &f, f.setup.owner_key.secret(), &archive, 200).await,
        saved
    );
    let state = call(&f, &reader, "shelves/state", &target(&f, &shelf["id"]), 200).await;
    assert_eq!(state["data"]["revision"], 3);
    assert_eq!(state["data"]["archived"], false);
    code(
        &change(
            &http,
            &f,
            f.setup.owner_key.secret(),
            &request(&f, &shelf["id"], 1, true),
            409,
        )
        .await,
        "content_conflict",
    );
    assert_eq!(receipts(&f), 2);
    f.finish().await;
}

async fn hidden(f: &Fixture, user: &User, shelf: &Value, book: &Value, entry: &Value) {
    for (path, body) in [
        ("shelves/get", target(f, shelf)),
        ("books/get", target(f, book)),
        ("entries/get", entry_target(f, entry)),
        ("entries/history", entry_target(f, entry)),
        (
            "entries/list",
            json!({"workspace_id":f.setup.workspace_id.to_string(),"book_id":book,"archived":false}),
        ),
    ] {
        code(&call(f, user, path, &body, 404).await, "not_found");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shelf_http_scopes_live_key_revocation_and_workspace_lifecycle_remain_authoritative() {
    let f = Fixture::new().await;
    let writer = enrolled(&f, "read_write").await;
    let (shelf, _) = hierarchy(&f, &writer).await;
    let http = f.public_client();
    let body = request(&f, &shelf["id"], 1, true);
    let second = f.second();
    for key in [f.setup.instance_key.secret(), second.owner_key.secret()] {
        code(&change(&http, &f, key, &body, 403).await, "forbidden");
    }
    let member = team_member(&f, "shelf admin").await;
    let key = member["secret"].as_str().unwrap();
    let saved = change(&http, &f, key, &body, 200).await;
    assert_eq!(saved["data"]["result"]["revision"], 2);
    let state = admin(
        &f,
        f.setup.instance_key.secret(),
        "shelves/state",
        &target(&f, &shelf["id"]),
        200,
    )
    .await;
    assert_eq!(state["data"]["archived"], true);
    admin_action(
        &f,
        f.setup.owner_key.secret(),
        "/api/v1/admin/keys/revoke",
        &json!({"workspace_id":f.setup.workspace_id.to_string(),"key_id":member["key_id"]}),
        200,
    )
    .await;
    code(&change(&http, &f, key, &body, 401).await, "unauthorized");
    let restore = request(&f, &shelf["id"], 2, false);
    change(&http, &f, f.setup.owner_key.secret(), &restore, 200).await;
    admin_action(
        &f,
        f.setup.owner_key.secret(),
        "/api/v1/admin/workspaces/archive",
        &json!({"workspace_id":f.setup.workspace_id.to_string()}),
        200,
    )
    .await;
    code(
        &change(&http, &f, f.setup.owner_key.secret(), &restore, 403).await,
        "forbidden",
    );
    assert_eq!(
        call(&f, &writer, "shelves/state", &target(&f, &shelf["id"]), 200).await["data"]["archived"],
        false
    );
    admin_action(
        &f,
        f.setup.instance_key.secret(),
        "/api/v1/admin/clients/disable",
        &json!({"client_id":writer.identity["client_id"]}),
        200,
    )
    .await;
    code(
        &call(&f, &writer, "shelves/state", &target(&f, &shelf["id"]), 403).await,
        "forbidden",
    );
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shelf_http_collision_and_private_storage_faults_roll_back_then_exact_retry_succeeds() {
    let f = Fixture::new().await;
    let user = enrolled(&f, "read_write").await;
    let (shelf, _) = hierarchy(&f, &user).await;
    let http = f.public_client();
    let key = f.setup.owner_key.secret();
    change(&http, &f, key, &request(&f, &shelf["id"], 1, true), 200).await;
    let other=call(&f,&user,"shelves/create",&json!({"workspace_id":f.setup.workspace_id.to_string(),"name":"notes","description":"replacement"}),200).await;
    let restore = request(&f, &shelf["id"], 2, false);
    code(
        &change(&http, &f, key, &restore, 409).await,
        "content_conflict",
    );
    assert_eq!(receipts(&f), 1);
    change(
        &http,
        &f,
        key,
        &request(&f, &other["data"]["id"], 1, true),
        200,
    )
    .await;
    for (trigger, sql) in [
        (
            "private_shelf_audit",
            "CREATE TRIGGER private_shelf_audit BEFORE INSERT ON ccp_access_audit WHEN NEW.action='restore_shelf' BEGIN SELECT RAISE(ABORT,'PRIVATE_SQL_DETAIL'); END;",
        ),
        (
            "private_shelf_receipt",
            "CREATE TRIGGER private_shelf_receipt BEFORE INSERT ON ccp_shelf_operations BEGIN SELECT RAISE(ABORT,'PRIVATE_SQL_DETAIL'); END;",
        ),
    ] {
        f.db().execute_batch(sql).unwrap();
        code(
            &change(&http, &f, key, &restore, 503).await,
            "service_unavailable",
        );
        let state = call(&f, &user, "shelves/state", &target(&f, &shelf["id"]), 200).await;
        assert_eq!(state["data"]["revision"], 2);
        assert_eq!(state["data"]["archived"], true);
        assert_eq!(receipts(&f), 2);
        f.db()
            .execute_batch(&format!("DROP TRIGGER {trigger}"))
            .unwrap();
    }
    change(&http, &f, key, &restore, 200).await;
    assert_eq!(
        call(&f, &user, "shelves/get", &target(&f, &shelf["id"]), 200).await["data"],
        shelf
    );
    assert_eq!(receipts(&f), 3);
    f.finish().await;
}
