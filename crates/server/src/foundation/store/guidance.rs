// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use crate::foundation::{
    ContentError, GUIDANCE_TEXT_BYTES, GuidanceBoard, GuidanceBundle, GuidancePrecedence,
    GuidancePublisher, GuidanceReceipt, GuidanceSave, GuidanceScope,
};
use rusqlite::OptionalExtension;
use sha2::{Digest, Sha256};

mod context;
mod read;

impl AccessStore<'_> {
    /// Every accepted new operation is a live save, including identical or empty
    /// text. A repeated operation returns only its original body-free proof.
    pub fn save_guidance(
        &mut self,
        actor: Actor,
        scope: GuidanceScope,
        request: &GuidanceSave,
        now: i64,
    ) -> anyhow::Result<GuidanceReceipt> {
        if request.operation_id.is_nil()
            || request.expected_revision >= i64::MAX.unsigned_abs()
            || request.text.len() > GUIDANCE_TEXT_BYTES
        {
            return Err(ContentError::InvalidInput.into());
        }
        let hash = fingerprint(scope, request)?;
        self.mutate(|tx| {
            let action = match scope {
                GuidanceScope::Global => Action::SaveGlobalBoard,
                GuidanceScope::Workspace(_) => Action::SaveWorkspaceBoard,
            };
            if now < 0 || !permitted(tx, actor, action, scope.workspace(), now)? {
                return Err(ContentError::Forbidden.into());
            }
            if let Some(receipt) = replay(tx, actor, scope, request, &hash)? {
                return Ok(receipt);
            }
            let current = head(tx, scope)?.unwrap_or(0);
            if current != request.expected_revision {
                return Err(ContentError::Conflict.into());
            }
            let next = current + 1;
            let publisher = publisher(tx, actor, now)?;
            insert_version(tx, scope, request, next, &publisher)?;
            advance_head(tx, scope, current, next)?;
            insert_receipt(tx, actor, scope, request, &hash, next)?;
            let action = match scope {
                GuidanceScope::Global => "save_global_guidance",
                GuidanceScope::Workspace(_) => "save_workspace_guidance",
            };
            audit(
                tx,
                actor,
                action,
                scope.workspace(),
                request.operation_id,
                now,
            )?;
            Ok(GuidanceReceipt {
                scope,
                operation_id: request.operation_id,
                revision: next,
            })
        })
    }
}

fn fingerprint(scope: GuidanceScope, request: &GuidanceSave) -> anyhow::Result<Vec<u8>> {
    let bytes = serde_json::to_vec(&(
        "guidance-save-1",
        scope.key(),
        request.expected_revision,
        &request.text,
    ))?;
    Ok(Sha256::digest(bytes).to_vec())
}
fn parts(actor: Actor) -> anyhow::Result<(&'static str, Uuid)> {
    match actor {
        Actor::Admin(id) => Ok(("admin", id)),
        Actor::AdminKey(id) => Ok(("key", id)),
        Actor::Credential(_) => Err(ContentError::Forbidden.into()),
    }
}
fn publisher(db: &Connection, actor: Actor, now: i64) -> anyhow::Result<GuidancePublisher> {
    let principal_id = match actor {
        Actor::Admin(id) => id,
        Actor::AdminKey(id) => parse_id(db.query_row(
            "SELECT admin_id FROM ccp_admin_keys WHERE id=?1",
            [id.to_string()],
            |r| r.get(0),
        )?)?,
        Actor::Credential(_) => return Err(ContentError::Forbidden.into()),
    };
    Ok(GuidancePublisher {
        actor,
        principal_id,
        recorded_at: now,
    })
}
fn head(db: &Connection, scope: GuidanceScope) -> anyhow::Result<Option<u64>> {
    Ok(db
        .query_row(
            "SELECT revision FROM ccp_guidance_heads WHERE board_key=?1",
            [scope.key()],
            |r| r.get(0),
        )
        .optional()?)
}
fn insert_version(
    db: &Connection,
    scope: GuidanceScope,
    request: &GuidanceSave,
    revision: u64,
    publisher: &GuidancePublisher,
) -> anyhow::Result<()> {
    let (kind, id) = parts(publisher.actor)?;
    db.execute("INSERT INTO ccp_guidance_versions(board_key,workspace_id,revision,operation_id,text,publisher_kind,publisher_id,principal_id,recorded_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![scope.key(),scope.workspace().map(|id|id.to_string()),i64::try_from(revision)?,request.operation_id.to_string(),request.text,kind,id.to_string(),publisher.principal_id.to_string(),publisher.recorded_at])?;
    Ok(())
}
fn replay(
    db: &Connection,
    actor: Actor,
    scope: GuidanceScope,
    request: &GuidanceSave,
    hash: &[u8],
) -> anyhow::Result<Option<GuidanceReceipt>> {
    let row=db.query_row("SELECT publisher_kind,publisher_id,request_hash,revision FROM ccp_guidance_operations WHERE board_key=?1 AND operation_id=?2",params![scope.key(),request.operation_id.to_string()],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,Vec<u8>>(2)?,r.get::<_,u64>(3)?))).optional()?;
    let Some((kind, id, saved, revision)) = row else {
        return Ok(None);
    };
    let current = parts(actor)?;
    if kind != current.0 || id != current.1.to_string() || saved != hash {
        return Err(ContentError::Conflict.into());
    }
    Ok(Some(GuidanceReceipt {
        scope,
        operation_id: request.operation_id,
        revision,
    }))
}

fn advance_head(
    db: &Connection,
    scope: GuidanceScope,
    current: u64,
    next: u64,
) -> anyhow::Result<()> {
    require_changed(db.execute("INSERT INTO ccp_guidance_heads(board_key,revision) VALUES(?1,?2) ON CONFLICT(board_key) DO UPDATE SET revision=excluded.revision WHERE ccp_guidance_heads.revision=?3",params![scope.key(),i64::try_from(next)?,i64::try_from(current)?])?)
}
fn insert_receipt(
    db: &Connection,
    actor: Actor,
    scope: GuidanceScope,
    request: &GuidanceSave,
    hash: &[u8],
    next: u64,
) -> anyhow::Result<()> {
    let (kind, id) = parts(actor)?;
    db.execute("INSERT INTO ccp_guidance_operations(board_key,workspace_id,operation_id,publisher_kind,publisher_id,expected_revision,request_hash,revision) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![scope.key(),scope.workspace().map(|id|id.to_string()),request.operation_id.to_string(),kind,id.to_string(),i64::try_from(request.expected_revision)?,hash,i64::try_from(next)?])?;
    Ok(())
}
