// SPDX-License-Identifier: AGPL-3.0-or-later
use super::invitations::{insert_individual_options, insert_initial, validate_bounds};
use super::*;
use crate::foundation::{
    InitialInvitationRequest, IssuedInvitation, LinkAdmission, LinkInvitationRequest,
};

impl AccessStore<'_> {
    /// Issuance-only bearer, under current scoped admission authority. Unlimited
    /// and non-expiring is refused; the one initial bulk window is never reopened.
    pub fn issue_link_invitation(
        &mut self,
        actor: Actor,
        request: LinkInvitationRequest,
        now: i64,
    ) -> anyhow::Result<IssuedInvitation> {
        let expires = request.lifetime.expires_at(now)?;
        let capacity = match request.admission {
            LinkAdmission::Individual => Some(1),
            LinkAdmission::InitialGroup { max_clients } => max_clients.map(u64::from),
        };
        validate_bounds(capacity, expires, now)?;
        if capacity.is_none() && request.lifetime.long_lived(now) {
            bail!("long-lived group links require a positive use cap");
        }
        self.mutate(|tx| {
            let issued = match request.admission {
                LinkAdmission::Individual => insert_individual_options(
                    tx,
                    actor,
                    request.workspace_id,
                    request.role,
                    expires,
                    now,
                )?,
                LinkAdmission::InitialGroup { max_clients } => insert_initial(
                    tx,
                    actor,
                    InitialInvitationRequest {
                        workspace_id: request.workspace_id,
                        role: request.role,
                        max_clients,
                        expires_at: expires,
                    },
                    now,
                )?,
            };
            audit(
                tx,
                actor,
                "issue_link_invitation",
                Some(request.workspace_id),
                issued.id,
                now,
            )?;
            if expires.is_none() {
                audit(
                    tx,
                    actor,
                    "confirm_non_expiring_invitation",
                    Some(request.workspace_id),
                    issued.id,
                    now,
                )?;
            }
            if expires.is_some() && request.lifetime.long_lived(now) {
                audit(
                    tx,
                    actor,
                    "confirm_long_lived_invitation",
                    Some(request.workspace_id),
                    issued.id,
                    now,
                )?;
            }
            Ok(issued)
        })
    }
}
