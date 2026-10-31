// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use crate::foundation::{EntryOperationOutcome, EntryOperationTarget};

impl AccessStore<'_> {
    /// Resolve only this client's original intent. Missing commits are fenced
    /// against delayed requests, not reported as a racy "not found" snapshot.
    pub fn resolve_entry_operation(
        &mut self,
        actor: Actor,
        workspace: Uuid,
        target: &EntryOperationTarget,
        now: i64,
    ) -> anyhow::Result<EntryOperationOutcome> {
        if target.operation_id.is_nil() {
            return Err(ContentError::InvalidInput.into());
        }
        let Actor::Credential(current) = actor else {
            return Err(ContentError::Forbidden.into());
        };
        self.mutate(|tx| {
            content::content_scope(tx, actor, workspace, Action::ReadContext, now)?;
            let client = authenticated_client(tx, current, now)?;
            let original: Option<String> = tx.query_row("SELECT client_id FROM ccp_credentials WHERE id=?1",[target.credential_id.to_string()],|r|r.get(0)).optional()?;
            if original.as_deref() != Some(client.to_string().as_str()) {
                return Err(ContentError::Forbidden.into());
            }
            if let Some((credential, hash)) = cancellation(tx, workspace, target.operation_id, client)? {
                binding(&credential, &hash, target.credential_id, &target.request_hash)?;
                return Ok(EntryOperationOutcome::Cancelled);
            }
            if let Some(receipt) = committed(tx, workspace, client, target)? {
                return Ok(EntryOperationOutcome::Committed(receipt));
            }
            tx.execute("INSERT INTO ccp_content_cancellations(workspace_id,operation_id,client_id,credential_id,request_hash,cancelled_at) VALUES (?1,?2,?3,?4,?5,?6)",params![workspace.to_string(),target.operation_id.to_string(),client.to_string(),target.credential_id.to_string(),target.request_hash.as_slice(),now])?;
            audit(tx, actor, "cancel_entry_operation", Some(workspace), target.operation_id, now)?;
            Ok(EntryOperationOutcome::Cancelled)
        })
    }
}

fn committed(
    db: &Connection,
    workspace: Uuid,
    client: Uuid,
    target: &EntryOperationTarget,
) -> anyhow::Result<Option<EntryReceipt>> {
    // Foreign-client/admin UUID collisions are neither disclosed nor modified.
    // A client cancellation cannot reserve their namespace or block their work.
    let row = db.query_row("SELECT o.actor_id,o.request_hash,o.entry_id,o.revision FROM ccp_content_operations o JOIN ccp_credentials c ON c.id=o.actor_id WHERE o.workspace_id=?1 AND o.operation_id=?2 AND o.actor_kind='credential' AND c.client_id=?3",params![workspace.to_string(),target.operation_id.to_string(),client.to_string()],|r|Ok((r.get::<_,String>(0)?,r.get::<_,Vec<u8>>(1)?,r.get::<_,String>(2)?,r.get::<_,u64>(3)?))).optional()?;
    let Some((credential, hash, entry, revision)) = row else {
        return Ok(None);
    };
    binding(
        &credential,
        &hash,
        target.credential_id,
        &target.request_hash,
    )?;
    Ok(Some(EntryReceipt {
        entry_id: parse_id(entry)?,
        revision,
    }))
}

fn cancellation(
    db: &Connection,
    workspace: Uuid,
    operation: Uuid,
    client: Uuid,
) -> anyhow::Result<Option<(String, Vec<u8>)>> {
    Ok(db.query_row("SELECT credential_id,request_hash FROM ccp_content_cancellations WHERE workspace_id=?1 AND operation_id=?2 AND client_id=?3",params![workspace.to_string(),operation.to_string(),client.to_string()],|r|Ok((r.get(0)?,r.get(1)?))).optional()?)
}

fn binding(
    credential: &str,
    hash: &[u8],
    expected: Uuid,
    expected_hash: &[u8],
) -> anyhow::Result<()> {
    if credential != expected.to_string() || hash != expected_hash {
        return Err(ContentError::Conflict.into());
    }
    Ok(())
}

pub(super) fn require_not_cancelled(
    db: &Connection,
    actor: Actor,
    workspace: Uuid,
    request: &EntryMutation,
    hash: &[u8],
    now: i64,
) -> anyhow::Result<()> {
    let Actor::Credential(credential) = actor else {
        return Ok(());
    };
    let client = authenticated_client(db, credential, now)?;
    if let Some((original, saved_hash)) = cancellation(db, workspace, request.operation_id, client)?
    {
        binding(&original, &saved_hash, credential, hash)?;
        return Err(ContentError::OperationCancelled.into());
    }
    Ok(())
}
