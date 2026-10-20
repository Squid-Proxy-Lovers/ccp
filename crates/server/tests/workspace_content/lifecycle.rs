// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn content_archive_summaries_never_leak_bodies_and_collision_restore_does_not_overwrite() {
    let f = Fixture::new().await;
    let owner = enrolled(&f, "read_write").await;
    let reader = enrolled(&f, "read").await;
    let (_, book) = hierarchy(&f, &owner).await;
    let first = call(
        &f,
        &owner,
        "entries/mutate",
        &create_entry(&f, &book["id"], "entry", "archived private body"),
        200,
    )
    .await;
    let id = first["data"]["entry_id"].clone();
    let archive = mutation(
        &f,
        json!({"action":"archive","entry_id":id,"expected_revision":1}),
    );
    let receipt = call(&f, &owner, "entries/mutate", &archive, 200).await;
    assert_eq!(receipt["data"].as_object().unwrap().len(), 2);
    for user in [&owner, &reader] {
        assert_archived(&f, user, &book["id"], &id).await;
    }
    code(
        &admin(
            &f,
            f.setup.owner_key.secret(),
            "entries/get",
            &entry_target(&f, &id),
            404,
        )
        .await,
        "not_found",
    );
    let other = call(
        &f,
        &owner,
        "entries/mutate",
        &create_entry(&f, &book["id"], "entry", "replacement"),
        200,
    )
    .await;
    let restore = mutation(
        &f,
        json!({"action":"restore","entry_id":id,"expected_revision":2}),
    );
    code(
        &call(&f, &reader, "entries/mutate", &restore, 403).await,
        "forbidden",
    );
    code(
        &call(&f, &owner, "entries/mutate", &restore, 409).await,
        "content_conflict",
    );
    assert_eq!(
        snapshot(&f, &owner, &other["data"]["entry_id"], None).await["content"]["text"],
        "replacement"
    );
    call(
        &f,
        &owner,
        "entries/mutate",
        &mutation(
            &f,
            json!({"action":"archive","entry_id":other["data"]["entry_id"],"expected_revision":1}),
        ),
        200,
    )
    .await;
    admin(
        &f,
        f.setup.owner_key.secret(),
        "entries/mutate",
        &restore,
        200,
    )
    .await;
    assert_eq!(
        snapshot(&f, &reader, &id, None).await["content"]["text"],
        "archived private body"
    );
    assert_eq!(
        snapshot(&f, &reader, &id, Some(2)).await["revision"]["action"],
        "archive"
    );
    assert_eq!(
        call(&f, &owner, "entries/mutate", &archive, 200).await,
        receipt
    );
    assert_eq!(
        snapshot(&f, &reader, &id, None).await["entry"]["revision"],
        3
    );
    f.finish().await;
}

async fn assert_archived(f: &Fixture, user: &User, book: &Value, id: &Value) {
    for revision in [Value::Null, json!(1), json!(2)] {
        code(&call(f,user,"entries/get",&json!({"workspace_id":f.setup.workspace_id.to_string(),"entry_id":id,"revision":revision}),404).await,"not_found");
    }
    code(
        &call(f, user, "entries/history", &entry_target(f, id), 404).await,
        "not_found",
    );
    let listed = call(
        f,
        user,
        "entries/list",
        &json!({"workspace_id":f.setup.workspace_id.to_string(),"book_id":book,"archived":true}),
        200,
    )
    .await;
    let summary = &listed["data"]["items"][0];
    assert_eq!(summary["id"], *id);
    assert_eq!(summary["archived"], true);
    for field in ["text", "content", "description", "labels", "appended_text"] {
        assert!(summary.get(field).is_none());
    }
    assert!(!listed.to_string().contains("archived private body"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn content_live_connections_recheck_workspace_archive_grant_revocation_and_client_disable() {
    let f = Fixture::new().await;
    let owner = enrolled(&f, "read_write").await;
    let reader = enrolled(&f, "read").await;
    let (_, book) = hierarchy(&f, &owner).await;
    let first = call(
        &f,
        &owner,
        "entries/mutate",
        &create_entry(&f, &book["id"], "entry", "retained"),
        200,
    )
    .await;
    let id = first["data"]["entry_id"].clone();
    admin_action(
        &f,
        f.setup.owner_key.secret(),
        "/api/v1/admin/workspaces/archive",
        &json!({"workspace_id":f.setup.workspace_id.to_string()}),
        200,
    )
    .await;
    assert_eq!(
        snapshot(&f, &owner, &id, None).await["content"]["text"],
        "retained"
    );
    code(
        &call(
            &f,
            &owner,
            "entries/mutate",
            &mutation(
                &f,
                json!({"action":"append","entry_id":id,"expected_revision":1,"text":"denied"}),
            ),
            403,
        )
        .await,
        "forbidden",
    );
    admin_action(
        &f,
        f.setup.owner_key.secret(),
        "/api/v1/admin/workspaces/reactivate",
        &json!({"workspace_id":f.setup.workspace_id.to_string()}),
        200,
    )
    .await;
    assert_eq!(
        snapshot(&f, &reader, &id, None).await["content"]["text"],
        "retained"
    );
    let grant: String = f
        .db()
        .query_row(
            "SELECT id FROM ccp_grants WHERE client_id=?1",
            [owner.identity["client_id"].as_str().unwrap()],
            |r| r.get(0),
        )
        .unwrap();
    admin_action(
        &f,
        f.setup.owner_key.secret(),
        "/api/v1/admin/grants/revoke",
        &json!({"grant_id":grant}),
        200,
    )
    .await;
    code(
        &call(&f, &owner, "entries/get", &entry_target(&f, &id), 403).await,
        "forbidden",
    );
    admin_action(
        &f,
        f.setup.instance_key.secret(),
        "/api/v1/admin/clients/disable",
        &json!({"client_id":reader.identity["client_id"]}),
        200,
    )
    .await;
    code(
        &call(&f, &reader, "entries/get", &entry_target(&f, &id), 403).await,
        "forbidden",
    );
    f.finish().await;
}
