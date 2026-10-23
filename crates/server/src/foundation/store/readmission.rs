// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use rusqlite::OptionalExtension;

use super::invitations::insert_individual;
use super::*;
use crate::foundation::{
    IndividualInvitationRequest, IssuedInvitation, ReadmissionInvitationRequest,
};

impl AccessStore<'_> {
    /// Explicit fresh individual admission, never recovery or generic token
    /// reuse. Captures all old grants so later admission invalidates this ticket
    /// even if that intervening grant is subsequently revoked again.
    pub fn issue_readmission_invitation(
        &mut self,
        actor: Actor,
        request: ReadmissionInvitationRequest,
        now: i64,
    ) -> anyhow::Result<IssuedInvitation> {
        self.mutate(|tx| {
            let issued = insert_individual(tx, actor, IndividualInvitationRequest {
                workspace_id: request.workspace_id, role: request.role, expires_at: request.expires_at,
            }, now)?;
            let enabled: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM ccp_clients WHERE id = ?1 AND disabled = 0)",
                [request.client_id.to_string()], |row| row.get(0))?;
            if !enabled {
                bail!("readmission target is unknown or globally disabled");
            }
            let history = tx.prepare("SELECT id, revoked_at IS NULL FROM ccp_grants WHERE client_id = ?1 AND workspace_id = ?2 ORDER BY id")?
                .query_map(params![request.client_id.to_string(), request.workspace_id.to_string()],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            if history.is_empty() || history.iter().any(|(_, live)| *live) {
                bail!("readmission requires existing fully revoked workspace grant history");
            }
            tx.execute("INSERT INTO ccp_readmission_targets(invitation_id, client_id, workspace_id) VALUES (?1, ?2, ?3)",
                params![issued.id.to_string(), request.client_id.to_string(), request.workspace_id.to_string()])?;
            for (grant, _) in history {
                tx.execute("INSERT INTO ccp_readmission_history(invitation_id, grant_id, client_id, workspace_id) VALUES (?1, ?2, ?3, ?4)",
                    params![issued.id.to_string(), grant, request.client_id.to_string(), request.workspace_id.to_string()])?;
            }
            audit(tx, actor, "issue_readmission_invitation", Some(request.workspace_id), issued.id, now)?;
            Ok(issued)
        })
    }
}

pub(super) fn readmission_target(
    tx: &Connection,
    invitation: Uuid,
    workspace: Uuid,
) -> anyhow::Result<Option<Uuid>> {
    let row = tx
        .query_row(
            "SELECT client_id, workspace_id FROM ccp_readmission_targets WHERE invitation_id = ?1",
            [invitation.to_string()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    row.map(|(client, scope)| {
        if parse_id(scope)? != workspace {
            bail!("persisted readmission scope does not match invitation");
        }
        parse_id(client)
    })
    .transpose()
}

pub(super) fn readmission_current(
    tx: &Connection,
    invitation: Uuid,
    client: Uuid,
    workspace: Uuid,
) -> anyhow::Result<bool> {
    Ok(tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM ccp_clients WHERE id = ?1 AND disabled = 0)
         AND EXISTS(SELECT 1 FROM ccp_readmission_history WHERE invitation_id = ?3)
         AND NOT EXISTS(SELECT 1 FROM ccp_grants g WHERE g.client_id = ?1 AND g.workspace_id = ?2
             AND (g.revoked_at IS NULL OR NOT EXISTS(SELECT 1 FROM ccp_readmission_history h
                 WHERE h.invitation_id = ?3 AND h.grant_id = g.id)))",
        params![
            client.to_string(),
            workspace.to_string(),
            invitation.to_string()
        ],
        |row| row.get(0),
    )?)
}
