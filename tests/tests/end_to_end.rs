// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::net::TcpListener;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use once_cell::sync::Lazy;
use protocol::{ClientRequest, ServerResponse, SessionMetadata, SessionStats};
use serde::Serialize;
use uuid::Uuid;

static ENV_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

struct TestServer {
    _guard: MutexGuard<'static, ()>,
    task: tokio::task::JoinHandle<anyhow::Result<()>>,
    base_url: String,
    data_dir: std::path::PathBuf,
}

#[derive(Serialize)]
struct Envelope {
    subscribed_session_ids: Vec<i64>,
    request: ClientRequest,
}

impl TestServer {
    async fn start(initial_session: &str) -> anyhow::Result<Self> {
        let guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let probe = TcpListener::bind("127.0.0.1:0")?;
        let port = probe.local_addr()?.port();
        drop(probe);
        let base_url = format!("http://127.0.0.1:{port}");
        let data_dir = std::env::temp_dir().join(format!("ccp-http-test-{}", Uuid::new_v4()));
        unsafe {
            std::env::set_var("CCP_SERVER_DATA_DIR", &data_dir);
            std::env::set_var("CCP_HTTP_LISTENER_ADDR", format!("127.0.0.1:{port}"));
            std::env::set_var("CCP_HTTP_BASE_URL", &base_url);
        }
        let name = initial_session.to_string();
        let task = tokio::spawn(async move { server::run_plain_server(Some(&name)).await });
        let client = reqwest::Client::new();
        for _ in 0..50 {
            if client
                .get(format!("{base_url}/health"))
                .send()
                .await
                .is_ok()
            {
                return Ok(Self {
                    _guard: guard,
                    task,
                    base_url,
                    data_dir,
                });
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        anyhow::bail!("HTTP server did not become ready")
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.task.abort();
        let _ = std::fs::remove_dir_all(&self.data_dir);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn management_is_limited_and_multi_session_stats_work() -> anyhow::Result<()> {
    let server = TestServer::start("topic-one").await?;
    let client = reqwest::Client::new();
    let open: Vec<SessionMetadata> = client
        .get(format!("{}/v1/sessions", server.base_url))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(open.len(), 1);

    let created: SessionMetadata = client
        .post(format!("{}/v1/admin/sessions", server.base_url))
        .header("X-CCP-Admin-Key", server::DEFAULT_ADMIN_KEY)
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
        .header("X-CCP-Admin-Key", server::DEFAULT_ADMIN_KEY)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(stats.entries, 0);

    client
        .put(format!("{}/v1/admin/master", server.base_url))
        .header("X-CCP-Admin-Key", server::DEFAULT_ADMIN_KEY)
        .json(&serde_json::json!({"content": "global command"}))
        .send()
        .await?
        .error_for_status()?;
    client
        .put(format!(
            "{}/v1/admin/sessions/topic-two/master",
            server.base_url
        ))
        .header("X-CCP-Admin-Key", server::DEFAULT_ADMIN_KEY)
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
        .get(format!("{}/admin", server.base_url))
        .send()
        .await?
        .error_for_status()?;

    client
        .delete(format!("{}/v1/admin/sessions/topic-two", server.base_url))
        .header("X-CCP-Admin-Key", server::DEFAULT_ADMIN_KEY)
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn requests_require_the_selected_topic_subscription() -> anyhow::Result<()> {
    let server = TestServer::start("open-topic").await?;
    let client = reqwest::Client::new();
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
    Ok(())
}
