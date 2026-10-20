// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Explicit, private workspace profiles, separate from native session storage.
pub(crate) mod cli;
mod crypto;
mod network;
mod store;

use anyhow::{Context, bail};
use protocol::workspace as api;
use serde::{Deserialize, Serialize};
use std::path::Path;
use uuid::Uuid;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceTrust {
    instance_id: String,
    https_origin: String,
    mtls_origin: String,
    ca_pem: String,
}

impl WorkspaceTrust {
    pub fn new(
        instance_id: &str,
        https_origin: &str,
        mtls_origin: &str,
        ca_pem: &str,
    ) -> anyhow::Result<Self> {
        let trust = Self {
            instance_id: Uuid::parse_str(instance_id)
                .context("invalid instance UUID")?
                .to_string(),
            https_origin: network::origin(https_origin)?,
            mtls_origin: network::origin(mtls_origin)?,
            ca_pem: ca_pem.to_owned(),
        };
        trust.validate()?;
        Ok(trust)
    }
    fn validate(&self) -> anyhow::Result<()> {
        crypto::canonical_id(&self.instance_id)?;
        if network::origin(&self.https_origin)? != self.https_origin
            || network::origin(&self.mtls_origin)? != self.mtls_origin
        {
            bail!("profile origins are not normalized");
        }
        crypto::ca_der(&self.ca_pem)?;
        Ok(())
    }
}

#[derive(Serialize)]
pub struct ProfileInfo {
    pub format: u32,
    pub instance_id: String,
    pub https_origin: String,
    pub mtls_origin: String,
    pub ca_sha256: String,
    pub pending_request_id: Option<String>,
    pub client_id: Option<String>,
    pub credential_id: Option<String>,
    pub expires_at: Option<i64>,
    pub expired: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pending {
    request_id: String,
    invitation_hash: String,
    key_pem: String,
    csr_pem: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Credential {
    receipt: api::CertificateReceipt,
    key_pem: String,
}

/// Holds the exclusive profile lease through network requests and commits.
/// No Debug/Clone/Serialize can expose its private key or duplicate the lease.
pub struct WorkspaceProfile {
    connection: rusqlite::Connection,
    trust: WorkspaceTrust,
    pending: Option<Pending>,
    credential: Option<Credential>,
}

impl WorkspaceProfile {
    pub async fn trust(directory: &Path, trust: WorkspaceTrust) -> anyhow::Result<ProfileInfo> {
        trust.validate()?;
        let http = network::client(&trust, None)?;
        network::health(&http, &trust.https_origin, &trust.instance_id).await?;
        store::create(directory, &trust)?;
        Self::open(directory)?.info()
    }

    pub fn open(directory: &Path) -> anyhow::Result<Self> {
        store::open(directory)
    }

    pub fn info(&self) -> anyhow::Result<ProfileInfo> {
        let now = crypto::now()?;
        if let Some(credential) = &self.credential {
            let historical = now.min(credential.receipt.expires_at.saturating_sub(1));
            crypto::credential(&self.trust, credential, historical)?;
        }
        let receipt = self.credential.as_ref().map(|c| &c.receipt);
        Ok(ProfileInfo {
            format: 1,
            instance_id: self.trust.instance_id.clone(),
            https_origin: self.trust.https_origin.clone(),
            mtls_origin: self.trust.mtls_origin.clone(),
            ca_sha256: crypto::hash(&crypto::ca_der(&self.trust.ca_pem)?),
            pending_request_id: self.pending.as_ref().map(|p| p.request_id.clone()),
            client_id: receipt.map(|r| r.client_id.clone()),
            credential_id: receipt.map(|r| r.credential_id.clone()),
            expires_at: receipt.map(|r| r.expires_at),
            expired: receipt.is_some_and(|r| now >= r.expires_at),
        })
    }

    pub async fn enroll(&mut self, invitation: &str) -> anyhow::Result<ProfileInfo> {
        if self.credential.is_some() {
            bail!("profile already has a registration; do not enroll it again");
        }
        let invitation = crypto::invitation(invitation)?;
        self.prepare(invitation)?;
        let pending = self
            .pending
            .as_ref()
            .context("missing pending enrollment")?;
        let http = network::client(&self.trust, None)?;
        network::health(&http, &self.trust.https_origin, &self.trust.instance_id).await?;
        let body = api::Enrollment {
            invitation: invitation.to_owned(),
            request_id: pending.request_id.clone(),
            csr_pem: pending.csr_pem.clone(),
        };
        let receipt: api::CertificateReceipt = network::json(
            http.post(format!("{}api/v1/enroll", self.trust.https_origin))
                .json(&body),
        )
        .await?;
        let material = Credential {
            receipt,
            key_pem: pending.key_pem.clone(),
        };
        self.activate(material)?;
        self.info()
    }

    pub async fn workspaces(
        &mut self,
        after: Option<&str>,
        limit: u32,
    ) -> anyhow::Result<api::Workspaces> {
        if limit == 0 || limit > 500 || after.is_some_and(|a| a.len() > 1024) {
            bail!("invalid workspace page bounds");
        }
        let material = self
            .credential
            .as_ref()
            .context("profile is not enrolled")?;
        crypto::credential(&self.trust, material, crypto::now()?)?;
        let http = network::client(&self.trust, Some(material))?;
        network::health(&http, &self.trust.mtls_origin, &self.trust.instance_id).await?;
        let mut params = vec![("limit", limit.to_string())];
        if let Some(after) = after {
            params.push(("after", after.to_owned()));
        }
        network::json(
            http.get(format!("{}api/v1/workspaces", self.trust.mtls_origin))
                .query(&params),
        )
        .await
    }
}

#[cfg(test)]
mod tests;
