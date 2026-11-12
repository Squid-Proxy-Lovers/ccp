// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use crate::foundation::{AdminKeyPage, AdminKeySummary, AdminPage, AdminSummary};

impl AccessStore<'_> {
    pub fn workspace_admins_page(
        &mut self,
        actor: Actor,
        workspace: Uuid,
        after: Option<Uuid>,
        limit: usize,
        now: i64,
    ) -> anyhow::Result<AdminPage> {
        check_limit(limit)?;
        let tx = self.connection.transaction()?;
        authorize(
            &tx,
            actor,
            Action::ManageWorkspaceKeys,
            Some(workspace),
            now,
        )?;
        let rows = tx
            .prepare(
                "SELECT p.id,p.name,p.enabled,p.id=w.owner_admin_id FROM ccp_admin_principals p
             JOIN ccp_workspaces w ON w.id=?1 WHERE (p.id=w.owner_admin_id OR EXISTS(
             SELECT 1 FROM ccp_workspace_admins a WHERE a.workspace_id=w.id AND a.admin_id=p.id))
             AND (?2 IS NULL OR p.id>?2) ORDER BY p.id LIMIT ?3",
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
                        r.get::<_, bool>(2)?,
                        r.get::<_, bool>(3)?,
                    ))
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut admins = rows
            .into_iter()
            .map(|(id, name, enabled, is_owner)| {
                Ok(AdminSummary {
                    id: parse_id(id)?,
                    name,
                    enabled,
                    is_owner,
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        let next_cursor = if admins.len() > limit {
            admins.pop();
            Some(admins.last().context("missing admin cursor")?.id)
        } else {
            None
        };
        tx.commit()?;
        Ok(AdminPage {
            admins,
            next_cursor,
        })
    }

    pub fn workspace_admin_keys_page(
        &mut self,
        actor: Actor,
        workspace: Uuid,
        after: Option<Uuid>,
        limit: usize,
        now: i64,
    ) -> anyhow::Result<AdminKeyPage> {
        check_limit(limit)?;
        let tx = self.connection.transaction()?;
        authorize(
            &tx,
            actor,
            Action::ManageWorkspaceKeys,
            Some(workspace),
            now,
        )?;
        let rows = tx
            .prepare(
                "SELECT k.id,k.admin_id,k.name,k.expires_at,k.revoked_at,
             p.enabled=1 AND (p.id=w.owner_admin_id OR EXISTS(SELECT 1 FROM ccp_workspace_admins a
                 WHERE a.workspace_id=w.id AND a.admin_id=p.id))
             FROM ccp_admin_keys k JOIN ccp_admin_principals p ON p.id=k.admin_id
             JOIN ccp_workspaces w ON w.id=k.workspace_id
             WHERE k.workspace_id=?1 AND (?2 IS NULL OR k.id>?2) ORDER BY k.id LIMIT ?3",
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
                        r.get::<_, Option<i64>>(3)?,
                        r.get::<_, Option<i64>>(4)?,
                        r.get::<_, bool>(5)?,
                    ))
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut keys = rows
            .into_iter()
            .map(
                |(id, principal, name, expires_at, revoked_at, role_active)| {
                    Ok(AdminKeySummary {
                        id: parse_id(id)?,
                        principal_id: parse_id(principal)?,
                        name,
                        expires_at,
                        revoked_at,
                        role_active,
                    })
                },
            )
            .collect::<anyhow::Result<Vec<_>>>()?;
        let next_cursor = if keys.len() > limit {
            keys.pop();
            Some(keys.last().context("missing admin key cursor")?.id)
        } else {
            None
        };
        tx.commit()?;
        Ok(AdminKeyPage { keys, next_cursor })
    }
}

fn check_limit(limit: usize) -> anyhow::Result<()> {
    if limit == 0 || limit > 500 {
        bail!("admin page limit must be 1..500");
    }
    Ok(())
}
