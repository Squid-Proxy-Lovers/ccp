// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::{
    dispatch::{decode, id},
    request::Input,
    response::{self, Error, Reply},
};
use crate::foundation::{
    AccessStore, Actor, ContentError, EntryRetentionChange, EntryRetentionPolicy,
};
use hyper::Method;
use protocol::workspace::retention as api;

const GET: &str = "/api/v1/admin/content/retention/get";
const CHANGE: &str = "/api/v1/admin/content/retention/change";

pub(super) fn handles(path: &str) -> bool {
    matches!(path, GET | CHANGE)
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
            let result = store
                .entry_retention(actor, id(&request.workspace_id)?, now)
                .map_err(operation)?;
            response::success(policy(result))
        }
        CHANGE => {
            let body: api::Change = decode(&input.body)?;
            let request = EntryRetentionChange {
                operation_id: id(&body.operation_id)?,
                expected_revision: body.expected_revision,
                retention_seconds: body.retention_seconds,
                confirm_timed_purge: body.confirm_timed_purge,
            };
            let result = store
                .change_entry_retention(actor, id(&body.workspace_id)?, &request, now)
                .map_err(operation)?;
            response::success(api::Receipt {
                operation_id: request.operation_id.to_string(),
                policy: policy(result),
            })
        }
        _ => Err(Error::NotFound),
    }
}

fn policy(value: EntryRetentionPolicy) -> api::Policy {
    api::Policy {
        workspace_id: value.workspace_id.to_string(),
        revision: value.revision,
        retention_seconds: value.retention_seconds,
        changed_at: value.changed_at,
    }
}

fn operation(error: anyhow::Error) -> Error {
    match error.downcast_ref::<ContentError>() {
        Some(ContentError::InvalidInput) => Error::BadRequest,
        Some(ContentError::Forbidden) => Error::Forbidden,
        Some(ContentError::Conflict) => Error::Conflict,
        Some(ContentError::LegacyConversionRequired) => Error::LegacyContent,
        _ => Error::Unavailable,
    }
}
