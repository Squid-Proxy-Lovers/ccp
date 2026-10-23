// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

fn decode<T: serde::de::DeserializeOwned>(value: &str, limit: usize) -> anyhow::Result<T> {
    if value.len() > limit {
        bail!("saved entry operation exceeds its size limit");
    }
    serde_json::from_str(value)
        .map_err(|_data| anyhow::anyhow!("invalid saved entry operation data"))
}

impl State {
    pub(crate) fn load(db: &Connection, version: u32) -> anyhow::Result<Self> {
        if version < 4 {
            return Ok(Self::default());
        }
        let (pending, last): (Option<String>, Option<String>) = db.query_row(
            "SELECT pending,last FROM content_state WHERE singleton=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok(Self {
            pending: pending
                .as_deref()
                .map(|v| decode(v, MAX_PENDING))
                .transpose()?,
            last: last.as_deref().map(|v| decode(v, 4096)).transpose()?,
            pending_record: pending,
            last_record: last,
        })
    }
    pub(in crate::workspace) fn validate(
        &self,
        trust: &WorkspaceTrust,
        credential: Option<&Credential>,
        lifecycle_pending: bool,
    ) -> anyhow::Result<()> {
        if self.pending.is_none() && self.last.is_none() {
            return Ok(());
        }
        let credential =
            credential.context("saved entry operation requires an enrolled identity")?;
        if let Some(p) = &self.pending {
            input::binding(&p.binding, trust, credential)?;
            if lifecycle_pending
                || p.binding.credential_id != credential.receipt.credential_id
                || !p.binding.matches(&p.request, &input::validate(&p.request)?)
            {
                bail!(
                    "saved pending entry operation differs from its original credential or payload"
                );
            }
        }
        if let Some(last) = &self.last {
            input::binding(&last.binding, trust, credential)?;
            if let Terminal::Committed { entry_id, revision } = &last.outcome {
                validate::ids(&[entry_id])?;
                validate::revision_number(*revision)?;
            }
        }
        Ok(())
    }
}

impl WorkspaceProfile {
    pub(super) fn prepare_content(
        &mut self,
        request: wire::EntryMutation,
        hash: String,
    ) -> anyhow::Result<()> {
        self.require_no_content_pending()?;
        if input::validate(&request)? != hash {
            bail!("entry request binding changed before saving; no mutation was sent");
        }
        if self.pending.is_some() {
            bail!("credential lifecycle operation is pending");
        }
        let credential = self
            .credential
            .as_ref()
            .context("entry mutation requires an enrolled profile")?;
        crypto::credential(&self.trust, credential, crypto::now()?)?;
        let binding = Binding {
            instance_id: self.trust.instance_id.clone(),
            client_id: credential.receipt.client_id.clone(),
            credential_id: credential.receipt.credential_id.clone(),
            workspace_id: request.workspace_id.clone(),
            operation_id: request.operation_id.clone(),
            request_hash: hash,
        };
        let pending = PendingMutation { binding, request };
        let record = serde_json::to_string(&pending)?;
        if record.len() > MAX_PENDING {
            bail!("saved entry request exceeds its size limit; no mutation was sent");
        }
        let changed=self.connection.execute("UPDATE content_state SET pending=?1 WHERE singleton=1 AND pending IS NULL AND last IS ?2 AND EXISTS(SELECT 1 FROM profile WHERE singleton=1 AND pending IS NULL AND credential IS ?3) AND NOT EXISTS(SELECT 1 FROM status_state WHERE pending IS NOT NULL)",params![record,self.content_state.last_record,self.credential_record]).map_err(|_write|anyhow::anyhow!("cannot persist original entry request; no mutation was sent"))?;
        if changed != 1 {
            bail!("private entry state changed; no mutation was sent");
        }
        self.content_state.pending = Some(pending);
        self.content_state.pending_record = Some(record);
        Ok(())
    }

    pub(super) fn finish_content(&mut self, outcome: Terminal) -> anyhow::Result<()> {
        let pending = self
            .content_state
            .pending
            .as_ref()
            .context("no pending entry operation to finish")?;
        let last = LastMutation {
            binding: pending.binding.clone(),
            outcome,
        };
        let record = serde_json::to_string(&last)?;
        let changed=self.connection.execute("UPDATE content_state SET pending=NULL,last=?1 WHERE singleton=1 AND pending IS ?2 AND last IS ?3 AND EXISTS(SELECT 1 FROM profile WHERE singleton=1 AND pending IS NULL AND credential IS ?4) AND NOT EXISTS(SELECT 1 FROM status_state WHERE pending IS NOT NULL)",params![record,self.content_state.pending_record,self.content_state.last_record,self.credential_record]).map_err(|_write|anyhow::anyhow!("cannot save entry outcome; original pending request retained"))?;
        if changed != 1 {
            bail!("private entry state changed; preserve the pending request");
        }
        self.content_state.pending = None;
        self.content_state.pending_record = None;
        self.content_state.last = Some(last);
        self.content_state.last_record = Some(record);
        Ok(())
    }
}
