// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;
use crate::foundation::{InvitationKind, InvitationPage, InvitationSummary};

impl AccessStore<'_> {
    /// Current scoped security metadata. Like individual status reads, persists
    /// observed expiry closures; one page is atomic, never an admission lease.
    pub fn workspace_invitations_page(
        &mut self,
        actor: Actor,
        workspace: Uuid,
        after: Option<Uuid>,
        limit: usize,
        now: i64,
    ) -> anyhow::Result<InvitationPage> {
        if limit == 0 || limit > 500 {
            bail!("invitation page limit must be 1..500");
        }
        self.mutate(|tx| {
            authorize(tx, actor, Action::RevokeGrant, Some(workspace), now)?;
            let mut rows = tx
                .prepare(
                    "SELECT id,kind,role FROM ccp_invitations WHERE workspace_id=?1
                 AND (?2 IS NULL OR id>?2) ORDER BY id LIMIT ?3",
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
                        ))
                    },
                )?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let next_cursor = if rows.len() > limit {
                rows.pop();
                Some(parse_id(
                    rows.last().context("missing invitation cursor")?.0.clone(),
                )?)
            } else {
                None
            };
            let invitations = rows
                .into_iter()
                .map(|(id, kind, role)| {
                    let kind = match kind.as_str() {
                        "initial_group" => InvitationKind::InitialGroup,
                        "individual" => InvitationKind::Individual,
                        _ => bail!("invalid persisted invitation kind"),
                    };
                    let role = match role.as_str() {
                        "read" => DataRole::Read,
                        "read_write" => DataRole::ReadWrite,
                        _ => bail!("invalid persisted invitation role"),
                    };
                    Ok(InvitationSummary {
                        kind,
                        role,
                        status: status(tx, workspace, parse_id(id)?, now)?,
                    })
                })
                .collect::<anyhow::Result<Vec<_>>>()?;
            Ok(InvitationPage {
                invitations,
                next_cursor,
            })
        })
    }
}
