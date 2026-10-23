// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

const COLUMNS: &str =
    "h.id,h.client_id,c.name,c.disabled,h.revision,h.agent,h.message,h.updated_at";

impl AccessStore<'_> {
    /// Current authorized readers see stale messages; freshness never hides rows.
    pub fn work_status(
        &mut self,
        actor: Actor,
        workspace: Uuid,
        status: Uuid,
        now: i64,
    ) -> anyhow::Result<WorkStatus> {
        let tx = self.connection.transaction()?;
        content::content_scope(&tx, actor, workspace, Action::ReadContext, now)?;
        let value = load(&tx, workspace, status, now)?.ok_or(ContentError::NotFound)?;
        tx.commit()?;
        Ok(value)
    }
    /// Latest uncleared messages only, including stale and disabled authors.
    pub fn work_statuses(
        &mut self,
        actor: Actor,
        workspace: Uuid,
        after: Option<Uuid>,
        limit: usize,
        now: i64,
    ) -> anyhow::Result<WorkStatusPage> {
        if !(1..=32).contains(&limit) {
            return Err(ContentError::InvalidInput.into());
        }
        let tx = self.connection.transaction()?;
        content::content_scope(&tx, actor, workspace, Action::ReadContext, now)?;
        let sql = format!(
            "SELECT {COLUMNS} FROM ccp_work_status h JOIN ccp_clients c ON c.id=h.client_id WHERE h.workspace_id=?1 AND h.message IS NOT NULL AND (?2 IS NULL OR h.id>?2) ORDER BY h.id LIMIT ?3"
        );
        let mut items = tx
            .prepare(&sql)?
            .query_map(
                params![
                    workspace.to_string(),
                    after.map(|id| id.to_string()),
                    i64::try_from(limit + 1)?
                ],
                row,
            )?
            .map(|r| value(r?, workspace, now))
            .collect::<anyhow::Result<Vec<_>>>()?;
        let next_cursor = if items.len() > limit {
            items.pop();
            items.last().map(|item| item.status_id)
        } else {
            None
        };
        tx.commit()?;
        Ok(WorkStatusPage {
            items,
            next_cursor,
            observed_at: now,
            stale_after_seconds: WORK_STATUS_STALE_SECONDS,
        })
    }
}

pub(super) fn load(
    db: &Connection,
    workspace: Uuid,
    status: Uuid,
    now: i64,
) -> anyhow::Result<Option<WorkStatus>> {
    let sql = format!(
        "SELECT {COLUMNS} FROM ccp_work_status h JOIN ccp_clients c ON c.id=h.client_id WHERE h.workspace_id=?1 AND h.id=?2"
    );
    db.query_row(
        &sql,
        params![workspace.to_string(), status.to_string()],
        row,
    )
    .optional()?
    .map(|r| value(r, workspace, now))
    .transpose()
}
struct Row {
    id: String,
    client: String,
    name: String,
    disabled: bool,
    revision: u64,
    agent: Option<String>,
    message: Option<String>,
    updated_at: i64,
}
fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Row> {
    Ok(Row {
        id: r.get(0)?,
        client: r.get(1)?,
        name: r.get(2)?,
        disabled: r.get(3)?,
        revision: r.get(4)?,
        agent: r.get(5)?,
        message: r.get(6)?,
        updated_at: r.get(7)?,
    })
}
fn value(row: Row, workspace: Uuid, now: i64) -> anyhow::Result<WorkStatus> {
    let stale = row.message.is_none()
        || now
            .checked_sub(row.updated_at)
            .is_none_or(|age| !(0..WORK_STATUS_STALE_SECONDS).contains(&age));
    Ok(WorkStatus {
        workspace_id: workspace,
        status_id: parse_id(row.id)?,
        client_id: parse_id(row.client)?,
        client_name: row.name,
        client_disabled: row.disabled,
        revision: row.revision,
        agent: row.agent,
        message: row.message,
        updated_at: row.updated_at,
        stale,
    })
}
