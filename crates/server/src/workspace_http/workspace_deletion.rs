// SPDX-License-Identifier: AGPL-3.0-or-later

use super::{
    dispatch::{decode, id},
    request::Input,
    response::{self, Error, Reply},
};
use crate::foundation::{
    AccessStore, Actor, ContentError, WorkspaceDeletion, WorkspaceDeletionRequest,
};
use hyper::Method;
use protocol::workspace::{self as api, deletion};

const PREVIEW: &str = "/api/v1/admin/workspaces/deletion/preview";
const START: &str = "/api/v1/admin/workspaces/deletion/start";
const STATUS: &str = "/api/v1/admin/workspaces/deletion/status";

pub(super) fn handles(path: &str) -> bool {
    matches!(path, PREVIEW | START | STATUS)
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
        PREVIEW => {
            let body: api::WorkspaceTarget = decode(&input.body)?;
            let p = store
                .workspace_deletion_preview(actor, id(&body.workspace_id)?, now)
                .map_err(operation)?;
            response::success(deletion::Preview {
                workspace_id: p.workspace_id.to_string(),
                name: p.name,
                lifecycle_revision: p.lifecycle_revision,
            })
        }
        START => {
            let body: deletion::Start = decode(&input.body)?;
            let request = WorkspaceDeletionRequest {
                workspace_id: id(&body.workspace_id)?,
                operation_id: id(&body.operation_id)?,
                expected_revision: body.expected_revision,
                confirm_workspace_id: id(&body.confirm_workspace_id)?,
            };
            let result = store
                .begin_workspace_deletion(actor, &request, now)
                .map_err(operation)?;
            response::success(receipt(result))
        }
        STATUS => {
            let body: deletion::Status = decode(&input.body)?;
            let result = store
                .workspace_deletion_status(
                    actor,
                    id(&body.workspace_id)?,
                    id(&body.operation_id)?,
                    now,
                )
                .map_err(operation)?;
            response::success(receipt(result))
        }
        _ => Err(Error::NotFound),
    }
}
fn receipt(value: WorkspaceDeletion) -> deletion::Receipt {
    deletion::Receipt {
        workspace_id: value.workspace_id.to_string(),
        operation_id: value.operation_id.to_string(),
        lifecycle_revision: value.lifecycle_revision,
        started_at: value.started_at,
        completed_at: value.completed_at,
    }
}
fn operation(error: anyhow::Error) -> Error {
    match error.downcast_ref::<ContentError>() {
        Some(ContentError::InvalidInput) => Error::BadRequest,
        Some(ContentError::Forbidden) => Error::Forbidden,
        Some(ContentError::NotFound) => Error::NotFound,
        Some(ContentError::Conflict) => Error::Conflict,
        Some(ContentError::LegacyConversionRequired) => Error::LegacyContent,
        _ => Error::Unavailable,
    }
}
