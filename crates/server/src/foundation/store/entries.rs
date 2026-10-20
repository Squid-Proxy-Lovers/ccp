// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use crate::foundation::{
    ContentError, EntryChange, EntryContent, EntryMutation, EntryPageRequest, EntryReceipt,
    EntryRevision, EntrySnapshot, EntrySummary, HierarchyPage, RevisionPage, RevisionPageRequest,
};
use rusqlite::OptionalExtension;
use sha2::{Digest, Sha256};

mod read;
mod write;

const BODY_BYTES: usize = 256 * 1024;
const HEAD_COLUMNS: &str =
    "id,workspace_id,book_id,name,owner_client_id,revision,archived,created_at,updated_at";
const REVISION_COLUMNS: &str =
    "revision,action,actor_kind,actor_id,client_id,admin_principal_id,agent,recorded_at";

impl AccessStore<'_> {
    /// The actor must come from trusted authentication. UUIDs in this internal
    /// API are targets, never proof of identity or permission.
    pub fn mutate_entry(
        &mut self,
        actor: Actor,
        workspace: Uuid,
        request: &EntryMutation,
        now: i64,
    ) -> anyhow::Result<EntryReceipt> {
        validate(request)?;
        let hash = request_hash(request)?;
        self.mutate(|tx| {
            content::content_scope(tx, actor, workspace, Action::WriteContext, now)?;
            if let Some(receipt) = replay(tx, actor, workspace, request, &hash)? {
                return Ok(receipt);
            }
            let receipt = write::apply(tx, actor, workspace, request, now)?;
            let (kind, id) = actor_parts(actor);
            tx.execute("INSERT INTO ccp_content_operations(workspace_id,operation_id,actor_kind,actor_id,request_hash,entry_id,revision) VALUES (?1,?2,?3,?4,?5,?6,?7)",params![workspace.to_string(),request.operation_id.to_string(),kind,id.to_string(),hash.as_slice(),receipt.entry_id.to_string(),i64::try_from(receipt.revision)?])?;
            audit(tx, actor, &format!("{}_entry", change_name(&request.change)), Some(workspace), receipt.entry_id, now)?;
            Ok(receipt)
        })
    }
}

fn actor_parts(actor: Actor) -> (&'static str, Uuid) {
    match actor {
        Actor::Admin(id) => ("admin", id),
        Actor::AdminKey(id) => ("key", id),
        Actor::Credential(id) => ("credential", id),
    }
}

fn change_name(change: &EntryChange) -> &'static str {
    match change {
        EntryChange::Create { .. } => "create",
        EntryChange::Append { .. } => "append",
        EntryChange::Revise { .. } => "revise",
        EntryChange::Archive { .. } => "archive",
        EntryChange::Restore { .. } => "restore",
    }
}

fn validate(request: &EntryMutation) -> anyhow::Result<()> {
    if request.operation_id.is_nil() || request.agent.as_deref().is_some_and(|s| !label(s, 256)) {
        return Err(ContentError::InvalidInput.into());
    }
    match &request.change {
        EntryChange::Create { name, content, .. } => {
            validate_name(name).map_err(|_invalid_name| ContentError::InvalidInput)?;
            validate_content(content)?;
        }
        EntryChange::Append {
            expected_revision,
            text,
            ..
        } => {
            mutable_revision(*expected_revision)?;
            if text.is_empty() || text.len() > BODY_BYTES {
                return Err(ContentError::InvalidInput.into());
            }
        }
        EntryChange::Revise {
            expected_revision,
            content,
            ..
        } => {
            mutable_revision(*expected_revision)?;
            validate_content(content)?;
        }
        EntryChange::Archive {
            expected_revision, ..
        }
        | EntryChange::Restore {
            expected_revision, ..
        } => {
            mutable_revision(*expected_revision)?;
        }
    }
    Ok(())
}

fn revision_number(value: u64) -> anyhow::Result<i64> {
    i64::try_from(value)
        .ok()
        .filter(|v| *v > 0)
        .ok_or_else(|| ContentError::InvalidInput.into())
}

fn mutable_revision(value: u64) -> anyhow::Result<()> {
    if revision_number(value)? == i64::MAX {
        return Err(ContentError::InvalidInput.into());
    }
    Ok(())
}

fn label(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}

fn validate_content(content: &EntryContent) -> anyhow::Result<()> {
    let labels = content
        .labels
        .iter()
        .collect::<std::collections::BTreeSet<_>>();
    if content.text.len() > BODY_BYTES
        || content.description.len() > 4096
        || content.labels.len() > 32
        || labels.len() != content.labels.len()
        || !content.labels.iter().all(|s| label(s, 128))
    {
        return Err(ContentError::InvalidInput.into());
    }
    Ok(())
}

fn request_hash(request: &EntryMutation) -> anyhow::Result<Vec<u8>> {
    use serde_json::json;
    let change = match &request.change {
        EntryChange::Create {
            book_id,
            name,
            content,
        } => json!([book_id.to_string(), name, content]),
        EntryChange::Append {
            id,
            expected_revision,
            text,
        } => json!([id.to_string(), expected_revision, text]),
        EntryChange::Revise {
            id,
            expected_revision,
            content,
        } => json!([id.to_string(), expected_revision, content]),
        EntryChange::Archive {
            id,
            expected_revision,
        }
        | EntryChange::Restore {
            id,
            expected_revision,
        } => json!([id.to_string(), expected_revision]),
    };
    let encoded = serde_json::to_vec(&json!([
        "entry-mutation-1",
        change_name(&request.change),
        request.agent,
        change
    ]))?;
    Ok(Sha256::digest(encoded).to_vec())
}

fn replay(
    db: &Connection,
    actor: Actor,
    workspace: Uuid,
    request: &EntryMutation,
    hash: &[u8],
) -> anyhow::Result<Option<EntryReceipt>> {
    let row = db.query_row("SELECT actor_kind,actor_id,request_hash,entry_id,revision FROM ccp_content_operations WHERE workspace_id=?1 AND operation_id=?2",params![workspace.to_string(),request.operation_id.to_string()],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,Vec<u8>>(2)?,r.get::<_,String>(3)?,r.get::<_,u64>(4)?))).optional()?;
    let Some((kind, id, saved, entry, revision)) = row else {
        return Ok(None);
    };
    let current = actor_parts(actor);
    if kind != current.0 || id != current.1.to_string() || saved != hash {
        return Err(ContentError::Conflict.into());
    }
    Ok(Some(EntryReceipt {
        entry_id: parse_id(entry)?,
        revision,
    }))
}

fn head(db: &Connection, workspace: Uuid, id: Uuid) -> anyhow::Result<EntrySummary> {
    let sql =
        format!("SELECT {HEAD_COLUMNS} FROM ccp_content_entries WHERE workspace_id=?1 AND id=?2");
    let result = db
        .query_row(
            &sql,
            params![workspace.to_string(), id.to_string()],
            read::head_row,
        )
        .optional()?
        .ok_or(ContentError::NotFound)?;
    content::book(db, workspace, result.book_id)?;
    Ok(result)
}

fn active_head(db: &Connection, workspace: Uuid, id: Uuid) -> anyhow::Result<EntrySummary> {
    let head = head(db, workspace, id)?;
    if head.archived {
        return Err(ContentError::NotFound.into());
    }
    Ok(head)
}

fn conflict(error: rusqlite::Error) -> anyhow::Error {
    // Only uniqueness denotes an expected conflict. FK/trigger/I/O failures
    // remain internal errors, so audit failures cannot masquerade as conflicts.
    if matches!(&error, rusqlite::Error::SqliteFailure(e, _) if e.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE)
    {
        ContentError::Conflict.into()
    } else {
        error.into()
    }
}
