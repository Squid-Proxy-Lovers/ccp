// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later
use super::{
    Surface,
    dispatch::decode,
    request::Input,
    response::{self, Error, Reply},
    session::Context,
};
use crate::{
    foundation::AccessStore,
    setup_code::{ClaimError, ClaimFailure, Refusal},
};
use hyper::Method;
use rusqlite::Connection;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Verify {
    code: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Claim {
    code: String,
    operator: String,
    workspace: String,
}
pub(super) fn handles(path: &str) -> bool {
    path.starts_with("/api/v1/setup/")
}
fn refusal(error: Refusal) -> Error {
    match error {
        Refusal::Unauthorized => Error::Unauthorized,
        Refusal::Expired => Error::SetupExpired,
        Refusal::Throttled => Error::SetupThrottled,
        Refusal::Claimed => Error::SetupComplete,
        Refusal::Unavailable => Error::Unavailable,
    }
}
pub(super) fn owned(connection: &Connection) -> Result<bool, Error> {
    let admins: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM ccp_server_admins)",
            [],
            |row| row.get(0),
        )
        .map_err(|_database| Error::Unavailable)?;
    if admins {
        return Ok(true);
    }
    let marker: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='ccp_setup_reservation')",[],|row|row.get(0)).map_err(|_database|Error::Unavailable)?;
    if !marker {
        return Ok(false);
    }
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM ccp_setup_reservation WHERE claimed_at IS NOT NULL)",
            [],
            |row| row.get(0),
        )
        .map_err(|_database| Error::Unavailable)
}
/// Called only by the existing blocking dispatcher; never on the async worker.
pub(super) fn dispatch(context: &Context, input: Input) -> Result<Reply, Error> {
    if !matches!(context.surface, Surface::Https) {
        return Err(Error::NotFound);
    }
    if !matches!(
        input.path.as_str(),
        "/api/v1/setup/verify" | "/api/v1/setup/claim"
    ) {
        return Err(Error::NotFound);
    }
    if input.method != Method::POST || input.query.is_some() {
        return Err(Error::BadRequest);
    }
    if !input.origin_present
        || input.csrf.as_deref() != Some("setup")
        || input.authorization.is_some()
    {
        return Err(Error::Forbidden);
    }
    let gate = context.state.setup.as_ref().ok_or(Error::SetupComplete)?;
    if input.path == "/api/v1/setup/verify" {
        let request: Verify = decode(&input.body)?;
        gate.verify(&request.code).map_err(refusal)?;
        let db = Connection::open(&context.state.database).map_err(|_db| Error::Unavailable)?;
        if owned(&db)? {
            let result = gate.claim(&request.code, || {
                Err::<(), ClaimFailure<Error>>(ClaimFailure::AlreadyClaimed)
            });
            return Err(match result {
                Err(ClaimError::Refused(e)) => refusal(e),
                _ => Error::Unavailable,
            });
        }
        return response::success(
            serde_json::json!({"verified":true,"instance_id":context.state.identity.trust().instance_id}),
        );
    }
    let request: Claim = decode(&input.body)?;
    crate::foundation::validate_name(&request.operator).map_err(|_name| Error::BadRequest)?;
    crate::foundation::validate_name(&request.workspace).map_err(|_name| Error::BadRequest)?;
    let result = gate.claim(&request.code, || {
        let mut db = Connection::open(&context.state.database)
            .map_err(|_db| ClaimFailure::Retryable(Error::Unavailable))?;
        db.busy_timeout(std::time::Duration::from_secs(2))
            .map_err(|_db| ClaimFailure::Retryable(Error::Unavailable))?;
        db.pragma_update(None, "synchronous", "FULL")
            .map_err(|_db| ClaimFailure::Retryable(Error::Unavailable))?;
        if owned(&db).map_err(ClaimFailure::Retryable)? {
            return Err(ClaimFailure::AlreadyClaimed);
        }
        let now = crate::instance_tls::now()
            .map_err(|_clock| ClaimFailure::Retryable(Error::Unavailable))?;
        let value = AccessStore::new(&mut db)
            .map_err(|_db| ClaimFailure::Retryable(Error::Unavailable))?
            .bootstrap_instance(&request.operator, &request.workspace, now);
        match value {
            Ok(value) => Ok(value),
            Err(_operation) => {
                if owned(&db).map_err(ClaimFailure::Retryable)? {
                    Err(ClaimFailure::AlreadyClaimed)
                } else {
                    Err(ClaimFailure::Retryable(Error::Unavailable))
                }
            }
        }
    });
    let value = match result {
        Ok(value) => value,
        Err(ClaimError::Refused(e)) => return Err(refusal(e)),
        Err(ClaimError::Operation(e)) => return Err(e),
    };
    // Secrets are returned only after durable commit and code consumption. No
    // cookie/session/role is cached; login uses the existing scoped-key endpoint.
    response::success(
        serde_json::json!({"instance_id":value.instance_id.to_string(),"workspace_id":value.workspace_id.to_string(),"principal_id":value.instance_key.principal_id.to_string(),"instance_admin_key":{"id":value.instance_key.id.to_string(),"secret":value.instance_key.secret()},"workspace_owner_key":{"id":value.owner_key.id.to_string(),"secret":value.owner_key.secret()}}),
    )
}
