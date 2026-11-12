// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Aggregate dashboard metrics under workspace inspection authority.
use super::{
    super::{
        dispatch::{decode, id},
        request::Input,
        response::{self, Error, Reply},
    },
    operation,
};
use crate::foundation::{AccessStore, Actor, OVERVIEW_MAX_DAYS, WorkspaceOverview};
use protocol::workspace::overview as api;

pub(super) const PATH: &str = "/api/v1/admin/workspaces/overview";

const ADMIN_LABEL: &str = "Admin";

pub(super) fn read(
    store: &mut AccessStore<'_>,
    actor: Actor,
    input: &Input,
    now: i64,
) -> Result<Reply, Error> {
    let body: api::Request = decode(&input.body)?;
    let workspace = id(&body.workspace_id)?;
    if !(1..=OVERVIEW_MAX_DAYS).contains(&body.days) {
        return Err(Error::BadRequest);
    }
    let overview = store
        .workspace_overview(actor, workspace, body.days, now)
        .map_err(operation)?;
    response::success(view(overview))
}

fn view(overview: WorkspaceOverview) -> api::Overview {
    api::Overview {
        workspace_id: overview.workspace_id.to_string(),
        generated_at: overview.generated_at,
        days: overview.days,
        totals: api::Totals {
            shelves: overview.totals.shelves,
            books: overview.totals.books,
            entries: overview.totals.entries,
            archived_entries: overview.totals.archived_entries,
        },
        activity: overview
            .activity
            .into_iter()
            .map(|day| api::ActivityDay {
                day: day.day,
                create: day.create,
                append: day.append,
                revise: day.revise,
                archive: day.archive,
                restore: day.restore,
            })
            .collect(),
        writers: overview
            .writers
            .into_iter()
            .map(|writer| api::Writer {
                label: match (&writer.client_id, writer.client_name) {
                    (Some(_), Some(name)) => name,
                    _ => ADMIN_LABEL.to_owned(),
                },
                client_id: writer.client_id.map(|client| client.to_string()),
                writes: writer.writes,
                last_write_at: writer.last_write_at,
            })
            .collect(),
        access: api::Access {
            active_clients: overview.access.active_clients,
            credentials_expiring_within_7d: overview.access.credentials_expiring_within_7d,
            pending_invitations: overview.access.pending_invitations,
        },
    }
}
