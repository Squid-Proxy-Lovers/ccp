// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

impl WorkspaceProfile {
    /// Recovers into a separate empty profile, retaining every original profile.
    /// The expected UUID checks continuity; only the fresh invitation grants authority.
    pub async fn recover(
        &mut self,
        expected_client_id: &str,
        invitation: &str,
    ) -> anyhow::Result<ProfileInfo> {
        let invitation = crypto::recovery_invitation(invitation)?;
        self.prepare_recovery(expected_client_id, invitation)?;
        let http = network::client(&self.trust, None)?;
        network::health(&http, &self.trust.https_origin, &self.trust.instance_id).await?;
        let p = self.pending.as_ref().context("missing pending recovery")?;
        let request = api::Enrollment {
            invitation: invitation.to_owned(),
            request_id: p.request_id.clone(),
            csr_pem: p.csr_pem.clone(),
        };
        let receipt = network::json(
            http.post(format!("{}api/v1/recover", self.trust.https_origin))
                .json(&request),
        )
        .await?;
        let material = Credential {
            receipt,
            key_pem: p.key_pem.clone(),
        };
        self.activate(material)?;
        self.info()
    }
}
