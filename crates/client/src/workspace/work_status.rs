// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Explicit status observations and durable, credential-bound status intents.
use super::*;
use protocol::workspace::work_status as wire;
pub use protocol::workspace::work_status::{Change, Get, List, Mutation, Page, Receipt, Status};
use rusqlite::{Connection, params};

pub(crate) mod cli;
mod disk;
mod input;
mod reads;
#[cfg(test)]
mod tests;

const MAX_REQUEST: usize = 64 * 1024;
const MAX_PENDING: usize = MAX_REQUEST + 4096;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkStatusTarget {
    pub workspace_id: String,
    pub status_id: String,
    pub operation_id: String,
    pub credential_id: String,
    pub request_hash: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Binding {
    target: WorkStatusTarget,
    instance_id: String,
    client_id: String,
    expected_revision: u64,
    cleared: bool,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingStatus {
    binding: Binding,
    request: wire::Mutation,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LastStatus {
    binding: Binding,
    receipt: Option<wire::Receipt>,
}
#[derive(Default)]
pub(crate) struct State {
    pending: Option<PendingStatus>,
    last: Option<LastStatus>,
    pending_record: Option<String>,
    last_record: Option<String>,
}
/// Local body-free metadata; neither pending nor last state is authority.
#[derive(Serialize)]
pub struct WorkStatusOperationInfo {
    #[serde(flatten)]
    pub target: WorkStatusTarget,
    pub instance_id: String,
    pub client_id: String,
    pub expected_revision: u64,
    pub action: &'static str,
    pub state: &'static str,
    pub receipt: Option<wire::Receipt>,
}
impl Binding {
    fn info(&self, state: &'static str, receipt: Option<wire::Receipt>) -> WorkStatusOperationInfo {
        WorkStatusOperationInfo {
            target: self.target.clone(),
            instance_id: self.instance_id.clone(),
            client_id: self.client_id.clone(),
            expected_revision: self.expected_revision,
            action: if self.cleared { "clear" } else { "set" },
            state,
            receipt,
        }
    }
    fn matches(&self, request: &wire::Mutation, hash: &str) -> bool {
        self.target.workspace_id == request.workspace_id
            && self.target.status_id == request.status_id
            && self.target.operation_id == request.operation_id
            && self.target.request_hash == hash
    }
}
impl State {
    pub(crate) fn is_pending(&self) -> bool {
        self.pending.is_some()
    }
    pub(crate) fn pending_info(&self) -> Option<WorkStatusOperationInfo> {
        self.pending
            .as_ref()
            .map(|p| p.binding.info("pending", None))
    }
    pub(crate) fn last_info(&self) -> Option<WorkStatusOperationInfo> {
        self.last.as_ref().map(|last| {
            last.binding.info(
                if last.receipt.is_some() {
                    "committed"
                } else {
                    "locally_forgotten"
                },
                last.receipt.clone(),
            )
        })
    }
}
impl WorkspaceProfile {
    /// Save before POST. No implicit replacement UUID, CAS or credential retry.
    pub async fn mutate_status(
        &mut self,
        request: wire::Mutation,
    ) -> anyhow::Result<wire::Receipt> {
        let hash = input::mutation(&request)?;
        if let Some(pending) = &self.status_state.pending {
            if !pending.binding.matches(&request, &hash) {
                bail!(
                    "finish or explicitly forget the original pending status intent; no new request was sent"
                );
            }
            let target = pending.binding.target.clone();
            return self.resume_status_operation(&target).await;
        }
        self.require_no_content_pending()?;
        if self.pending.is_some() {
            bail!("finish the credential lifecycle operation before status publication");
        }
        let repeated = self.status_state.last.as_ref().filter(|last| {
            last.binding.target.workspace_id == request.workspace_id
                && last.binding.target.operation_id == request.operation_id
        });
        if let Some(last) = repeated {
            let credential = self
                .credential
                .as_ref()
                .context("status publication requires an enrolled profile")?;
            if !last.binding.matches(&request, &hash)
                || last.binding.target.credential_id != credential.receipt.credential_id
            {
                bail!(
                    "the original status operation cannot be changed or remapped to another credential"
                );
            }
        } else {
            self.status_messages(&wire::List {
                workspace_id: request.workspace_id.clone(),
                after: None,
                limit: Some(1),
            })
            .await?;
        }
        self.prepare_status(request, hash)?;
        let target = self
            .status_state
            .pending
            .as_ref()
            .context("missing saved status intent")?
            .binding
            .target
            .clone();
        self.resume_status_operation(&target).await
    }
    /// Deliberately send the exact saved intent once under its original key.
    pub async fn resume_status_operation(
        &mut self,
        expected: &WorkStatusTarget,
    ) -> anyhow::Result<wire::Receipt> {
        self.require_status_target(expected)?;
        let pending = self
            .status_state
            .pending
            .as_ref()
            .context("no pending status intent")?;
        let credential = self
            .credential
            .as_ref()
            .context("status publication requires an enrolled profile")?;
        if credential.receipt.credential_id != pending.binding.target.credential_id {
            bail!("status intent cannot be remapped to another credential");
        }
        let value: wire::Receipt = self
            .authenticated_request("api/v1/work/status/mutate", &pending.request)
            .await?;
        input::receipt(&pending.binding, &value)?;
        self.finish_status(Some(value.clone()))?;
        Ok(value)
    }
    /// Local discard ONLY. The server request is not cancelled and may complete.
    pub fn forget_status_operation(
        &mut self,
        expected: &WorkStatusTarget,
        confirm: bool,
    ) -> anyhow::Result<()> {
        if !confirm {
            bail!("explicitly confirm local forgetting; it does not cancel the server operation");
        }
        self.require_status_target(expected)?;
        self.finish_status(None)
    }
    fn require_status_target(&self, expected: &WorkStatusTarget) -> anyhow::Result<()> {
        input::target(expected)?;
        let original = &self
            .status_state
            .pending
            .as_ref()
            .context("no pending status intent; inspect last metadata separately")?
            .binding
            .target;
        if original != expected {
            bail!("saved status intent differs from the expected target; no request was sent");
        }
        Ok(())
    }
}
