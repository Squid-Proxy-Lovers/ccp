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
    let (mime, bytes) = asset(&input.path)?;
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
            "default-src 'none'; script-src 'self'; style-src 'self'; font-src 'self'; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'; object-src 'none'",
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

/// The Svelte build in `crates/server/web/dist`, committed so the server builds
/// without Node. File names are fixed by `vite.config.ts`.
fn asset(path: &str) -> Result<(&'static str, &'static [u8]), Error> {
    const JS: &str = "text/javascript; charset=utf-8";
    const FONT: &str = "font/woff2";
    let value: (&'static str, &'static [u8]) = match path {
        "/admin" | "/admin/" => (
            "text/html; charset=utf-8",
            include_bytes!("../../web/dist/index.html"),
        ),
        "/admin/app.css" => (
            "text/css; charset=utf-8",
            include_bytes!("../../web/dist/app.css"),
        ),
        "/admin/app.js" => (JS, include_bytes!("../../web/dist/app.js")),
        "/admin/theme.js" => (JS, include_bytes!("../../web/dist/theme.js")),
        "/admin/fonts/geist-latin-wght-normal.woff2" => (
            FONT,
            include_bytes!("../../web/dist/fonts/geist-latin-wght-normal.woff2"),
        ),
        "/admin/fonts/geist-latin-ext-wght-normal.woff2" => (
            FONT,
            include_bytes!("../../web/dist/fonts/geist-latin-ext-wght-normal.woff2"),
        ),
        "/admin/fonts/geist-mono-latin-wght-normal.woff2" => (
            FONT,
            include_bytes!("../../web/dist/fonts/geist-mono-latin-wght-normal.woff2"),
        ),
        _ => return Err(Error::NotFound),
    };
    Ok(value)
}
