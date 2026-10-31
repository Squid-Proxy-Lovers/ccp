// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

impl AccessStore<'_> {
    pub fn get_entry(
        &mut self,
        actor: Actor,
        workspace: Uuid,
        id: Uuid,
        now: i64,
    ) -> anyhow::Result<EntrySnapshot> {
        self.read_entry(actor, workspace, id, None, now)
    }

    pub fn entry_version(
        &mut self,
        actor: Actor,
        workspace: Uuid,
        id: Uuid,
        revision: u64,
        now: i64,
    ) -> anyhow::Result<EntrySnapshot> {
        revision_number(revision)?;
        self.read_entry(actor, workspace, id, Some(revision), now)
    }

    fn read_entry(
        &mut self,
        actor: Actor,
        workspace: Uuid,
        id: Uuid,
        revision: Option<u64>,
        now: i64,
    ) -> anyhow::Result<EntrySnapshot> {
        let tx = self.connection.transaction()?;
        content::content_scope(&tx, actor, workspace, Action::ReadContext, now)?;
        let entry = active_head(&tx, workspace, id)?;
        let value = snapshot(&tx, &entry, revision.unwrap_or(entry.revision))?;
        tx.commit()?;
        Ok(value)
    }

    pub fn entries(
        &mut self,
        actor: Actor,
        workspace: Uuid,
        request: &EntryPageRequest,
        now: i64,
    ) -> anyhow::Result<HierarchyPage<EntrySummary>> {
        page_limit(request.limit)?;
        let tx = self.connection.transaction()?;
        content::content_scope(&tx, actor, workspace, Action::ReadContext, now)?;
        content::book(&tx, workspace, request.book_id)?;
        let sql = format!(
            "SELECT {HEAD_COLUMNS} FROM ccp_content_entries WHERE workspace_id=?1 AND book_id=?2 AND archived=?3 AND (?4 IS NULL OR id>?4) ORDER BY id LIMIT ?5"
        );
        let mut items = tx
            .prepare(&sql)?
            .query_map(
                params![
                    workspace.to_string(),
                    request.book_id.to_string(),
                    request.archived,
                    request.after.map(|v| v.to_string()),
                    i64::try_from(request.limit + 1)?
                ],
                head_row,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let next_cursor = if items.len() > request.limit {
            items.pop();
            items.last().map(|item| item.id)
        } else {
            None
        };
        tx.commit()?;
        Ok(HierarchyPage { items, next_cursor })
    }

    pub fn entry_history(
        &mut self,
        actor: Actor,
        workspace: Uuid,
        request: &RevisionPageRequest,
        now: i64,
    ) -> anyhow::Result<RevisionPage> {
        page_limit(request.limit)?;
        let after =
            i64::try_from(request.after).map_err(|_out_of_range| ContentError::InvalidInput)?;
        let tx = self.connection.transaction()?;
        content::content_scope(&tx, actor, workspace, Action::ReadContext, now)?;
        active_head(&tx, workspace, request.entry_id)?;
        let sql = format!(
            "SELECT {REVISION_COLUMNS} FROM ccp_content_versions WHERE workspace_id=?1 AND entry_id=?2 AND revision>?3 ORDER BY revision LIMIT ?4"
        );
        let mut items = tx
            .prepare(&sql)?
            .query_map(
                params![
                    workspace.to_string(),
                    request.entry_id.to_string(),
                    after,
                    i64::try_from(request.limit + 1)?
                ],
                revision_row,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let next_cursor = if items.len() > request.limit {
            items.pop();
            items.last().map(|item| item.revision)
        } else {
            None
        };
        tx.commit()?;
        Ok(RevisionPage { items, next_cursor })
    }
}

fn page_limit(limit: usize) -> anyhow::Result<()> {
    if !(1..=500).contains(&limit) {
        return Err(ContentError::InvalidInput.into());
    }
    Ok(())
}

pub(super) fn snapshot(
    db: &Connection,
    head: &EntrySummary,
    revision: u64,
) -> anyhow::Result<EntrySnapshot> {
    let sql = format!(
        "SELECT {REVISION_COLUMNS},content_json,appended_text FROM ccp_content_versions WHERE workspace_id=?1 AND entry_id=?2 AND revision=?3"
    );
    let (revision, json, delta) = db
        .query_row(
            &sql,
            params![
                head.workspace_id.to_string(),
                head.id.to_string(),
                i64::try_from(revision)?
            ],
            |r| {
                Ok((
                    revision_row(r)?,
                    r.get::<_, String>(8)?,
                    r.get::<_, Option<String>>(9)?,
                ))
            },
        )
        .optional()?
        .ok_or(ContentError::NotFound)?;
    Ok(EntrySnapshot {
        entry: head.clone(),
        revision,
        content: serde_json::from_str(&json)?,
        appended_text: delta,
    })
}

fn id(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<Uuid> {
    let raw: String = row.get(index)?;
    Uuid::parse_str(&raw).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(index, rusqlite::types::Type::Text, Box::new(e))
    })
}

fn optional_id(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<Option<Uuid>> {
    if row.get::<_, Option<String>>(index)?.is_some() {
        Ok(Some(id(row, index)?))
    } else {
        Ok(None)
    }
}

pub(super) fn head_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<EntrySummary> {
    Ok(EntrySummary {
        id: id(row, 0)?,
        workspace_id: id(row, 1)?,
        book_id: id(row, 2)?,
        name: row.get(3)?,
        owner_client_id: optional_id(row, 4)?,
        revision: row.get(5)?,
        archived: row.get(6)?,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
    })
}

fn revision_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<EntryRevision> {
    let actor_id = id(row, 3)?;
    let kind: String = row.get(2)?;
    let actor = match kind.as_str() {
        "admin" => Actor::Admin(actor_id),
        "key" => Actor::AdminKey(actor_id),
        "credential" => Actor::Credential(actor_id),
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    Ok(EntryRevision {
        revision: row.get(0)?,
        action: row.get(1)?,
        actor,
        client_id: optional_id(row, 4)?,
        admin_principal_id: optional_id(row, 5)?,
        agent: row.get(6)?,
        recorded_at: row.get(7)?,
    })
}
