// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use crate::foundation::{GuidanceContext, GuidanceRead};

impl AccessStore<'_> {
    /// Identity, current bound grant and guidance share one authorized snapshot.
    /// Only a verified client credential can identify this delivery context.
    pub fn client_guidance(
        &mut self,
        actor: Actor,
        workspace: Option<Uuid>,
        now: i64,
    ) -> anyhow::Result<GuidanceRead> {
        let Actor::Credential(credential_id) = actor else {
            return Err(ContentError::Forbidden.into());
        };
        let tx = self.connection.transaction()?;
        let guidance = read::bundle(&tx, actor, workspace, now)?;
        let client_id = authenticated_client(&tx, credential_id, now)?;
        let instance_id = parse_id(tx.query_row(
            "SELECT instance_id FROM ccp_instance WHERE singleton=1",
            [],
            |r| r.get(0),
        )?)?;
        let workspace_grant_id = workspace
            .map(|ws| grant(&tx, credential_id, client_id, ws))
            .transpose()?;
        tx.commit()?;
        Ok(GuidanceRead {
            context: GuidanceContext {
                instance_id,
                client_id,
                credential_id,
                workspace_grant_id,
            },
            guidance,
        })
    }
}

fn grant(db: &Connection, credential: Uuid, client: Uuid, workspace: Uuid) -> anyhow::Result<Uuid> {
    let value:Option<String>=db.query_row("SELECT g.id FROM ccp_credential_grants b JOIN ccp_grants g ON g.id=b.grant_id AND g.client_id=b.client_id WHERE b.credential_id=?1 AND g.client_id=?2 AND g.workspace_id=?3 AND g.revoked_at IS NULL AND NOT EXISTS(SELECT 1 FROM ccp_binding_revocations v WHERE v.credential_id=b.credential_id AND v.grant_id=b.grant_id)",params![credential.to_string(),client.to_string(),workspace.to_string()],|r|r.get(0)).optional()?;
    parse_id(value.ok_or(ContentError::Forbidden)?)
}
