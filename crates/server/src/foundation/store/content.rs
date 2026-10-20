// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use crate::foundation::{ContentError, HierarchyPage, NodeDraft, WorkspaceBook, WorkspaceShelf};
use rusqlite::OptionalExtension;

type NodeRow = (String, String, String, i64);

impl AccessStore<'_> {
    pub fn create_shelf(
        &mut self,
        actor: Actor,
        workspace: Uuid,
        draft: &NodeDraft,
        now: i64,
    ) -> anyhow::Result<WorkspaceShelf> {
        let name = node(draft)?;
        self.mutate(|tx| {
            content_scope(tx, actor, workspace, Action::WriteContext, now)?;
            let id = Uuid::new_v4();
            tx.execute("INSERT INTO ccp_content_shelves(id,workspace_id,name,description,created_at) VALUES (?1,?2,?3,?4,?5)", params![id.to_string(),workspace.to_string(),name,draft.description,now]).map_err(content_conflict)?;
            tx.execute("INSERT INTO ccp_shelf_lifecycle(shelf_id,workspace_id,revision) VALUES (?1,?2,1)", params![id.to_string(), workspace.to_string()])?;
            audit(tx, actor, "create_shelf", Some(workspace), id, now)?;
            shelf(tx, workspace, id)
        })
    }

    pub fn create_book(
        &mut self,
        actor: Actor,
        workspace: Uuid,
        shelf_id: Uuid,
        draft: &NodeDraft,
        now: i64,
    ) -> anyhow::Result<WorkspaceBook> {
        let name = node(draft)?;
        self.mutate(|tx| {
            content_scope(tx, actor, workspace, Action::WriteContext, now)?;
            shelf(tx, workspace, shelf_id)?;
            let id = Uuid::new_v4();
            tx.execute("INSERT INTO ccp_content_books(id,workspace_id,shelf_id,name,description,created_at) VALUES (?1,?2,?3,?4,?5,?6)", params![id.to_string(),workspace.to_string(),shelf_id.to_string(),name,draft.description,now]).map_err(content_conflict)?;
            audit(tx, actor, "create_book", Some(workspace), id, now)?;
            book(tx, workspace, id)
        })
    }

    pub fn get_shelf(
        &mut self,
        actor: Actor,
        workspace: Uuid,
        id: Uuid,
        now: i64,
    ) -> anyhow::Result<WorkspaceShelf> {
        let tx = self.connection.transaction()?;
        content_scope(&tx, actor, workspace, Action::ReadContext, now)?;
        let result = shelf(&tx, workspace, id)?;
        tx.commit()?;
        Ok(result)
    }

    pub fn get_book(
        &mut self,
        actor: Actor,
        workspace: Uuid,
        id: Uuid,
        now: i64,
    ) -> anyhow::Result<WorkspaceBook> {
        let tx = self.connection.transaction()?;
        content_scope(&tx, actor, workspace, Action::ReadContext, now)?;
        let result = book(&tx, workspace, id)?;
        tx.commit()?;
        Ok(result)
    }

    pub fn shelves(
        &mut self,
        actor: Actor,
        workspace: Uuid,
        after: Option<Uuid>,
        limit: usize,
        now: i64,
    ) -> anyhow::Result<HierarchyPage<WorkspaceShelf>> {
        bounds(limit)?;
        let tx = self.connection.transaction()?;
        content_scope(&tx, actor, workspace, Action::ReadContext, now)?;
        let rows = tx.prepare("SELECT id,name,description,created_at FROM ccp_content_shelves WHERE workspace_id=?1 AND archived=0 AND (?2 IS NULL OR id>?2) ORDER BY id LIMIT ?3")?
            .query_map(params![workspace.to_string(),after.map(|v|v.to_string()),i64::try_from(limit+1)?], node_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        let mut items = rows
            .into_iter()
            .map(|row| shelf_value(workspace, row))
            .collect::<anyhow::Result<Vec<_>>>()?;
        let next_cursor = if items.len() > limit {
            items.pop();
            items.last().map(|s| s.id)
        } else {
            None
        };
        tx.commit()?;
        Ok(HierarchyPage { items, next_cursor })
    }

    pub fn books(
        &mut self,
        actor: Actor,
        workspace: Uuid,
        shelf_id: Uuid,
        after: Option<Uuid>,
        limit: usize,
        now: i64,
    ) -> anyhow::Result<HierarchyPage<WorkspaceBook>> {
        bounds(limit)?;
        let tx = self.connection.transaction()?;
        content_scope(&tx, actor, workspace, Action::ReadContext, now)?;
        shelf(&tx, workspace, shelf_id)?;
        let rows=tx.prepare("SELECT id,name,description,created_at FROM ccp_content_books WHERE workspace_id=?1 AND shelf_id=?2 AND (?3 IS NULL OR id>?3) ORDER BY id LIMIT ?4")?
            .query_map(params![workspace.to_string(),shelf_id.to_string(),after.map(|v|v.to_string()),i64::try_from(limit+1)?],node_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        let mut items = rows
            .into_iter()
            .map(|row| book_value(workspace, shelf_id, row))
            .collect::<anyhow::Result<Vec<_>>>()?;
        let next_cursor = if items.len() > limit {
            items.pop();
            items.last().map(|b| b.id)
        } else {
            None
        };
        tx.commit()?;
        Ok(HierarchyPage { items, next_cursor })
    }
}

pub(super) fn content_scope(
    db: &Connection,
    actor: Actor,
    workspace: Uuid,
    action: Action,
    now: i64,
) -> anyhow::Result<()> {
    if now < 0 || !permitted(db, actor, action, Some(workspace), now)? {
        return Err(ContentError::Forbidden.into());
    }
    let legacy: bool = db.query_row(
        "SELECT legacy_session_id IS NOT NULL FROM ccp_workspaces WHERE id=?1",
        [workspace.to_string()],
        |r| r.get(0),
    )?;
    if legacy {
        return Err(ContentError::LegacyConversionRequired.into());
    }
    Ok(())
}

fn node(draft: &NodeDraft) -> anyhow::Result<&str> {
    let name = validate_name(&draft.name).map_err(|_invalid_name| ContentError::InvalidInput)?;
    if draft.description.len() > 4096 {
        return Err(ContentError::InvalidInput.into());
    }
    Ok(name)
}

fn bounds(limit: usize) -> anyhow::Result<()> {
    if !(1..=500).contains(&limit) {
        return Err(ContentError::InvalidInput.into());
    }
    Ok(())
}

fn node_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<NodeRow> {
    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
}

fn shelf_value(workspace: Uuid, row: NodeRow) -> anyhow::Result<WorkspaceShelf> {
    Ok(WorkspaceShelf {
        id: parse_id(row.0)?,
        workspace_id: workspace,
        name: row.1,
        description: row.2,
        created_at: row.3,
    })
}

fn book_value(workspace: Uuid, shelf_id: Uuid, row: NodeRow) -> anyhow::Result<WorkspaceBook> {
    Ok(WorkspaceBook {
        id: parse_id(row.0)?,
        workspace_id: workspace,
        shelf_id,
        name: row.1,
        description: row.2,
        created_at: row.3,
    })
}

fn shelf(db: &Connection, workspace: Uuid, id: Uuid) -> anyhow::Result<WorkspaceShelf> {
    let row=db.query_row("SELECT id,name,description,created_at FROM ccp_content_shelves WHERE workspace_id=?1 AND id=?2 AND archived=0",params![workspace.to_string(),id.to_string()],node_row).optional()?.ok_or(ContentError::NotFound)?;
    shelf_value(workspace, row)
}

pub(super) fn book(db: &Connection, workspace: Uuid, id: Uuid) -> anyhow::Result<WorkspaceBook> {
    let row=db.query_row("SELECT b.id,b.name,b.description,b.created_at,b.shelf_id FROM ccp_content_books b JOIN ccp_content_shelves s ON s.id=b.shelf_id AND s.workspace_id=b.workspace_id WHERE b.workspace_id=?1 AND b.id=?2 AND s.archived=0",params![workspace.to_string(),id.to_string()],|r|Ok((node_row(r)?,r.get::<_,String>(4)?))).optional()?.ok_or(ContentError::NotFound)?;
    book_value(workspace, parse_id(row.1)?, row.0)
}
