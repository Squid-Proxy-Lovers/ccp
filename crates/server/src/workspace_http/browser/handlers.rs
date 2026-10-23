// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use crate::foundation::{AdminKeyIdentity, AdminKeyScope};
use protocol::workspace::browser as api;
use uuid::Uuid;

pub(super) fn identity(
    store: &mut AccessStore<'_>,
    key: Uuid,
    now: i64,
) -> Result<AdminKeyIdentity, Error> {
    store
        .browser_admin_key(key, now)
        .map_err(|error| response::operation(error, Error::Unauthorized))?
        .ok_or(Error::Unauthorized)
}
pub(super) fn login(
    state: &State,
    store: &mut AccessStore<'_>,
    input: &Input,
    now: i64,
) -> Result<Reply, Error> {
    if !input.origin_present || input.csrf.as_deref() != Some("login") {
        return Err(Error::Forbidden);
    }
    let request: api::Login = dispatch::decode(&input.body)?;
    let actor = store
        .authenticate_admin_key(&request.key, now)
        .map_err(|error| response::operation(error, Error::Unauthorized))?;
    let Actor::AdminKey(key) = actor else {
        return Err(Error::Unauthorized);
    };
    let metadata = identity(store, key, now)?;
    let workspace = match metadata.scope {
        AdminKeyScope::Instance => None,
        AdminKeyScope::Workspace(id) => Some(id),
    };
    let previous = cookie::read(input.cookie.as_deref(), &cookie::name(state))?;
    let issued =
        state
            .browser_sessions
            .issue(key, workspace, &input.target_origin, previous, now)?;
    let mut reply = response::success(view(state, issued.session, metadata))?;
    cookie::set(&mut reply, state, &issued.token, sessions::MAX_AGE)?;
    Ok(reply)
}
pub(super) fn status(
    state: &State,
    store: &mut AccessStore<'_>,
    input: &Input,
    now: i64,
) -> Result<Reply, Error> {
    let session = resolve(state, input)?;
    let metadata = identity(store, session.key_id, now)?;
    response::success(view(state, session, metadata))
}
pub(super) fn logout(state: &State, input: &Input) -> Result<Reply, Error> {
    let _: api::Logout = dispatch::decode(&input.body)?;
    resolve(state, input)?;
    let token =
        cookie::read(input.cookie.as_deref(), &cookie::name(state))?.ok_or(Error::Unauthorized)?;
    state.browser_sessions.remove(token)?;
    let mut reply = response::success(api::LoggedOut { signed_out: true })?;
    cookie::set(&mut reply, state, "", 0)?;
    Ok(reply)
}
fn view(state: &State, session: sessions::Resolved, identity: AdminKeyIdentity) -> api::Session {
    let scope = match identity.scope {
        AdminKeyScope::Instance => api::Scope::Instance {},
        AdminKeyScope::Workspace(id) => api::Scope::Workspace {
            workspace_id: id.to_string(),
            role: if identity.is_owner {
                api::WorkspaceRole::Owner
            } else {
                api::WorkspaceRole::Admin
            },
        },
    };
    api::Session {
        instance_id: state.identity.trust().instance_id.clone(),
        expires_at: session.expires_at,
        csrf_token: session.csrf,
        identity: api::Identity {
            key_id: identity.key_id.to_string(),
            principal_id: identity.principal_id.to_string(),
            principal_name: identity.principal_name,
            key_name: identity.key_name,
            scope,
        },
    }
}
