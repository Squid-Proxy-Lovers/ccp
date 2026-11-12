// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::{Credential, WorkspaceTrust, crypto};
use anyhow::{Context, bail};
use protocol::workspace as api;
use reqwest::{Client, Url};
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
use rustls::{ClientConfig, RootCertStore};
use serde::de::DeserializeOwned;
use std::{sync::Arc, time::Duration};

pub(super) fn origin(value: &str) -> anyhow::Result<String> {
    if value.len() > 2048
        || value.trim() != value
        || value.contains('\\')
        || value.chars().any(char::is_control)
    {
        bail!("invalid workspace origin");
    }
    let raw = value
        .split_once("://")
        .context("workspace origin requires an HTTPS authority")?
        .1;
    if raw.contains('@')
        || raw
            .split_once('/')
            .is_some_and(|(_, path)| !path.is_empty())
    {
        bail!("workspace URL must be an origin");
    }
    let url = Url::parse(value).map_err(|_url| anyhow::anyhow!("invalid workspace origin"))?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        bail!("workspace origin must be HTTPS without credentials, path, query or fragment");
    }
    Ok(url.to_string())
}

pub(super) fn client(
    trust: &WorkspaceTrust,
    material: Option<&Credential>,
) -> anyhow::Result<Client> {
    crypto::root_time(&trust.ca_pem, crypto::now()?)?;
    let mut roots = RootCertStore::empty();
    roots.add(CertificateDer::from(crypto::ca_der(&trust.ca_pem)?))?;
    let builder =
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()?
            .with_root_certificates(roots);
    let mut config = match material {
        Some(material) => builder
            .with_client_auth_cert(
                vec![CertificateDer::from(crypto::pem(
                    &material.receipt.certificate_pem,
                    "CERTIFICATE",
                )?)],
                PrivatePkcs8KeyDer::from(crypto::pem(&material.key_pem, "PRIVATE KEY")?).into(),
            )
            .map_err(|_key| anyhow::anyhow!("invalid saved client key or certificate"))?,
        None => builder.with_no_client_auth(),
    };
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    config.enable_early_data = false;
    Client::builder()
        .https_only(true)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .http1_only()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(20))
        .use_preconfigured_tls(config)
        .build()
        .map_err(|_http| anyhow::anyhow!("cannot configure workspace HTTPS client"))
}

pub(super) async fn health(http: &Client, origin: &str, instance: &str) -> anyhow::Result<()> {
    let health: api::Health = json(http.get(format!("{origin}api/v1/health"))).await?;
    if health.instance_id != instance {
        bail!("server instance does not match the trusted profile");
    }
    Ok(())
}

pub(super) async fn json<T: DeserializeOwned>(
    request: reqwest::RequestBuilder,
) -> anyhow::Result<T> {
    limited_json(request, 1024 * 1024, false).await
}

pub(super) async fn snapshot_json<T: DeserializeOwned>(
    request: reqwest::RequestBuilder,
) -> anyhow::Result<T> {
    limited_json(request, 4 * 1024 * 1024, false).await
}

pub(super) async fn operation_json<T: DeserializeOwned>(
    request: reqwest::RequestBuilder,
) -> anyhow::Result<T> {
    limited_json(request, 1024 * 1024, true).await
}

#[derive(Debug)]
struct Refusal {
    status: u16,
    conflict: bool,
}
impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "workspace request refused (HTTP {})", self.status)
    }
}
impl std::error::Error for Refusal {}

pub(super) fn content_conflict(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<Refusal>()
        .is_some_and(|e| e.status == 409 && e.conflict)
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ConflictFailure {
    api_version: u32,
    error: ConflictCode,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ConflictCode {
    code: String,
}

async fn conflict_reply(response: &mut reqwest::Response) -> bool {
    if response.status().as_u16() != 409
        || response.headers().contains_key("content-encoding")
        || response.content_length().is_some_and(|v| v > 8192)
    {
        return false;
    }
    let mut bytes = Vec::new();
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                if bytes.len().saturating_add(chunk.len()) > 8192 {
                    return false;
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok(None) => break,
            Err(_error) => return false,
        }
    }
    serde_json::from_slice::<ConflictFailure>(&bytes)
        .is_ok_and(|v| v.api_version == api::API_VERSION && v.error.code == "content_conflict")
}

async fn limited_json<T: DeserializeOwned>(
    request: reqwest::RequestBuilder,
    max_bytes: usize,
    inspect_conflict: bool,
) -> anyhow::Result<T> {
    let mut response = request.send().await.map_err(|_network| {
        anyhow::anyhow!("workspace HTTPS request failed; trust, name and connection must be valid")
    })?;
    if !response.status().is_success() {
        let status = response.status().as_u16();
        let conflict = inspect_conflict && conflict_reply(&mut response).await;
        return Err(Refusal { status, conflict }.into());
    }
    if response.headers().get("content-encoding").is_some()
        || response
            .content_length()
            .is_some_and(|n| !usize::try_from(n).is_ok_and(|n| n <= max_bytes))
    {
        bail!("invalid workspace response encoding or size");
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_body| {
        anyhow::anyhow!("incomplete workspace response; preserve pending request for retry")
    })? {
        if bytes.len().saturating_add(chunk.len()) > max_bytes {
            bail!("workspace response exceeds size limit");
        }
        bytes.extend_from_slice(&chunk);
    }
    let response: api::Success<T> = serde_json::from_slice(&bytes)
        .map_err(|_json| anyhow::anyhow!("invalid workspace response JSON"))?;
    if response.api_version != api::API_VERSION {
        bail!("unsupported workspace API version");
    }
    Ok(response.data)
}
