// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use url::{Host, Url};
use uuid::Uuid;

/// Public routing/pinning metadata. The workspace name is an untrusted display
/// hint, not a path, permission or verified server description.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkTarget {
    pub https_origin: String,
    pub mtls_origin: String,
    pub instance_id: Uuid,
    pub ca_sha256: [u8; 32],
    pub workspace_id: Uuid,
    pub workspace_name: String,
}
impl LinkTarget {
    /// Normalize routing hints only; this does not establish trust or permission.
    pub fn validated(mut self) -> Result<Self, LinkError> {
        self.https_origin = origin(&self.https_origin)?;
        self.mtls_origin = origin(&self.mtls_origin)?;
        id(&self.instance_id.to_string())?;
        id(&self.workspace_id.to_string())?;
        let name = &self.workspace_name;
        if name.is_empty()
            || name.len() > 256
            || name.trim() != name
            || name.chars().any(char::is_control)
        {
            return Err(LinkError::WorkspaceName);
        }
        Ok(self)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkError {
    Size,
    Syntax,
    Version,
    Origin,
    Fields,
    Identity,
    Pin,
    Invitation,
    WorkspaceName,
}
impl std::fmt::Display for LinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Size => "enrollment link exceeds the size limit",
            Self::Syntax => "enrollment link contains invalid encoding or characters",
            Self::Version => "unsupported enrollment link version",
            Self::Origin => {
                "enrollment link requires valid HTTPS origins without credentials or queries"
            }
            Self::Fields => "enrollment link has missing, repeated or unknown fields",
            Self::Identity => "enrollment link has an invalid instance or workspace UUID",
            Self::Pin => "enrollment link requires a SHA-256 CA certificate pin",
            Self::Invitation => "enrollment link has an invalid invitation format",
            Self::WorkspaceName => "enrollment link has an invalid workspace display name",
        })
    }
}
impl std::error::Error for LinkError {}
pub(super) fn id(value: &str) -> Result<Uuid, LinkError> {
    let id = Uuid::parse_str(value).map_err(|_id| LinkError::Identity)?;
    if id.is_nil() || id.to_string() != value {
        return Err(LinkError::Identity);
    }
    Ok(id)
}
pub(super) fn pin(value: &str) -> Result<[u8; 32], LinkError> {
    let value = value.strip_prefix("sha256:").ok_or(LinkError::Pin)?;
    let raw = URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_base64| LinkError::Pin)?;
    if URL_SAFE_NO_PAD.encode(&raw) != value {
        return Err(LinkError::Pin);
    }
    raw.try_into().map_err(|_length| LinkError::Pin)
}
pub(super) fn origin(value: &str) -> Result<String, LinkError> {
    if value.len() > 2048
        || !value.is_ascii()
        || !value.starts_with("https://")
        || value.contains(['@', '\\', '%', '?', '#'])
        || value
            .chars()
            .any(|c| c.is_control() || c.is_ascii_whitespace())
    {
        return Err(LinkError::Origin);
    }
    let raw = value.strip_prefix("https://").ok_or(LinkError::Origin)?;
    if raw.is_empty()
        || raw.starts_with('/')
        || raw
            .split_once('/')
            .is_some_and(|(_, path)| !path.is_empty())
    {
        return Err(LinkError::Origin);
    }
    let authority = raw.strip_suffix('/').unwrap_or(raw);
    let host = raw_host(authority)?;
    let url = Url::parse(value).map_err(|_url| LinkError::Origin)?;
    if host.to_ascii_lowercase() != url.host_str().ok_or(LinkError::Origin)? {
        return Err(LinkError::Origin);
    }
    if url.scheme() != "https"
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
        || url.port() == Some(0)
    {
        return Err(LinkError::Origin);
    }
    match url.host().ok_or(LinkError::Origin)? {
        Host::Domain(name) => {
            if name.len() > 253
                || name.split('.').any(|label| {
                    label.is_empty()
                        || label.len() > 63
                        || label.as_bytes().get(2..4) == Some(b"--")
                        || label.starts_with('-')
                        || label.ends_with('-')
                        || !label
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
                })
            {
                return Err(LinkError::Origin);
            }
        }
        Host::Ipv4(ip) => {
            let authority = raw.strip_suffix('/').unwrap_or(raw);
            let host = authority
                .split_once(':')
                .map_or(authority, |(host, _)| host);
            if ip.is_unspecified() || host.parse::<std::net::Ipv4Addr>() != Ok(ip) {
                return Err(LinkError::Origin);
            }
        }
        Host::Ipv6(ip) if ip.is_unspecified() || ip.to_ipv4_mapped().is_some() => {
            return Err(LinkError::Origin);
        }
        Host::Ipv6(_) => {}
    }
    Ok(url.to_string())
}
/// Raw authorities may differ only in ASCII letter case and explicit port443.
/// Reject empty/zero/leading-zero ports before URL libraries normalize them.
fn raw_host(authority: &str) -> Result<&str, LinkError> {
    let (host, port) = if authority.starts_with('[') {
        let end = authority.find(']').ok_or(LinkError::Origin)?;
        let host = authority.get(..=end).ok_or(LinkError::Origin)?;
        let suffix = authority.get(end + 1..).ok_or(LinkError::Origin)?;
        let port = if suffix.is_empty() {
            None
        } else {
            Some(suffix.strip_prefix(':').ok_or(LinkError::Origin)?)
        };
        (host, port)
    } else {
        authority
            .split_once(':')
            .map_or((authority, None), |(host, port)| (host, Some(port)))
    };
    if let Some(value) = port {
        let parsed = value.parse::<u16>().map_err(|_port| LinkError::Origin)?;
        if parsed == 0 || parsed.to_string() != value {
            return Err(LinkError::Origin);
        }
    }
    Ok(host)
}
pub(super) fn mtls_origin(https: &str, value: &str) -> Result<String, LinkError> {
    if value.starts_with("https://") {
        return origin(value);
    }
    let port = value.parse::<u16>().map_err(|_port| LinkError::Origin)?;
    if port == 0 || port.to_string() != value {
        return Err(LinkError::Origin);
    }
    let mut url = Url::parse(https).map_err(|_url| LinkError::Origin)?;
    url.set_port(Some(port)).map_err(|()| LinkError::Origin)?;
    origin(url.as_str())
}
/// Form decoding with strict percent and UTF-8 validation. URL form decoders
/// normally replace malformed bytes, which is inappropriate for a pinned link.
pub(super) fn decode(value: &str) -> Result<String, LinkError> {
    let mut bytes = value.bytes();
    let mut decoded = Vec::with_capacity(value.len());
    while let Some(byte) = bytes.next() {
        decoded.push(match byte {
            b'+' => b' ',
            b'%' => {
                let high = bytes.next().and_then(hex).ok_or(LinkError::Syntax)?;
                let low = bytes.next().and_then(hex).ok_or(LinkError::Syntax)?;
                high * 16 + low
            }
            other => other,
        });
    }
    String::from_utf8(decoded).map_err(|_utf8| LinkError::Syntax)
}
fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}
