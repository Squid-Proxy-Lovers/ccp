// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn status_http_strict_dtos_methods_query_media_and_original_body_budget() {
    let f = Fixture::new().await;
    let writer = enrolled(&f, "read_write").await;
    let id = Uuid::new_v4();
    let request = set(&f, id, 0, "valid");
    for (field, value) in [
        ("client_id", json!(writer.identity["client_id"])),
        ("actor", json!("admin")),
        ("expected_revision", json!(-1)),
        ("expected_revision", json!(1.5)),
        ("expected_revision", json!("0")),
        ("status_id", json!("invalid")),
        ("operation_id", json!(Uuid::nil().to_string())),
    ] {
        let mut bad = request.clone();
        bad[field] = value;
        status(&f, &writer.client, "mutate", &bad, 400).await;
    }
    for change in [
        json!({"action":"set","message":"okay","client_id":writer.identity["client_id"]}),
        json!({"action":"clear","message":"not allowed"}),
        json!({"action":"set","agent":"","message":"okay"}),
        json!({"action":"set","agent":null,"message":"é".repeat(2049)}),
        json!({"action":"unknown"}),
    ] {
        let mut bad = request.clone();
        bad["change"] = change;
        status(&f, &writer.client, "mutate", &bad, 400).await;
    }
    for limit in [0, 33, 65535] {
        status(
            &f,
            &writer.client,
            "list",
            &json!({"workspace_id":f.setup.workspace_id.to_string(),"limit":limit}),
            400,
        )
        .await;
    }
    status(&f,&writer.client,"get",&json!({"workspace_id":f.setup.workspace_id.to_string(),"status_id":id.to_string(),"unknown":true}),400).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn status_http_methods_query_media_and_original_body_budget() {
    let f = Fixture::new().await;
    let writer = enrolled(&f, "read_write").await;
    let request = set(&f, Uuid::new_v4(), 0, "valid");
    json_reply(
        writer
            .client
            .get(f.mtls(&format!("{BASE}list")))
            .send()
            .await
            .unwrap(),
        405,
    )
    .await;
    json_reply(
        writer
            .client
            .post(f.mtls(&format!("{BASE}list?key=x")))
            .json(&json!({"workspace_id":f.setup.workspace_id.to_string()}))
            .send()
            .await
            .unwrap(),
        400,
    )
    .await;
    json_reply(
        writer
            .client
            .post(f.mtls(&format!("{BASE}mutate")))
            .header("content-type", "text/plain")
            .body(request.to_string())
            .send()
            .await
            .unwrap(),
        415,
    )
    .await;
    json_reply(
        writer
            .client
            .post(f.mtls(&format!("{BASE}mutate")))
            .header("content-type", "application/json")
            .body("x".repeat(65537))
            .send()
            .await
            .unwrap(),
        413,
    )
    .await;
    json_reply(
        writer
            .client
            .post(f.mtls(&format!("{BASE}list")))
            .bearer_auth(f.setup.owner_key.secret())
            .json(&json!({"workspace_id":f.setup.workspace_id.to_string()}))
            .send()
            .await
            .unwrap(),
        400,
    )
    .await;
    json_reply(
        f.public_client()
            .post(f.url(&format!("{BASE}list")))
            .json(&json!({"workspace_id":f.setup.workspace_id.to_string()}))
            .send()
            .await
            .unwrap(),
        401,
    )
    .await;
    assert_eq!(
        f.db()
            .query_row("SELECT count(*) FROM ccp_work_status", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn status_http_real_pages_bound_output_and_audit_fault_rolls_back_without_private_detail() {
    let f = Fixture::new().await;
    let writer = enrolled(&f, "read_write").await;
    let mut ids = Vec::new();
    for _ in 0..3 {
        let id = Uuid::new_v4();
        status(
            &f,
            &writer.client,
            "mutate",
            &set(&f, id, 0, "private message"),
            200,
        )
        .await;
        ids.push(id.to_string());
    }
    let mut seen = Vec::new();
    let mut body = json!({"workspace_id":f.setup.workspace_id.to_string(),"limit":1});
    loop {
        let page = status(&f, &writer.client, "list", &body, 200).await;
        let items = page["data"]["items"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        seen.push(items[0]["status_id"].as_str().unwrap().to_string());
        if page["data"]["next_cursor"].is_null() {
            break;
        }
        assert_eq!(page["data"]["next_cursor"], items[0]["status_id"]);
        body["after"] = page["data"]["next_cursor"].clone();
    }
    ids.sort();
    assert_eq!(seen, ids);
    let id = Uuid::parse_str(&ids[0]).unwrap();
    let request = set(&f, id, 1, "must roll back");
    for fault in ["ABORT,'PRIVATE SQL DETAILS'", "IGNORE"] {
        f.db().execute_batch(&format!("CREATE TRIGGER status_http_fault BEFORE INSERT ON ccp_access_audit WHEN NEW.action='change_work_status' BEGIN SELECT RAISE({fault}); END;")).unwrap();
        let error = status(&f, &writer.client, "mutate", &request, 503).await;
        code(&error, "service_unavailable");
        assert!(!error.to_string().contains("PRIVATE SQL"));
        assert!(!error.to_string().contains("must roll back"));
        let state = status(&f, &writer.client, "get", &target(&f, id), 200).await;
        assert_eq!(state["data"]["revision"], 1);
        assert_eq!(state["data"]["message"], "private message");
        f.db()
            .execute_batch("DROP TRIGGER status_http_fault")
            .unwrap();
    }
    assert_eq!(
        status(&f, &writer.client, "mutate", &request, 200).await["data"]["revision"],
        2
    );
}
