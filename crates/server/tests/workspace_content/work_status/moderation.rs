// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
const CLEAR: &str = "/api/v1/admin/work/status/clear";
fn clear(f: &Fixture, id: Uuid, revision: u64) -> Value {
    json!({"workspace_id":f.setup.workspace_id.to_string(),"status_id":id.to_string(),"operation_id":Uuid::new_v4().to_string(),"expected_revision":revision})
}
async fn moderate(f: &Fixture, key: &str, body: &Value, expected: u16) -> Value {
    json_reply(
        f.public_client()
            .post(f.url(CLEAR))
            .bearer_auth(key)
            .json(body)
            .send()
            .await
            .unwrap(),
        expected,
    )
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn status_moderation_https_preserves_owner_and_historical_client_and_key_receipts() {
    let f = Fixture::new().await;
    let writer = enrolled(&f, "read_write").await;
    let id = Uuid::new_v4();
    let request = set(&f, id, 0, "private original");
    let original = status(&f, &writer.client, "mutate", &request, 200).await;
    let action = clear(&f, id, 1);
    let owner = f.setup.owner_key.secret();
    moderate(&f, f.setup.instance_key.secret(), &action, 403).await;
    let receipt = moderate(&f, owner, &action, 200).await;
    assert_eq!(receipt["data"]["client_id"], writer.identity["client_id"]);
    assert_eq!(receipt["data"]["revision"], 2);
    assert_eq!(receipt["data"]["cleared"], true);
    assert_eq!(receipt["data"].as_object().unwrap().len(), 7);
    assert!(!receipt.to_string().contains("private original"));
    assert_eq!(
        status(&f, &writer.client, "mutate", &request, 200).await,
        original
    );
    assert!(
        status(&f, &writer.client, "get", &target(&f, id), 200).await["data"]["message"].is_null()
    );
    status(
        &f,
        &writer.client,
        "mutate",
        &set(&f, id, 2, "new report"),
        200,
    )
    .await;
    assert_eq!(moderate(&f, owner, &action, 200).await, receipt);
    assert_eq!(
        status(&f, &writer.client, "get", &target(&f, id), 200).await["data"]["message"],
        "new report"
    );
    let mut changed = action.clone();
    changed["expected_revision"] = json!(3);
    moderate(&f, owner, &changed, 409).await;
    // The same operation UUID cannot switch from a client operation to moderation.
    let mut collision = clear(&f, id, 3);
    collision["operation_id"] = request["operation_id"].clone();
    moderate(&f, owner, &collision, 409).await;
    let foreign = f.second();
    let mut wrong = action.clone();
    wrong["workspace_id"] = json!(foreign.workspace_id.to_string());
    moderate(&f, owner, &wrong, 403).await;
    json_reply(
        f.public_client()
            .post(f.url("/api/v1/admin/workspaces/archive"))
            .bearer_auth(owner)
            .json(&json!({"workspace_id":f.setup.workspace_id.to_string()}))
            .send()
            .await
            .unwrap(),
        200,
    )
    .await;
    moderate(&f, owner, &action, 403).await;
    moderate(&f, owner, &clear(&f, id, 3), 403).await;
    assert_eq!(
        inspect(
            &f,
            owner,
            "list",
            &json!({"workspace_id":f.setup.workspace_id.to_string()}),
            200
        )
        .await["data"]["items"][0]["message"],
        "new report"
    );
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn status_moderation_rejects_claims_types_wrong_surfaces_and_keeps_the_original_request_cap()
{
    let f = Fixture::new().await;
    let writer = enrolled(&f, "read_write").await;
    let id = Uuid::new_v4();
    let body = clear(&f, id, 1);
    let owner = f.setup.owner_key.secret();
    for (field, value) in [
        ("client_id", json!("PRIVATE_CLAIM")),
        ("change", json!({"action":"set","message":"PRIVATE_CLAIM"})),
        ("message", json!("PRIVATE_CLAIM")),
        ("expected_revision", json!(true)),
        ("expected_revision", json!(1.5)),
        ("expected_revision", json!(i64::MAX)),
        ("operation_id", json!(Uuid::nil().to_string())),
    ] {
        let mut bad = body.clone();
        bad[field] = value;
        assert!(
            !moderate(&f, owner, &bad, 400)
                .await
                .to_string()
                .contains("PRIVATE_CLAIM")
        );
    }
    json_reply(
        f.public_client()
            .post(f.url(CLEAR))
            .json(&body)
            .send()
            .await
            .unwrap(),
        401,
    )
    .await;
    json_reply(
        f.public_client()
            .get(f.url(CLEAR))
            .bearer_auth(owner)
            .send()
            .await
            .unwrap(),
        405,
    )
    .await;
    json_reply(
        f.public_client()
            .post(f.url(&format!("{CLEAR}?key=x")))
            .bearer_auth(owner)
            .json(&body)
            .send()
            .await
            .unwrap(),
        400,
    )
    .await;
    json_reply(
        f.public_client()
            .post(f.url(CLEAR))
            .bearer_auth(owner)
            .header("content-type", "text/plain")
            .body(body.to_string())
            .send()
            .await
            .unwrap(),
        415,
    )
    .await;
    json_reply(
        f.public_client()
            .post(f.url(CLEAR))
            .bearer_auth(owner)
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
            .post(f.mtls(CLEAR))
            .json(&body)
            .send()
            .await
            .unwrap(),
        404,
    )
    .await;
    assert_eq!(
        f.db()
            .query_row("SELECT count(*) FROM ccp_work_status_operations", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn status_moderation_audit_failure_returns_sanitized_unavailable_with_original_text_and_revision()
 {
    for fault in ["IGNORE", "ABORT,'PRIVATE_WORK'"] {
        let f = Fixture::new().await;
        let writer = enrolled(&f, "read_write").await;
        let id = Uuid::new_v4();
        status(
            &f,
            &writer.client,
            "mutate",
            &set(&f, id, 0, "retained"),
            200,
        )
        .await;
        f.db().execute_batch(&format!("CREATE TRIGGER moderation_fault BEFORE INSERT ON ccp_access_audit WHEN NEW.action='clear_client_work_status' BEGIN SELECT RAISE({fault}); END;")).unwrap();
        let body = clear(&f, id, 1);
        let failed = moderate(&f, f.setup.owner_key.secret(), &body, 503).await;
        assert!(!failed.to_string().contains("PRIVATE_WORK"));
        assert_eq!(
            status(&f, &writer.client, "get", &target(&f, id), 200).await["data"]["message"],
            "retained"
        );
        assert_eq!(
            f.db()
                .query_row("SELECT count(*) FROM ccp_work_status_operations", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        f.db()
            .execute_batch("DROP TRIGGER moderation_fault")
            .unwrap();
        assert_eq!(
            moderate(&f, f.setup.owner_key.secret(), &body, 200).await["data"]["revision"],
            2
        );
        f.finish().await;
    }
}
