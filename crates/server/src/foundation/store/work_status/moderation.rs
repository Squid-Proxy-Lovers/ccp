// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use crate::foundation::WorkStatusClear;

impl AccessStore<'_> {
    /// Clear a client's message as a current scoped administrator. Ownership and
    /// historical receipts remain; this cannot publish as that client.
    pub fn clear_client_work_status(
        &mut self,
        actor: Actor,
        workspace: Uuid,
        clear: &WorkStatusClear,
        now: i64,
    ) -> anyhow::Result<WorkStatusReceipt> {
        let request = WorkStatusMutation {
            operation_id: clear.operation_id,
            status_id: clear.status_id,
            expected_revision: clear.expected_revision,
            change: WorkStatusChange::Clear,
        };
        let hash = validate(workspace, &request, now)?;
        self.mutate(|tx| {
            content::content_scope(tx, actor, workspace, Action::WriteContext, now)?;
            if matches!(actor, Actor::Credential(_)) {
                return Err(ContentError::Forbidden.into());
            }
            if let Some(receipt) = records::replay(tx, actor, workspace, &request, &hash)? {
                return Ok(receipt);
            }
            let head =
                read::load(tx, workspace, request.status_id, now)?.ok_or(ContentError::NotFound)?;
            if head.message.is_none()
                || request.expected_revision != head.revision
                || now < head.updated_at
            {
                return Err(ContentError::Conflict.into());
            }
            let receipt = WorkStatusReceipt {
                workspace_id: workspace,
                status_id: head.status_id,
                operation_id: request.operation_id,
                client_id: head.client_id,
                revision: head.revision + 1,
                cleared: true,
                updated_at: now,
            };
            records::save(tx, &request, &receipt, head.revision)?;
            records::record(tx, actor, &request, &receipt, &hash)?;
            audit(
                tx,
                actor,
                "clear_client_work_status",
                Some(workspace),
                request.operation_id,
                now,
            )?;
            Ok(receipt)
        })
    }
}
