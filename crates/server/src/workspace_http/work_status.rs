// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::{
    dispatch::{decode, id},
    request::Input,
    response::{self, Error, Reply},
};
use crate::foundation::{AccessStore, Actor, ContentError, WorkStatusChange, WorkStatusMutation};
use hyper::Method;
use protocol::workspace::work_status as api;

pub(super) const MUTATE: &str = "/api/v1/work/status/mutate";
const GET: &str = "/api/v1/work/status/get";
const LIST: &str = "/api/v1/work/status/list";
pub(super) fn reads(path: &str) -> bool {
    matches!(path, GET | LIST)
}
pub(super) fn handles(path: &str) -> bool {
    reads(path) || path == MUTATE
}

pub(super) fn dispatch(
    store: &mut AccessStore<'_>,
    actor: Actor,
    input: &Input,
    now: i64,
) -> Result<Reply, Error> {
    if input.method != Method::POST {
        return Err(Error::Method);
    }
    match input.path.as_str() {
        GET => {
            let request: api::Get = decode(&input.body)?;
            let status = store
                .work_status(
                    actor,
                    id(&request.workspace_id)?,
                    id(&request.status_id)?,
                    now,
                )
                .map_err(operation)?;
            response::success(project(status))
        }
        LIST => {
            let request: api::List = decode(&input.body)?;
            let limit = request.limit.unwrap_or(32);
            if !(1..=32).contains(&limit) {
                return Err(Error::BadRequest);
            }
            let page = store
                .work_statuses(
                    actor,
                    id(&request.workspace_id)?,
                    request.after.as_deref().map(id).transpose()?,
                    usize::from(limit),
                    now,
                )
                .map_err(operation)?;
            response::success(api::Page {
                items: page.items.into_iter().map(project).collect(),
                next_cursor: page.next_cursor.map(|id| id.to_string()),
                observed_at: page.observed_at,
                stale_after_seconds: page.stale_after_seconds,
            })
        }
        MUTATE => {
            let body: api::Mutation = decode(&input.body)?;
            let request = WorkStatusMutation {
                operation_id: id(&body.operation_id)?,
                status_id: id(&body.status_id)?,
                expected_revision: body.expected_revision,
                change: match body.change {
                    api::Change::Set { agent, message } => WorkStatusChange::Set { agent, message },
                    api::Change::Clear {} => WorkStatusChange::Clear,
                },
            };
            let value = store
                .mutate_work_status(actor, id(&body.workspace_id)?, &request, now)
                .map_err(operation)?;
            response::success(api::Receipt {
                workspace_id: value.workspace_id.to_string(),
                status_id: value.status_id.to_string(),
                operation_id: value.operation_id.to_string(),
                client_id: value.client_id.to_string(),
                revision: value.revision,
                cleared: value.cleared,
                updated_at: value.updated_at,
            })
        }
        _ => Err(Error::NotFound),
    }
}
fn project(value: crate::foundation::WorkStatus) -> api::Status {
    api::Status {
        workspace_id: value.workspace_id.to_string(),
        status_id: value.status_id.to_string(),
        client_id: value.client_id.to_string(),
        client_name: value.client_name,
        client_disabled: value.client_disabled,
        revision: value.revision,
        agent: value.agent,
        message: value.message,
        updated_at: value.updated_at,
        stale: value.stale,
    }
}
fn operation(error: anyhow::Error) -> Error {
    match error.downcast_ref::<ContentError>() {
        Some(ContentError::InvalidInput) => Error::BadRequest,
        Some(ContentError::Forbidden) => Error::Forbidden,
        Some(ContentError::NotFound) => Error::NotFound,
        Some(ContentError::Conflict) => Error::Conflict,
        Some(ContentError::OperationCancelled) => Error::OperationCancelled,
        Some(ContentError::LegacyConversionRequired) => Error::LegacyContent,
        None => Error::Unavailable,
    }
}
