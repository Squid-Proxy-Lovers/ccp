// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use serde::de::DeserializeOwned;

impl WorkspaceProfile {
    pub(super) async fn authenticated_read<Q: Serialize, T: DeserializeOwned>(
        &mut self,
        path: &str,
        request: &Q,
        allow_renewal: bool,
        check: impl Fn(&WorkspaceProfile, &T) -> anyhow::Result<()>,
    ) -> anyhow::Result<T> {
        if self
            .pending
            .as_ref()
            .is_some_and(|p| p.operation == Operation::Renewal)
        {
            self.renew().await?;
        }
        let value = self.authenticated_request(path, request).await?;
        check(self, &value)?;
        // A validated workspace target proves a usable grant even when empty.
        // Global-only guidance proves identity and cannot initiate renewal.
        if allow_renewal
            && self.pending.is_none()
            && !self.content_state.is_pending()
            && !self.status_state.is_pending()
            && self.renewal_due(crypto::now()?)?
        {
            self.renew().await?;
            let value = self.authenticated_request(path, request).await?;
            check(self, &value)?;
            return Ok(value);
        }
        Ok(value)
    }

    pub(super) async fn authenticated_request<Q: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        request: &Q,
    ) -> anyhow::Result<T> {
        let material = self
            .credential
            .as_ref()
            .context("workspace access requires an enrolled profile")?;
        crypto::credential(&self.trust, material, crypto::now()?)?;
        let http = network::client(&self.trust, Some(material))?;
        network::health(&http, &self.trust.mtls_origin, &self.trust.instance_id).await?;
        let request = http
            .post(format!("{}{path}", self.trust.mtls_origin))
            .json(request);
        if path == "api/v1/content/entries/get" {
            network::snapshot_json(request).await
        } else {
            network::json(request).await
        }
    }
}
