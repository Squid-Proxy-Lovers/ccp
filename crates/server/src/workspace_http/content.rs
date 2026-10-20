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
use protocol::workspace::content as api;

mod entries;
mod nodes;
mod project;
mod purge;
mod shelves;

pub(super) const SHELF_TRANSITION_PATH: &str = "/api/v1/admin/content/shelves/transition";

pub(super) const ENTRY_PURGE_PATH: &str = "/api/v1/admin/content/entries/purge";

pub(super) const MUTATION_PATH: &str = "/api/v1/content/entries/mutate";

pub(super) fn handles(path: &str) -> bool {
    matches!(
        path,
        SHELF_TRANSITION_PATH
            | ENTRY_PURGE_PATH
            | "/api/v1/content/shelves/state"
            | "/api/v1/content/shelves/archived"
            | "/api/v1/content/shelves/create"
            | "/api/v1/content/shelves/get"
            | "/api/v1/content/shelves/list"
            | "/api/v1/content/books/create"
            | "/api/v1/content/books/get"
            | "/api/v1/content/books/list"
            | MUTATION_PATH
            | "/api/v1/content/entries/get"
            | "/api/v1/content/entries/list"
            | "/api/v1/content/entries/history"
            | "/api/v1/content/entries/resolve-operation"
    )
}

pub(super) fn client_handles(path: &str) -> bool {
    !matches!(path, SHELF_TRANSITION_PATH | ENTRY_PURGE_PATH) && handles(path)
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
        ENTRY_PURGE_PATH => purge::entry(store, actor, input, now),
        SHELF_TRANSITION_PATH => shelves::transition(store, actor, input, now),
        "/api/v1/content/shelves/state" => shelves::state(store, actor, input, now),
        "/api/v1/content/shelves/archived" => shelves::archived(store, actor, input, now),
        MUTATION_PATH => entries::mutate(store, actor, input, now),
        "/api/v1/content/entries/get" => entries::get(store, actor, input, now),
        "/api/v1/content/entries/list" => entries::list(store, actor, input, now),
        "/api/v1/content/entries/history" => entries::history(store, actor, input, now),
        "/api/v1/content/entries/resolve-operation" => entries::resolve(store, actor, input, now),
        _ => nodes::dispatch(store, actor, input, now),
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

fn limit(value: Option<u16>) -> Result<usize, Error> {
    // 32 worst-case 4096-byte descriptions with 6x JSON escaping, names and
    // bounded identity metadata fit the unchanged 1 MiB list response budget.
    let value = value.unwrap_or(32);
    if !(1..=32).contains(&value) {
        return Err(Error::BadRequest);
    }
    Ok(usize::from(value))
}
