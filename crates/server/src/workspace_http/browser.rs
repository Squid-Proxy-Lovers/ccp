// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Ephemeral HTTPS browser authentication under existing current admin key scope.
use super::{
    State, dispatch,
    request::Input,
    response::{self, Error, Reply},
};
use crate::foundation::{AccessStore, Actor};
use hyper::Method;

mod cookie;
mod handlers;
mod sessions;
pub(super) use sessions::Sessions;

const LOGIN: &str = "/api/v1/admin/session/login";
const STATUS: &str = "/api/v1/admin/session";
const LOGOUT: &str = "/api/v1/admin/session/logout";

pub(super) fn handles(path: &str) -> bool {
    matches!(path, LOGIN | STATUS | LOGOUT)
}

pub(super) fn dispatch(
    state: &State,
    store: &mut AccessStore<'_>,
    input: &Input,
    now: i64,
) -> Result<Reply, Error> {
    if input.authorization.is_some() {
        return Err(Error::BadRequest);
    }
    match (&input.method, input.path.as_str()) {
        (&Method::POST, LOGIN) => handlers::login(state, store, input, now),
        (&Method::GET, STATUS) => handlers::status(state, store, input, now),
        (&Method::POST, LOGOUT) => handlers::logout(state, input),
        _ => Err(Error::Method),
    }
}

pub(super) fn authenticate(
    state: &State,
    store: &mut AccessStore<'_>,
    input: &Input,
    now: i64,
) -> Result<Actor, Error> {
    let token = cookie::read(input.cookie.as_deref(), &cookie::name(state))?;
    if input.authorization.is_some() {
        if token.is_some() {
            return Err(Error::BadRequest);
        }
        return dispatch::admin(store, input.authorization.as_deref(), now);
    }
    let session = resolve(state, input)?;
    let identity = handlers::identity(store, session.key_id, now)?;
    Ok(Actor::AdminKey(identity.key_id))
}

fn resolve(state: &State, input: &Input) -> Result<sessions::Resolved, Error> {
    let token =
        cookie::read(input.cookie.as_deref(), &cookie::name(state))?.ok_or(Error::Unauthorized)?;
    let proof = if input.method == Method::GET {
        None
    } else {
        if !input.origin_present {
            return Err(Error::Forbidden);
        }
        Some(input.csrf.as_deref().ok_or(Error::Forbidden)?)
    };
    state
        .browser_sessions
        .resolve(token, &input.target_origin, proof)
}
