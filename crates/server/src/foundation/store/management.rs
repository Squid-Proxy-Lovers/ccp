// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use crate::foundation::{GrantPage, GrantSummary, WorkspaceAdministration};

impl AccessStore<'_> {
    /// Administrative metadata only; data clients cannot enumerate principals.
    pub fn workspace_administration(
        &mut self,
        actor: Actor,
        workspace: Uuid,
        now: i64,
    ) -> anyhow::Result<WorkspaceAdministration> {
        let tx = self.connection.transaction()?;
        require_reader(&tx, actor, workspace, now)?;
        let (name, state, owner) = tx.query_row(
            "SELECT name, state, owner_admin_id FROM ccp_workspaces WHERE id=?1",
            [workspace.to_string()],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                ))
            },
        )?;
        let result = WorkspaceAdministration {
            workspace: WorkspaceSummary {
                id: workspace,
                name,
                archived: state == "archived",
            },
            owner_id: owner.map(parse_id).transpose()?,
        };
        tx.commit()?;
        Ok(result)
    }

    /// Snapshot pagination of this workspace's retained grant history only.
    /// A cursor filters the authorized scope; it never selects another scope.
    pub fn workspace_grants_page(
        &mut self,
        actor: Actor,
        workspace: Uuid,
        after: Option<Uuid>,
        limit: usize,
        now: i64,
    ) -> anyhow::Result<GrantPage> {
        if limit == 0 || limit > 500 {
            bail!("grant page limit must be 1..500");
        }
        let tx = self.connection.transaction()?;
        require_reader(&tx, actor, workspace, now)?;
        let rows = tx
            .prepare(
                "SELECT g.id,g.client_id,c.name,g.role,g.revoked_at,c.disabled
             FROM ccp_grants g JOIN ccp_clients c ON c.id=g.client_id
             WHERE g.workspace_id=?1 AND (?2 IS NULL OR g.id>?2) ORDER BY g.id LIMIT ?3",
            )?
            .query_map(
                params![
                    workspace.to_string(),
                    after.map(|id| id.to_string()),
                    i64::try_from(limit + 1)?
                ],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, Option<i64>>(4)?,
                        r.get::<_, bool>(5)?,
                    ))
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut grants = rows
            .into_iter()
            .map(|(id, client, name, role, revoked, disabled)| {
                Ok(GrantSummary {
                    id: parse_id(id)?,
                    client_id: parse_id(client)?,
                    client_name: name,
                    role: match role.as_str() {
                        "read" => DataRole::Read,
                        "read_write" => DataRole::ReadWrite,
                        _ => bail!("invalid persisted grant role"),
                    },
                    revoked_at: revoked,
                    client_disabled: disabled,
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        let next_cursor = if grants.len() > limit {
            grants.pop();
            Some(grants.last().context("missing grant cursor")?.id)
        } else {
            None
        };
        tx.commit()?;
        Ok(GrantPage {
            grants,
            next_cursor,
        })
    }
}

fn require_reader(
    connection: &Connection,
    actor: Actor,
    workspace: Uuid,
    now: i64,
) -> anyhow::Result<()> {
    if matches!(actor, Actor::Credential(_)) {
        bail!("administrative metadata requires an admin identity");
    }
    authorize(connection, actor, Action::ReadContext, Some(workspace), now)
}
