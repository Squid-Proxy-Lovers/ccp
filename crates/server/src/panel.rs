// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The bundled admin panel. Every asset is public; all data still needs the
//! admin key on each `/v1/admin/*` request. `build.rs` embeds the Svelte build.
use axum::{
    extract::Path,
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};

const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; font-src 'self'; img-src 'self'; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'; object-src 'none'";

macro_rules! panel_asset {
    ($path:literal) => {
        include_bytes!(concat!(env!("OUT_DIR"), "/panel/", $path))
    };
}

fn asset(path: &str) -> Option<(&'static str, &'static [u8])> {
    const JS: &str = "text/javascript; charset=utf-8";
    const FONT: &str = "font/woff2";
    const TEXT: &str = "text/plain; charset=utf-8";
    Some(match path {
        "" | "index.html" => ("text/html; charset=utf-8", panel_asset!("index.html")),
        "app.css" => ("text/css; charset=utf-8", panel_asset!("app.css")),
        "app.js" => (JS, panel_asset!("app.js")),
        "theme.js" => (JS, panel_asset!("theme.js")),
        "favicon.svg" => ("image/svg+xml", panel_asset!("favicon.svg")),
        "fonts/FragmentMono-OFL.txt" => (TEXT, panel_asset!("fonts/FragmentMono-OFL.txt")),
        "fonts/OverusedGrotesk-OFL.txt" => (TEXT, panel_asset!("fonts/OverusedGrotesk-OFL.txt")),
        "fonts/OverusedGroteskRoman-VF.woff2" => {
            (FONT, panel_asset!("fonts/OverusedGroteskRoman-VF.woff2"))
        }
        "fonts/fragment-mono-latin-400-normal.woff2" => (
            FONT,
            panel_asset!("fonts/fragment-mono-latin-400-normal.woff2"),
        ),
        "fonts/fragment-mono-latin-ext-400-normal.woff2" => (
            FONT,
            panel_asset!("fonts/fragment-mono-latin-ext-400-normal.woff2"),
        ),
        _ => return None,
    })
}

fn reply(mime: &'static str, bytes: &'static [u8], html: bool) -> Response {
    let mut response = (StatusCode::OK, bytes).into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
    // The HTML shell must always be fresh; fixed-name assets revalidate each load.
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    if html {
        headers.insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(CSP),
        );
        headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
        headers.insert(
            "cross-origin-opener-policy",
            HeaderValue::from_static("same-origin"),
        );
        headers.insert(
            "permissions-policy",
            HeaderValue::from_static("camera=(), microphone=(), geolocation=()"),
        );
    }
    response
}

/// `GET /admin` and `GET /admin/`.
pub async fn index() -> Response {
    let (mime, bytes) = asset("").expect("panel index is embedded");
    reply(mime, bytes, true)
}

/// `GET /admin/{*path}`: a bundled asset.
pub async fn file(Path(path): Path<String>) -> Response {
    match asset(&path) {
        Some((mime, bytes)) => reply(mime, bytes, path.ends_with(".html")),
        None => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::asset;

    #[test]
    fn embeds_every_built_asset() {
        for path in [
            "",
            "app.css",
            "app.js",
            "theme.js",
            "favicon.svg",
            "fonts/OverusedGroteskRoman-VF.woff2",
            "fonts/fragment-mono-latin-400-normal.woff2",
            "fonts/fragment-mono-latin-ext-400-normal.woff2",
        ] {
            let (_, bytes) = asset(path).unwrap_or_else(|| panic!("missing {path}"));
            assert!(!bytes.is_empty(), "{path} is empty");
        }
        assert!(asset("../Cargo.toml").is_none());
        assert!(asset("app.js.map").is_none());
    }
}
