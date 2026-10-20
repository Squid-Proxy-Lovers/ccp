// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::{
    State, Surface, invitations, management,
    request::Input,
    response::{self, Error, Reply},
    session::Context,
    team,
};
use crate::foundation::{
    AccessStore, Actor, DataRole, RecoveryRedemptionRequest, RedemptionRequest, RenewalRequest,
};
use hyper::Method;
use protocol::workspace as api;
use rusqlite::{Connection, OpenFlags};
use serde::de::DeserializeOwned;
use uuid::Uuid;

pub(super) fn dispatch(context: &Context, input: Input) -> Result<Reply, Error> {
    if input.query.is_some()
        && !matches!(
            input.path.as_str(),
            "/api/v1/workspaces" | "/api/v1/admin/workspaces"
        )
    {
        return Err(Error::BadRequest);
    }
    if input.path == "/api/v1/health" {
        if input.method != Method::GET {
            return Err(Error::Method);
        }
        return response::success(api::Health {
            instance_id: context.state.identity.trust().instance_id.clone(),
        });
    }
    let mut connection =
        Connection::open_with_flags(&context.state.database, OpenFlags::SQLITE_OPEN_READ_WRITE)
            .map_err(|_database_error| Error::Unavailable)?;
    connection
        .busy_timeout(std::time::Duration::from_secs(2))
        .map_err(|_database_error| Error::Unavailable)?;
    connection
        .pragma_update(None, "synchronous", "FULL")
        .map_err(|_database_error| Error::Unavailable)?;
    let mut store =
        AccessStore::new(&mut connection).map_err(|_database_error| Error::Unavailable)?;
    let now = crate::instance_tls::now().map_err(|_clock_error| Error::Unavailable)?;
    match context.surface {
        Surface::Https => https(context.state.as_ref(), &mut store, input, now),
        Surface::Client => client(context, &mut store, input, now),
    }
}

fn https(
    state: &State,
    store: &mut AccessStore<'_>,
    input: Input,
    now: i64,
) -> Result<Reply, Error> {
    if matches!(input.path.as_str(), "/api/v1/enroll" | "/api/v1/recover") {
        return public_certificate(state, store, &input, now);
    }
    if input.path != "/api/v1/admin/workspaces"
        && !invitations::handles(&input.path)
        && !management::handles(&input.path)
        && !team::handles(&input.path)
    {
        return Err(Error::NotFound);
    }
    let actor = admin(store, input.authorization.as_deref(), now)?;
    if invitations::handles(&input.path) {
        return invitations::dispatch(store, actor, &input, now);
    }
    if management::handles(&input.path) {
        return management::dispatch(store, actor, &input, now);
    }
    if team::handles(&input.path) {
        return team::dispatch(store, actor, &input, now);
    }
    if input.method != Method::GET {
        return Err(Error::Method);
    }
    workspaces(store, actor, input.query.as_deref(), now)
}

fn public_certificate(
    state: &State,
    store: &mut AccessStore<'_>,
    input: &Input,
    now: i64,
) -> Result<Reply, Error> {
    if input.method != Method::POST {
        return Err(Error::Method);
    }
    if input.authorization.is_some() {
        return Err(Error::BadRequest);
    }
    let body: api::Enrollment = decode(&input.body)?;
    let request_id = id(&body.request_id)?;
    let issuer = state
        .identity
        .issuing_authority()
        .map_err(|_issuer_error| Error::Unavailable)?;
    if input.path == "/api/v1/recover" {
        let request = RecoveryRedemptionRequest {
            request_id,
            csr_pem: body.csr_pem,
        };
        let receipt = store
            .redeem_recovery_invitation(&body.invitation, &request, &issuer, now)
            .map_err(|error| response::operation(error, Error::Forbidden))?;
        return certificate(
            state,
            receipt.client_id,
            receipt.credential_id,
            receipt.certificate_pem,
            receipt.expires_at,
        );
    }
    let request = RedemptionRequest {
        request_id,
        csr_pem: body.csr_pem,
        existing_credential: None,
    };
    let receipt = store
        .redeem_invitation(&body.invitation, &request, &issuer, now)
        .map_err(|error| response::operation(error, Error::Forbidden))?;
    certificate(
        state,
        receipt.client_id,
        receipt.credential_id,
        receipt.certificate_pem,
        receipt.expires_at,
    )
}

fn client(
    context: &Context,
    store: &mut AccessStore<'_>,
    input: Input,
    now: i64,
) -> Result<Reply, Error> {
    if input.authorization.is_some() {
        return Err(Error::BadRequest);
    }
    if !matches!(
        input.path.as_str(),
        "/api/v1/workspaces" | "/api/v1/admissions" | "/api/v1/credentials/renew"
    ) {
        return Err(Error::NotFound);
    }
    let peer = context
        .peer
        .as_ref()
        .ok_or(Error::Forbidden)?
        .verify(&context.state.identity, now)
        .map_err(|_peer_error| Error::Forbidden)?;
    if input.path == "/api/v1/credentials/renew" {
        if input.method != Method::POST {
            return Err(Error::Method);
        }
        let body: api::Renewal = decode(&input.body)?;
        let request = RenewalRequest {
            request_id: id(&body.request_id)?,
            csr_pem: body.csr_pem,
        };
        let issuer = context
            .state
            .identity
            .issuing_authority()
            .map_err(|_issuer_error| Error::Unavailable)?;
        let receipt = store
            .renew_verified_peer(&peer, &request, &issuer, now)
            .map_err(|error| response::operation(error, Error::Forbidden))?;
        return certificate(
            &context.state,
            receipt.client_id,
            receipt.credential_id,
            receipt.certificate_pem,
            receipt.expires_at,
        );
    }
    let actor = store
        .authenticate_peer(&peer, now)
        .map_err(|error| response::operation(error, Error::Forbidden))?;
    match (input.method, input.path.as_str()) {
        (Method::GET, "/api/v1/workspaces") => {
            workspaces(store, actor, input.query.as_deref(), now)
        }
        (Method::POST, "/api/v1/admissions") => {
            let body: api::Enrollment = decode(&input.body)?;
            let request = RedemptionRequest {
                request_id: id(&body.request_id)?,
                csr_pem: body.csr_pem,
                existing_credential: Some(peer.credential()),
            };
            let issuer = context
                .state
                .identity
                .issuing_authority()
                .map_err(|_issuer_error| Error::Unavailable)?;
            let receipt = store
                .admit_with_invitation(actor, &body.invitation, &request, &issuer, now)
                .map_err(|error| response::operation(error, Error::Forbidden))?;
            certificate(
                &context.state,
                receipt.client_id,
                receipt.credential_id,
                receipt.certificate_pem,
                receipt.expires_at,
            )
        }
        _ => Err(Error::Method),
    }
}

fn admin(store: &mut AccessStore<'_>, header: Option<&str>, now: i64) -> Result<Actor, Error> {
    let (scheme, secret) = header
        .and_then(|header| header.split_once(' '))
        .ok_or(Error::Unauthorized)?;
    if !scheme.eq_ignore_ascii_case("Bearer") || secret.trim() != secret {
        return Err(Error::Unauthorized);
    }
    store
        .authenticate_admin_key(secret, now)
        .map_err(|error| response::operation(error, Error::Unauthorized))
}

fn workspaces(
    store: &mut AccessStore<'_>,
    actor: Actor,
    query: Option<&str>,
    now: i64,
) -> Result<Reply, Error> {
    let (after, limit) = page_query(query)?;
    let visible = store
        .visible_workspaces_page(actor, after.as_deref(), limit, now)
        .map_err(|error| response::operation(error, Error::Forbidden))?;
    response::success(api::Workspaces {
        workspaces: visible
            .workspaces
            .into_iter()
            .map(|w| api::Workspace {
                id: w.id.to_string(),
                name: w.name,
                archived: w.archived,
            })
            .collect(),
        next_cursor: visible.next_cursor,
    })
}

fn page_query(query: Option<&str>) -> Result<(Option<String>, usize), Error> {
    let mut after = None;
    let mut limit = None;
    for (name, value) in url::form_urlencoded::parse(query.unwrap_or_default().as_bytes()) {
        match name.as_ref() {
            "after" if after.is_none() => {
                if value.len() > 1024 {
                    return Err(Error::BadRequest);
                }
                let (_, id): (String, String) =
                    serde_json::from_str(&value).map_err(|_invalid_cursor| Error::BadRequest)?;
                Uuid::parse_str(&id).map_err(|_invalid_id| Error::BadRequest)?;
                after = Some(value.into_owned());
            }
            "limit" if limit.is_none() => {
                let value = value
                    .parse::<usize>()
                    .map_err(|_invalid_limit| Error::BadRequest)?;
                if value == 0 || value > 500 {
                    return Err(Error::BadRequest);
                }
                limit = Some(value);
            }
            _ => return Err(Error::BadRequest),
        }
    }
    Ok((after, limit.unwrap_or(100)))
}

fn certificate(
    state: &State,
    client: Uuid,
    credential: Uuid,
    pem: String,
    expires_at: i64,
) -> Result<Reply, Error> {
    response::success(api::CertificateReceipt {
        instance_id: state.identity.trust().instance_id.clone(),
        client_id: client.to_string(),
        credential_id: credential.to_string(),
        certificate_pem: pem,
        expires_at,
    })
}
pub(super) fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, Error> {
    serde_json::from_slice(bytes).map_err(|_invalid_body| Error::BadRequest)
}
pub(super) fn id(value: &str) -> Result<Uuid, Error> {
    Uuid::parse_str(value).map_err(|_invalid_id| Error::BadRequest)
}
pub(super) fn role(role: api::Role) -> DataRole {
    match role {
        api::Role::Read => DataRole::Read,
        api::Role::ReadWrite => DataRole::ReadWrite,
    }
}
