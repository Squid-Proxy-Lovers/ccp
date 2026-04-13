// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::fs;
use std::sync::Arc;

use anyhow::Context;
use protocol::SessionMetadata;
use rcgen::{
    CertificateParams, CertificateSigningRequestParams, DnType, ExtendedKeyUsagePurpose, Ia5String,
    KeyPair, KeyUsagePurpose, SanType,
};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use time::{Duration as TimeDuration, OffsetDateTime};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use url::Url;
use uuid::Uuid;

use crate::identity::{access_identity_uri, session_identity_uri};
use crate::init::{
    auth_redeem_url, ca_cert_path, ca_key_path, client_cert_ttl_seconds,
    ensure_active_session_binding, mtls_server_base_url, open_sqlite_connection,
    unix_timestamp_to_sqlite,
};
use crate::state::ServerState;

#[cfg(test)]
#[path = "tests/auth_request.rs"]
mod tests;

enum AuthError {
    InvalidOrExpiredToken,
    Internal(anyhow::Error),
}

#[derive(Deserialize, PartialEq, Eq, Debug)]
struct AuthRedeemRequest {
    token: String,
    csr_pem: String,
}

#[derive(Clone)]
struct RedeemableAuthGrant {
    session_id: i64,
    access_level: String,
    metadata: SessionMetadata,
}

#[derive(Serialize)]
struct AuthRedeemResponse {
    session: SessionMetadata,
    access: String,
    client_common_name: String,
    mtls_endpoint: String,
    client_cert_expires_at: u64,
    cert_warning_window_seconds: u64,
    ca_cert_pem: String,
    client_cert_pem: String,
}

pub async fn try_handle_auth_request(
    socket: &mut TcpStream,
    request: &str,
    state: Arc<ServerState>,
) -> anyhow::Result<bool> {
    // our main function to deal with the auth post request

    let payload = match extract_auth_request(request) {
        Ok(Some(payload)) => payload,
        Ok(None) => return Ok(false),
        Err(error) => {
            // if we fail to extract the auth request then we send a 400 Bad Request
            send_json_response(
                socket,
                "400 Bad Request",
                &serde_json::json!({ "error": error.to_string() }).to_string(),
            )
            .await?;
            return Ok(true);
        }
    };

    // attempt to perform the token redemption and complete the auth request
    match complete_auth_request(&state, &payload).await {
        Ok(response_body) => send_json_response(socket, "200 OK", &response_body).await?,
        Err(AuthError::InvalidOrExpiredToken) => {
            // if the auth token is invalid or expired then we send a 401 Unauthorized
            send_json_response(
                socket,
                "401 Unauthorized",
                &serde_json::json!({ "error": "invalid or expired token" }).to_string(),
            )
            .await?;
        }
        Err(AuthError::Internal(error)) => {
            // if we fail to complete the auth request then we send a 500 Internal Server Error
            send_json_response(
                socket,
                "500 Internal Server Error",
                &serde_json::json!({ "error": "auth request failed" }).to_string(),
            )
            .await?;
            return Err(error);
        }
    }

    Ok(true)
}

async fn complete_auth_request(
    state: &ServerState,
    payload: &AuthRedeemRequest,
) -> Result<String, AuthError> {
    let grant = consume_auth_token(&payload.token).map_err(|error| {
        if error.to_string().contains("invalid or expired token") {
            AuthError::InvalidOrExpiredToken
        } else {
            AuthError::Internal(error) // if we fail to consume the auth token then we return a 500 Internal Server Error
        }
    })?;

    let (common_name, cert_pem, ca_cert_pem, cert_expires_at, cert_expires_at_text) =
        issue_client_certificate(grant.session_id, &grant.access_level, &payload.csr_pem)
            .map_err(AuthError::Internal)?;

    // record the issued certificate in the state
    state
        .record_issued_cert(
            grant.session_id,
            &common_name,
            &grant.access_level,
            &cert_pem,
            &cert_expires_at_text,
        )
        .await
        .map_err(AuthError::Internal)?;

    serde_json::to_string(&AuthRedeemResponse {
        session: grant.metadata,
        access: grant.access_level,
        client_common_name: common_name,
        mtls_endpoint: mtls_server_base_url(),
        client_cert_expires_at: cert_expires_at,
        cert_warning_window_seconds: crate::init::cert_warning_window_seconds(),
        ca_cert_pem,
        client_cert_pem: cert_pem,
    })
    .map_err(|error| {
        AuthError::Internal(anyhow::Error::new(error).context("failed to serialize auth response"))
    })
}

fn consume_auth_token(raw_token: &str) -> anyhow::Result<RedeemableAuthGrant> {
    let token_hash = crate::init::hash_token(raw_token);
    let mut connection = open_sqlite_connection()?;
    let transaction = connection
        .transaction()
        .context("failed to start auth token transaction")?;

    let row = transaction
        .query_row(
            "SELECT auth_tokens.id,
                    auth_tokens.session_id,
                    auth_tokens.access_level,
                    sessions.name,
                    sessions.description,
                    sessions.owner,
                    sessions.labels,
                    sessions.visibility,
                    sessions.purpose
             FROM auth_tokens
             INNER JOIN sessions ON sessions.id = auth_tokens.session_id
             WHERE auth_tokens.token_hash = ?1
               AND auth_tokens.expires_at > CURRENT_TIMESTAMP",
            [token_hash.as_str()],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    SessionMetadata {
                        session_name: row.get(3)?,
                        session_id: row.get(1)?,
                        description: row.get(4)?,
                        owner: row.get(5)?,
                        labels: deserialize_labels(&row.get::<_, String>(6)?),
                        visibility: row.get(7)?,
                        purpose: row.get(8)?,
                    },
                ))
            },
        )
        .optional()
        .context("failed to load enrollment token")?
        .with_context(|| "invalid or expired token")?;

    transaction
        .commit()
        .context("failed to commit enrollment token transaction")?;

    Ok(RedeemableAuthGrant {
        session_id: row.1,
        access_level: row.2,
        metadata: row.3,
    })
}

fn issue_client_certificate(
    session_id: i64,
    access_level: &str,
    csr_pem: &str,
) -> anyhow::Result<(String, String, String, u64, String)> {
    // make sure we have a running session binding
    let _binding = ensure_active_session_binding(session_id)?;

    // get file pathings for the CA certificate and key
    let ca_cert_path = ca_cert_path();

    let ca_key_path = ca_key_path();

    let ca_cert_pem = fs::read_to_string(&ca_cert_path).with_context(|| {
        format!(
            "failed to read CA certificate from {}",
            ca_cert_path.display()
        )
    })?;

    let ca_key_pem = fs::read_to_string(&ca_key_path)
        .with_context(|| format!("failed to read CA key from {}", ca_key_path.display()))?;

    let ca_key = KeyPair::from_pem(&ca_key_pem).context("failed to parse CA private key")?;

    let ca_params = CertificateParams::from_ca_cert_pem(&ca_cert_pem)
        .context("failed to parse CA certificate")?;

    let ca_cert = ca_params
        .self_signed(&ca_key)
        .context("failed to reconstruct CA certificate")?;

    let csr = CertificateSigningRequestParams::from_pem(csr_pem)
        .context("failed to parse certificate signing request")?;

    let common_name = Uuid::new_v4().to_string(); // generate a unique common name for the client certificate

    let mut client_params = CertificateParams::new(Vec::<String>::new())
        .context("failed to build client cert params")?;

    client_params
        .distinguished_name
        .push(DnType::CommonName, common_name.clone());

    let ttl = i64::try_from(client_cert_ttl_seconds()) // get the TTL from the config
        .context("TTL overflows i64")?;

    client_params.not_before = OffsetDateTime::now_utc() - TimeDuration::hours(1);
    client_params.not_after = OffsetDateTime::now_utc() + TimeDuration::seconds(ttl);

    client_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
    client_params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    client_params.subject_alt_names.push(SanType::URI(
        Ia5String::try_from(session_identity_uri(session_id))
            .context("failed to encode session identity URI for client cert")?,
    ));
    client_params.subject_alt_names.push(SanType::URI(
        Ia5String::try_from(access_identity_uri(access_level))
            .context("failed to encode access identity URI for client cert")?,
    ));
    let cert_expires_at = u64::try_from(client_params.not_after.unix_timestamp())
        .context("expiry timestamp is negative")?;

    let client_cert = client_params
        .signed_by(&csr.public_key, &ca_cert, &ca_key)
        .context("failed to sign client certificate")?;
    let cert_expires_at_text = unix_timestamp_to_sqlite(cert_expires_at)?;

    Ok((
        common_name,
        client_cert.pem(),
        ca_cert_pem,
        cert_expires_at,
        cert_expires_at_text,
    ))
}

fn deserialize_labels(labels: &str) -> Vec<String> {
    labels
        .split(',')
        .map(|label| label.trim())
        .filter(|label| !label.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn extract_auth_request(request: &str) -> anyhow::Result<Option<AuthRedeemRequest>> {
    let bytes = request.as_bytes();
    let mut headers = [httparse::EMPTY_HEADER; 16];
    let mut req = httparse::Request::new(&mut headers);

    let header_len = match req.parse(bytes) {
        Ok(httparse::Status::Complete(len)) => len,
        _ => return Ok(None),
    };

    let expected_path = Url::parse(&auth_redeem_url())
        .ok()
        .map(|u| u.path().to_string())
        .unwrap_or_else(|| "/auth/redeem".to_string());

    let path = req
        .path
        .and_then(|p| p.split_once('?').map(|(s, _)| s).or(Some(p)))
        .unwrap_or("");
    if req.method != Some("POST") || path != expected_path {
        return Ok(None);
    }

    let body =
        std::str::from_utf8(&bytes[header_len..]).context("request body is not valid UTF-8")?;
    let payload: AuthRedeemRequest =
        serde_json::from_str(body).context("invalid auth redeem JSON body")?;
    if payload.token.trim().is_empty() {
        anyhow::bail!("missing token in auth redeem request");
    }
    if payload.csr_pem.trim().is_empty() {
        anyhow::bail!("missing csr_pem in auth redeem request");
    }
    Ok(Some(payload))
}

async fn send_json_response(
    socket: &mut TcpStream,
    status: &str,
    body: &str,
) -> anyhow::Result<()> {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    socket
        .write_all(response.as_bytes())
        .await
        .context("failed to write auth response")?;
    Ok(())
}
