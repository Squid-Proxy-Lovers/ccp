// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
mod boundaries;
mod moderation;
const BASE: &str = "/api/v1/work/status/";
fn target(f: &Fixture, id: Uuid) -> Value {
    json!({"workspace_id":f.setup.workspace_id.to_string(),"status_id":id.to_string()})
}
fn set(f: &Fixture, id: Uuid, revision: u64, text: &str) -> Value {
    json!({"workspace_id":f.setup.workspace_id.to_string(),"status_id":id.to_string(),"operation_id":Uuid::new_v4().to_string(),"expected_revision":revision,"change":{"action":"set","agent":"same label","message":text}})
}
async fn status(f: &Fixture, client: &Client, action: &str, body: &Value, expected: u16) -> Value {
    json_reply(
        client
            .post(f.mtls(&format!("{BASE}{action}")))
            .json(body)
            .send()
            .await
            .unwrap(),
        expected,
    )
    .await
}
async fn inspect(f: &Fixture, key: &str, action: &str, body: &Value, expected: u16) -> Value {
    json_reply(
        f.public_client()
            .post(f.url(&format!("{BASE}{action}")))
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
async fn status_http_client_identity_stale_reads_clear_and_historical_replay_preserve_latest_state()
{
    let f = Fixture::new().await;
    let writer = enrolled(&f, "read_write").await;
    let reader = enrolled(&f, "read").await;
    let other = enrolled(&f, "read_write").await;
    let id = Uuid::new_v4();
    let request = set(&f, id, 0, "Original private work");
    let receipt = status(&f, &writer.client, "mutate", &request, 200).await;
    assert_eq!(receipt["data"].as_object().unwrap().len(), 7);
    assert_eq!(receipt["data"]["client_id"], writer.identity["client_id"]);
    assert!(!receipt.to_string().contains("Original private work"));
    assert!(!receipt.to_string().contains("same label"));
    assert_eq!(
        status(&f, &writer.client, "mutate", &request, 200).await,
        receipt
    );
    let current = status(&f, &reader.client, "get", &target(&f, id), 200).await;
    assert_eq!(current["data"]["stale"], false);
    assert_eq!(current["data"]["message"], "Original private work");
    assert_eq!(current["data"]["agent"], "same label");
    assert_eq!(current["data"].as_object().unwrap().len(), 10);
    f.db()
        .execute(
            "UPDATE ccp_work_status SET updated_at=?2 WHERE id=?1",
            rusqlite::params![id.to_string(), now() - 301],
        )
        .unwrap();
    let page = status(
        &f,
        &reader.client,
        "list",
        &json!({"workspace_id":f.setup.workspace_id.to_string(),"limit":32}),
        200,
    )
    .await;
    assert_eq!(page["data"]["items"][0]["stale"], true);
    assert_eq!(page["data"]["stale_after_seconds"], 300);
    assert_eq!(page["data"]["items"][0]["message"], "Original private work");
    status(
        &f,
        &reader.client,
        "mutate",
        &set(&f, Uuid::new_v4(), 0, "read only"),
        403,
    )
    .await;
    status(
        &f,
        &other.client,
        "mutate",
        &set(&f, id, 1, "impersonation"),
        403,
    )
    .await;
    status(&f, &writer.client, "mutate", &set(&f, id, 1, "latest"), 200).await;
    let clear = json!({"workspace_id":f.setup.workspace_id.to_string(),"status_id":id.to_string(),"operation_id":Uuid::new_v4().to_string(),"expected_revision":2,"change":{"action":"clear"}});
    status(&f, &other.client, "mutate", &clear, 403).await;
    let cleared = status(&f, &writer.client, "mutate", &clear, 200).await;
    assert_eq!(cleared["data"]["revision"], 3);
    assert_eq!(cleared["data"]["cleared"], true);
    assert_eq!(
        status(&f, &writer.client, "mutate", &request, 200).await,
        receipt
    );
    let value = status(&f, &reader.client, "get", &target(&f, id), 200).await;
    assert_eq!(value["data"]["message"], Value::Null);
    assert_eq!(value["data"]["agent"], Value::Null);
    assert_eq!(value["data"]["revision"], 3);
    assert_eq!(value["data"]["stale"], true);
    let page = status(
        &f,
        &reader.client,
        "list",
        &json!({"workspace_id":f.setup.workspace_id.to_string()}),
        200,
    )
    .await;
    assert!(page["data"]["items"].as_array().unwrap().is_empty());
    assert_eq!(page["data"]["next_cursor"], Value::Null);
    let mut changed = request.clone();
    changed["change"]["message"] = json!("changed request");
    code(
        &status(&f, &writer.client, "mutate", &changed, 409).await,
        "content_conflict",
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn status_http_real_renewal_keeps_client_ownership_while_writer_grant_revocation_blocks_calls()
 {
    let f = Fixture::new().await;
    let writer = enrolled(&f, "read_write").await;
    let id = Uuid::new_v4();
    let original = set(&f, id, 0, "before renewal");
    status(&f, &writer.client, "mutate", &original, 200).await;
    let key = KeyPair::generate().unwrap();
    let renewed = json_reply(
        writer
            .client
            .post(f.mtls("/api/v1/credentials/renew"))
            .json(&json!({"request_id":Uuid::new_v4().to_string(),"csr_pem":csr(&key)}))
            .send()
            .await
            .unwrap(),
        200,
    )
    .await;
    assert_eq!(renewed["data"]["client_id"], writer.identity["client_id"]);
    let client = f.private_client(renewed["data"]["certificate_pem"].as_str().unwrap(), &key);
    status(&f, &writer.client, "get", &target(&f, id), 403).await;
    status(&f, &writer.client, "mutate", &original, 403).await;
    status(&f, &client, "mutate", &original, 409).await;
    let proof = status(
        &f,
        &client,
        "mutate",
        &set(&f, id, 1, "same client after renewal"),
        200,
    )
    .await;
    assert_eq!(proof["data"]["client_id"], writer.identity["client_id"]);
    let grant:String=f.db().query_row("SELECT id FROM ccp_grants WHERE workspace_id=?1 AND client_id=?2 AND revoked_at IS NULL",rusqlite::params![f.setup.workspace_id.to_string(),writer.identity["client_id"].as_str().unwrap()],|r|r.get(0)).unwrap();
    json_reply(
        f.public_client()
            .post(f.url("/api/v1/admin/grants/revoke"))
            .bearer_auth(f.setup.owner_key.secret())
            .json(&json!({"grant_id":grant}))
            .send()
            .await
            .unwrap(),
        200,
    )
    .await;
    status(&f, &client, "get", &target(&f, id), 403).await;
    status(&f, &client, "mutate", &set(&f, id, 2, "revoked"), 403).await;
    let retained = inspect(
        &f,
        f.setup.instance_key.secret(),
        "get",
        &target(&f, id),
        200,
    )
    .await;
    assert_eq!(retained["data"]["message"], "same client after renewal");
    assert_eq!(retained["data"]["client_id"], writer.identity["client_id"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn status_http_archive_disable_foreign_scope_and_display_only_metadata_follow_current_authority()
 {
    let f = Fixture::new().await;
    let writer = enrolled(&f, "read_write").await;
    let id = Uuid::new_v4();
    let request = set(&f, id, 0, "last message");
    status(&f, &writer.client, "mutate", &request, 200).await;
    let other = f.second();
    inspect(&f, other.owner_key.secret(), "get", &target(&f, id), 403).await;
    let foreign = json!({"workspace_id":other.workspace_id.to_string()});
    status(&f, &writer.client, "list", &foreign, 403).await;
    json_reply(
        f.public_client()
            .post(f.url("/api/v1/admin/workspaces/archive"))
            .bearer_auth(f.setup.owner_key.secret())
            .json(&json!({"workspace_id":f.setup.workspace_id.to_string()}))
            .send()
            .await
            .unwrap(),
        200,
    )
    .await;
    status(&f, &writer.client, "get", &target(&f, id), 200).await;
    status(&f, &writer.client, "mutate", &request, 403).await;
    let clear = json!({"workspace_id":f.setup.workspace_id.to_string(),"status_id":id.to_string(),"operation_id":Uuid::new_v4().to_string(),"expected_revision":1,"change":{"action":"clear"}});
    status(&f, &writer.client, "mutate", &clear, 403).await;
    json_reply(
        f.public_client()
            .post(f.url("/api/v1/admin/clients/disable"))
            .bearer_auth(f.setup.instance_key.secret())
            .json(&json!({"client_id":writer.identity["client_id"]}))
            .send()
            .await
            .unwrap(),
        200,
    )
    .await;
    status(&f, &writer.client, "get", &target(&f, id), 403).await;
    let retained = inspect(
        &f,
        f.setup.instance_key.secret(),
        "get",
        &target(&f, id),
        200,
    )
    .await;
    assert_eq!(retained["data"]["message"], "last message");
    assert_eq!(retained["data"]["client_disabled"], true);
    for key in [f.setup.instance_key.secret(), f.setup.owner_key.secret()] {
        inspect(&f, key, "mutate", &request, 404).await;
    }
}
