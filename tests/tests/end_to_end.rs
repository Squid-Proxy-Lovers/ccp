// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use ccp_tests::harness::TestServer;
use protocol::{ClientRequest, ServerResponse, SessionMetadata, SessionStats};
use serde::Serialize;

#[derive(Serialize)]
struct Envelope {
    subscribed_session_ids: Vec<i64>,
    request: ClientRequest,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn management_is_limited_and_multi_session_stats_work() -> anyhow::Result<()> {
    let server = TestServer::start_named("topic-one").await?;
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("X-CCP-Client-Key", server.client_key.parse()?);
    let client = reqwest::Client::builder()
        .default_headers(headers)
        .build()?;
    let open: Vec<SessionMetadata> = client
        .get(format!("{}/v1/sessions", server.base_url))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(open.len(), 1);
    for key in [None, Some("wrong-admin-key")] {
        let mut request = client
            .post(format!("{}/v1/admin/sessions", server.base_url))
            .json(&serde_json::json!({"session_name": "must-not-create"}));
        if let Some(key) = key {
            request = request.header("X-CCP-Admin-Key", key);
        }
        assert_eq!(
            request.send().await?.status(),
            reqwest::StatusCode::UNAUTHORIZED
        );
    }

    let created: SessionMetadata = client
        .post(format!("{}/v1/admin/sessions", server.base_url))
        .header("X-CCP-Admin-Key", &server.admin_key)
        .json(&serde_json::json!({"session_name": "topic-two"}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(created.session_name, "topic-two");

    let stats: SessionStats = client
        .get(format!(
            "{}/v1/admin/sessions/topic-two/stats",
            server.base_url
        ))
        .header("X-CCP-Admin-Key", &server.admin_key)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(stats.entries, 0);

    client
        .put(format!("{}/v1/admin/master", server.base_url))
        .header("X-CCP-Admin-Key", &server.admin_key)
        .json(&serde_json::json!({"content": "global command"}))
        .send()
        .await?
        .error_for_status()?;
    client
        .put(format!(
            "{}/v1/admin/sessions/topic-two/master",
            server.base_url
        ))
        .header("X-CCP-Admin-Key", &server.admin_key)
        .json(&serde_json::json!({"content": "session command"}))
        .send()
        .await?
        .error_for_status()?;
    let instructions: ServerResponse = client
        .post(format!("{}/v1/request", server.base_url))
        .json(&Envelope {
            subscribed_session_ids: vec![created.session_id],
            request: ClientRequest::GetMasterInstructions {
                session_id: created.session_id,
            },
        })
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert!(
        matches!(instructions, ServerResponse::MasterInstructions(value)
        if value.global.content == "global command" && value.session.content == "session command")
    );

    client
        .post(format!("{}/v1/request", server.base_url))
        .json(&Envelope {
            subscribed_session_ids: vec![created.session_id],
            request: ClientRequest::AddEntry {
                session_id: created.session_id,
                name: "agent-progress".to_string(),
                description: "live work update".to_string(),
                labels: vec!["status".to_string()],
                context: "Finished the first implementation phase.".to_string(),
                shelf_name: "main".to_string(),
                book_name: "default".to_string(),
            },
        })
        .send()
        .await?
        .error_for_status()?;
    let activity: serde_json::Value = client
        .get(format!(
            "{}/v1/admin/activity?session=topic-two&limit=20",
            server.base_url
        ))
        .header("X-CCP-Admin-Key", &server.admin_key)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(activity[0]["session_name"], "topic-two");
    assert_eq!(activity[0]["entry_name"], "agent-progress");
    assert_eq!(
        activity[0]["content"],
        "Finished the first implementation phase."
    );

    client
        .get(format!("{}/admin", server.base_url))
        .send()
        .await?
        .error_for_status()?;

    client
        .delete(format!("{}/v1/admin/sessions/topic-two", server.base_url))
        .header("X-CCP-Admin-Key", &server.admin_key)
        .send()
        .await?
        .error_for_status()?;
    server.stop().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn requests_require_the_selected_topic_subscription() -> anyhow::Result<()> {
    let server = TestServer::start_named("open-topic").await?;
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("X-CCP-Client-Key", server.client_key.parse()?);
    let client = reqwest::Client::builder()
        .default_headers(headers)
        .build()?;
    let sessions: Vec<SessionMetadata> = client
        .get(format!("{}/v1/sessions", server.base_url))
        .send()
        .await?
        .json()
        .await?;
    let session_id = sessions[0].session_id;

    let denied = client
        .post(format!("{}/v1/request", server.base_url))
        .json(&Envelope {
            subscribed_session_ids: Vec::new(),
            request: ClientRequest::List { session_id },
        })
        .send()
        .await?;
    assert_eq!(denied.status(), reqwest::StatusCode::FORBIDDEN);

    let allowed: ServerResponse = client
        .post(format!("{}/v1/request", server.base_url))
        .json(&Envelope {
            subscribed_session_ids: vec![session_id],
            request: ClientRequest::List { session_id },
        })
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert!(matches!(allowed, ServerResponse::EntrySummaries(entries) if entries.is_empty()));
    server.stop().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_harness_crud_and_load_work() -> anyhow::Result<()> {
    use ccp_tests::harness::{LoadOperation, run_persistent_load};
    let server = TestServer::start().await?;
    let client = server.subscribe().await?;
    client.add("seed", "test entry", "initial context").await?;
    client.append("seed", "appended context").await?;
    assert!(
        client
            .get("seed")
            .await?
            .context
            .contains("appended context")
    );
    let result =
        run_persistent_load(vec![client.clone(), client.clone()], 3, LoadOperation::List).await?;
    assert_eq!(result.total_requests, 6);
    assert!(
        run_persistent_load(
            vec![client.clone()],
            1,
            LoadOperation::Get {
                entry_names: Vec::new()
            }
        )
        .await
        .is_err()
    );
    let deleted = client.delete("seed").await?;
    client.restore(&deleted.entry_key).await?;
    assert_eq!(client.list().await?.len(), 1);
    server.stop().await?;
    Ok(())
}
