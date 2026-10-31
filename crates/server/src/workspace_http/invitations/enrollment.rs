// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;

pub(super) fn get(
    store: &mut AccessStore<'_>,
    actor: Actor,
    input: &Input,
    now: i64,
) -> Result<Reply, Error> {
    let body: api::WorkspaceTarget = decode(&input.body)?;
    let value = store
        .initial_enrollment_status(actor, id(&body.workspace_id)?, now)
        .map_err(operation)?;
    response::success(api::InitialEnrollmentStatus {
        workspace_id: value.workspace_id.to_string(),
        phase: match value.phase {
            crate::foundation::InitialEnrollmentPhase::Unused => {
                api::InitialEnrollmentPhase::Unused
            }
            crate::foundation::InitialEnrollmentPhase::Open => api::InitialEnrollmentPhase::Open,
            crate::foundation::InitialEnrollmentPhase::Closed => {
                api::InitialEnrollmentPhase::Closed
            }
        },
        closed_at: value.closed_at,
        invitation: value.invitation.map(status_view),
        has_admission_history: value.has_admission_history,
        workspace_archived: value.workspace_archived,
        can_open: value.can_open,
    })
}
