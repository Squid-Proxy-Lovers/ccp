// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

impl WorkspaceProfile {
    pub async fn join(&mut self, invitation: &str) -> anyhow::Result<ProfileInfo> {
        if self
            .pending
            .as_ref()
            .is_some_and(|p| p.operation == Operation::Renewal)
        {
            self.renew().await?;
        }
        let invitation = crypto::invitation(invitation)?;
        let active = self
            .credential
            .as_ref()
            .context("join requires an enrolled registration")?;
        crypto::credential(&self.trust, active, crypto::now()?)?;
        self.prepare_operation(Operation::Admission, Some(invitation))?;
        let http = network::client(&self.trust, self.credential.as_ref())?;
        network::health(&http, &self.trust.mtls_origin, &self.trust.instance_id).await?;
        let p = self.pending.as_ref().context("missing pending admission")?;
        let request = api::Enrollment {
            invitation: invitation.to_owned(),
            request_id: p.request_id.clone(),
            csr_pem: p.csr_pem.clone(),
        };
        let receipt = network::json(
            http.post(format!("{}api/v1/admissions", self.trust.mtls_origin))
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

    pub async fn renew(&mut self) -> anyhow::Result<ProfileInfo> {
        let active = self
            .credential
            .as_ref()
            .context("renewal requires an enrolled registration")?;
        crypto::credential(&self.trust, active, crypto::now()?)?;
        self.prepare_operation(Operation::Renewal, None)?;
        let http = network::client(&self.trust, self.credential.as_ref())?;
        network::health(&http, &self.trust.mtls_origin, &self.trust.instance_id).await?;
        let p = self.pending.as_ref().context("missing pending renewal")?;
        let request = api::Renewal {
            request_id: p.request_id.clone(),
            csr_pem: p.csr_pem.clone(),
        };
        let receipt = network::json(
            http.post(format!(
                "{}api/v1/credentials/renew",
                self.trust.mtls_origin
            ))
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

    pub(super) fn renewal_due(&self, now: i64) -> anyhow::Result<bool> {
        let Some(active) = &self.credential else {
            return Ok(false);
        };
        let expiry = active.receipt.expires_at;
        Ok(now < expiry
            && expiry.saturating_sub(now) <= 7 * 24 * 60 * 60
            && crypto::root_expiry(&self.trust.ca_pem)? > expiry)
    }
}

pub(super) fn validate_result(
    pending: &Pending,
    result: &Credential,
    old: Option<&Credential>,
) -> anyhow::Result<()> {
    if pending.operation == Operation::Enrollment {
        return Ok(());
    }
    let old = old.context("operation lost its original credential")?;
    if result.receipt.client_id != old.receipt.client_id {
        bail!("operation changed the registered client identity");
    }
    match pending.operation {
        Operation::Admission => {
            if result.receipt.credential_id != old.receipt.credential_id
                || result.receipt.expires_at != old.receipt.expires_at
                || crypto::pem(&result.receipt.certificate_pem, "CERTIFICATE")?
                    != crypto::pem(&old.receipt.certificate_pem, "CERTIFICATE")?
            {
                bail!("admission changed its existing credential");
            }
        }
        Operation::Renewal => {
            if result.receipt.credential_id == old.receipt.credential_id {
                bail!("renewal did not rotate its credential");
            }
        }
        Operation::Enrollment => {}
    }
    Ok(())
}
