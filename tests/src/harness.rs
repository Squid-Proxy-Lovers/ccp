// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::env;
use std::fs;
use std::net::TcpListener as StdTcpListener;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use once_cell::sync::Lazy;
use protocol::{
    AgentStatus, AppendMetadata, AppendResult, ClearStatusResult, ClientRequest, DeleteResult,
    DeletedEntrySummary, EntrySummary, ErrorCode, ErrorResponse, MessageEntry, MessageHistoryEntry,
    PROTOCOL_VERSION, RestoreResult, SearchContextMatch, ServerResponse, SessionMetadata,
    VersionInfo,
};
use reqwest::StatusCode;
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;
use tokio::time::sleep;
use uuid::Uuid;

static TEST_ENV_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));
const CONNECT_SETUP_CONCURRENCY: usize = 128;

pub struct TestServer {
    _guard: MutexGuard<'static, ()>,
    saved_env: Vec<(&'static str, Option<std::ffi::OsString>)>,
    data_dir: PathBuf,
    server_task: Option<JoinHandle<anyhow::Result<()>>>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    pub session_name: String,
    pub session_id: i64,
    pub base_url: String,
    pub client_key: String,
    pub admin_key: String,
    http_client: reqwest::Client,
}

#[derive(Clone)]
pub struct SubscribedClient {
    pub session_name: String,
    pub session_id: i64,
    base_url: String,
    client_key: String,
    http_client: reqwest::Client,
}

/// A reusable HTTP pool. Requests remain independent JSON envelopes; there is
/// no session handshake or certificate enrollment on the current transport.
pub struct ProtocolConnection {
    base_url: String,
    client_key: String,
    subscribed_session_ids: Vec<i64>,
    http_client: reqwest::Client,
}

#[derive(Clone)]
pub enum LoadOperation {
    List,
    Get {
        entry_names: Vec<String>,
    },
    SearchEntries {
        query: String,
    },
    SearchContext {
        query: String,
    },
    Append {
        entry_name: String,
        prefix: String,
    },
    DeleteRestore {
        entry_names: Vec<String>,
    },
    Mixed {
        entry_names: Vec<String>,
        label_query: String,
        complex_label_query: String,
        context_query: String,
        complex_context_query: String,
        nonsense_query: String,
        prefix: String,
    },
}

pub struct LoadResult {
    pub elapsed: Duration,
    pub total_requests: usize,
    pub requests_per_second: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
}

impl TestServer {
    pub async fn start() -> anyhow::Result<Self> {
        Self::start_named(&format!("session-{}", Uuid::new_v4())).await
    }

    pub async fn start_named(session_name: &str) -> anyhow::Result<Self> {
        let guard = TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let listener = StdTcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        drop(listener);
        let base_url = format!("http://{address}");
        let data_dir = env::temp_dir().join(format!("ccp-http-test-{}", Uuid::new_v4()));
        let client_key = Uuid::new_v4().to_string();
        let admin_key = Uuid::new_v4().to_string();
        let values = [
            ("CCP_SERVER_DATA_DIR", data_dir.as_os_str().to_owned()),
            ("CCP_HTTP_LISTENER_ADDR", address.to_string().into()),
            ("CCP_HTTP_BASE_URL", base_url.clone().into()),
            ("CCP_CLIENT_KEY", client_key.clone().into()),
            ("CCP_ADMIN_KEY", admin_key.clone().into()),
        ];
        let saved_env = values
            .iter()
            .map(|(key, _)| (*key, env::var_os(key)))
            .collect();
        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()?;
        // Construct the cleanup guard before mutating environment or starting
        // the task so failed startup restores the caller's configuration too.
        let mut server = Self {
            _guard: guard,
            saved_env,
            data_dir,
            server_task: None,
            shutdown: None,
            session_name: session_name.to_string(),
            session_id: 0,
            base_url,
            client_key,
            admin_key,
            http_client,
        };
        for (key, value) in values {
            unsafe {
                env::set_var(key, value);
            }
        }
        let name = server.session_name.clone();
        let (shutdown, receiver) = tokio::sync::oneshot::channel();
        server.shutdown = Some(shutdown);
        server.server_task = Some(tokio::spawn(async move {
            server::run_plain_server_with_shutdown(Some(&name), async {
                let _ = receiver.await;
            })
            .await
        }));
        for _ in 0..100 {
            if let Ok(response) = server
                .http_client
                .get(format!("{}/health", server.base_url))
                .send()
                .await
                && response.status().is_success()
                && response.json::<serde_json::Value>().await?["status"] == "ok"
            {
                let sessions: Vec<SessionMetadata> = server
                    .http_client
                    .get(format!("{}/v1/sessions", server.base_url))
                    .header("X-CCP-Client-Key", &server.client_key)
                    .send()
                    .await?
                    .error_for_status()?
                    .json()
                    .await?;
                server.session_id = sessions
                    .into_iter()
                    .find(|session| session.session_name == server.session_name)
                    .context("initial session missing from discovery")?
                    .session_id;
                return Ok(server);
            }
            if server
                .server_task
                .as_ref()
                .is_some_and(|task| task.is_finished())
            {
                let result = server.server_task.take().unwrap().await?;
                result?;
                bail!("HTTP server stopped before becoming ready");
            }
            sleep(Duration::from_millis(50)).await;
        }
        server.stop().await?;
        bail!("HTTP server did not become ready")
    }

    /// Finish the server's journal and snapshot before restoring process env.
    pub async fn stop(mut self) -> anyhow::Result<()> {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(mut task) = self.server_task.take() {
            match tokio::time::timeout(Duration::from_secs(10), &mut task).await {
                Ok(result) => result??,
                Err(_) => {
                    task.abort();
                    let _ = task.await;
                    bail!("HTTP server shutdown timed out");
                }
            }
        }
        Ok(())
    }

    pub async fn subscribe(&self) -> anyhow::Result<SubscribedClient> {
        let metadata: SessionMetadata = self
            .http_client
            .post(format!("{}/v1/subscribe", self.base_url))
            .header("X-CCP-Client-Key", &self.client_key)
            .json(&serde_json::json!({ "session": self.session_name }))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(SubscribedClient {
            session_name: metadata.session_name,
            session_id: metadata.session_id,
            base_url: self.base_url.clone(),
            client_key: self.client_key.clone(),
            http_client: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()?,
        })
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(task) = &self.server_task {
            task.abort();
        }
        for (key, value) in &self.saved_env {
            unsafe {
                match value {
                    Some(value) => env::set_var(key, value),
                    None => env::remove_var(key),
                }
            }
        }
        let _ = fs::remove_dir_all(&self.data_dir);
    }
}

impl SubscribedClient {
    pub async fn connect(&self) -> anyhow::Result<ProtocolConnection> {
        let mut connection = ProtocolConnection {
            base_url: self.base_url.clone(),
            client_key: self.client_key.clone(),
            subscribed_session_ids: vec![self.session_id],
            http_client: self.http_client.clone(),
        };
        match connection
            .request(ClientRequest::Handshake(VersionInfo {
                protocol_version: PROTOCOL_VERSION,
                client_version: "ccp-http-test".to_string(),
            }))
            .await?
        {
            ServerResponse::HandshakeOk(info) if info.compatible => Ok(connection),
            other => Err(extract_protocol_error(other)),
        }
    }

    pub async fn list(&self) -> anyhow::Result<Vec<EntrySummary>> {
        let mut connection = self.connect().await?;
        let response = connection
            .request(ClientRequest::List {
                session_id: self.session_id,
            })
            .await?;
        let ServerResponse::EntrySummaries(entries) = response else {
            return Err(extract_protocol_error(response));
        };
        Ok(entries)
    }

    pub async fn set_status(
        &self,
        team: &str,
        agent_name: &str,
        status: &str,
    ) -> anyhow::Result<AgentStatus> {
        let mut connection = self.connect().await?;
        let response = connection
            .request(ClientRequest::SetStatus {
                session_id: self.session_id,
                team: team.to_string(),
                agent_name: agent_name.to_string(),
                status: status.to_string(),
            })
            .await?;
        let ServerResponse::StatusSet(status) = response else {
            return Err(extract_protocol_error(response));
        };
        Ok(status)
    }

    pub async fn clear_status(
        &self,
        team: &str,
        agent_name: &str,
    ) -> anyhow::Result<ClearStatusResult> {
        let mut connection = self.connect().await?;
        let response = connection
            .request(ClientRequest::ClearStatus {
                session_id: self.session_id,
                team: team.to_string(),
                agent_name: agent_name.to_string(),
            })
            .await?;
        let ServerResponse::StatusCleared(result) = response else {
            return Err(extract_protocol_error(response));
        };
        Ok(result)
    }

    pub async fn list_team_status(&self, team: &str) -> anyhow::Result<Vec<AgentStatus>> {
        let mut connection = self.connect().await?;
        let response = connection
            .request(ClientRequest::ListTeamStatus {
                session_id: self.session_id,
                team: team.to_string(),
            })
            .await?;
        let ServerResponse::TeamStatuses(statuses) = response else {
            return Err(extract_protocol_error(response));
        };
        Ok(statuses)
    }

    pub async fn search_team_status(
        &self,
        team: &str,
        query: &str,
    ) -> anyhow::Result<Vec<AgentStatus>> {
        let mut connection = self.connect().await?;
        let response = connection
            .request(ClientRequest::SearchTeamStatus {
                session_id: self.session_id,
                team: team.to_string(),
                query: query.to_string(),
            })
            .await?;
        let ServerResponse::TeamStatuses(statuses) = response else {
            return Err(extract_protocol_error(response));
        };
        Ok(statuses)
    }

    pub async fn get(&self, name: &str) -> anyhow::Result<MessageEntry> {
        self.get_in_location(name, None, None).await
    }

    pub async fn get_in_location(
        &self,
        name: &str,
        shelf_name: Option<&str>,
        book_name: Option<&str>,
    ) -> anyhow::Result<MessageEntry> {
        let mut connection = self.connect().await?;
        let response = connection
            .request(ClientRequest::Get {
                session_id: self.session_id,
                name: name.to_string(),
                shelf_name: shelf_name.map(ToString::to_string),
                book_name: book_name.map(ToString::to_string),
            })
            .await?;
        let ServerResponse::Entry(entry) = response else {
            return Err(extract_protocol_error(response));
        };
        Ok(entry)
    }

    pub async fn search_entries(&self, query: &str) -> anyhow::Result<Vec<EntrySummary>> {
        let mut connection = self.connect().await?;
        let response = connection
            .request(ClientRequest::SearchEntries {
                session_id: self.session_id,
                query: query.to_string(),
            })
            .await?;
        let ServerResponse::EntrySummaries(entries) = response else {
            return Err(extract_protocol_error(response));
        };
        Ok(entries)
    }

    pub async fn search_shelves(&self, query: &str) -> anyhow::Result<Vec<protocol::ShelfSummary>> {
        let mut connection = self.connect().await?;
        let response = connection
            .request(ClientRequest::SearchShelves {
                session_id: self.session_id,
                query: query.to_string(),
            })
            .await?;
        let ServerResponse::ShelfSummaries(entries) = response else {
            return Err(extract_protocol_error(response));
        };
        Ok(entries)
    }

    pub async fn search_books(&self, query: &str) -> anyhow::Result<Vec<protocol::BookSummary>> {
        let mut connection = self.connect().await?;
        let response = connection
            .request(ClientRequest::SearchBooks {
                session_id: self.session_id,
                query: query.to_string(),
            })
            .await?;
        let ServerResponse::BookSummaries(entries) = response else {
            return Err(extract_protocol_error(response));
        };
        Ok(entries)
    }

    pub async fn search_context(&self, query: &str) -> anyhow::Result<Vec<SearchContextMatch>> {
        let mut connection = self.connect().await?;
        let response = connection
            .request(ClientRequest::SearchContext {
                session_id: self.session_id,
                query: query.to_string(),
            })
            .await?;
        let ServerResponse::SearchContextResults(results) = response else {
            return Err(extract_protocol_error(response));
        };
        Ok(results)
    }

    pub async fn search_deleted(&self, query: &str) -> anyhow::Result<Vec<DeletedEntrySummary>> {
        let mut connection = self.connect().await?;
        let response = connection
            .request(ClientRequest::SearchDeleted {
                session_id: self.session_id,
                query: query.to_string(),
            })
            .await?;
        match response {
            ServerResponse::DeletedEntries(entries) => Ok(entries),
            other => Err(extract_protocol_error(other)),
        }
    }

    pub async fn history(&self, name: &str) -> anyhow::Result<Vec<MessageHistoryEntry>> {
        self.history_in_location(name, None, None).await
    }

    pub async fn history_in_location(
        &self,
        name: &str,
        shelf_name: Option<&str>,
        book_name: Option<&str>,
    ) -> anyhow::Result<Vec<MessageHistoryEntry>> {
        let mut connection = self.connect().await?;
        let response = connection
            .request(ClientRequest::GetHistory {
                session_id: self.session_id,
                name: name.to_string(),
                shelf_name: shelf_name.map(ToString::to_string),
                book_name: book_name.map(ToString::to_string),
            })
            .await?;
        let ServerResponse::History(history) = response else {
            return Err(extract_protocol_error(response));
        };
        Ok(history)
    }

    pub async fn append(&self, name: &str, content: &str) -> anyhow::Result<AppendResult> {
        self.append_in_location(name, content, None, None).await
    }

    pub async fn append_in_location(
        &self,
        name: &str,
        content: &str,
        shelf_name: Option<&str>,
        book_name: Option<&str>,
    ) -> anyhow::Result<AppendResult> {
        let mut connection = self.connect().await?;
        let response = connection
            .request(ClientRequest::Append {
                session_id: self.session_id,
                name: name.to_string(),
                content: content.to_string(),
                metadata: AppendMetadata {
                    agent_name: Some("harness".to_string()),
                    host_name: Some("test-host".to_string()),
                    reason: None,
                },
                shelf_name: shelf_name.map(ToString::to_string),
                book_name: book_name.map(ToString::to_string),
            })
            .await?;
        let ServerResponse::AppendResult(result) = response else {
            return Err(extract_protocol_error(response));
        };
        Ok(result)
    }

    pub async fn add(
        &self,
        name: &str,
        description: &str,
        context: &str,
    ) -> anyhow::Result<MessageEntry> {
        self.add_with_labels_in_location(name, description, &[], context, None, None)
            .await
    }

    pub async fn add_with_labels(
        &self,
        name: &str,
        description: &str,
        labels: &[String],
        context: &str,
    ) -> anyhow::Result<MessageEntry> {
        self.add_with_labels_in_location(name, description, labels, context, None, None)
            .await
    }

    pub async fn add_with_labels_in_location(
        &self,
        name: &str,
        description: &str,
        labels: &[String],
        context: &str,
        shelf_name: Option<&str>,
        book_name: Option<&str>,
    ) -> anyhow::Result<MessageEntry> {
        self.add_with_labels_and_library_metadata_in_location(
            name,
            description,
            labels,
            context,
            shelf_name,
            book_name,
            None,
            None,
        )
        .await
    }

    pub async fn add_with_labels_and_library_metadata_in_location(
        &self,
        name: &str,
        description: &str,
        labels: &[String],
        context: &str,
        shelf_name: Option<&str>,
        book_name: Option<&str>,
        shelf_description: Option<&str>,
        book_description: Option<&str>,
    ) -> anyhow::Result<MessageEntry> {
        let shelf_name = shelf_name.unwrap_or("main");
        let book_name = book_name.unwrap_or("default");
        self.add_shelf(shelf_name, shelf_description.unwrap_or(""))
            .await?;
        self.add_book(shelf_name, book_name, book_description.unwrap_or(""))
            .await?;

        let mut connection = self.connect().await?;
        let response = connection
            .request(ClientRequest::AddEntry {
                session_id: self.session_id,
                name: name.to_string(),
                description: description.to_string(),
                labels: labels.to_vec(),
                context: context.to_string(),
                shelf_name: shelf_name.to_string(),
                book_name: book_name.to_string(),
            })
            .await?;
        let ServerResponse::EntryAdded { entry, .. } = response else {
            return Err(extract_protocol_error(response));
        };
        Ok(entry)
    }

    pub async fn add_shelf(&self, shelf_name: &str, description: &str) -> anyhow::Result<()> {
        let mut connection = self.connect().await?;
        let response = connection
            .request(ClientRequest::AddShelf {
                session_id: self.session_id,
                shelf_name: shelf_name.to_string(),
                description: description.to_string(),
            })
            .await?;
        let ServerResponse::ShelfAdded(_) = response else {
            return Err(extract_protocol_error(response));
        };
        Ok(())
    }

    pub async fn add_book(
        &self,
        shelf_name: &str,
        book_name: &str,
        description: &str,
    ) -> anyhow::Result<()> {
        let mut connection = self.connect().await?;
        let response = connection
            .request(ClientRequest::AddBook {
                session_id: self.session_id,
                shelf_name: shelf_name.to_string(),
                book_name: book_name.to_string(),
                description: description.to_string(),
            })
            .await?;
        let ServerResponse::BookAdded(_) = response else {
            return Err(extract_protocol_error(response));
        };
        Ok(())
    }

    pub async fn append_response(
        &self,
        name: &str,
        content: &str,
    ) -> anyhow::Result<(StatusCode, String)> {
        let mut connection = self.connect().await?;
        let response = connection
            .request(ClientRequest::Append {
                session_id: self.session_id,
                name: name.to_string(),
                content: content.to_string(),
                metadata: AppendMetadata {
                    agent_name: Some("harness".to_string()),
                    host_name: Some("test-host".to_string()),
                    reason: None,
                },
                shelf_name: None,
                book_name: None,
            })
            .await?;
        match response {
            ServerResponse::AppendResult(result) => {
                Ok((StatusCode::OK, serde_json::to_string(&result)?))
            }
            ServerResponse::Error(error) => Ok((
                status_from_error_code(&error.code),
                serde_json::to_string(&error)?,
            )),
            other => bail!("unexpected append response: {other:?}"),
        }
    }

    pub async fn delete(&self, name: &str) -> anyhow::Result<DeleteResult> {
        self.delete_in_location(name, None, None).await
    }

    pub async fn delete_in_location(
        &self,
        name: &str,
        shelf_name: Option<&str>,
        book_name: Option<&str>,
    ) -> anyhow::Result<DeleteResult> {
        let mut connection = self.connect().await?;
        let response = connection
            .request(ClientRequest::Delete {
                session_id: self.session_id,
                name: name.to_string(),
                shelf_name: shelf_name.map(ToString::to_string),
                book_name: book_name.map(ToString::to_string),
            })
            .await?;
        let ServerResponse::Deleted(result) = response else {
            return Err(extract_protocol_error(response));
        };
        Ok(result)
    }

    pub async fn restore(&self, entry_key: &str) -> anyhow::Result<RestoreResult> {
        let mut connection = self.connect().await?;
        let response = connection
            .request(ClientRequest::RestoreDeleted {
                session_id: self.session_id,
                entry_key: entry_key.to_string(),
            })
            .await?;
        let ServerResponse::Restored(result) = response else {
            return Err(extract_protocol_error(response));
        };
        Ok(result)
    }
}

impl ProtocolConnection {
    pub async fn request(&mut self, request: ClientRequest) -> anyhow::Result<ServerResponse> {
        let response = self
            .http_client
            .post(format!("{}/v1/request", self.base_url))
            .header("X-CCP-Client-Key", &self.client_key)
            .json(&serde_json::json!({
                "subscribed_session_ids": self.subscribed_session_ids, "request": request,
            }))
            .send()
            .await
            .context("HTTP request failed")?;
        let status = response.status();
        let body = response.text().await?;
        serde_json::from_str(&body)
            .with_context(|| format!("invalid HTTP response ({status}): {body}"))
    }
}

pub async fn run_persistent_load(
    clients: Vec<SubscribedClient>,
    requests_per_client: usize,
    operation: LoadOperation,
) -> anyhow::Result<LoadResult> {
    if clients.is_empty() || requests_per_client == 0 {
        bail!("load requires clients and requests");
    }
    match &operation {
        LoadOperation::Get { entry_names }
        | LoadOperation::DeleteRestore { entry_names }
        | LoadOperation::Mixed { entry_names, .. }
            if entry_names.is_empty() =>
        {
            bail!("load operation requires seed entries");
        }
        LoadOperation::DeleteRestore { entry_names } if entry_names.len() < clients.len() => {
            bail!("delete/restore load requires a distinct entry per client");
        }
        _ => {}
    }
    let total_requests = clients
        .len()
        .checked_mul(requests_per_client)
        .context("load request count overflow")?;
    let established_connections = establish_persistent_connections(clients).await?;
    let mut join_set = tokio::task::JoinSet::new();

    let started_at = Instant::now();
    for (client_index, client, mut connection) in established_connections {
        let operation = operation.clone();
        join_set.spawn(async move {
            let mut latencies = Vec::with_capacity(requests_per_client);
            let mut archived_entry_key: Option<String> = None;
            let dedicated_entry_name = match &operation {
                LoadOperation::DeleteRestore { entry_names }
                | LoadOperation::Mixed { entry_names, .. } => {
                    entry_names[client_index % entry_names.len()].clone()
                }
                _ => String::new(),
            };
            for request_index in 0..requests_per_client {
                let request_started = Instant::now();
                let request = match &operation {
                    LoadOperation::List => ClientRequest::List {
                        session_id: client.session_id,
                    },
                    LoadOperation::Get { entry_names } => ClientRequest::Get {
                        session_id: client.session_id,
                        name: entry_names[request_index % entry_names.len()].clone(),
                        shelf_name: None,
                        book_name: None,
                    },
                    LoadOperation::SearchEntries { query } => ClientRequest::SearchEntries {
                        session_id: client.session_id,
                        query: query.clone(),
                    },
                    LoadOperation::SearchContext { query } => ClientRequest::SearchContext {
                        session_id: client.session_id,
                        query: query.clone(),
                    },
                    LoadOperation::Append { entry_name, prefix } => ClientRequest::Append {
                        session_id: client.session_id,
                        name: entry_name.clone(),
                        content: format!("{prefix}-{request_index}"),
                        metadata: AppendMetadata {
                            agent_name: Some("benchmark".to_string()),
                            host_name: Some("test-host".to_string()),
                            reason: None,
                        },
                        shelf_name: None,
                        book_name: None,
                    },
                    LoadOperation::DeleteRestore { .. } => {
                        if request_index % 2 == 0 {
                            ClientRequest::Delete {
                                session_id: client.session_id,
                                name: dedicated_entry_name.clone(),
                                shelf_name: None,
                                book_name: None,
                            }
                        } else {
                            let entry_key = archived_entry_key
                                .clone()
                                .context("missing archived entry key for restore benchmark")?;
                            ClientRequest::RestoreDeleted {
                                session_id: client.session_id,
                                entry_key,
                            }
                        }
                    }
                    LoadOperation::Mixed {
                        label_query,
                        complex_label_query,
                        context_query,
                        complex_context_query,
                        nonsense_query,
                        prefix,
                        ..
                    } => match request_index % 5 {
                        0 => ClientRequest::List {
                            session_id: client.session_id,
                        },
                        1 => ClientRequest::Get {
                            session_id: client.session_id,
                            name: dedicated_entry_name.clone(),
                            shelf_name: None,
                            book_name: None,
                        },
                        2 => ClientRequest::SearchEntries {
                            session_id: client.session_id,
                            query: if request_index % 10 == 2 {
                                complex_label_query.clone()
                            } else {
                                label_query.clone()
                            },
                        },
                        3 => ClientRequest::SearchContext {
                            session_id: client.session_id,
                            query: if request_index % 15 == 3 {
                                nonsense_query.clone()
                            } else if request_index % 10 == 3 {
                                complex_context_query.clone()
                            } else {
                                context_query.clone()
                            },
                        },
                        _ => ClientRequest::Append {
                            session_id: client.session_id,
                            name: dedicated_entry_name.clone(),
                            content: format!("{prefix}-{request_index}"),
                            metadata: AppendMetadata {
                                agent_name: Some("benchmark".to_string()),
                                host_name: Some("test-host".to_string()),
                                reason: Some("mixed-load".to_string()),
                            },
                            shelf_name: None,
                            book_name: None,
                        },
                    },
                };
                let expected = match &request {
                    ClientRequest::List { .. } | ClientRequest::SearchEntries { .. } => "summaries",
                    ClientRequest::Get { .. } => "entry",
                    ClientRequest::SearchContext { .. } => "context",
                    ClientRequest::Append { .. } => "append",
                    ClientRequest::Delete { .. } => "delete",
                    ClientRequest::RestoreDeleted { .. } => "restore",
                    _ => bail!("unsupported load request"),
                };
                let response = connection.request(request).await?;
                match (expected, response) {
                    ("summaries", ServerResponse::EntrySummaries(_))
                    | ("append", ServerResponse::AppendResult(_))
                    | ("entry", ServerResponse::Entry(_))
                    | ("context", ServerResponse::SearchContextResults(_))
                    | ("restore", ServerResponse::Restored(_)) => {}
                    ("delete", ServerResponse::Deleted(result)) => {
                        archived_entry_key = Some(result.entry_key);
                    }
                    (_, other) => return Err(extract_protocol_error(other)),
                }
                latencies.push(request_started.elapsed());
            }
            Ok::<Vec<Duration>, anyhow::Error>(latencies)
        });
    }

    let mut latencies = Vec::with_capacity(total_requests);
    while let Some(result) = join_set.join_next().await {
        latencies.extend(result.context("load task panicked")??);
    }

    Ok(LoadResult::new(started_at.elapsed(), latencies))
}

async fn establish_persistent_connections(
    clients: Vec<SubscribedClient>,
) -> anyhow::Result<Vec<(usize, SubscribedClient, ProtocolConnection)>> {
    let total_clients = clients.len();
    let connect_limit = std::sync::Arc::new(Semaphore::new(
        CONNECT_SETUP_CONCURRENCY.min(total_clients.max(1)),
    ));
    let mut join_set = tokio::task::JoinSet::new();

    for (client_index, client) in clients.into_iter().enumerate() {
        let connect_limit = std::sync::Arc::clone(&connect_limit);
        join_set.spawn(async move {
            // Avoid a localhost HTTP connection thundering herd before the benchmarked request phase.
            let _permit = connect_limit
                .acquire_owned()
                .await
                .context("persistent connection limiter closed")?;
            let connection = client
                .connect()
                .await
                .with_context(|| format!("failed to connect persistent client {client_index}"))?;
            Ok::<_, anyhow::Error>((client_index, client, connection))
        });
    }

    let mut established_connections = Vec::with_capacity(total_clients);
    while let Some(result) = join_set.join_next().await {
        established_connections.push(result.context("connect task panicked")??);
    }
    established_connections.sort_by_key(|(client_index, _, _)| *client_index);
    Ok(established_connections)
}

impl LoadResult {
    fn new(elapsed: Duration, latencies: Vec<Duration>) -> Self {
        let total_requests = latencies.len();
        let requests_per_second = if elapsed.is_zero() {
            0.0
        } else {
            total_requests as f64 / elapsed.as_secs_f64()
        };
        let mut latency_ms = latencies
            .into_iter()
            .map(|duration| duration.as_secs_f64() * 1000.0)
            .collect::<Vec<_>>();
        latency_ms.sort_by(f64::total_cmp);

        Self {
            elapsed,
            total_requests,
            requests_per_second,
            p50_ms: percentile(&latency_ms, 0.50),
            p95_ms: percentile(&latency_ms, 0.95),
            p99_ms: percentile(&latency_ms, 0.99),
        }
    }
}

fn percentile(values: &[f64], quantile: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let index = ((values.len() - 1) as f64 * quantile).round() as usize;
    values[index]
}

fn status_from_error_code(code: &ErrorCode) -> StatusCode {
    match code {
        ErrorCode::BadRequest => StatusCode::BAD_REQUEST,
        ErrorCode::Forbidden => StatusCode::FORBIDDEN,
        ErrorCode::NotFound => StatusCode::NOT_FOUND,
        ErrorCode::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

fn extract_protocol_error(response: ServerResponse) -> anyhow::Error {
    match response {
        ServerResponse::Error(ErrorResponse { code, message }) => {
            let label = match code {
                ErrorCode::BadRequest => "bad request",
                ErrorCode::Forbidden => "forbidden",
                ErrorCode::NotFound => "not found",
                ErrorCode::Internal => "internal error",
            };
            anyhow::anyhow!("{label}: {message}")
        }
        other => anyhow::anyhow!("unexpected protocol response: {other:?}"),
    }
}
