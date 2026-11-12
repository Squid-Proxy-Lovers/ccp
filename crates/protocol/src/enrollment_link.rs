// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Versioned, secret-bearing enrollment links. Parsing performs no network I/O.
//! Display names are hints, never authority. Discovery must pin the CA and instance
//! before transmitting the invitation or creating an enrollment request.
mod fields;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
pub use fields::{LinkError, LinkTarget};
use std::collections::BTreeMap;

pub const LINK_VERSION: u32 = 1;
pub const MAX_LINK_BYTES: usize = 8192;

/// No Debug, Display, Clone or Serialize: formatting must explicitly opt in to
/// including the bearer secret. Never attach the source URI to parse errors.
pub struct EnrollmentLink {
    target: LinkTarget,
    invitation: String,
}
impl EnrollmentLink {
    pub fn new(target: LinkTarget, invitation: &str) -> Result<Self, LinkError> {
        let target = target.validated()?;
        if invitation.len() != 73
            || !invitation.starts_with("ccp_join_")
            || !invitation
                .bytes()
                .skip(9)
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(LinkError::Invitation);
        }
        Ok(Self {
            target,
            invitation: invitation.to_owned(),
        })
    }
    pub fn target(&self) -> &LinkTarget {
        &self.target
    }
    pub fn invitation(&self) -> &str {
        &self.invitation
    }

    pub fn parse(value: &str) -> Result<Self, LinkError> {
        if value.len() > MAX_LINK_BYTES {
            return Err(LinkError::Size);
        }
        if value.contains('\\')
            || value
                .chars()
                .any(|c| c.is_control() || c.is_ascii_whitespace())
        {
            return Err(LinkError::Syntax);
        }
        let (base, fragment) = value.split_once('#').ok_or(LinkError::Fields)?;
        if !base.starts_with("https://") || fragment.contains('#') {
            return Err(LinkError::Origin);
        }
        let (authority, path) = base.rsplit_once('/').ok_or(LinkError::Origin)?;
        if path != "join" {
            return Err(LinkError::Origin);
        }
        let https = fields::origin(authority)?;
        let fields = decode_fields(fragment)?;
        if field(&fields, "v")? != "1" {
            return Err(LinkError::Version);
        }
        let target = LinkTarget {
            instance_id: fields::id(field(&fields, "i")?)?,
            workspace_id: fields::id(field(&fields, "w")?)?,
            workspace_name: field(&fields, "n")?.to_owned(),
            mtls_origin: fields::mtls_origin(&https, field(&fields, "m")?)?,
            https_origin: https,
            ca_sha256: fields::pin(field(&fields, "ca")?)?,
        };
        Self::new(target, field(&fields, "t")?)
    }

    /// This output contains the invitation secret. It belongs only in a private
    /// one-time issuance result or an explicit user-requested copy action.
    pub fn to_uri(&self) -> String {
        let t = &self.target;
        let mut query = url::form_urlencoded::Serializer::new(String::new());
        query
            .append_pair("v", "1")
            .append_pair("i", &t.instance_id.to_string())
            .append_pair("m", &t.mtls_origin)
            .append_pair(
                "ca",
                &format!("sha256:{}", URL_SAFE_NO_PAD.encode(t.ca_sha256)),
            )
            .append_pair("t", &self.invitation)
            .append_pair("w", &t.workspace_id.to_string())
            .append_pair("n", &t.workspace_name);
        format!("{}join#{}", t.https_origin, query.finish())
    }
}
fn field<'a>(fields: &'a BTreeMap<String, String>, key: &str) -> Result<&'a str, LinkError> {
    fields.get(key).map(String::as_str).ok_or(LinkError::Fields)
}
fn decode_fields(fragment: &str) -> Result<BTreeMap<String, String>, LinkError> {
    let mut fields = BTreeMap::new();
    for pair in fragment.split('&') {
        let (key, value) = pair.split_once('=').ok_or(LinkError::Fields)?;
        let key = fields::decode(key)?;
        if !matches!(key.as_str(), "v" | "i" | "m" | "ca" | "t" | "w" | "n")
            || fields.insert(key, fields::decode(value)?).is_some()
        {
            return Err(LinkError::Fields);
        }
    }
    if fields.len() != 7 {
        return Err(LinkError::Fields);
    }
    Ok(fields)
}
#[cfg(test)]
mod tests;
