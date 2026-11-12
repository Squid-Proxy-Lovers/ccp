// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use hyper::header::{HeaderValue, SET_COOKIE};

pub(super) fn name(state: &State) -> String {
    format!("__Host-ccp_admin_{}", state.identity.trust().instance_id)
}

pub(super) fn read<'a>(header: Option<&'a str>, name: &str) -> Result<Option<&'a str>, Error> {
    let mut token = None;
    for pair in header.unwrap_or_default().split(';') {
        if let Some((key, value)) = pair.trim().split_once('=')
            && key == name
        {
            if token.is_some() {
                return Err(Error::BadRequest);
            }
            token = Some(value);
        }
    }
    Ok(token)
}

pub(super) fn set(
    reply: &mut Reply,
    state: &State,
    token: &str,
    max_age: u64,
) -> Result<(), Error> {
    let value = format!(
        "{}={token}; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age={max_age}",
        name(state)
    );
    reply.headers_mut().insert(
        SET_COOKIE,
        HeaderValue::from_str(&value).map_err(|_invalid| Error::Unavailable)?,
    );
    Ok(())
}
