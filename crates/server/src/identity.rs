// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use anyhow::{Context, bail};

pub const SESSION_URI_PREFIX: &str = "ccp://session/";
pub const ACCESS_URI_PREFIX: &str = "ccp://access/";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeerIdentity {
    pub common_name: String,
    pub session_id: i64,
    pub access_level: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectionAuthContext {
    pub common_name: String,
    pub session_id: i64,
    pub can_write: bool,
    pub can_revoke_others: bool,
}

impl TryFrom<&PeerIdentity> for ConnectionAuthContext {
    type Error = anyhow::Error;

    fn try_from(value: &PeerIdentity) -> Result<Self, Self::Error> {
        let (can_write, can_revoke_others) = match value.access_level.as_str() {
            "read" => (false, false),
            "read_write" => (true, false),
            "admin" => (true, true),
            other => bail!("unsupported access level '{other}'"),
        };

        Ok(Self {
            common_name: value.common_name.clone(),
            session_id: value.session_id,
            can_write,
            can_revoke_others,
        })
    }
}

pub fn session_identity_uri(session_id: i64) -> String {
    format!("{SESSION_URI_PREFIX}{session_id}")
}

pub fn parse_session_identity_uri(uri: &str) -> anyhow::Result<i64> {
    let session_id = uri
        .strip_prefix(SESSION_URI_PREFIX)
        .with_context(|| format!("unsupported session identity URI '{uri}'"))?;
    let session_id = session_id
        .parse::<i64>()
        .with_context(|| format!("invalid session id in session identity URI '{uri}'"))?;
    if session_id <= 0 {
        bail!("session identity URI must contain a positive session id");
    }
    Ok(session_id)
}

pub fn access_identity_uri(access_level: &str) -> String {
    format!("{ACCESS_URI_PREFIX}{access_level}")
}

pub fn parse_access_identity_uri(uri: &str) -> anyhow::Result<String> {
    let access_level = uri
        .strip_prefix(ACCESS_URI_PREFIX)
        .with_context(|| format!("unsupported access identity URI '{uri}'"))?;
    if access_level != "read" && access_level != "read_write" && access_level != "admin" {
        bail!("unsupported access level in access identity URI '{uri}'");
    }
    Ok(access_level.to_string())
}
