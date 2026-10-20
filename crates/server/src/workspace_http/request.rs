// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::response::Error;
use http_body_util::{BodyExt, Limited};
use hyper::{
    Method, Request, Version,
    body::{Body, Incoming},
    header::{AUTHORIZATION, CONTENT_ENCODING, CONTENT_TYPE, HOST, ORIGIN, TRANSFER_ENCODING},
};
use std::time::Duration;
use tokio::time::timeout;

pub(super) struct Input {
    pub method: Method,
    pub path: String,
    pub query: Option<String>,
    pub authorization: Option<String>,
    pub body: Vec<u8>,
}
const MAX_BODY: usize = 64 * 1024;

pub(super) async fn read(request: Request<Incoming>) -> Result<Input, Error> {
    let (parts, body) = request.into_parts();
    if parts.version != Version::HTTP_11
        || parts.uri.scheme().is_some()
        || parts.uri.authority().is_some()
    {
        return Err(Error::BadRequest);
    }
    validate_headers(&parts.headers)?;
    if body.size_hint().lower()
        > u64::try_from(MAX_BODY).map_err(|_invalid_limit| Error::Unavailable)?
    {
        return Err(Error::TooLarge);
    }
    let authorization = single(&parts.headers, AUTHORIZATION)?.map(str::to_string);
    if parts.method == Method::POST {
        let content_type = single(&parts.headers, CONTENT_TYPE)?.ok_or(Error::Media)?;
        if !content_type
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .eq_ignore_ascii_case("application/json")
        {
            return Err(Error::Media);
        }
    }
    let collected = timeout(
        Duration::from_secs(10),
        Limited::new(body, MAX_BODY).collect(),
    )
    .await
    .map_err(|_timeout| Error::Timeout)?
    .map_err(|error| {
        if error.is::<http_body_util::LengthLimitError>() {
            Error::TooLarge
        } else {
            Error::BadRequest
        }
    })?;
    if collected.trailers().is_some() {
        return Err(Error::BadRequest);
    }
    let bytes = collected.to_bytes();
    if parts.method == Method::GET && !bytes.is_empty() {
        return Err(Error::BadRequest);
    }
    Ok(Input {
        method: parts.method,
        path: parts.uri.path().to_string(),
        query: parts.uri.query().map(str::to_string),
        authorization,
        body: bytes.to_vec(),
    })
}

fn validate_headers(headers: &hyper::HeaderMap) -> Result<(), Error> {
    let host = single(headers, HOST)?.ok_or(Error::BadRequest)?;
    let target =
        url::Url::parse(&format!("https://{host}")).map_err(|_invalid_host| Error::BadRequest)?;
    if !host.is_ascii()
        || host.contains('@')
        || target.host_str().is_none()
        || target.path() != "/"
        || target.query().is_some()
        || target.fragment().is_some()
    {
        return Err(Error::BadRequest);
    }
    if let Some(origin) = single(headers, ORIGIN)? {
        let origin = url::Url::parse(origin).map_err(|_invalid_origin| Error::Forbidden)?;
        if origin.scheme() != "https"
            || origin.origin() != target.origin()
            || origin.path() != "/"
            || origin.query().is_some()
            || origin.fragment().is_some()
            || !origin.username().is_empty()
            || origin.password().is_some()
        {
            return Err(Error::Forbidden);
        }
    }
    if let Some(encoding) = single(headers, TRANSFER_ENCODING)?
        && !encoding.eq_ignore_ascii_case("chunked")
    {
        return Err(Error::Encoding);
    }
    if let Some(encoding) = single(headers, CONTENT_ENCODING)?
        && !encoding.eq_ignore_ascii_case("identity")
    {
        return Err(Error::Media);
    }
    Ok(())
}

fn single(
    headers: &hyper::HeaderMap,
    name: hyper::header::HeaderName,
) -> Result<Option<&str>, Error> {
    let mut values = headers.get_all(name).iter();
    let value = values
        .next()
        .map(|value| value.to_str().map_err(|_invalid_header| Error::BadRequest))
        .transpose()?;
    if values.next().is_some() {
        return Err(Error::BadRequest);
    }
    Ok(value)
}
