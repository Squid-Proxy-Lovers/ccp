// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;

pub(super) fn list(
    store: &mut AccessStore<'_>,
    actor: Actor,
    input: &Input,
    now: i64,
) -> Result<Reply, Error> {
    let body: api::RecoveryPage = decode(&input.body)?;
    let limit = body.limit.unwrap_or(100);
    if limit == 0 || limit > 500 {
        return Err(Error::BadRequest);
    }
    let scope = match body.scope {
        api::RecoveryScope::Instance {} => RecoveryScope::Instance,
        api::RecoveryScope::Workspace { workspace_id } => {
            RecoveryScope::Workspace(id(&workspace_id)?)
        }
    };
    let page = store
        .recovery_invitations_page(
            actor,
            crate::foundation::RecoveryPageRequest {
                client_id: id(&body.client_id)?,
                scope,
                after: body.after.as_deref().map(id).transpose()?,
                limit: usize::try_from(limit).map_err(|_limit| Error::BadRequest)?,
            },
            now,
        )
        .map_err(operation)?;
    response::success(api::RecoveryInvitations {
        invitations: page
            .invitations
            .into_iter()
            .map(|item| api::RecoveryInvitationSummary {
                invitation_id: item.id.to_string(),
                status: recovery_status_view(item.status),
            })
            .collect(),
        next_cursor: page.next_cursor.map(|id| id.to_string()),
    })
}
