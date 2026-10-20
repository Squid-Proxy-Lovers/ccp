// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use protocol::workspace::content as wire;
use rusqlite::{Connection, params};

pub(crate) mod cli;
mod disk;
mod input;
#[cfg(test)]
mod tests;

const MAX_REQUEST: usize = 2 * 1024 * 1024;
const MAX_PENDING: usize = MAX_REQUEST + 4096;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Binding {
    instance_id: String,
    client_id: String,
    credential_id: String,
    workspace_id: String,
    operation_id: String,
    request_hash: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingMutation {
    binding: Binding,
    request: wire::EntryMutation,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
enum Terminal {
    Committed { entry_id: String, revision: u64 },
    Cancelled,
    Conflict,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LastMutation {
    binding: Binding,
    outcome: Terminal,
}

#[derive(Default)]
pub(crate) struct State {
    pending: Option<PendingMutation>,
    last: Option<LastMutation>,
    pending_record: Option<String>,
    last_record: Option<String>,
}

/// Local metadata only. It is not a server authority assertion and contains no
/// stored request body, private key or invitation.
#[derive(Serialize)]
pub struct ContentOperationInfo {
    pub instance_id: String,
    pub client_id: String,
    pub credential_id: String,
    pub workspace_id: String,
    pub operation_id: String,
    pub request_hash: String,
    pub state: &'static str,
}

impl Binding {
    fn info(&self, state: &'static str) -> ContentOperationInfo {
        ContentOperationInfo {
            instance_id: self.instance_id.clone(),
            client_id: self.client_id.clone(),
            credential_id: self.credential_id.clone(),
            workspace_id: self.workspace_id.clone(),
            operation_id: self.operation_id.clone(),
            request_hash: self.request_hash.clone(),
            state,
        }
    }
    fn query(&self) -> wire::ResolveEntryOperation {
        wire::ResolveEntryOperation {
            workspace_id: self.workspace_id.clone(),
            operation_id: self.operation_id.clone(),
            credential_id: self.credential_id.clone(),
            request_hash: self.request_hash.clone(),
        }
    }
    fn matches(&self, request: &wire::EntryMutation, hash: &str) -> bool {
        self.workspace_id == request.workspace_id
            && self.operation_id == request.operation_id
            && self.request_hash == hash
    }
}

impl State {
    pub(crate) fn is_pending(&self) -> bool {
        self.pending.is_some()
    }
    pub(crate) fn pending_info(&self) -> Option<ContentOperationInfo> {
        self.pending.as_ref().map(|p| p.binding.info("pending"))
    }
    pub(crate) fn last_info(&self) -> Option<ContentOperationInfo> {
        self.last.as_ref().map(|v| {
            v.binding.info(match v.outcome {
                Terminal::Committed { .. } => "committed",
                Terminal::Cancelled => "cancelled",
                Terminal::Conflict => "conflict",
            })
        })
    }
}

impl WorkspaceProfile {
    fn require_expected_operation(
        &self,
        expected: &wire::ResolveEntryOperation,
    ) -> anyhow::Result<()> {
        let binding = self.content_binding()?;
        if binding.workspace_id != expected.workspace_id
            || binding.operation_id != expected.operation_id
            || binding.credential_id != expected.credential_id
            || binding.request_hash != expected.request_hash
        {
            bail!("saved entry intent differs from the expected binding; no request was sent");
        }
        Ok(())
    }

    pub(crate) fn require_no_content_pending(&self) -> anyhow::Result<()> {
        if self.content_state.is_pending() {
            bail!(
                "resolve the original pending entry operation before rotating credentials or starting another lifecycle operation"
            );
        }
        Ok(())
    }
    /// Metadata for the original request saved before POST, without its body.
    pub fn pending_entry_operation(&self) -> Option<ContentOperationInfo> {
        self.content_state.pending_info()
    }

    /// Save and submit one explicit intent. A pending intent must finish before
    /// another starts; retries never choose a new UUID, revision or credential.
    pub async fn mutate_entry(
        &mut self,
        request: wire::EntryMutation,
    ) -> anyhow::Result<wire::EntryReceipt> {
        let hash = input::validate(&request)?;
        if let Some(p) = &self.content_state.pending {
            if !p.binding.matches(&request, &hash) {
                bail!("finish the original pending entry intent; no new operation ID was chosen");
            }
            return self.resume_entry_operation().await;
        }
        if let Some(last) = self.content_state.last.as_ref().filter(|last| {
            last.binding.workspace_id == request.workspace_id
                && last.binding.operation_id == request.operation_id
        }) {
            if !last.binding.matches(&request, &hash) {
                bail!("operation ID was already used with different input");
            }
            let result = self.resolve_entry_operation().await?;
            return receipt(result);
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|p| p.operation == Operation::Renewal)
        {
            self.renew().await?;
        }
        if self.pending.is_some() {
            bail!("finish the pending credential operation before starting an entry mutation");
        }
        // Current target access and normal renewal finish before the original
        // credential is captured and its immutable pending request is saved.
        self.content_shelves(&ShelfPage {
            workspace_id: request.workspace_id.clone(),
            after: None,
            limit: Some(1),
        })
        .await?;
        self.prepare_content(request, hash)?;
        self.resume_entry_operation().await
    }

    /// Retry the exact saved request, or resolve the most recent terminal intent.
    pub async fn resume_entry_operation(&mut self) -> anyhow::Result<wire::EntryReceipt> {
        let sent = self.send_pending().await;
        match sent {
            Ok(value) => {
                self.finish_content(Terminal::Committed {
                    entry_id: value.entry_id.clone(),
                    revision: value.revision,
                })?;
                Ok(value)
            }
            Err(_request_error) => receipt(self.resolve_entry_operation().await?),
        }
    }

    async fn send_pending(&self) -> anyhow::Result<wire::EntryReceipt> {
        let pending = self
            .content_state
            .pending
            .as_ref()
            .context("no pending entry operation")?;
        let material = self
            .credential
            .as_ref()
            .context("entry mutation requires an enrolled profile")?;
        if material.receipt.credential_id != pending.binding.credential_id {
            bail!("pending intent cannot be remapped to another credential");
        }
        crypto::credential(&self.trust, material, crypto::now()?)?;
        let http = network::client(&self.trust, Some(material))?;
        network::health(&http, &self.trust.mtls_origin, &self.trust.instance_id).await?;
        let value = network::operation_json(
            http.post(format!(
                "{}api/v1/content/entries/mutate",
                self.trust.mtls_origin
            ))
            .json(&pending.request),
        )
        .await?;
        input::receipt(&pending.request, &value)?;
        Ok(value)
    }

    /// Prove a committed result or cancel an uncommitted original intent.
    pub async fn resolve_entry_operation(
        &mut self,
    ) -> anyhow::Result<wire::EntryOperationResolution> {
        let binding = self.content_binding()?;
        if self
            .pending
            .as_ref()
            .is_some_and(|p| p.operation == Operation::Renewal)
        {
            self.renew().await?;
        }
        let result = self.query_content_resolution(&binding).await;
        self.accept_resolution(binding, result)
    }

    /// Resolve using another currently authorized profile for the same pinned
    /// instance and registered client. Keys and grants remain separate.
    pub async fn resolve_entry_operation_via(
        &mut self,
        other: &mut WorkspaceProfile,
    ) -> anyhow::Result<wire::EntryOperationResolution> {
        let binding = self.content_binding()?;
        if other.trust.instance_id != binding.instance_id
            || crypto::ca_der(&other.trust.ca_pem)? != crypto::ca_der(&self.trust.ca_pem)?
        {
            bail!("resolution profile does not share the pinned instance and CA");
        }
        if other
            .credential
            .as_ref()
            .map(|v| v.receipt.client_id.as_str())
            != Some(binding.client_id.as_str())
        {
            bail!("resolution profile has a different registered client identity");
        }
        if other
            .pending
            .as_ref()
            .is_some_and(|p| p.operation == Operation::Renewal)
        {
            other.renew().await?;
        }
        let result = other.query_content_resolution(&binding).await;
        self.accept_resolution(binding, result)
    }

    fn content_binding(&self) -> anyhow::Result<Binding> {
        self.content_state
            .pending
            .as_ref()
            .map(|p| p.binding.clone())
            .or_else(|| self.content_state.last.as_ref().map(|l| l.binding.clone()))
            .context("no pending or recent terminal entry operation")
    }

    async fn query_content_resolution(
        &self,
        binding: &Binding,
    ) -> anyhow::Result<wire::EntryOperationResolution> {
        let material = self
            .credential
            .as_ref()
            .context("resolution requires a current enrolled profile")?;
        crypto::credential(&self.trust, material, crypto::now()?)?;
        let http = network::client(&self.trust, Some(material))?;
        network::health(&http, &self.trust.mtls_origin, &self.trust.instance_id).await?;
        let value: wire::EntryOperationResolution = network::operation_json(
            http.post(format!(
                "{}api/v1/content/entries/resolve-operation",
                self.trust.mtls_origin
            ))
            .json(&binding.query()),
        )
        .await?;
        if value.workspace_id != binding.workspace_id
            || value.operation_id != binding.operation_id
            || value.credential_id != binding.credential_id
            || value.request_hash != binding.request_hash
        {
            bail!("operation resolution differs from the original intent");
        }
        Ok(value)
    }

    fn accept_resolution(
        &mut self,
        binding: Binding,
        result: anyhow::Result<wire::EntryOperationResolution>,
    ) -> anyhow::Result<wire::EntryOperationResolution> {
        let value = match result {
            Ok(value) => value,
            Err(error) if network::content_conflict(&error) => {
                if self.content_state.is_pending() {
                    self.finish_content(Terminal::Conflict)?;
                }
                bail!(
                    "operation ID conflicts with a recorded intent; inspect current state before choosing a fresh intent"
                );
            }
            Err(_error) => bail!(
                "entry outcome is uncertain; preserve the pending intent and resume or resolve with an authorized profile"
            ),
        };
        let terminal = match &value.outcome {
            wire::EntryOperationOutcome::Committed { entry_id, revision } => {
                validate::ids(&[entry_id])?;
                validate::revision_number(*revision)?;
                if let Some(pending) = &self.content_state.pending {
                    input::receipt(
                        &pending.request,
                        &wire::EntryReceipt {
                            entry_id: entry_id.clone(),
                            revision: *revision,
                        },
                    )?;
                }
                Terminal::Committed {
                    entry_id: entry_id.clone(),
                    revision: *revision,
                }
            }
            wire::EntryOperationOutcome::Cancelled => Terminal::Cancelled,
        };
        if self.content_state.is_pending() {
            self.finish_content(terminal)?;
        } else if self.content_state.last.as_ref().is_none_or(|last| {
            last.outcome != terminal || last.binding.request_hash != binding.request_hash
        }) {
            bail!("remote terminal operation differs from the saved result; no mutation was sent");
        }
        Ok(value)
    }
}

fn receipt(value: wire::EntryOperationResolution) -> anyhow::Result<wire::EntryReceipt> {
    match value.outcome {
        wire::EntryOperationOutcome::Committed { entry_id, revision } => {
            Ok(wire::EntryReceipt { entry_id, revision })
        }
        wire::EntryOperationOutcome::Cancelled => bail!(
            "entry operation cancelled; inspect current state before explicitly choosing a fresh operation ID"
        ),
    }
}
