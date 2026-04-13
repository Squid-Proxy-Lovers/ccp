// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context;
use rcgen::{CertificateParams, KeyPair};
use reqwest::Url;
use serde::{Deserialize, Serialize};

use crate::enrollment_structs::{EnrollmentMaterial, EnrollmentMetadata, StoredEnrollment};
use crate::storage::save_enrollment;

// inital payload we send to the redeem endpoint
#[derive(Serialize)]
struct AuthRedeemPayload {
    token: String,
    csr_pem: String,
}

// response from the redeem endpoint
#[derive(Deserialize)]
struct AuthRedeemResponse {
    session: AuthSessionMetadata,
    access: String,
    client_common_name: String,
    mtls_endpoint: String,
    client_cert_expires_at: u64,
    #[serde(rename = "cert_warning_window_seconds")]
    _cert_warning_window_seconds: u64,
    ca_cert_pem: String,
    client_cert_pem: String,
}

// session metadata
#[derive(Deserialize)]
struct AuthSessionMetadata {
    session_name: String,
    session_id: i64,
    description: String,
    owner: String,
    labels: Vec<String>,
    visibility: String,
    purpose: String,
}

pub(crate) async fn enroll(redeem_url: &str, token: &str) -> anyhow::Result<()> {
    let stored = enroll_and_save(redeem_url, token).await?;

    println!(
        "Saved enrollment for session '{}' (id={}) access={} client_cn={}",
        stored.metadata.session_name,
        stored.metadata.session_id,
        stored.metadata.access,
        stored.metadata.client_cn
    );

    println!(
        "Client certificate expires at unix={}",
        stored.metadata.client_cert_expires_at
    );

    println!("Stored at {}", stored.directory.display());
    Ok(())
}

pub(crate) async fn enroll_and_save(
    redeem_url: &str,
    token: &str,
) -> anyhow::Result<StoredEnrollment> {
    let redeem_url = Url::parse(redeem_url).context("invalid redeem URL")?;

    // format validation for the redeem URL
    if redeem_url.path() != "/auth/redeem" {
        anyhow::bail!("redeem URL must point to /auth/redeem");
    }

    // format validation for the token
    let token = token.trim();
    if token.is_empty() {
        anyhow::bail!("token must not be empty");
    }

    // generate a keypair for mTLS — stays local, only the CSR goes to the server
    let client_key = KeyPair::generate().context("failed to generate client key pair")?;
    let csr_pem = CertificateParams::new(Vec::<String>::new())
        .context("failed to build CSR params")?
        .serialize_request(&client_key)
        .context("failed to serialize CSR")?
        .pem()
        .context("failed to encode CSR PEM")?;

    let client_key_pem = client_key.serialize_pem();

    // disable redirects, auth is only allowed on the exact endpoint passed in
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .context("failed to build enrollment HTTP client")?;

    // post request to redeem the auth token
    let response = client
        .post(redeem_url)
        .json(&AuthRedeemPayload {
            token: token.to_string(),
            csr_pem,
        })
        .send()
        .await
        .context("failed to redeem auth token")?
        .error_for_status()
        .context("auth redeem returned an error status")?
        .json::<AuthRedeemResponse>()
        .await
        .context("failed to parse auth redeem response")?;

    // validate the mtls endpoint
    let _ =
        Url::parse(&response.mtls_endpoint).context("server returned malformed mtls endpoint")?;

    let current_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before UNIX_EPOCH")?
        .as_secs();

    if response.client_cert_expires_at.saturating_sub(current_time) < 3600 {
        anyhow::bail!("client certificate expiration time is within 1 hour of now");
    }

    let material = EnrollmentMaterial {
        metadata: EnrollmentMetadata {
            session_name: response.session.session_name,
            session_id: response.session.session_id,
            session_description: response.session.description,
            owner: response.session.owner,
            labels: response.session.labels,
            visibility: response.session.visibility,
            purpose: response.session.purpose,
            access: response.access,
            client_cn: response.client_common_name,
            mtls_endpoint: response.mtls_endpoint,
            client_cert_expires_at: response.client_cert_expires_at,
            enrolled_at: current_time,
        },
        ca_pem: response.ca_cert_pem,
        client_cert_pem: response.client_cert_pem,
        client_key_pem,
    };
    save_enrollment(&material)
}
