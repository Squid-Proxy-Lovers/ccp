// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::env;
use std::fs;
use std::net::TcpListener as StdTcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use once_cell::sync::Lazy;
use protocol::{
    AppendMetadata, AppendResult, ClientRequest, DeleteResult, DeletedEntrySummary, EntrySummary,
    ErrorCode, ErrorResponse, MessageEntry, MessageHistoryEntry, RestoreResult, SearchContextMatch,
    ServerResponse, decode, encode,
};
use rcgen::{CertificateParams, DnType, KeyPair};
use reqwest::StatusCode;
use rusqlite::Connection;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, pem::PemObject};
use rustls::{ClientConfig, RootCertStore};
use serde::Deserialize;
use server::init::{
    AUTH_LISTENER_ADDR_ENV, AUTH_SERVER_BASE_URL_ENV, MTLS_LISTENER_ADDR_ENV,
    MTLS_SERVER_BASE_URL_ENV, SERVER_DATA_DIR_ENV,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout};
use tokio_rustls::{TlsConnector, client::TlsStream};
use uuid::Uuid;

static TEST_ENV_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));
static CRYPTO_PROVIDER_READY: Lazy<()> = Lazy::new(|| {
    let _ = rustls::crypto::ring::default_provider().install_default();
});
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const TLS_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const CONNECT_MAX_RETRIES: u32 = 3;
const CONNECT_SETUP_CONCURRENCY: usize = 128;

pub struct TestServer {
    _guard: MutexGuard<'static, ()>,
    data_dir: PathBuf,
    server_task: JoinHandle<anyhow::Result<()>>,
    pub session_name: String,
    pub session_id: i64,
    pub auth_redeem_url: String,
    pub mtls_endpoint: String,
    http_client: reqwest::Client,
}

#[derive(Clone)]
pub struct EnrolledClient {
    pub session_name: String,
    pub session_id: i64,
    pub access: String,
    pub client_cn: String,
    pub mtls_endpoint: String,
    ca_pem: String,
    client_cert_pem: String,
    client_key_pem: String,
}

pub struct ProtocolConnection {
    stream: TlsStream<TcpStream>,
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

struct EnrollmentMaterial {
    session_name: String,
    session_id: i64,
    access: String,
    client_cn: String,
    mtls_endpoint: String,
    ca_pem: String,
    client_cert_pem: String,
    client_key_pem: String,
}

#[derive(Debug, Deserialize)]
struct AuthRedeemResponse {
    session: AuthSessionMetadata,
    access: String,
    client_common_name: String,
    mtls_endpoint: String,
    ca_cert_pem: String,
    client_cert_pem: String,
}

#[derive(Debug, Deserialize)]
struct AuthSessionMetadata {
    #[serde(alias = "name")]
    session_name: String,
    #[serde(alias = "id")]
    session_id: i64,
}

impl TestServer {
    pub async fn start() -> anyhow::Result<Self> {
        Lazy::force(&CRYPTO_PROVIDER_READY);
        let guard = TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let auth_port = allocate_port()?;
        let mtls_port = allocate_port()?;
        let auth_addr = format!("127.0.0.1:{auth_port}");
        let mtls_addr = format!("127.0.0.1:{mtls_port}");
        let data_dir = std::env::temp_dir().join(format!("ccp-e2e-{}", Uuid::new_v4()));
        let session_name = format!("session-{}", Uuid::new_v4());

        set_server_env(&data_dir, &auth_addr, &mtls_addr);

        let server_task = tokio::spawn({
            let session_name = session_name.clone();
            async move { server::run_server(&session_name).await }
        });

        wait_for_listener(&auth_addr).await?;
        let session_id = query_session_id(&data_dir.join("ccp.sqlite3"), &session_name)?;

        Ok(Self {
            _guard: guard,
            data_dir,
            server_task,
            session_name,
            session_id,
            auth_redeem_url: server::init::auth_redeem_url(),
            mtls_endpoint: format!("https://localhost:{mtls_port}"),
            http_client: reqwest::Client::new(),
        })
    }

    pub async fn enroll_read(&self) -> anyhow::Result<EnrolledClient> {
        let issued_token = self.issue_token("read")?;
        self.redeem_token(&issued_token.token).await
    }

    pub async fn enroll_read_write(&self) -> anyhow::Result<EnrolledClient> {
        let issued_token = self.issue_token("read_write")?;
        self.redeem_token(&issued_token.token).await
    }

    pub async fn unauthenticated_list_status(
        &self,
    ) -> anyhow::Result<Result<ServerResponse, anyhow::Error>> {
        let port = self
            .mtls_endpoint
            .rsplit(':')
            .next()
            .context("missing mTLS port")?
            .parse::<u16>()
            .context("invalid mTLS port")?;
        let address = format!("127.0.0.1:{port}");
        let ca_pem = fs::read(self.data_dir.join("ccp_ca_cert.pem"))
            .context("failed to read CA certificate")?;

        let mut root_store = RootCertStore::empty();
        for cert in CertificateDer::pem_slice_iter(&ca_pem) {
            root_store
                .add(cert.context("failed to parse CA certificate")?)
                .context("failed to add CA certificate to root store")?;
        }

        let config = ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth();
        let connector = TlsConnector::from(std::sync::Arc::new(config));
        let stream = TcpStream::connect(&address)
            .await
            .with_context(|| format!("failed to connect to {address}"))?;
        let server_name =
            ServerName::try_from("localhost".to_string()).context("invalid server name")?;
        let tls_stream = connector.connect(server_name, stream).await;
        match tls_stream {
            Ok(stream) => {
                let mut connection = ProtocolConnection { stream };
                Ok(connection
                    .request(ClientRequest::List {
                        session_id: self.session_id,
                    })
                    .await)
            }
            Err(error) => Ok(Err(
                anyhow::Error::new(error).context("TLS handshake failed")
            )),
        }
    }

    pub fn issue_token(
        &self,
        access_level: &str,
    ) -> anyhow::Result<server::init::IssuedEnrollmentToken> {
        server::init::issue_enrollment_token(&self.session_name, access_level, None)
    }

    pub async fn redeem_token(&self, token: &str) -> anyhow::Result<EnrolledClient> {
        let client_key =
            KeyPair::generate().context("failed to generate enrollment client keypair")?;
        let mut client_params =
            CertificateParams::new(Vec::<String>::new()).context("failed to build CSR params")?;
        client_params.distinguished_name.push(
            DnType::CommonName,
            format!("test-client-{}", Uuid::new_v4()),
        );
        let csr_pem = client_params
            .serialize_request(&client_key)
            .context("failed to serialize enrollment CSR")?
            .pem()
            .context("failed to serialize CSR to PEM")?;

        const MAX_RETRIES: u32 = 3;
        for attempt in 0..=MAX_RETRIES {
            let result = self
                .http_client
                .post(&self.auth_redeem_url)
                .json(&serde_json::json!({
                    "token": token,
                    "csr_pem": csr_pem,
                }))
                .send()
                .await;

            let response = match result {
                Ok(r) => r,
                Err(e) => {
                    if attempt < MAX_RETRIES {
                        sleep(Duration::from_millis(50u64 * u64::from(attempt + 1))).await;
                        continue;
                    }
                    anyhow::bail!(
                        "failed to redeem auth token after {} attempts: {}",
                        MAX_RETRIES + 1,
                        e
                    );
                }
            };

            let response = match response.error_for_status() {
                Ok(r) => r,
                Err(e) => {
                    if attempt < MAX_RETRIES {
                        sleep(Duration::from_millis(50u64 * u64::from(attempt + 1))).await;
                        continue;
                    }
                    return Err(
                        anyhow::Error::new(e).context("auth redeem returned an error status")
                    );
                }
            };

            match response.json::<AuthRedeemResponse>().await {
                Ok(body) => {
                    let material = parse_enrollment_response(body, client_key.serialize_pem());
                    return Ok(EnrolledClient {
                        session_name: material.session_name,
                        session_id: material.session_id,
                        access: material.access,
                        client_cn: material.client_cn,
                        mtls_endpoint: material.mtls_endpoint,
                        ca_pem: material.ca_pem,
                        client_cert_pem: material.client_cert_pem,
                        client_key_pem: material.client_key_pem,
                    });
                }
                Err(e) => {
                    if attempt < MAX_RETRIES {
                        sleep(Duration::from_millis(50u64 * u64::from(attempt + 1))).await;
                        continue;
                    }
                    return Err(
                        anyhow::Error::new(e).context("failed to decode auth redeem JSON response")
                    );
                }
            }
        }

        unreachable!()
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.server_task.abort();
        clear_server_env();
        let _ = fs::remove_dir_all(&self.data_dir);
    }
}

impl EnrolledClient {
    pub async fn connect(&self) -> anyhow::Result<ProtocolConnection> {
        let port = self
            .mtls_endpoint
            .rsplit(':')
            .next()
            .context("missing mTLS port")?
            .parse::<u16>()
            .context("invalid mTLS port")?;
        let address = format!("127.0.0.1:{port}");

        for attempt in 0..=CONNECT_MAX_RETRIES {
            match self.connect_once(&address).await {
                Ok(connection) => return Ok(connection),
                Err(_) if attempt < CONNECT_MAX_RETRIES => {
                    sleep(Duration::from_millis(50u64 * u64::from(attempt + 1))).await;
                    continue;
                }
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!(
                            "failed to establish mTLS connection to {address} after {} attempts",
                            CONNECT_MAX_RETRIES + 1
                        )
                    });
                }
            }
        }

        unreachable!()
    }

    async fn connect_once(&self, address: &str) -> anyhow::Result<ProtocolConnection> {
        let mut root_store = RootCertStore::empty();
        for cert in CertificateDer::pem_slice_iter(self.ca_pem.as_bytes()) {
            root_store
                .add(cert.context("failed to parse CA certificate")?)
                .context("failed to add CA certificate to root store")?;
        }

        let cert_chain = CertificateDer::pem_slice_iter(self.client_cert_pem.as_bytes())
            .collect::<Result<Vec<_>, _>>()
            .context("failed to parse client certificate chain")?;
        let private_key = PrivateKeyDer::from_pem_slice(self.client_key_pem.as_bytes())
            .context("failed to parse client private key")?;

        let config = ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_client_auth_cert(cert_chain, private_key)
            .context("failed to build rustls client config")?;
        let connector = TlsConnector::from(std::sync::Arc::new(config));
        let stream = timeout(CONNECT_TIMEOUT, TcpStream::connect(address))
            .await
            .context("TCP connect timed out")?
            .with_context(|| format!("failed to connect to {address}"))?;
        stream.set_nodelay(true).ok();
        let server_name =
            ServerName::try_from("localhost".to_string()).context("invalid server name")?;
        let stream = timeout(TLS_CONNECT_TIMEOUT, connector.connect(server_name, stream))
            .await
            .context("mTLS handshake timed out")?
            .context("mTLS handshake failed")?;
        Ok(ProtocolConnection { stream })
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
        write_frame(&mut self.stream, &request).await?;
        read_frame(&mut self.stream)
            .await?
            .context("server closed the TLS session before responding")
    }
}

pub async fn run_persistent_load(
    clients: Vec<EnrolledClient>,
    requests_per_client: usize,
    operation: LoadOperation,
) -> anyhow::Result<LoadResult> {
    let total_requests = clients.len() * requests_per_client;
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
                let response = connection.request(request).await?;
                match response {
                    ServerResponse::EntrySummaries(_)
                    | ServerResponse::AppendResult(_)
                    | ServerResponse::Entry(_)
                    | ServerResponse::SearchContextResults(_)
                    | ServerResponse::Restored(_) => {}
                    ServerResponse::Deleted(result) => {
                        archived_entry_key = Some(result.entry_key);
                    }
                    other => return Err(extract_protocol_error(other)),
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
    clients: Vec<EnrolledClient>,
) -> anyhow::Result<Vec<(usize, EnrolledClient, ProtocolConnection)>> {
    let total_clients = clients.len();
    let connect_limit = std::sync::Arc::new(Semaphore::new(
        CONNECT_SETUP_CONCURRENCY.min(total_clients.max(1)),
    ));
    let mut join_set = tokio::task::JoinSet::new();

    for (client_index, client) in clients.into_iter().enumerate() {
        let connect_limit = std::sync::Arc::clone(&connect_limit);
        join_set.spawn(async move {
            // Avoid a localhost TCP/TLS thundering herd before the benchmarked request phase.
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

fn allocate_port() -> anyhow::Result<u16> {
    let listener = StdTcpListener::bind("127.0.0.1:0").context("failed to allocate a port")?;
    Ok(listener.local_addr()?.port())
}

fn set_server_env(data_dir: &Path, auth_addr: &str, mtls_addr: &str) {
    let auth_base_url = format!("http://{auth_addr}");
    let mtls_port = mtls_addr.rsplit(':').next().unwrap_or("");
    let mtls_base_url = format!("https://localhost:{mtls_port}");
    // Tests serialize access to the process environment via TEST_ENV_LOCK.
    unsafe {
        env::set_var(SERVER_DATA_DIR_ENV, data_dir);
        env::set_var(AUTH_LISTENER_ADDR_ENV, auth_addr);
        env::set_var(MTLS_LISTENER_ADDR_ENV, mtls_addr);
        env::set_var(AUTH_SERVER_BASE_URL_ENV, auth_base_url);
        env::set_var(MTLS_SERVER_BASE_URL_ENV, mtls_base_url);
    }
}

fn clear_server_env() {
    unsafe {
        env::remove_var(SERVER_DATA_DIR_ENV);
        env::remove_var(AUTH_LISTENER_ADDR_ENV);
        env::remove_var(MTLS_LISTENER_ADDR_ENV);
        env::remove_var(AUTH_SERVER_BASE_URL_ENV);
        env::remove_var(MTLS_SERVER_BASE_URL_ENV);
    }
}

async fn wait_for_listener(addr: &str) -> anyhow::Result<()> {
    for _ in 0..100 {
        if let Ok(mut stream) = TcpStream::connect(addr).await {
            let probe = format!("GET /ready HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
            if stream.write_all(probe.as_bytes()).await.is_ok() {
                return Ok(());
            }
        }
        sleep(Duration::from_millis(50)).await;
    }
    bail!("timed out waiting for listener at {addr}");
}

fn query_session_id(db_path: &Path, session_name: &str) -> anyhow::Result<i64> {
    let connection = Connection::open(db_path)
        .with_context(|| format!("failed to open {}", db_path.display()))?;
    connection
        .query_row(
            "SELECT id FROM sessions WHERE name = ?1",
            [session_name],
            |row| row.get(0),
        )
        .with_context(|| format!("failed to load session id for {session_name}"))
}

fn parse_enrollment_response(
    response: AuthRedeemResponse,
    client_key_pem: String,
) -> EnrollmentMaterial {
    EnrollmentMaterial {
        session_name: response.session.session_name,
        session_id: response.session.session_id,
        access: response.access,
        client_cn: response.client_common_name,
        mtls_endpoint: response.mtls_endpoint,
        ca_pem: response.ca_cert_pem,
        client_cert_pem: response.client_cert_pem,
        client_key_pem,
    }
}

async fn read_frame<T, R>(reader: &mut R) -> anyhow::Result<Option<T>>
where
    T: serde::de::DeserializeOwned,
    R: AsyncRead + Unpin,
{
    let mut header = [0u8; 4];
    match reader.read_exact(&mut header).await {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error).context("failed to read frame header"),
    }

    let frame_len = u32::from_be_bytes(header) as usize;
    if frame_len == 0 {
        bail!("empty frames are not allowed");
    }

    let mut payload = vec![0u8; frame_len];
    reader
        .read_exact(&mut payload)
        .await
        .context("failed to read frame payload")?;
    decode(&payload).context("failed to decode frame").map(Some)
}

async fn write_frame<T, W>(writer: &mut W, value: &T) -> anyhow::Result<()>
where
    T: serde::Serialize,
    W: AsyncWrite + Unpin,
{
    let payload = encode(value).context("failed to encode frame")?;
    let frame_len = u32::try_from(payload.len()).context("encoded frame is too large")?;
    writer
        .write_all(&frame_len.to_be_bytes())
        .await
        .context("failed to write frame header")?;
    writer
        .write_all(&payload)
        .await
        .context("failed to write frame payload")?;
    writer.flush().await.context("failed to flush frame")?;
    Ok(())
}
