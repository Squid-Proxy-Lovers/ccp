// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

impl AccessStore<'_> {
    /// Global guidance is for authenticated identities, including clients with
    /// no selected workspace. Optional workspace guidance requires current read
    /// access. Both sources and permission checks share one database snapshot.
    pub fn guidance(
        &mut self,
        actor: Actor,
        workspace: Option<Uuid>,
        now: i64,
    ) -> anyhow::Result<GuidanceBundle> {
        let tx = self.connection.transaction()?;
        let value = bundle(&tx, actor, workspace, now)?;
        tx.commit()?;
        Ok(value)
    }
}

pub(super) fn bundle(
    db: &Connection,
    actor: Actor,
    workspace: Option<Uuid>,
    now: i64,
) -> anyhow::Result<GuidanceBundle> {
    validate_actor(db, actor, now)?;
    if let Some(workspace) = workspace
        && !permitted(db, actor, Action::ReadContext, Some(workspace), now)?
    {
        return Err(ContentError::Forbidden.into());
    }
    let global = read(db, GuidanceScope::Global)?;
    let workspace = workspace
        .map(|id| read(db, GuidanceScope::Workspace(id)))
        .transpose()?;
    Ok(GuidanceBundle {
        precedence: GuidancePrecedence::GlobalFirst,
        global,
        workspace,
    })
}

fn read(db: &Connection, scope: GuidanceScope) -> anyhow::Result<GuidanceBoard> {
    let Some(revision) = head(db, scope)? else {
        return Ok(GuidanceBoard {
            scope,
            revision: 0,
            text: String::new(),
            publisher: None,
        });
    };
    let row=db.query_row("SELECT text,publisher_kind,publisher_id,principal_id,recorded_at,workspace_id FROM ccp_guidance_versions WHERE board_key=?1 AND revision=?2",params![scope.key(),i64::try_from(revision)?],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,i64>(4)?,r.get::<_,Option<String>>(5)?)))
        .optional()?.context("published guidance revision is unavailable")?;
    if row.5 != scope.workspace().map(|id| id.to_string())
        || revision == 0
        || row.0.len() > GUIDANCE_TEXT_BYTES
        || row.4 < 0
    {
        bail!("published guidance metadata is invalid");
    }
    let id = parse_id(row.2)?;
    let actor = match row.1.as_str() {
        "admin" => Actor::Admin(id),
        "key" => Actor::AdminKey(id),
        _ => bail!("published guidance actor is invalid"),
    };
    Ok(GuidanceBoard {
        scope,
        revision,
        text: row.0,
        publisher: Some(GuidancePublisher {
            actor,
            principal_id: parse_id(row.3)?,
            recorded_at: row.4,
        }),
    })
}
