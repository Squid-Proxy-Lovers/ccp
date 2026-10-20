// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

pub(super) fn entry(
    store: &mut AccessStore<'_>,
    actor: Actor,
    input: &Input,
    now: i64,
) -> Result<Reply, Error> {
    let body: api::EntryPurge = decode(&input.body)?;
    let workspace = id(&body.workspace_id)?;
    let request = domain::EntryPurge {
        entry_id: id(&body.entry_id)?,
        operation_id: id(&body.operation_id)?,
        expected_revision: body.expected_revision,
        confirm_purge: body.confirm_purge,
    };
    let result = store
        .purge_entry(actor, workspace, &request, now)
        .map_err(operation)?;
    response::success(api::EntryPurgeReceipt {
        workspace_id: workspace.to_string(),
        entry_id: result.entry_id.to_string(),
        operation_id: result.operation_id.to_string(),
        revision: result.revision,
        purged_at: result.purged_at,
    })
}
