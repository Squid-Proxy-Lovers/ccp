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
    let body: api::GrantPage = decode(&input.body)?;
    let limit = body.limit.unwrap_or(100);
    if limit == 0 || limit > 500 {
        return Err(Error::BadRequest);
    }
    let page = store
        .workspace_invitations_page(
            actor,
            id(&body.workspace_id)?,
            body.after.as_deref().map(id).transpose()?,
            usize::try_from(limit).map_err(|_limit| Error::BadRequest)?,
            now,
        )
        .map_err(operation)?;
    response::success(api::Invitations {
        invitations: page
            .invitations
            .into_iter()
            .map(|item| api::InvitationSummary {
                kind: match item.kind {
                    crate::foundation::InvitationKind::InitialGroup => {
                        api::InvitationKind::InitialGroup
                    }
                    crate::foundation::InvitationKind::Individual => {
                        api::InvitationKind::Individual
                    }
                },
                role: match item.role {
                    crate::foundation::DataRole::Read => api::Role::Read,
                    crate::foundation::DataRole::ReadWrite => api::Role::ReadWrite,
                },
                status: status_view(item.status),
            })
            .collect(),
        next_cursor: page.next_cursor.map(|id| id.to_string()),
    })
}
