// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn content_real_mtls_owner_revisions_shared_append_and_current_reader_history() {
    let f = Fixture::new().await;
    let owner = enrolled(&f, "read_write").await;
    let other = enrolled(&f, "read_write").await;
    let reader = enrolled(&f, "read").await;
    let (shelf, book) = hierarchy(&f, &owner).await;
    assert_nodes(&f, &reader, &shelf, &book).await;
    let created = create_entry(&f, &book["id"], " entry ", "first");
    let first = call(&f, &owner, "entries/mutate", &created, 200).await;
    let id = first["data"]["entry_id"].clone();
    assert_eq!(
        call(&f, &owner, "entries/mutate", &created, 200).await,
        first
    );
    let append = mutation(
        &f,
        json!({"action":"append","entry_id":id,"expected_revision":1,"text":"\nother"}),
    );
    assert_eq!(
        call(&f, &other, "entries/mutate", &append, 200).await["data"]["revision"],
        2
    );
    call(&f, &other, "entries/mutate", &append, 200).await;
    let next = json!({"action":"revise","entry_id":id,"expected_revision":2,"content":{"text":"new text","description":"changed description","labels":["changed","order"]}});
    code(
        &call(
            &f,
            &other,
            "entries/mutate",
            &mutation(&f, next.clone()),
            403,
        )
        .await,
        "forbidden",
    );
    code(
        &call(
            &f,
            &reader,
            "entries/mutate",
            &mutation(&f, next.clone()),
            403,
        )
        .await,
        "forbidden",
    );
    call(&f, &owner, "entries/mutate", &mutation(&f, next), 200).await;
    code(
        &call(
            &f,
            &owner,
            "entries/mutate",
            &mutation(
                &f,
                json!({"action":"append","entry_id":id,"expected_revision":2,"text":"stale"}),
            ),
            409,
        )
        .await,
        "content_conflict",
    );
    let current = snapshot(&f, &reader, &id, None).await;
    assert_eq!(
        current["entry"]["owner_client_id"],
        owner.identity["client_id"]
    );
    assert_eq!(current["entry"]["name"], "entry");
    assert_eq!(
        current["content"],
        json!({"text":"new text","description":"changed description","labels":["changed","order"]})
    );
    let old = snapshot(&f, &reader, &id, Some(2)).await;
    assert_eq!(old["entry"]["revision"], 3);
    assert_eq!(old["revision"]["revision"], 2);
    assert_eq!(old["revision"]["client_id"], other.identity["client_id"]);
    assert_eq!(
        old["revision"]["actor"],
        json!({"kind":"credential","id":other.identity["credential_id"]})
    );
    assert_eq!(old["content"], content_body("first\nother"));
    assert_eq!(old["appended_text"], "\nother");
    assert_http_history(&f, &reader, &id).await;
    f.finish().await;
}

async fn assert_nodes(f: &Fixture, reader: &User, shelf: &Value, book: &Value) {
    for (kind, node) in [("shelves", shelf), ("books", book)] {
        let got = call(
            f,
            reader,
            &format!("{kind}/get"),
            &json!({"workspace_id":f.setup.workspace_id.to_string(),"node_id":node["id"]}),
            200,
        )
        .await;
        assert_eq!(got["data"], *node);
        let mut body = json!({"workspace_id":f.setup.workspace_id.to_string()});
        if kind == "books" {
            body["shelf_id"] = shelf["id"].clone();
        }
        let list = call(f, reader, &format!("{kind}/list"), &body, 200).await;
        assert_eq!(list["data"]["items"][0], *node);
        assert_eq!(list["data"]["next_cursor"], Value::Null);
    }
    code(&call(f,reader,"shelves/create",&json!({"workspace_id":f.setup.workspace_id.to_string(),"name":"denied","description":""}),403).await,"forbidden");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn content_https_admin_key_overrides_and_global_key_reads_without_workspace_write() {
    let f = Fixture::new().await;
    let owner = enrolled(&f, "read_write").await;
    let (_, book) = hierarchy(&f, &owner).await;
    let first = call(
        &f,
        &owner,
        "entries/mutate",
        &create_entry(&f, &book["id"], "client entry", "owned"),
        200,
    )
    .await;
    let id = first["data"]["entry_id"].clone();
    let revise = mutation(
        &f,
        json!({"action":"revise","entry_id":id,"expected_revision":1,"content":content_body("admin override")}),
    );
    assert_eq!(
        f.setup.owner_key.principal_id,
        f.setup.instance_key.principal_id
    );
    code(
        &admin(
            &f,
            f.setup.instance_key.secret(),
            "entries/mutate",
            &revise,
            403,
        )
        .await,
        "forbidden",
    );
    admin(
        &f,
        f.setup.owner_key.secret(),
        "entries/mutate",
        &revise,
        200,
    )
    .await;
    let read = admin(
        &f,
        f.setup.instance_key.secret(),
        "entries/get",
        &entry_target(&f, &id),
        200,
    )
    .await;
    assert_eq!(
        read["data"]["entry"]["owner_client_id"],
        owner.identity["client_id"]
    );
    assert_eq!(
        read["data"]["revision"]["actor"],
        json!({"kind":"admin_key","id":f.setup.owner_key.id.to_string()})
    );
    assert_eq!(
        read["data"]["revision"]["admin_principal_id"],
        f.setup.owner_key.principal_id.to_string()
    );
    let created = admin(
        &f,
        f.setup.owner_key.secret(),
        "entries/mutate",
        &create_entry(&f, &book["id"], "human entry", "human"),
        200,
    )
    .await;
    let human = snapshot(&f, &owner, &created["data"]["entry_id"], None).await;
    assert_eq!(human["entry"]["owner_client_id"], Value::Null);
    let second = f.second();
    code(
        &admin(
            &f,
            second.owner_key.secret(),
            "books/get",
            &json!({"workspace_id":second.workspace_id.to_string(),"node_id":book["id"]}),
            404,
        )
        .await,
        "not_found",
    );
    code(
        &admin(
            &f,
            f.setup.owner_key.secret(),
            "books/get",
            &json!({"workspace_id":second.workspace_id.to_string(),"node_id":book["id"]}),
            403,
        )
        .await,
        "forbidden",
    );
    code(&admin(&f,second.owner_key.secret(),"books/create",&json!({"workspace_id":second.workspace_id.to_string(),"shelf_id":Uuid::new_v4().to_string(),"name":"missing","description":""}),404).await,"not_found");
    f.finish().await;
}

async fn assert_http_history(f: &Fixture, reader: &User, id: &Value) {
    let history = call(
        f,
        reader,
        "entries/history",
        &json!({"workspace_id":f.setup.workspace_id.to_string(),"entry_id":id,"limit":2}),
        200,
    )
    .await;
    assert_eq!(history["data"]["items"].as_array().unwrap().len(), 2);
    assert_eq!(history["data"]["next_cursor"], 2);
    assert!(history["data"]["items"][1].get("content").is_none());
    let last = call(
        f,
        reader,
        "entries/history",
        &json!({"workspace_id":f.setup.workspace_id.to_string(),"entry_id":id,"after":2,"limit":2}),
        200,
    )
    .await;
    assert_eq!(last["data"]["items"][0]["action"], "revise");
    assert_eq!(last["data"]["next_cursor"], Value::Null);
}
