// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, bail};
use protocol::{ClientRequest, ErrorCode, ErrorResponse, ServerResponse, decode, encode};
use reqwest::Url;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, pem::PemObject};
use rustls::{ClientConfig, RootCertStore};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;

use crate::enrollment_structs::StoredEnrollment;

pub(crate) struct PersistentClientConnection {
    pub(crate) stream: TlsStream<TcpStream>,
}

pub(crate) async fn connect_mtls(
    enrollment: &StoredEnrollment,
) -> anyhow::Result<PersistentClientConnection> {
    check_cert_time(enrollment)?;
    let url =
        Url::parse(&enrollment.metadata.mtls_endpoint).context("invalid mTLS endpoint URL")?;
    let host = url
        .host_str()
        .context("mTLS endpoint missing host")?
        .to_string();
    let port = url
        .port_or_known_default()
        .context("mTLS endpoint missing port")?;
    let address = if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };

    let ca_pem = std::fs::read(enrollment.directory.join("ca.pem")).with_context(|| {
        format!(
            "failed to read {}",
            enrollment.directory.join("ca.pem").display()
        )
    })?;
    let client_cert_pem =
        std::fs::read(enrollment.directory.join("client.pem")).with_context(|| {
            format!(
                "failed to read {}",
                enrollment.directory.join("client.pem").display()
            )
        })?;
    let client_key_pem =
        std::fs::read(enrollment.directory.join("client.key")).with_context(|| {
            format!(
                "failed to read {}",
                enrollment.directory.join("client.key").display()
            )
        })?;

    let mut root_store = RootCertStore::empty();
    for cert in CertificateDer::pem_slice_iter(&ca_pem) {
        root_store
            .add(cert.context("failed to parse CA certificate")?)
            .context("failed to add CA certificate to root store")?;
    }

    let cert_chain = CertificateDer::pem_slice_iter(&client_cert_pem)
        .collect::<Result<Vec<_>, _>>()
        .context("failed to parse client certificate chain")?;
    let private_key = PrivateKeyDer::from_pem_slice(&client_key_pem)
        .context("failed to parse client private key")?;

    let config = ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_client_auth_cert(cert_chain, private_key)
        .context("failed to build rustls client config")?;
    let connector = TlsConnector::from(Arc::new(config));

    let stream = TcpStream::connect(&address)
        .await
        .with_context(|| format!("failed to connect to {address}"))?;
    stream.set_nodelay(true).ok();
    let server_name = ServerName::try_from(host.clone()).context("invalid TLS server name")?;
    let tls_stream = connector
        .connect(server_name, stream)
        .await
        .context("mTLS handshake failed")?;

    Ok(PersistentClientConnection { stream: tls_stream })
}

fn check_cert_time(enrollment: &StoredEnrollment) -> anyhow::Result<()> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before UNIX_EPOCH")?
        .as_secs();
    if now >= enrollment.metadata.client_cert_expires_at {
        bail!(
            "client certificate expired at unix={}; request a new enrollment token and re-enroll",
            enrollment.metadata.client_cert_expires_at
        );
    }
    Ok(())
}

impl PersistentClientConnection {
    pub(crate) async fn request(
        &mut self,
        request: ClientRequest,
    ) -> anyhow::Result<ServerResponse> {
        write_frame(&mut self.stream, &request).await?;
        read_frame(&mut self.stream)
            .await?
            .context("server closed the TLS session before responding")
    }
}

pub(crate) fn response_to_json_string(response: ServerResponse) -> anyhow::Result<String> {
    match response {
        ServerResponse::EntrySummaries(entries) => {
            serde_json::to_string(&entries).context("failed to serialize entry summaries")
        }
        ServerResponse::ShelfSummaries(entries) => {
            serde_json::to_string(&entries).context("failed to serialize shelf summaries")
        }
        ServerResponse::BookSummaries(entries) => {
            serde_json::to_string(&entries).context("failed to serialize book summaries")
        }
        ServerResponse::SearchContextResults(results) => {
            serde_json::to_string(&results).context("failed to serialize context search results")
        }
        ServerResponse::DeletedEntries(entries) => {
            serde_json::to_string(&entries).context("failed to serialize deleted entries")
        }
        ServerResponse::Entry(entry)
        | ServerResponse::EntryAdded { entry, .. }
        | ServerResponse::EntryAtTime(entry) => {
            serde_json::to_string(&entry).context("failed to serialize message entry")
        }
        ServerResponse::AppendResult(result) => {
            serde_json::to_string(&result).context("failed to serialize append result")
        }
        ServerResponse::Deleted(result) => {
            serde_json::to_string(&result).context("failed to serialize delete result")
        }
        ServerResponse::Restored(result) => {
            serde_json::to_string(&result).context("failed to serialize restore result")
        }
        ServerResponse::History(history) => {
            serde_json::to_string(&history).context("failed to serialize history")
        }
        ServerResponse::ExportedBundle(bundle) => {
            serde_json::to_string(&bundle).context("failed to serialize bundle")
        }
        ServerResponse::ImportResult(result) => {
            serde_json::to_string(&result).context("failed to serialize import result")
        }
        ServerResponse::CertRevoked(result) => {
            serde_json::to_string(&result).context("failed to serialize revoke result")
        }
        ServerResponse::Pong => serde_json::to_string(&serde_json::json!({ "status": "ok" }))
            .context("failed to serialize pong"),
        ServerResponse::ShelfAdded(result) => {
            serde_json::to_string(&result).context("failed to serialize shelf added result")
        }
        ServerResponse::BookAdded(result) => {
            serde_json::to_string(&result).context("failed to serialize book added result")
        }
        ServerResponse::HandshakeOk(info) => {
            serde_json::to_string(&info).context("failed to serialize handshake response")
        }
        ServerResponse::ShelfDeleted(result) => {
            serde_json::to_string(&result).context("failed to serialize shelf deleted result")
        }
        ServerResponse::Brief(brief) => {
            serde_json::to_string(&brief).context("failed to serialize session brief")
        }
        ServerResponse::HandshakeRejected(info) => {
            anyhow::bail!(
                "protocol version mismatch: server={}, client={}",
                info.protocol_version,
                protocol::PROTOCOL_VERSION
            )
        }
        ServerResponse::Error(error) => error_response_to_anyhow(error),
    }
}

pub(crate) fn error_response_to_anyhow(error: ErrorResponse) -> anyhow::Result<String> {
    let label = match error.code {
        ErrorCode::BadRequest => "bad request",
        ErrorCode::Forbidden => "forbidden",
        ErrorCode::NotFound => "not found",
        ErrorCode::Internal => "internal error",
    };
    bail!("{label}: {}", error.message)
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
    if frame_len > 8 * 1024 * 1024 {
        bail!("frame exceeds maximum size");
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
