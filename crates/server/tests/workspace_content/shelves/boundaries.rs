// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shelf_http_strict_inputs_surface_routing_and_original_body_budget_are_enforced() {
    let f = Fixture::new().await;
    let user = enrolled(&f, "read_write").await;
    let reader = enrolled(&f, "read").await;
    let (shelf, _) = hierarchy(&f, &user).await;
    let http = f.public_client();
    let body = request(&f, &shelf["id"], 1, true);
    for (field, value) in [
        ("actor", json!("PRIVATE_BODY")),
        ("admin_key_id", json!(Uuid::new_v4().to_string())),
        ("expected_revision", json!(true)),
        ("expected_revision", json!(0)),
        ("expected_revision", json!(i64::MAX)),
        ("expected_revision", json!(u64::MAX)),
        ("archived", json!("true")),
        ("shelf_id", json!("bad")),
        ("operation_id", json!(Uuid::nil().to_string())),
    ] {
        let mut invalid = body.clone();
        invalid[field] = value;
        code(
            &change(&http, &f, f.setup.owner_key.secret(), &invalid, 400).await,
            "invalid_request",
        );
    }
    assert_surfaces(&f, &http, &user, &reader, &body, &shelf).await;
    let mut wire = Wire::connect(&f, None).await;
    wire.send(&format!("POST {TRANSITION} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: 65537\r\n\r\n")).await;
    let (status, reply) = wire.response().await;
    assert_eq!(status, 413);
    code(&reply, "request_too_large");
    drop(wire);
    assert_eq!(receipts(&f), 0);
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shelf_http_archive_pages_are_safe_bounded_and_current_scope_only() {
    let f = Fixture::new().await;
    let user = enrolled(&f, "read_write").await;
    let reader = enrolled(&f, "read").await;
    let (shelf, _) = hierarchy(&f, &user).await;
    let http = f.public_client();
    let key = f.setup.owner_key.secret();
    let mut ids = vec![shelf["id"].clone()];
    let long = "é".repeat(128);
    for name in [long.as_str(), "third"] {
        let shelf=call(&f,&user,"shelves/create",&json!({"workspace_id":f.setup.workspace_id.to_string(),"name":name,"description":"PRIVATE_DESCRIPTION"}),200).await;
        ids.push(shelf["data"]["id"].clone());
    }
    for id in &ids {
        change(&http, &f, key, &request(&f, id, 1, true), 200).await;
    }
    let mut after = Value::Null;
    let mut seen = Vec::new();
    loop {
        let page = call(
            &f,
            &reader,
            "shelves/archived",
            &json!({"workspace_id":f.setup.workspace_id.to_string(),"after":after,"limit":1}),
            200,
        )
        .await;
        let items = page["data"]["items"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].as_object().unwrap().len(), 5);
        seen.push(items[0]["id"].clone());
        after = page["data"]["next_cursor"].clone();
        if after.is_null() {
            break;
        }
        assert!(seen.len() < 4);
    }
    ids.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
    assert_eq!(seen, ids);
    let query = json!({"workspace_id":f.setup.workspace_id.to_string(),"limit":32});
    let all = call(&f, &reader, "shelves/archived", &query, 200).await;
    assert!(!all.to_string().contains("PRIVATE_DESCRIPTION"));
    assert!(
        all["data"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["name"] == long)
    );
    assert_eq!(
        admin(
            &f,
            f.setup.instance_key.secret(),
            "shelves/archived",
            &query,
            200
        )
        .await,
        all
    );
    assert_page_bounds_scope(&f, &reader, &shelf).await;
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shelf_http_legacy_conversion_refusal_follows_current_auth_without_empty_fallback() {
    let f = Fixture::new().await;
    let user = enrolled(&f, "read_write").await;
    let (shelf, _) = hierarchy(&f, &user).await;
    let second = f.second();
    f.db()
        .execute_batch("INSERT INTO sessions(id,name) VALUES(99,'legacy');")
        .unwrap();
    f.db()
        .execute(
            "UPDATE ccp_workspaces SET legacy_session_id=99 WHERE id=?1",
            [f.setup.workspace_id.to_string()],
        )
        .unwrap();
    let http = f.public_client();
    let body = request(&f, &shelf["id"], 1, true);
    code(
        &change(&http, &f, f.setup.owner_key.secret(), &body, 409).await,
        "content_conversion_required",
    );
    code(
        &change(&http, &f, second.owner_key.secret(), &body, 403).await,
        "forbidden",
    );
    for (path, body) in [
        ("shelves/state", target(&f, &shelf["id"])),
        (
            "shelves/archived",
            json!({"workspace_id":f.setup.workspace_id.to_string()}),
        ),
    ] {
        code(
            &call(&f, &user, path, &body, 409).await,
            "content_conversion_required",
        );
        code(
            &admin(&f, f.setup.instance_key.secret(), path, &body, 409).await,
            "content_conversion_required",
        );
        code(
            &admin(&f, second.owner_key.secret(), path, &body, 403).await,
            "forbidden",
        );
    }
    assert_eq!(receipts(&f), 0);
    f.finish().await;
}

async fn assert_surfaces(
    f: &Fixture,
    http: &Client,
    user: &User,
    reader: &User,
    body: &Value,
    shelf: &Value,
) {
    for u in [user, reader] {
        code(
            &json_reply(
                u.client
                    .post(f.mtls(TRANSITION))
                    .json(body)
                    .send()
                    .await
                    .unwrap(),
                404,
            )
            .await,
            "not_found",
        );
    }
    for path in [
        TRANSITION,
        "/api/v1/content/shelves/state",
        "/api/v1/content/shelves/archived",
    ] {
        code(
            &json_reply(http.post(f.url(path)).json(body).send().await.unwrap(), 401).await,
            "unauthorized",
        );
        code(
            &json_reply(
                http.get(f.url(path))
                    .bearer_auth(f.setup.owner_key.secret())
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
                http.post(f.url(&format!("{path}?actor=claimed")))
                    .bearer_auth(f.setup.owner_key.secret())
                    .json(body)
                    .send()
                    .await
                    .unwrap(),
                400,
            )
            .await,
            "invalid_request",
        );
    }
    code(
        &json_reply(
            user.client
                .post(f.mtls(TRANSITION))
                .bearer_auth(f.setup.owner_key.secret())
                .json(body)
                .send()
                .await
                .unwrap(),
            400,
        )
        .await,
        "invalid_request",
    );
    for (path, body) in [
        (
            "shelves/state",
            json!({"workspace_id":f.setup.workspace_id.to_string(),"node_id":shelf["id"],"actor":"PRIVATE_BODY"}),
        ),
        (
            "shelves/archived",
            json!({"workspace_id":f.setup.workspace_id.to_string(),"actor":"PRIVATE_BODY"}),
        ),
    ] {
        code(&call(f, user, path, &body, 400).await, "invalid_request");
    }
}

async fn assert_page_bounds_scope(f: &Fixture, reader: &User, shelf: &Value) {
    for limit in [0, 33, 501] {
        code(
            &call(
                f,
                reader,
                "shelves/archived",
                &json!({"workspace_id":f.setup.workspace_id.to_string(),"limit":limit}),
                400,
            )
            .await,
            "invalid_request",
        );
    }
    code(
        &call(
            f,
            reader,
            "shelves/archived",
            &json!({"workspace_id":f.setup.workspace_id.to_string(),"after":"bad"}),
            400,
        )
        .await,
        "invalid_request",
    );
    let second = f.second();
    let wrong = json!({"workspace_id":second.workspace_id.to_string(),"node_id":shelf["id"]});
    code(
        &admin(f, second.owner_key.secret(), "shelves/state", &wrong, 404).await,
        "not_found",
    );
    code(
        &call(f, reader, "shelves/state", &wrong, 403).await,
        "forbidden",
    );
    assert!(
        admin(
            f,
            second.owner_key.secret(),
            "shelves/archived",
            &json!({"workspace_id":second.workspace_id.to_string()}),
            200
        )
        .await["data"]["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
