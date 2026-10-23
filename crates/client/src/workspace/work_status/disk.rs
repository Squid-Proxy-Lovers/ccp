// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

fn decode<T: serde::de::DeserializeOwned>(text: &str, max: usize) -> anyhow::Result<T> {
    if text.len() > max {
        bail!("saved status metadata exceeds limits");
    }
    serde_json::from_str(text)
        .map_err(|_data| anyhow::anyhow!("invalid private status operation data"))
}
impl State {
    pub(in crate::workspace) fn load(db: &Connection, version: u32) -> anyhow::Result<Self> {
        if version < 5 {
            return Ok(Self::default());
        }
        let (pending, last): (Option<String>, Option<String>) = db.query_row(
            "SELECT pending,last FROM status_state WHERE singleton=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok(Self {
            pending: pending
                .as_deref()
                .map(|s| decode(s, MAX_PENDING))
                .transpose()?,
            last: last.as_deref().map(|s| decode(s, 4096)).transpose()?,
            pending_record: pending,
            last_record: last,
        })
    }
    pub(in crate::workspace) fn validate(
        &self,
        trust: &WorkspaceTrust,
        credential: Option<&Credential>,
        other_pending: bool,
    ) -> anyhow::Result<()> {
        if self.pending.is_none() && self.last.is_none() {
            return Ok(());
        }
        let credential = credential.context("status operations require an enrolled identity")?;
        if let Some(p) = &self.pending {
            input::binding(&p.binding, trust, credential)?;
            if other_pending
                || p.binding.target.credential_id != credential.receipt.credential_id
                || !p.binding.matches(&p.request, &input::mutation(&p.request)?)
                || p.binding.expected_revision != p.request.expected_revision
                || p.binding.cleared != matches!(p.request.change, wire::Change::Clear {})
            {
                bail!(
                    "saved status request differs from its original binding or another operation is pending"
                );
            }
        }
        if let Some(last) = &self.last {
            input::binding(&last.binding, trust, credential)?;
            if let Some(receipt) = &last.receipt {
                input::receipt(&last.binding, receipt)?;
            }
        }
        Ok(())
    }
}
impl WorkspaceProfile {
    pub(super) fn prepare_status(
        &mut self,
        request: wire::Mutation,
        hash: String,
    ) -> anyhow::Result<()> {
        self.require_no_content_pending()?;
        if self.pending.is_some() || input::mutation(&request)? != hash {
            bail!("status request cannot be saved while another operation is pending");
        }
        let credential = self
            .credential
            .as_ref()
            .context("status publication requires an enrolled profile")?;
        crypto::credential(&self.trust, credential, crypto::now()?)?;
        let binding = Binding {
            target: WorkStatusTarget {
                workspace_id: request.workspace_id.clone(),
                status_id: request.status_id.clone(),
                operation_id: request.operation_id.clone(),
                credential_id: credential.receipt.credential_id.clone(),
                request_hash: hash,
            },
            instance_id: self.trust.instance_id.clone(),
            client_id: credential.receipt.client_id.clone(),
            expected_revision: request.expected_revision,
            cleared: matches!(request.change, wire::Change::Clear {}),
        };
        let pending = PendingStatus { binding, request };
        let record = serde_json::to_string(&pending)?;
        if record.len() > MAX_PENDING {
            bail!("private pending status request exceeds limits");
        }
        let changed=self.connection.execute("UPDATE status_state SET pending=?1 WHERE singleton=1 AND pending IS NULL AND last IS ?2 AND NOT EXISTS(SELECT 1 FROM content_state WHERE pending IS NOT NULL) AND EXISTS(SELECT 1 FROM profile WHERE singleton=1 AND pending IS NULL AND credential IS ?3)",params![record,self.status_state.last_record,self.credential_record]).map_err(|_error|anyhow::anyhow!("cannot persist original status request; no write was sent"))?;
        if changed != 1 {
            bail!("private status state changed; no write was sent");
        }
        self.status_state.pending = Some(pending);
        self.status_state.pending_record = Some(record);
        Ok(())
    }
    pub(super) fn finish_status(&mut self, receipt: Option<wire::Receipt>) -> anyhow::Result<()> {
        let binding = self
            .status_state
            .pending
            .as_ref()
            .context("no pending status intent")?
            .binding
            .clone();
        if let Some(value) = &receipt {
            input::receipt(&binding, value)?;
        }
        let last = LastStatus { binding, receipt };
        let record = serde_json::to_string(&last)?;
        if record.len() > 4096 {
            bail!("terminal status metadata exceeds limits");
        }
        let changed=self.connection.execute("UPDATE status_state SET pending=NULL,last=?1 WHERE singleton=1 AND pending IS ?2 AND last IS ?3 AND NOT EXISTS(SELECT 1 FROM content_state WHERE pending IS NOT NULL) AND EXISTS(SELECT 1 FROM profile WHERE singleton=1 AND pending IS NULL AND credential IS ?4)",params![record,self.status_state.pending_record,self.status_state.last_record,self.credential_record]).map_err(|_error|anyhow::anyhow!("cannot save status outcome; original pending request retained"))?;
        if changed != 1 {
            bail!("private status state changed; original intent retained");
        }
        self.status_state.pending = None;
        self.status_state.pending_record = None;
        self.status_state.last = Some(last);
        self.status_state.last_record = Some(record);
        Ok(())
    }
}
