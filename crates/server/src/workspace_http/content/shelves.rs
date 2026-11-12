// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

pub(super) fn transition(
    store: &mut AccessStore<'_>,
    actor: Actor,
    input: &Input,
    now: i64,
) -> Result<Reply, Error> {
    let body: api::ShelfTransition = decode(&input.body)?;
    let request = domain::ShelfTransition {
        operation_id: id(&body.operation_id)?,
        shelf_id: id(&body.shelf_id)?,
        expected_revision: body.expected_revision,
        archived: body.archived,
    };
    let result = store
        .transition_shelf(actor, id(&body.workspace_id)?, &request, now)
        .map_err(operation)?;
    response::success(api::ShelfTransitionReceipt {
        operation_id: request.operation_id.to_string(),
        result: project(result),
    })
}

pub(super) fn state(
    store: &mut AccessStore<'_>,
    actor: Actor,
    input: &Input,
    now: i64,
) -> Result<Reply, Error> {
    let body: api::NodeTarget = decode(&input.body)?;
    let value = store
        .shelf_state(actor, id(&body.workspace_id)?, id(&body.node_id)?, now)
        .map_err(operation)?;
    response::success(project(value))
}

pub(super) fn archived(
    store: &mut AccessStore<'_>,
    actor: Actor,
    input: &Input,
    now: i64,
) -> Result<Reply, Error> {
    let body: api::ShelfPage = decode(&input.body)?;
    let page = store
        .archived_shelves(
            actor,
            id(&body.workspace_id)?,
            body.after.as_deref().map(id).transpose()?,
            limit(body.limit)?,
            now,
        )
        .map_err(operation)?;
    response::success(api::Page {
        items: page
            .items
            .into_iter()
            .map(|value| api::ArchivedShelf {
                id: value.id.to_string(),
                workspace_id: value.workspace_id.to_string(),
                name: value.name,
                revision: value.revision,
                created_at: value.created_at,
            })
            .collect(),
        next_cursor: page.next_cursor.map(|id| id.to_string()),
    })
}

fn project(value: domain::ShelfState) -> api::ShelfState {
    api::ShelfState {
        shelf_id: value.shelf_id.to_string(),
        workspace_id: value.workspace_id.to_string(),
        revision: value.revision,
        archived: value.archived,
    }
}
