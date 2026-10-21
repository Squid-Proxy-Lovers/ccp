// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use crate::foundation::{
    ContentError, WORK_STATUS_AGENT_BYTES, WORK_STATUS_LIVE_LIMIT, WORK_STATUS_MESSAGE_BYTES,
    WORK_STATUS_STALE_SECONDS, WorkStatus, WorkStatusChange, WorkStatusMutation, WorkStatusPage,
    WorkStatusReceipt,
};
use rusqlite::OptionalExtension;
use sha2::{Digest, Sha256};

mod moderation;
mod read;
mod records;

impl AccessStore<'_> {
    /// Explicit client-owned status only. Normal traffic never refreshes it.
    /// Replays return body-free historical proof without reapplying a change.
    pub fn mutate_work_status(
        &mut self,
        actor: Actor,
        workspace: Uuid,
        request: &WorkStatusMutation,
        now: i64,
    ) -> anyhow::Result<WorkStatusReceipt> {
        let hash = validate(workspace, request, now)?;
        self.mutate(|tx| {
            content::content_scope(tx, actor, workspace, Action::WriteContext, now)?;
            let Actor::Credential(credential) = actor else {
                return Err(ContentError::Forbidden.into());
            };
            let client = authenticated_client(tx, credential, now)?;
            if let Some(receipt) = records::replay(tx, actor, workspace, request, &hash)? {
                return Ok(receipt);
            }
            let current = read::load(tx, workspace, request.status_id, now)?;
            if current
                .as_ref()
                .is_some_and(|head| head.client_id != client)
            {
                return Err(ContentError::Forbidden.into());
            }
            let revision = current.as_ref().map_or(0, |head| head.revision);
            if request.expected_revision != revision
                || current.as_ref().is_some_and(|head| now < head.updated_at)
            {
                return Err(ContentError::Conflict.into());
            }
            match &request.change {
                WorkStatusChange::Clear if current.is_none() => {
                    return Err(ContentError::NotFound.into());
                }
                WorkStatusChange::Clear
                    if current.as_ref().is_some_and(|h| h.message.is_none()) =>
                {
                    return Err(ContentError::Conflict.into());
                }
                WorkStatusChange::Set { .. }
                    if current.as_ref().is_none_or(|h| h.message.is_none()) =>
                {
                    quota(tx, workspace, client)?;
                }
                _ => {}
            }
            let receipt = WorkStatusReceipt {
                workspace_id: workspace,
                status_id: request.status_id,
                operation_id: request.operation_id,
                client_id: client,
                revision: revision + 1,
                cleared: matches!(request.change, WorkStatusChange::Clear),
                updated_at: now,
            };
            records::save(tx, request, &receipt, revision)?;
            records::record(tx, actor, request, &receipt, &hash)?;
            audit(
                tx,
                actor,
                "change_work_status",
                Some(workspace),
                request.operation_id,
                now,
            )?;
            Ok(receipt)
        })
    }
}

fn validate(workspace: Uuid, request: &WorkStatusMutation, now: i64) -> anyhow::Result<Vec<u8>> {
    if now < 0
        || workspace.is_nil()
        || request.operation_id.is_nil()
        || request.status_id.is_nil()
        || request.expected_revision >= i64::MAX.unsigned_abs()
    {
        return Err(ContentError::InvalidInput.into());
    }
    let (agent, message) = match &request.change {
        WorkStatusChange::Set { agent, message } => {
            if message.trim().is_empty()
                || message.len() > WORK_STATUS_MESSAGE_BYTES
                || message.contains('\0')
                || agent.as_ref().is_some_and(|a| {
                    a.trim().is_empty() || a.len() > WORK_STATUS_AGENT_BYTES || a.contains('\0')
                })
            {
                return Err(ContentError::InvalidInput.into());
            }
            (agent.as_deref(), Some(message.as_str()))
        }
        WorkStatusChange::Clear => (None, None),
    };
    let bytes = serde_json::to_vec(&(
        "work-status-1",
        workspace.to_string(),
        request.status_id.to_string(),
        request.expected_revision,
        agent,
        message,
    ))?;
    Ok(Sha256::digest(bytes).to_vec())
}
fn quota(db: &Connection, workspace: Uuid, client: Uuid) -> anyhow::Result<()> {
    let count: usize = db.query_row("SELECT count(*) FROM ccp_work_status WHERE workspace_id=?1 AND client_id=?2 AND message IS NOT NULL", params![workspace.to_string(),client.to_string()], |r|r.get(0))?;
    if count >= WORK_STATUS_LIVE_LIMIT {
        return Err(ContentError::InvalidInput.into());
    }
    Ok(())
}
fn parts(actor: Actor) -> (&'static str, Uuid) {
    match actor {
        Actor::Credential(id) => ("credential", id),
        Actor::Admin(id) => ("admin", id),
        Actor::AdminKey(id) => ("key", id),
    }
}
