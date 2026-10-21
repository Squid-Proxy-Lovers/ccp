// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;
use crate::foundation::{InitialEnrollmentPhase, InitialEnrollmentStatus};

impl AccessStore<'_> {
    /// Current initial-phase observation, including terminal closure refresh.
    /// `can_open` is never an authorization lease for a later issuance.
    pub fn initial_enrollment_status(
        &mut self,
        actor: Actor,
        workspace: Uuid,
        now: i64,
    ) -> anyhow::Result<InitialEnrollmentStatus> {
        self.mutate(|tx| {
            authorize(tx,actor,Action::RevokeGrant,Some(workspace),now)?;
            refresh_initial(tx,workspace,now)?;
            let row=tx.query_row(
                "SELECT w.invitation_id,w.closed_at,i.kind FROM ccp_initial_enrollment w
                 LEFT JOIN ccp_invitations i ON i.id=w.invitation_id AND i.workspace_id=w.workspace_id
                 WHERE w.workspace_id=?1",[workspace.to_string()],
                |r|Ok((r.get::<_,Option<String>>(0)?,r.get::<_,Option<i64>>(1)?,r.get::<_,Option<String>>(2)?)),
            ).optional()?;
            let (phase,closed_at,invitation)=match row {
                None=>(InitialEnrollmentPhase::Unused,None,None),
                Some((id,closed,kind))=>{
                    if id.is_some() && kind.as_deref()!=Some("initial_group") || id.is_none() && closed.is_none() {
                        bail!("invalid initial enrollment association");
                    }
                    let invitation=id.map(|id|status(tx,workspace,parse_id(id)?,now)).transpose()?;
                    (if closed.is_some() {InitialEnrollmentPhase::Closed}else {InitialEnrollmentPhase::Open},closed,invitation)
                },
            };
            let has_admission_history=tx.query_row("SELECT EXISTS(SELECT 1 FROM ccp_grants WHERE workspace_id=?1)",[workspace.to_string()],|r|r.get::<_,bool>(0))?;
            let workspace_archived=tx.query_row("SELECT state='archived' FROM ccp_workspaces WHERE id=?1",[workspace.to_string()],|r|r.get::<_,bool>(0))?;
            let can_open=phase==InitialEnrollmentPhase::Unused && !has_admission_history
                && permitted(tx,actor,Action::AdmitClient,Some(workspace),now)?;
            Ok(InitialEnrollmentStatus {workspace_id:workspace,phase,closed_at,invitation,has_admission_history,workspace_archived,can_open})
        })
    }
}
