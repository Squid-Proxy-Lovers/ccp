// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::{
    Surface, request,
    response::{Error, Reply},
};
use ccp_transport::{CLIENT_PATH, ENROLL_PATH, SUBPROTOCOL, websocket::tungstenite};
use http_body_util::Full;
use hyper::{
    Method, Request, Version,
    body::{Body, Bytes, Incoming},
    header::{self, HeaderValue},
};
use std::sync::{Arc, Mutex};

pub(super) struct Pending {
    pub upgrade: hyper::upgrade::OnUpgrade,
    pub surface: Surface,
}
pub(super) type Slot = Arc<Mutex<Option<Pending>>>;

pub(super) fn handles(path: &str) -> bool {
    path.starts_with("/ccp/tunnel/")
}

/// Validate before installing an upgrade. No application credentials are used here.
pub(super) fn accept(mut input: Request<Incoming>, slot: &Slot) -> Result<Reply, Error> {
    let surface = validate(&input)?;
    let mut response = tungstenite::handshake::server::create_response_with_body(&input, || {
        Full::new(Bytes::new())
    })
    .map_err(|_handshake| Error::BadRequest)?;
    response.headers_mut().insert(
        "sec-websocket-protocol",
        HeaderValue::from_static(SUBPROTOCOL),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    let mut slot = slot.lock().map_err(|_poisoned| Error::Unavailable)?;
    if slot.is_some() {
        return Err(Error::BadRequest);
    }
    *slot = Some(Pending {
        upgrade: hyper::upgrade::on(&mut input),
        surface,
    });
    Ok(response)
}
fn validate(input: &Request<Incoming>) -> Result<Surface, Error> {
    let surface = match input.uri().path() {
        ENROLL_PATH => Surface::Bootstrap,
        CLIENT_PATH => Surface::Client,
        _ => return Err(Error::NotFound),
    };
    if input.method() != Method::GET
        || input.version() != Version::HTTP_11
        || input.uri().scheme().is_some()
        || input.uri().authority().is_some()
        || input.uri().query().is_some()
        || !input.body().is_end_stream()
    {
        return Err(Error::BadRequest);
    }
    request::validate_headers(input.headers())?;
    for name in [
        header::ORIGIN,
        header::SEC_WEBSOCKET_EXTENSIONS,
        header::TRANSFER_ENCODING,
        header::AUTHORIZATION,
        header::COOKIE,
    ] {
        if input.headers().contains_key(name) {
            return Err(Error::BadRequest);
        }
    }
    for name in [
        header::CONNECTION,
        header::UPGRADE,
        header::SEC_WEBSOCKET_VERSION,
        header::SEC_WEBSOCKET_KEY,
        header::SEC_WEBSOCKET_PROTOCOL,
    ] {
        request::single(input.headers(), name)?.ok_or(Error::BadRequest)?;
    }
    if request::single(input.headers(), header::SEC_WEBSOCKET_PROTOCOL)? != Some(SUBPROTOCOL)
        || request::single(input.headers(), header::CONTENT_LENGTH)?.is_some_and(|v| v != "0")
    {
        return Err(Error::BadRequest);
    }
    Ok(surface)
}

pub(super) fn bootstrap_allows(method: &Method, path: &str) -> bool {
    matches!(
        (method, path),
        (&Method::GET, "/api/v1/health" | "/api/v1/software-info")
            | (&Method::POST, "/api/v1/enroll" | "/api/v1/recover")
    )
}
