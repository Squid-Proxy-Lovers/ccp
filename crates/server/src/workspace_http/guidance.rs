// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::{
    dispatch::{decode, id},
    request::Input,
    response::{self, Error, Reply},
};
use crate::foundation::{self as domain, AccessStore, Actor, ContentError};
use hyper::Method;
use protocol::workspace::guidance as api;

pub(super) const GET: &str = "/api/v1/guidance/get";
pub(super) const SAVE: &str = "/api/v1/admin/guidance/save";

pub(super) fn handles(path: &str) -> bool {
    matches!(path, GET | SAVE)
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
        GET => get(store, actor, input, now),
        SAVE => save(store, actor, input, now),
        _ => Err(Error::NotFound),
    }
}

fn get(store: &mut AccessStore<'_>, actor: Actor, input: &Input, now: i64) -> Result<Reply, Error> {
    let body: api::Get = decode(&input.body)?;
    let workspace = body.workspace_id.as_deref().map(id).transpose()?;
    let bundle = store.guidance(actor, workspace, now).map_err(operation)?;
    let precedence = match bundle.precedence {
        domain::GuidancePrecedence::GlobalFirst => api::Precedence::GlobalFirst,
    };
    response::success(api::Bundle {
        precedence,
        global: board(bundle.global)?,
        workspace: bundle.workspace.map(board).transpose()?,
    })
}

fn save(
    store: &mut AccessStore<'_>,
    actor: Actor,
    input: &Input,
    now: i64,
) -> Result<Reply, Error> {
    let body: api::Save = decode(&input.body)?;
    let scope = match body.scope {
        api::Scope::Global {} => domain::GuidanceScope::Global,
        api::Scope::Workspace { workspace_id } => {
            domain::GuidanceScope::Workspace(id(&workspace_id)?)
        }
    };
    let request = domain::GuidanceSave {
        operation_id: id(&body.operation_id)?,
        expected_revision: body.expected_revision,
        text: body.text,
    };
    let result = store
        .save_guidance(actor, scope, &request, now)
        .map_err(operation)?;
    response::success(api::Receipt {
        scope: project_scope(result.scope),
        operation_id: result.operation_id.to_string(),
        revision: result.revision,
    })
}

fn project_scope(scope: domain::GuidanceScope) -> api::Scope {
    match scope {
        domain::GuidanceScope::Global => api::Scope::Global {},
        domain::GuidanceScope::Workspace(id) => api::Scope::Workspace {
            workspace_id: id.to_string(),
        },
    }
}

fn board(value: domain::GuidanceBoard) -> Result<api::Board, Error> {
    Ok(api::Board {
        scope: project_scope(value.scope),
        revision: value.revision,
        text: value.text,
        publisher: value.publisher.map(publisher).transpose()?,
    })
}

fn publisher(value: domain::GuidancePublisher) -> Result<api::Publisher, Error> {
    let (actor_kind, actor_id) = match value.actor {
        Actor::Admin(id) => (api::PublisherKind::Admin, id),
        Actor::AdminKey(id) => (api::PublisherKind::Key, id),
        Actor::Credential(_) => return Err(Error::Unavailable),
    };
    Ok(api::Publisher {
        actor_kind,
        actor_id: actor_id.to_string(),
        principal_id: value.principal_id.to_string(),
        recorded_at: value.recorded_at,
    })
}

fn operation(error: anyhow::Error) -> Error {
    match error.downcast_ref::<ContentError>() {
        Some(ContentError::InvalidInput) => Error::BadRequest,
        Some(ContentError::Forbidden) => Error::Forbidden,
        Some(ContentError::Conflict) => Error::Conflict,
        _ => Error::Unavailable,
    }
}
