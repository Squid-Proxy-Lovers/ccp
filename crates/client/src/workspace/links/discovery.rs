// SPDX-License-Identifier: AGPL-3.0-or-later
use super::super::{WorkspaceTrust, crypto};
use anyhow::bail;
use protocol::enrollment_link::LinkTarget;
use sha2::{Digest, Sha256};
use std::time::Duration;

/// A separate anonymous, bounded CA fetch. No bearer, CSR, cookie, profile or
/// credential is supplied to this client. Its bytes remain untrusted until pin
/// comparison; normal workspace HTTPS verification is never relaxed.
pub(super) async fn trust(target: &LinkTarget) -> anyhow::Result<WorkspaceTrust> {
    let http = reqwest::Client::builder()
        .https_only(true)
        .no_proxy()
        .http1_only()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .danger_accept_invalid_certs(true)
        .build()
        .map_err(|_http| anyhow::anyhow!("cannot configure anonymous CA discovery"))?;
    let mut response = http
        .get(format!("{}api/v1/ca.pem", target.https_origin))
        .send()
        .await
        .map_err(|_network| {
            anyhow::anyhow!("public CA discovery failed; no invitation was sent")
        })?;
    if response.status() != reqwest::StatusCode::OK
        || response.headers().contains_key("content-encoding")
        || response
            .content_length()
            .is_some_and(|size| size > 16 * 1024)
    {
        bail!("public CA discovery refused or exceeded its bounds; no invitation was sent");
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_body| anyhow::anyhow!("incomplete CA discovery response"))?
    {
        if bytes.len().saturating_add(chunk.len()) > 16 * 1024 {
            bail!("public CA response exceeds size limit");
        }
        bytes.extend_from_slice(&chunk);
    }
    let pem =
        String::from_utf8(bytes).map_err(|_utf8| anyhow::anyhow!("invalid public CA encoding"))?;
    let der = crypto::ca_der(&pem)?;
    let digest: [u8; 32] = Sha256::digest(der).into();
    if digest != target.ca_sha256 {
        bail!("CA pin mismatch; no invitation was sent");
    }
    WorkspaceTrust::new(
        &target.instance_id.to_string(),
        &target.https_origin,
        &target.mtls_origin,
        &pem,
    )
}
