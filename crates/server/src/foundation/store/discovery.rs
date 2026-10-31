// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use crate::foundation::WorkspacePage;

impl AccessStore<'_> {
    /// Bounded HTTP discovery. Candidate SQL follows the actor's credential
    /// bindings/key scope; the canonical permission check remains authoritative.
    pub fn visible_workspaces_page(
        &mut self,
        actor: Actor,
        after: Option<&str>,
        limit: usize,
        now: i64,
    ) -> anyhow::Result<WorkspacePage> {
        if limit == 0 || limit > 500 {
            bail!("workspace page limit must be 1..500");
        }
        let cursor = after.map(parse_cursor).transpose()?;
        let tx = self.connection.transaction()?;
        validate_actor(&tx, actor, now)?;
        let (from, predicate, actor_id) = candidates(&tx, actor)?;
        let sql = format!(
            "SELECT w.id, w.name, w.state FROM {from} WHERE w.state != 'purged' AND ({predicate}) AND (?2 IS NULL OR (w.name, w.id) > (?2, ?3)) ORDER BY w.name, w.id LIMIT ?4"
        );
        let rows = tx
            .prepare(&sql)?
            .query_map(
                params![
                    actor_id.to_string(),
                    cursor.as_ref().map(|v| v.0.as_str()),
                    cursor.as_ref().map(|v| v.1.as_str()),
                    i64::try_from(limit + 1)?
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut workspaces = Vec::new();
        for (id, name, state) in rows {
            let id = parse_id(id)?;
            if permitted(&tx, actor, Action::ReadContext, Some(id), now)? {
                workspaces.push(WorkspaceSummary {
                    id,
                    name,
                    archived: state == "archived",
                });
            }
        }
        let next_cursor = if workspaces.len() > limit {
            workspaces.pop();
            let last = workspaces.last().context("missing workspace page cursor")?;
            let value = serde_json::to_string(&(&last.name, last.id.to_string()))?;
            if value.len() > 1024 {
                bail!("workspace name exceeds cursor transport limit");
            }
            Some(value)
        } else {
            None
        };
        tx.commit()?;
        Ok(WorkspacePage {
            workspaces,
            next_cursor,
        })
    }
}

fn parse_cursor(value: &str) -> anyhow::Result<(String, String)> {
    if value.len() > 1024 {
        bail!("workspace cursor is too large");
    }
    let (name, id): (String, String) =
        serde_json::from_str(value).context("invalid workspace cursor")?;
    let id = Uuid::parse_str(&id).context("invalid workspace cursor UUID")?;
    Ok((name, id.to_string()))
}

fn candidates(
    connection: &Connection,
    actor: Actor,
) -> anyhow::Result<(&'static str, &'static str, Uuid)> {
    match actor {
        Actor::Credential(id) => Ok((
            "ccp_credential_grants b JOIN ccp_grants g ON g.id=b.grant_id AND g.client_id=b.client_id JOIN ccp_workspaces w ON w.id=g.workspace_id",
            "b.credential_id=?1 AND g.revoked_at IS NULL AND NOT EXISTS(SELECT 1 FROM ccp_binding_revocations v WHERE v.credential_id=b.credential_id AND v.grant_id=b.grant_id)",
            id,
        )),
        Actor::AdminKey(id) => {
            let scope: Option<String> = connection.query_row(
                "SELECT workspace_id FROM ccp_admin_keys WHERE id=?1",
                [id.to_string()],
                |r| r.get(0),
            )?;
            if let Some(scope) = scope {
                Ok(("ccp_workspaces w", "w.id=?1", parse_id(scope)?))
            } else {
                Ok(("ccp_workspaces w", "1=1 AND ?1 IS NOT NULL", id))
            }
        }
        Actor::Admin(id) => Ok((
            "ccp_workspaces w",
            "w.owner_admin_id=?1 OR EXISTS(SELECT 1 FROM ccp_workspace_admins a WHERE a.workspace_id=w.id AND a.admin_id=?1) OR EXISTS(SELECT 1 FROM ccp_server_admins s WHERE s.admin_id=?1)",
            id,
        )),
    }
}
