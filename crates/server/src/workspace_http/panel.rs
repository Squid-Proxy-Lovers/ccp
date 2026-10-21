// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Public, bundled admin shell; all data still needs current session authority.
use super::{
    Surface,
    request::Input,
    response::{Error, Reply},
};
use http_body_util::Full;
use hyper::{Method, Response, body::Bytes, header::HeaderValue};

pub(super) fn handles(path: &str) -> bool {
    path == "/admin" || path.starts_with("/admin/")
}

pub(super) fn serve(surface: Surface, input: &Input) -> Result<Reply, Error> {
    if !matches!(surface, Surface::Https) {
        return Err(Error::NotFound);
    }
    if input.query.is_some() {
        // Never accept query-string keys. Fragments stay in the browser.
        return Err(Error::BadRequest);
    }
    if input.method != Method::GET {
        return Err(Error::Method);
    }
    let (mime, bytes): (&str, &'static [u8]) = match input.path.as_str() {
        "/admin" | "/admin/" => (
            "text/html; charset=utf-8",
            include_bytes!("panel/index.html"),
        ),
        "/admin/panel.css" => ("text/css; charset=utf-8", include_bytes!("panel/panel.css")),
        "/admin/app.js" => (
            "text/javascript; charset=utf-8",
            include_bytes!("panel/app.js"),
        ),
        "/admin/model.js" => (
            "text/javascript; charset=utf-8",
            include_bytes!("panel/model.js"),
        ),
        "/admin/view.js" => (
            "text/javascript; charset=utf-8",
            include_bytes!("panel/view.js"),
        ),
        "/admin/context.js" => (
            "text/javascript; charset=utf-8",
            include_bytes!("panel/context.js"),
        ),
        "/admin/access.js" => (
            "text/javascript; charset=utf-8",
            include_bytes!("panel/access.js"),
        ),
        "/admin/guidance.js" => (
            "text/javascript; charset=utf-8",
            include_bytes!("panel/guidance.js"),
        ),
        "/admin/settings.js" => (
            "text/javascript; charset=utf-8",
            include_bytes!("panel/settings.js"),
        ),
        "/admin/team.js" => (
            "text/javascript; charset=utf-8",
            include_bytes!("panel/team.js"),
        ),
        "/admin/transfer.js" => (
            "text/javascript; charset=utf-8",
            include_bytes!("panel/transfer.js"),
        ),
        "/admin/invitations.js" => (
            "text/javascript; charset=utf-8",
            include_bytes!("panel/invitations.js"),
        ),
        "/admin/enrollment.js" => (
            "text/javascript; charset=utf-8",
            include_bytes!("panel/enrollment.js"),
        ),
        "/admin/client_access.js" => (
            "text/javascript; charset=utf-8",
            include_bytes!("panel/client_access.js"),
        ),
        _ => return Err(Error::NotFound),
    };
    let mut reply = Response::new(Full::new(Bytes::from_static(bytes)));
    let headers = reply.headers_mut();
    headers.insert(
        "content-type",
        HeaderValue::from_str(mime).map_err(|_mime| Error::Unavailable)?,
    );
    for (name, value) in [
        ("cache-control", "no-store"),
        ("x-content-type-options", "nosniff"),
        ("referrer-policy", "no-referrer"),
        ("x-frame-options", "DENY"),
        (
            "content-security-policy",
            "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'; object-src 'none'",
        ),
        ("cross-origin-opener-policy", "same-origin"),
        (
            "permissions-policy",
            "camera=(), microphone=(), geolocation=()",
        ),
    ] {
        headers.insert(name, HeaderValue::from_static(value));
    }
    Ok(reply)
}
