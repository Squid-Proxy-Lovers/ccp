// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

pub(super) fn mutate(
    store: &mut AccessStore<'_>,
    actor: Actor,
    input: &Input,
    now: i64,
) -> Result<Reply, Error> {
    let body: api::EntryMutation = decode(&input.body)?;
    let workspace = id(&body.workspace_id)?;
    let request = domain::EntryMutation {
        operation_id: id(&body.operation_id)?,
        agent: body.agent,
        change: change(body.change)?,
    };
    let result = store
        .mutate_entry(actor, workspace, &request, now)
        .map_err(operation)?;
    response::success(api::EntryReceipt {
        entry_id: result.entry_id.to_string(),
        revision: result.revision,
    })
}

fn content(value: api::EntryContent) -> domain::EntryContent {
    domain::EntryContent {
        text: value.text,
        description: value.description,
        labels: value.labels,
    }
}

fn change(value: api::EntryChange) -> Result<domain::EntryChange, Error> {
    Ok(match value {
        api::EntryChange::Create {
            book_id,
            name,
            content: value,
        } => domain::EntryChange::Create {
            book_id: id(&book_id)?,
            name,
            content: content(value),
        },
        api::EntryChange::Append {
            entry_id,
            expected_revision,
            text,
        } => domain::EntryChange::Append {
            id: id(&entry_id)?,
            expected_revision,
            text,
        },
        api::EntryChange::Revise {
            entry_id,
            expected_revision,
            content: value,
        } => domain::EntryChange::Revise {
            id: id(&entry_id)?,
            expected_revision,
            content: content(value),
        },
        api::EntryChange::Archive {
            entry_id,
            expected_revision,
        } => domain::EntryChange::Archive {
            id: id(&entry_id)?,
            expected_revision,
        },
        api::EntryChange::Restore {
            entry_id,
            expected_revision,
        } => domain::EntryChange::Restore {
            id: id(&entry_id)?,
            expected_revision,
        },
    })
}

pub(super) fn get(
    store: &mut AccessStore<'_>,
    actor: Actor,
    input: &Input,
    now: i64,
) -> Result<Reply, Error> {
    let body: api::EntryTarget = decode(&input.body)?;
    let workspace = id(&body.workspace_id)?;
    let entry = id(&body.entry_id)?;
    let result = match body.revision {
        Some(revision) => store.entry_version(actor, workspace, entry, revision, now),
        None => store.get_entry(actor, workspace, entry, now),
    }
    .map_err(operation)?;
    response::content_snapshot(project::snapshot(result))
}

pub(super) fn list(
    store: &mut AccessStore<'_>,
    actor: Actor,
    input: &Input,
    now: i64,
) -> Result<Reply, Error> {
    let body: api::EntryPage = decode(&input.body)?;
    let request = domain::EntryPageRequest {
        book_id: id(&body.book_id)?,
        archived: body.archived,
        after: body.after.as_deref().map(id).transpose()?,
        limit: limit(body.limit)?,
    };
    let result = store
        .entries(actor, id(&body.workspace_id)?, &request, now)
        .map_err(operation)?;
    response::success(api::Page {
        items: result.items.into_iter().map(project::summary).collect(),
        next_cursor: result.next_cursor.map(|v| v.to_string()),
    })
}

pub(super) fn history(
    store: &mut AccessStore<'_>,
    actor: Actor,
    input: &Input,
    now: i64,
) -> Result<Reply, Error> {
    let body: api::HistoryPage = decode(&input.body)?;
    let request = domain::RevisionPageRequest {
        entry_id: id(&body.entry_id)?,
        after: body.after.unwrap_or(0),
        limit: limit(body.limit)?,
    };
    let result = store
        .entry_history(actor, id(&body.workspace_id)?, &request, now)
        .map_err(operation)?;
    response::success(api::History {
        items: result.items.into_iter().map(project::revision).collect(),
        next_cursor: result.next_cursor,
    })
}
