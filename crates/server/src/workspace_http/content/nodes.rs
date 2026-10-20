// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

pub(super) fn dispatch(
    store: &mut AccessStore<'_>,
    actor: Actor,
    input: &Input,
    now: i64,
) -> Result<Reply, Error> {
    match input.path.as_str() {
        "/api/v1/content/shelves/create" => {
            let body: api::CreateShelf = decode(&input.body)?;
            let node = store
                .create_shelf(
                    actor,
                    id(&body.workspace_id)?,
                    &domain::NodeDraft {
                        name: body.name,
                        description: body.description,
                    },
                    now,
                )
                .map_err(operation)?;
            response::success(project::shelf(node))
        }
        "/api/v1/content/books/create" => {
            let body: api::CreateBook = decode(&input.body)?;
            let node = store
                .create_book(
                    actor,
                    id(&body.workspace_id)?,
                    id(&body.shelf_id)?,
                    &domain::NodeDraft {
                        name: body.name,
                        description: body.description,
                    },
                    now,
                )
                .map_err(operation)?;
            response::success(project::book(node))
        }
        "/api/v1/content/shelves/get" => {
            let body: api::NodeTarget = decode(&input.body)?;
            let node = store
                .get_shelf(actor, id(&body.workspace_id)?, id(&body.node_id)?, now)
                .map_err(operation)?;
            response::success(project::shelf(node))
        }
        "/api/v1/content/books/get" => {
            let body: api::NodeTarget = decode(&input.body)?;
            let node = store
                .get_book(actor, id(&body.workspace_id)?, id(&body.node_id)?, now)
                .map_err(operation)?;
            response::success(project::book(node))
        }
        "/api/v1/content/shelves/list" => shelves(store, actor, input, now),
        "/api/v1/content/books/list" => books(store, actor, input, now),
        _ => Err(Error::NotFound),
    }
}

fn shelves(
    store: &mut AccessStore<'_>,
    actor: Actor,
    input: &Input,
    now: i64,
) -> Result<Reply, Error> {
    let body: api::ShelfPage = decode(&input.body)?;
    let page = store
        .shelves(
            actor,
            id(&body.workspace_id)?,
            body.after.as_deref().map(id).transpose()?,
            limit(body.limit)?,
            now,
        )
        .map_err(operation)?;
    response::success(api::Page {
        items: page.items.into_iter().map(project::shelf).collect(),
        next_cursor: page.next_cursor.map(|v| v.to_string()),
    })
}

fn books(
    store: &mut AccessStore<'_>,
    actor: Actor,
    input: &Input,
    now: i64,
) -> Result<Reply, Error> {
    let body: api::BookPage = decode(&input.body)?;
    let page = store
        .books(
            actor,
            id(&body.workspace_id)?,
            id(&body.shelf_id)?,
            body.after.as_deref().map(id).transpose()?,
            limit(body.limit)?,
            now,
        )
        .map_err(operation)?;
    response::success(api::Page {
        items: page.items.into_iter().map(project::book).collect(),
        next_cursor: page.next_cursor.map(|v| v.to_string()),
    })
}
