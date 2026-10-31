// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

pub(super) fn replay(
    db: &Connection,
    actor: Actor,
    workspace: Uuid,
    request: &WorkStatusMutation,
    hash: &[u8],
) -> anyhow::Result<Option<WorkStatusReceipt>> {
    let row = db.query_row("SELECT actor_kind,actor_id,request_hash,status_id,client_id,revision,cleared,updated_at FROM ccp_work_status_operations WHERE workspace_id=?1 AND operation_id=?2",params![workspace.to_string(),request.operation_id.to_string()],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,Vec<u8>>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,u64>(5)?,r.get::<_,bool>(6)?,r.get::<_,i64>(7)?))).optional()?;
    let Some((kind, id, saved, status, client, revision, cleared, updated_at)) = row else {
        return Ok(None);
    };
    let (actor_kind, actor_id) = parts(actor);
    if kind != actor_kind || id != actor_id.to_string() || saved != hash {
        return Err(ContentError::Conflict.into());
    }
    Ok(Some(WorkStatusReceipt {
        workspace_id: workspace,
        status_id: parse_id(status)?,
        operation_id: request.operation_id,
        client_id: parse_id(client)?,
        revision,
        cleared,
        updated_at,
    }))
}
pub(super) fn save(
    db: &Connection,
    request: &WorkStatusMutation,
    receipt: &WorkStatusReceipt,
    previous: u64,
) -> anyhow::Result<()> {
    let (agent, message) = match &request.change {
        WorkStatusChange::Set { agent, message } => (agent.as_deref(), Some(message.as_str())),
        WorkStatusChange::Clear => (None, None),
    };
    if previous == 0 {
        require_changed(db.execute("INSERT INTO ccp_work_status(workspace_id,id,client_id,revision,agent,message,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![receipt.workspace_id.to_string(),receipt.status_id.to_string(),receipt.client_id.to_string(),i64::try_from(receipt.revision)?,agent,message,receipt.updated_at])?)?;
    } else {
        require_changed(db.execute("UPDATE ccp_work_status SET revision=?3,agent=?4,message=?5,updated_at=?6 WHERE workspace_id=?1 AND id=?2 AND revision=?7 AND client_id=?8",params![receipt.workspace_id.to_string(),receipt.status_id.to_string(),i64::try_from(receipt.revision)?,agent,message,receipt.updated_at,i64::try_from(previous)?,receipt.client_id.to_string()])?)?;
    }
    Ok(())
}
pub(super) fn record(
    db: &Connection,
    actor: Actor,
    request: &WorkStatusMutation,
    receipt: &WorkStatusReceipt,
    hash: &[u8],
) -> anyhow::Result<()> {
    let (kind, id) = parts(actor);
    require_changed(db.execute("INSERT INTO ccp_work_status_operations(workspace_id,operation_id,actor_kind,actor_id,request_hash,status_id,client_id,revision,cleared,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![receipt.workspace_id.to_string(),request.operation_id.to_string(),kind,id.to_string(),hash,receipt.status_id.to_string(),receipt.client_id.to_string(),i64::try_from(receipt.revision)?,receipt.cleared,receipt.updated_at])?)?;
    Ok(())
}
