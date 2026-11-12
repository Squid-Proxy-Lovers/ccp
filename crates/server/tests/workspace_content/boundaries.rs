// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn content_strict_requests_methods_auth_and_typed_errors_never_accept_claimed_identity() {
    let f = Fixture::new().await;
    let user = enrolled(&f, "read_write").await;
    let (shelf, book) = hierarchy(&f, &user).await;
    for (path, body) in [
        (
            "shelves/create",
            json!({"workspace_id":f.setup.workspace_id.to_string(),"name":"notes","description":""}),
        ),
        (
            "books/create",
            json!({"workspace_id":f.setup.workspace_id.to_string(),"shelf_id":shelf["id"],"name":"book","description":""}),
        ),
    ] {
        code(&call(&f, &user, path, &body, 409).await, "content_conflict");
    }
    let good = create_entry(&f, &book["id"], "entry", "original");
    let first = call(&f, &user, "entries/mutate", &good, 200).await;
    let id = &first["data"]["entry_id"];
    let mut forged = good.clone();
    forged["owner_client_id"] = json!(Uuid::new_v4().to_string());
    let mut nested = good.clone();
    nested["change"]["owner_client_id"] = json!(Uuid::new_v4().to_string());
    let mut content = good.clone();
    content["change"]["content"]["owner_client_id"] = json!(Uuid::new_v4().to_string());
    let mut unknown = good.clone();
    unknown["change"]["action"] = json!("purge");
    let mut wrong = good.clone();
    wrong["operation_id"] = json!("bad uuid");
    for body in [forged, nested, content, unknown, wrong] {
        code(
            &call(&f, &user, "entries/mutate", &body, 400).await,
            "invalid_request",
        );
    }
    let mut different = good.clone();
    different["agent"] = json!("different attribution");
    code(
        &call(&f, &user, "entries/mutate", &different, 409).await,
        "content_conflict",
    );
    for limit in [0, 33, 501] {
        code(
            &call(
                &f,
                &user,
                "shelves/list",
                &json!({"workspace_id":f.setup.workspace_id.to_string(),"limit":limit}),
                400,
            )
            .await,
            "invalid_request",
        );
        code(&call(&f,&user,"entries/list",&json!({"workspace_id":f.setup.workspace_id.to_string(),"book_id":book["id"],"archived":false,"limit":limit}),400).await,"invalid_request");
    }
    code(
        &call(
            &f,
            &user,
            "shelves/list",
            &json!({"workspace_id":f.setup.workspace_id.to_string(),"after":"bad"}),
            400,
        )
        .await,
        "invalid_request",
    );
    code(&call(&f,&user,"shelves/create",&json!({"workspace_id":f.setup.workspace_id.to_string(),"name":"bad\nname","description":""}),400).await,"invalid_request");
    code(&call(&f,&user,"shelves/get",&json!({"workspace_id":f.setup.workspace_id.to_string(),"node_id":Uuid::new_v4().to_string()}),404).await,"not_found");
    assert_http_routing(&f, &user, id).await;
    assert_eq!(
        snapshot(&f, &user, id, None).await["content"]["text"],
        "original"
    );
    f.finish().await;
}

async fn assert_http_routing(f: &Fixture, user: &User, id: &Value) {
    code(
        &json_reply(
            user.client
                .get(f.mtls("/api/v1/content/entries/get"))
                .send()
                .await
                .unwrap(),
            405,
        )
        .await,
        "method_not_allowed",
    );
    code(
        &json_reply(
            f.public_client()
                .post(f.url("/api/v1/content/entries/get"))
                .json(&entry_target(f, id))
                .send()
                .await
                .unwrap(),
            401,
        )
        .await,
        "unauthorized",
    );
    code(
        &json_reply(
            user.client
                .post(f.mtls("/api/v1/content/entries/get"))
                .bearer_auth(f.setup.owner_key.secret())
                .json(&entry_target(f, id))
                .send()
                .await
                .unwrap(),
            400,
        )
        .await,
        "invalid_request",
    );
    code(
        &call(f, user, "entries/get?extra=yes", &entry_target(f, id), 400).await,
        "invalid_request",
    );
    code(
        &call(f, user, "entries/purge", &entry_target(f, id), 404).await,
        "not_found",
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn content_sql_fault_is_sanitized_and_rolled_back_and_legacy_refusal_follows_authorization() {
    let f = Fixture::new().await;
    let user = enrolled(&f, "read_write").await;
    let (_, book) = hierarchy(&f, &user).await;
    f.db().execute_batch("CREATE TRIGGER content_http_fault BEFORE INSERT ON ccp_access_audit WHEN NEW.action='create_entry' BEGIN SELECT RAISE(ABORT,'private fixture sql detail'); END;").unwrap();
    let create = create_entry(&f, &book["id"], "entry", "private body");
    code(
        &call(&f, &user, "entries/mutate", &create, 503).await,
        "service_unavailable",
    );
    for table in [
        "ccp_content_entries",
        "ccp_content_versions",
        "ccp_content_operations",
    ] {
        let count: u64 = f
            .db()
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }
    f.db()
        .execute_batch("DROP TRIGGER content_http_fault")
        .unwrap();
    let first = call(&f, &user, "entries/mutate", &create, 200).await;
    let id = &first["data"]["entry_id"];
    assert_eq!(
        snapshot(&f, &user, id, None).await["content"]["text"],
        "private body"
    );
    f.db().execute_batch("INSERT INTO sessions(id,name) VALUES(99,'old'); INSERT INTO message_packs(session_id,name,context) VALUES(99,'entry','native private');").unwrap();
    f.db()
        .execute(
            "UPDATE ccp_workspaces SET legacy_session_id=99 WHERE id=?1",
            [f.setup.workspace_id.to_string()],
        )
        .unwrap();
    code(
        &call(&f, &user, "entries/get", &entry_target(&f, id), 409).await,
        "content_conversion_required",
    );
    let other = f.second();
    code(
        &admin(
            &f,
            other.owner_key.secret(),
            "entries/get",
            &entry_target(&f, id),
            403,
        )
        .await,
        "forbidden",
    );
    let text: String = f
        .db()
        .query_row(
            "SELECT context FROM message_packs WHERE session_id=99",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(text, "native private");
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn content_maximum_escaped_text_and_append_delta_round_trip_with_explicit_byte_budgets() {
    let f = Fixture::new().await;
    let user = enrolled(&f, "read_write").await;
    let (_, book) = hierarchy(&f, &user).await;
    let first = call(
        &f,
        &user,
        "entries/mutate",
        &create_entry(&f, &book["id"], "entry", ""),
        200,
    )
    .await;
    let id = &first["data"]["entry_id"];
    let text = "\0".repeat(256 * 1024);
    let append = mutation(
        &f,
        json!({"action":"append","entry_id":id,"expected_revision":1,"text":text}),
    );
    let encoded = serde_json::to_vec(&append).unwrap();
    assert!(encoded.len() > 1024 * 1024);
    assert!(encoded.len() < 2 * 1024 * 1024);
    call(&f, &user, "entries/mutate", &append, 200).await;
    let response = user
        .client
        .post(f.mtls("/api/v1/content/entries/get"))
        .json(&entry_target(&f, id))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let bytes = response.bytes().await.unwrap();
    assert!(bytes.len() > 3 * 1024 * 1024);
    assert!(bytes.len() < 4 * 1024 * 1024);
    let snapshot: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(snapshot["data"]["content"]["text"], text);
    assert_eq!(snapshot["data"]["appended_text"], text);
    let too_large = json!({"workspace_id":f.setup.workspace_id.to_string(),"name":"large","description":"x".repeat(65536)});
    code(
        &call(&f, &user, "shelves/create", &too_large, 413).await,
        "request_too_large",
    );
    // Announce an oversized body without continuing to upload after the early
    // 413/close. An eager uploader may observe a connection reset instead.
    let mut wire = Wire::connect(&f, None).await;
    wire.send("POST /api/v1/content/entries/mutate HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: 2097153\r\n\r\n").await;
    let (status, reply) = wire.response().await;
    assert_eq!(status, 413);
    code(&reply, "request_too_large");
    drop(wire);
    let oversize = mutation(
        &f,
        json!({"action":"append","entry_id":id,"expected_revision":2,"text":"x"}),
    );
    code(
        &call(&f, &user, "entries/mutate", &oversize, 400).await,
        "invalid_request",
    );
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn content_full_escaped_description_pages_stay_below_the_existing_response_cap() {
    let f = Fixture::new().await;
    let user = enrolled(&f, "read_write").await;
    let description = "\0".repeat(4096);
    let mut expected = Vec::new();
    for index in 0..33 {
        let result=call(&f,&user,"shelves/create",&json!({"workspace_id":f.setup.workspace_id.to_string(),"name":format!("{index:02}{}","\\".repeat(254)),"description":description}),200).await;
        expected.push(result["data"]["id"].as_str().unwrap().to_owned());
    }
    expected.sort();
    let response = user
        .client
        .post(f.mtls("/api/v1/content/shelves/list"))
        .json(&json!({"workspace_id":f.setup.workspace_id.to_string()}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    let bytes = response.bytes().await.unwrap();
    assert!(bytes.len() < 1024 * 1024);
    let page: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(page["data"]["items"].as_array().unwrap().len(), 32);
    assert_eq!(page["data"]["items"][0]["description"], description);
    assert_eq!(page["data"]["next_cursor"], expected[31]);
    let next=call(&f,&user,"shelves/list",&json!({"workspace_id":f.setup.workspace_id.to_string(),"after":page["data"]["next_cursor"]}),200).await;
    assert_eq!(next["data"]["items"][0]["id"], expected[32]);
    assert_eq!(next["data"]["next_cursor"], Value::Null);
    f.finish().await;
}
