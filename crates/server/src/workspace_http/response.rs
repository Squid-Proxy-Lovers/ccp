// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use http_body_util::Full;
use hyper::{
    Response, StatusCode,
    body::Bytes,
    header::{
        CACHE_CONTROL, CONNECTION, CONTENT_TYPE, HeaderValue, WWW_AUTHENTICATE,
        X_CONTENT_TYPE_OPTIONS,
    },
};
use protocol::workspace::{API_VERSION, ApiError as WireError, Failure, Success};
use serde::Serialize;

pub(super) type Reply = Response<Full<Bytes>>;
#[derive(Clone, Copy, Debug)]
pub(super) enum Error {
    BadRequest,
    Unauthorized,
    Forbidden,
    NotFound,
    Conflict,
    OperationCancelled,
    LegacyContent,
    Method,
    TooLarge,
    Media,
    Unavailable,
    Timeout,
    Encoding,
}

impl Error {
    pub fn reply(self) -> Reply {
        let (status, code) = match self {
            Self::BadRequest => (StatusCode::BAD_REQUEST, "invalid_request"),
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
            Self::Forbidden => (StatusCode::FORBIDDEN, "forbidden"),
            Self::NotFound => (StatusCode::NOT_FOUND, "not_found"),
            Self::Conflict => (StatusCode::CONFLICT, "content_conflict"),
            Self::OperationCancelled => (StatusCode::CONFLICT, "operation_cancelled"),
            Self::LegacyContent => (StatusCode::CONFLICT, "content_conversion_required"),
            Self::Method => (StatusCode::METHOD_NOT_ALLOWED, "method_not_allowed"),
            Self::TooLarge => (StatusCode::PAYLOAD_TOO_LARGE, "request_too_large"),
            Self::Media => (StatusCode::UNSUPPORTED_MEDIA_TYPE, "unsupported_media_type"),
            Self::Unavailable => (StatusCode::SERVICE_UNAVAILABLE, "service_unavailable"),
            Self::Encoding => (StatusCode::NOT_IMPLEMENTED, "unsupported_transfer_encoding"),
            Self::Timeout => (StatusCode::REQUEST_TIMEOUT, "request_timeout"),
        };
        let bytes = serde_json::to_vec(&Failure {
            api_version: API_VERSION,
            error: WireError { code: code.into() },
        })
        .unwrap_or_else(|_serialization_error| {
            b"{\"api_version\":1,\"error\":{\"code\":\"service_unavailable\"}}".to_vec()
        });
        let mut reply = build(status, bytes);
        if status == StatusCode::UNAUTHORIZED {
            reply.headers_mut().insert(
                WWW_AUTHENTICATE,
                HeaderValue::from_static("Bearer realm=\"ccp\""),
            );
        }
        reply
    }
}

pub(super) fn success(value: impl Serialize) -> Result<Reply, Error> {
    limited_success(value, 1024 * 1024)
}

pub(super) fn content_snapshot(value: impl Serialize) -> Result<Reply, Error> {
    // Full snapshot plus retained append delta can each escape to 6x the
    // 256 KiB text bound. Other routes retain the original 1 MiB ceiling.
    limited_success(value, 4 * 1024 * 1024)
}

fn limited_success(value: impl Serialize, max_bytes: usize) -> Result<Reply, Error> {
    let bytes = serde_json::to_vec(&Success {
        api_version: API_VERSION,
        data: value,
    })
    .map_err(|_serialization_error| Error::Unavailable)?;
    if bytes.len() > max_bytes {
        return Err(Error::Unavailable);
    }
    Ok(build(StatusCode::OK, bytes))
}

fn build(status: StatusCode, bytes: Vec<u8>) -> Reply {
    let mut reply = Response::new(Full::new(Bytes::from(bytes)));
    *reply.status_mut() = status;
    reply
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    reply
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    reply
        .headers_mut()
        .insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    reply
}

pub(super) fn close(mut reply: Reply) -> Reply {
    reply
        .headers_mut()
        .insert(CONNECTION, HeaderValue::from_static("close"));
    reply
}

pub(super) fn operation(error: anyhow::Error, fallback: Error) -> Error {
    if error.chain().any(|cause| cause.is::<rusqlite::Error>()) {
        Error::Unavailable
    } else {
        fallback
    }
}
