// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;
use crate::foundation::{RecoveryInvitationPage, RecoveryInvitationSummary, RecoveryPageRequest};

impl AccessStore<'_> {
    /// Metadata for a client and exact recovery scope, never a scope union.
    /// Only returned rows observe expiry; lookahead remains untouched.
    pub fn recovery_invitations_page(
        &mut self,
        actor: Actor,
        request: RecoveryPageRequest,
        now: i64,
    ) -> anyhow::Result<RecoveryInvitationPage> {
        if request.limit == 0 || request.limit > 500 {
            bail!("recovery page limit must be 1..500");
        }
        self.mutate(|tx| {
            authorize(
                tx,
                actor,
                request.scope.action(),
                request.scope.workspace(),
                now,
            )?;
            let mut ids = tx
                .prepare(
                    "SELECT id FROM ccp_recovery_invitations WHERE client_id=?1
                     AND workspace_id IS ?2 AND (?3 IS NULL OR id>?3)
                     ORDER BY id LIMIT ?4",
                )?
                .query_map(
                    params![
                        request.client_id.to_string(),
                        request.scope.workspace().map(|id| id.to_string()),
                        request.after.map(|id| id.to_string()),
                        i64::try_from(request.limit + 1)?
                    ],
                    |row| row.get::<_, String>(0),
                )?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let next_cursor = if ids.len() > request.limit {
                ids.pop();
                Some(parse_id(
                    ids.last().context("missing recovery cursor")?.clone(),
                )?)
            } else {
                None
            };
            let invitations = ids
                .into_iter()
                .map(|id| {
                    let id = parse_id(id)?;
                    let invite = load_recovery(tx, id)?;
                    if invite.client != request.client_id || invite.scope != request.scope {
                        bail!("recovery page target does not match");
                    }
                    Ok(RecoveryInvitationSummary {
                        id,
                        status: recovery_status(tx, &invite, now)?,
                    })
                })
                .collect::<anyhow::Result<Vec<_>>>()?;
            Ok(RecoveryInvitationPage {
                invitations,
                next_cursor,
            })
        })
    }
}
