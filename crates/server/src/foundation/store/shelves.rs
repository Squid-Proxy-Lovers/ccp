// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use crate::foundation::{ArchivedShelf, ContentError, HierarchyPage, ShelfState, ShelfTransition};
use rusqlite::OptionalExtension;

impl AccessStore<'_> {
    /// Requires a trusted authenticated actor; request IDs are targets only.
    /// The returned receipt describes this operation, not necessarily current state.
    pub fn transition_shelf(
        &mut self,
        actor: Actor,
        workspace: Uuid,
        request: &ShelfTransition,
        now: i64,
    ) -> anyhow::Result<ShelfState> {
        let revision = i64::try_from(request.expected_revision).ok();
        if request.operation_id.is_nil()
            || !revision.is_some_and(|value| value > 0 && value < i64::MAX)
        {
            return Err(ContentError::InvalidInput.into());
        }
        self.mutate(|tx| {
            content::content_scope(tx, actor, workspace, Action::ManageArchives, now)?;
            if let Some(receipt) = replay(tx, actor, workspace, request)? {
                return Ok(receipt);
            }
            let mut current = state(tx, workspace, request.shelf_id)?;
            if current.revision != request.expected_revision || current.archived == request.archived
            {
                return Err(ContentError::Conflict.into());
            }
            let changed = tx.execute(
                "UPDATE ccp_content_shelves SET archived=?1 WHERE workspace_id=?2 AND id=?3",
                params![
                    request.archived,
                    workspace.to_string(),
                    request.shelf_id.to_string()
                ],
            )
            .map_err(content_conflict)?;
            require_changed(changed)?;
            current.revision += 1;
            current.archived = request.archived;
            let changed = tx.execute(
                "UPDATE ccp_shelf_lifecycle SET revision=?1 WHERE workspace_id=?2 AND shelf_id=?3 AND revision=?4",
                params![
                    i64::try_from(current.revision)?,
                    workspace.to_string(),
                    request.shelf_id.to_string(),
                    i64::try_from(request.expected_revision)?
                ],
            )?;
            require_changed(changed)?;
            save(tx, actor, request, &current)?;
            let action = if request.archived {
                "archive_shelf"
            } else {
                "restore_shelf"
            };
            audit(tx, actor, action, Some(workspace), request.shelf_id, now)?;
            Ok(current)
        })
    }

    pub fn shelf_state(
        &mut self,
        actor: Actor,
        workspace: Uuid,
        shelf_id: Uuid,
        now: i64,
    ) -> anyhow::Result<ShelfState> {
        let tx = self.connection.transaction()?;
        content::content_scope(&tx, actor, workspace, Action::ReadContext, now)?;
        let value = state(&tx, workspace, shelf_id)?;
        tx.commit()?;
        Ok(value)
    }

    pub fn archived_shelves(
        &mut self,
        actor: Actor,
        workspace: Uuid,
        after: Option<Uuid>,
        limit: usize,
        now: i64,
    ) -> anyhow::Result<HierarchyPage<ArchivedShelf>> {
        if !(1..=500).contains(&limit) {
            return Err(ContentError::InvalidInput.into());
        }
        let tx = self.connection.transaction()?;
        content::content_scope(&tx, actor, workspace, Action::ReadContext, now)?;
        let rows = tx.prepare("SELECT s.id,s.name,l.revision,s.created_at FROM ccp_content_shelves s JOIN ccp_shelf_lifecycle l ON l.shelf_id=s.id AND l.workspace_id=s.workspace_id WHERE s.workspace_id=?1 AND s.archived=1 AND (?2 IS NULL OR s.id>?2) ORDER BY s.id LIMIT ?3")?
            .query_map(params![workspace.to_string(),after.map(|id|id.to_string()),i64::try_from(limit+1)?],|row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,u64>(2)?,row.get::<_,i64>(3)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut items = rows
            .into_iter()
            .map(|(id, name, revision, created_at)| {
                Ok(ArchivedShelf {
                    id: parse_id(id)?,
                    workspace_id: workspace,
                    name,
                    revision,
                    created_at,
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        let next_cursor = if items.len() > limit {
            items.pop();
            items.last().map(|item| item.id)
        } else {
            None
        };
        tx.commit()?;
        Ok(HierarchyPage { items, next_cursor })
    }
}

fn state(db: &Connection, workspace: Uuid, id: Uuid) -> anyhow::Result<ShelfState> {
    let value = db.query_row("SELECT l.revision,s.archived FROM ccp_content_shelves s JOIN ccp_shelf_lifecycle l ON l.shelf_id=s.id AND l.workspace_id=s.workspace_id WHERE s.workspace_id=?1 AND s.id=?2",
        params![workspace.to_string(),id.to_string()],|row|Ok((row.get(0)?,row.get(1)?)))
        .optional()?.ok_or(ContentError::NotFound)?;
    Ok(ShelfState {
        shelf_id: id,
        workspace_id: workspace,
        revision: value.0,
        archived: value.1,
    })
}

fn actor_parts(actor: Actor) -> (&'static str, Uuid) {
    match actor {
        Actor::Admin(id) => ("admin", id),
        Actor::AdminKey(id) => ("key", id),
        Actor::Credential(id) => ("credential", id),
    }
}

fn replay(
    db: &Connection,
    actor: Actor,
    workspace: Uuid,
    request: &ShelfTransition,
) -> anyhow::Result<Option<ShelfState>> {
    let row = db.query_row("SELECT actor_kind,actor_id,shelf_id,expected_revision,archived,revision FROM ccp_shelf_operations WHERE workspace_id=?1 AND operation_id=?2",
        params![workspace.to_string(),request.operation_id.to_string()],|row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,u64>(3)?,row.get::<_,bool>(4)?,row.get::<_,u64>(5)?))).optional()?;
    let Some((kind, id, shelf, expected, archived, revision)) = row else {
        return Ok(None);
    };
    let actual = actor_parts(actor);
    if kind != actual.0
        || id != actual.1.to_string()
        || shelf != request.shelf_id.to_string()
        || expected != request.expected_revision
        || archived != request.archived
    {
        return Err(ContentError::Conflict.into());
    }
    Ok(Some(ShelfState {
        shelf_id: request.shelf_id,
        workspace_id: workspace,
        revision,
        archived,
    }))
}

fn save(
    db: &Connection,
    actor: Actor,
    request: &ShelfTransition,
    value: &ShelfState,
) -> anyhow::Result<()> {
    let (kind, id) = actor_parts(actor);
    db.execute("INSERT INTO ccp_shelf_operations(workspace_id,operation_id,actor_kind,actor_id,shelf_id,expected_revision,archived,revision) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        params![value.workspace_id.to_string(),request.operation_id.to_string(),kind,id.to_string(),request.shelf_id.to_string(),i64::try_from(request.expected_revision)?,request.archived,i64::try_from(value.revision)?])?;
    Ok(())
}
