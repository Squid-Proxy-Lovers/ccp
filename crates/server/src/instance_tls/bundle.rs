// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, bail};
use rcgen::{
    BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, KeyPair,
    KeyUsagePurpose,
};
use rustls::{
    RootCertStore,
    client::{WebPkiServerVerifier, danger::ServerCertVerifier},
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use uuid::Uuid;
use x509_parser::extensions::GeneralName;

use super::{TrustInfo, provider};
use crate::foundation::{IssuingAuthority, strict_pem_der};

const CA_SECONDS: i64 = 10 * 365 * 24 * 60 * 60;
const SERVER_SECONDS: i64 = 365 * 24 * 60 * 60;

// Serialization is confined to the private disk format, never returned by the
// public API. No Debug/Clone: the bundle contains both private signing keys.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PrivateBundle {
    pub format: u32,
    pub instance_id: String,
    pub names: Vec<String>,
    pub ca_certificate: String,
    pub ca_key: String,
    pub server_certificate: String,
    pub server_key: String,
}

impl PrivateBundle {
    pub fn generate(instance: Uuid, names: Vec<String>, now: i64) -> anyhow::Result<Self> {
        let mut ca = CertificateParams::new(Vec::<String>::new())?;
        ca.distinguished_name
            .push(DnType::CommonName, format!("CCP instance {instance}"));
        ca.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
        ca.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        ca.not_before = OffsetDateTime::from_unix_timestamp(now.saturating_sub(60).max(0))?;
        ca.not_after = OffsetDateTime::from_unix_timestamp(
            now.checked_add(CA_SECONDS).context("CA expiry overflow")?,
        )?;
        let ca_key = KeyPair::generate()?;
        let ca = ca.self_signed(&ca_key)?;
        let mut server = CertificateParams::new(names.clone())?;
        server
            .distinguished_name
            .push(DnType::CommonName, format!("CCP server {instance}"));
        server.not_before = OffsetDateTime::from_unix_timestamp(now.saturating_sub(60).max(0))?;
        server.not_after = OffsetDateTime::from_unix_timestamp(
            now.checked_add(SERVER_SECONDS)
                .context("server expiry overflow")?,
        )?;
        server.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        server.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        server.use_authority_key_identifier_extension = true;
        let server_key = KeyPair::generate()?;
        let server = server.signed_by(&server_key, &ca, &ca_key)?;
        Ok(Self {
            format: 1,
            instance_id: instance.to_string(),
            names,
            ca_certificate: ca.pem(),
            ca_key: ca_key.serialize_pem(),
            server_certificate: server.pem(),
            server_key: server_key.serialize_pem(),
        })
    }

    pub fn renew_server(
        &self,
        instance: Uuid,
        names: Vec<String>,
        now: i64,
    ) -> anyhow::Result<Self> {
        let old = self.validate_for_renewal(instance, now)?;
        let ca_key = KeyPair::from_pem(&self.ca_key)?;
        let ca = CertificateParams::from_ca_cert_pem(&self.ca_certificate)?.self_signed(&ca_key)?;
        let mut params = CertificateParams::new(names.clone())?;
        params
            .distinguished_name
            .push(DnType::CommonName, format!("CCP server {instance}"));
        let der = self.ca_der()?;
        let (_, root) = x509_parser::parse_x509_certificate(&der)
            .map_err(|_invalid| anyhow::anyhow!("invalid CA certificate"))?;
        params.not_before = OffsetDateTime::from_unix_timestamp(
            now.saturating_sub(60)
                .max(0)
                .max(root.validity().not_before.timestamp()),
        )?;
        params.not_after = OffsetDateTime::from_unix_timestamp(
            now.checked_add(SERVER_SECONDS)
                .context("server expiry overflow")?
                .min(old.ca_expires_at),
        )?;
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        params.use_authority_key_identifier_extension = true;
        let key = KeyPair::generate()?;
        let leaf = params.signed_by(&key, &ca, &ca_key)?;
        Ok(Self {
            format: self.format,
            instance_id: self.instance_id.clone(),
            names,
            ca_certificate: self.ca_certificate.clone(),
            ca_key: self.ca_key.clone(),
            server_certificate: leaf.pem(),
            server_key: key.serialize_pem(),
        })
    }

    /// Validate an expired leaf at its last valid instant, while requiring the
    /// original CA to remain valid at the actual renewal time. No trust reset.
    fn validate_for_renewal(&self, instance: Uuid, now: i64) -> anyhow::Result<TrustInfo> {
        let der = self.ca_der()?;
        let (_, root) = x509_parser::parse_x509_certificate(&der)
            .map_err(|_invalid| anyhow::anyhow!("invalid CA certificate"))?;
        if now < 0
            || now < root.validity().not_before.timestamp()
            || now >= root.validity().not_after.timestamp()
        {
            bail!("current CA validity is required; renewal cannot rotate the trust root");
        }
        let der = strict_pem_der(&self.server_certificate, "CERTIFICATE")?;
        let (_, leaf) = x509_parser::parse_x509_certificate(&der)
            .map_err(|_invalid| anyhow::anyhow!("invalid server certificate"))?;
        if now < leaf.validity().not_before.timestamp() {
            bail!("server certificate is not valid yet");
        }
        self.validate(
            instance,
            now.min(leaf.validity().not_after.timestamp().saturating_sub(1)),
        )
    }

    pub fn ca_der(&self) -> anyhow::Result<CertificateDer<'static>> {
        Ok(CertificateDer::from(strict_pem_der(
            &self.ca_certificate,
            "CERTIFICATE",
        )?))
    }

    pub fn server_chain(&self) -> anyhow::Result<Vec<CertificateDer<'static>>> {
        Ok(vec![CertificateDer::from(strict_pem_der(
            &self.server_certificate,
            "CERTIFICATE",
        )?)])
    }

    pub fn server_private_key(&self) -> anyhow::Result<PrivateKeyDer<'static>> {
        Ok(PrivatePkcs8KeyDer::from(strict_pem_der(&self.server_key, "PRIVATE KEY")?).into())
    }

    pub fn validate(&self, instance: Uuid, now: i64) -> anyhow::Result<TrustInfo> {
        if self.format != 1 || self.instance_id != instance.to_string() || now < 0 {
            bail!("unsupported TLS bundle or wrong instance");
        }
        if normalize_names(&self.names)? != self.names {
            bail!("noncanonical TLS names");
        }
        // Verify the CA key/constraints as well as its actual time interval;
        // trust-anchor validation by itself doesn't check root expiry.
        IssuingAuthority::from_pem(&self.ca_certificate, &self.ca_key)?;
        strict_pem_der(&self.ca_key, "PRIVATE KEY")?;
        let ca_der = self.ca_der()?;
        let (remaining, ca) = x509_parser::parse_x509_certificate(&ca_der)
            .map_err(|_invalid_input| anyhow::anyhow!("invalid CA certificate"))?;
        if !remaining.is_empty()
            || now < ca.validity().not_before.timestamp()
            || now >= ca.validity().not_after.timestamp()
        {
            bail!("CA certificate is not currently valid");
        }
        let leaf_der =
            CertificateDer::from(strict_pem_der(&self.server_certificate, "CERTIFICATE")?);
        let (remaining, leaf) = x509_parser::parse_x509_certificate(&leaf_der)
            .map_err(|_invalid_input| anyhow::anyhow!("invalid server certificate"))?;
        if !remaining.is_empty() {
            bail!("trailing server certificate data");
        }
        validate_leaf(&leaf, &self.names)?;
        rustls::sign::CertifiedKey::from_der(
            vec![leaf_der.clone()],
            self.server_private_key()?,
            &provider(),
        )?
        .keys_match()?;
        let mut roots = RootCertStore::empty();
        roots.add(ca_der.clone())?;
        let verifier =
            WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider()).build()?;
        let timestamp = UnixTime::since_unix_epoch(Duration::from_secs(u64::try_from(now)?));
        for name in &self.names {
            verifier.verify_server_cert(
                &leaf_der,
                &[],
                &ServerName::try_from(name.clone())?,
                &[],
                timestamp,
            )?;
        }
        if leaf.validity().not_after > ca.validity().not_after {
            bail!("server certificate outlives CA");
        }
        Ok(TrustInfo {
            instance_id: instance.to_string(),
            ca_certificate_pem: self.ca_certificate.clone(),
            ca_sha256: hex(&Sha256::digest(ca_der.as_ref())),
            server_sha256: hex(&Sha256::digest(leaf_der.as_ref())),
            names: self.names.clone(),
            ca_expires_at: ca.validity().not_after.timestamp(),
            server_expires_at: leaf.validity().not_after.timestamp(),
        })
    }
}

fn validate_leaf(
    leaf: &x509_parser::certificate::X509Certificate<'_>,
    names: &[String],
) -> anyhow::Result<()> {
    let key_usage = leaf.key_usage()?.context("missing server key usage")?;
    let eku = leaf
        .extended_key_usage()?
        .context("missing server extended key usage")?;
    let basic = leaf.basic_constraints()?;
    if key_usage.value.flags != 1
        || !eku.value.server_auth
        || eku.value.client_auth
        || eku.value.any
        || !eku.value.other.is_empty()
        || eku.value.code_signing
        || eku.value.email_protection
        || eku.value.time_stamping
        || eku.value.ocsp_signing
        || basic.is_some_and(|value| value.value.ca)
    {
        bail!("server certificate has unexpected capabilities");
    }
    let sans = leaf
        .subject_alternative_name()?
        .context("missing server names")?;
    let mut actual = Vec::new();
    for san in &sans.value.general_names {
        match san {
            GeneralName::DNSName(name) => actual.push(name.to_string()),
            GeneralName::IPAddress(bytes) => {
                let ip = match bytes.len() {
                    4 => IpAddr::from(<[u8; 4]>::try_from(*bytes)?),
                    16 => IpAddr::from(<[u8; 16]>::try_from(*bytes)?),
                    _ => bail!("invalid server IP name"),
                };
                actual.push(ip.to_string());
            }
            _ => bail!("unexpected server subject alternative name"),
        }
    }
    actual.sort();
    if actual != names {
        bail!("server names differ from bundle");
    }
    Ok(())
}

pub(super) fn normalize_names(names: &[String]) -> anyhow::Result<Vec<String>> {
    if names.is_empty() || names.len() > 32 {
        bail!("supply 1..32 explicit server DNS/IP names");
    }
    let mut normalized = Vec::new();
    for name in names {
        if name.len() > 253 || name.is_empty() || !name.is_ascii() {
            bail!("invalid server DNS/IP name");
        }
        let name = if let Ok(ip) = name.parse::<IpAddr>() {
            ip.to_string()
        } else {
            if name.split('.').any(|label| {
                label.is_empty()
                    || label.len() > 63
                    || label.starts_with('-')
                    || label.ends_with('-')
                    || !label
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            }) {
                bail!("invalid server DNS name; URLs and wildcards are not accepted");
            }
            let lowered = name.to_ascii_lowercase();
            ServerName::try_from(lowered.clone()).context("invalid TLS server name")?;
            lowered
        };
        normalized.push(name);
    }
    normalized.sort();
    normalized.dedup();
    Ok(normalized)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
