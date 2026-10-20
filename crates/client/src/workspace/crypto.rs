// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::{Credential, Pending, WorkspaceTrust};
use anyhow::{Context, bail};
use rcgen::{CertificateParams, CertificateSigningRequestParams, KeyPair};
use rustls::pki_types::{
    CertificateDer, CertificateSigningRequestDer, PrivatePkcs8KeyDer, UnixTime,
};
use rustls::{RootCertStore, server::WebPkiClientVerifier};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;
use x509_parser::prelude::{FromDer, GeneralName, X509Certificate};

pub(super) fn now() -> anyhow::Result<i64> {
    Ok(i64::try_from(
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
    )?)
}
pub(super) fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
pub(super) fn canonical_id(value: &str) -> anyhow::Result<Uuid> {
    let id = Uuid::parse_str(value).context("invalid profile identity UUID")?;
    if id.to_string() != value {
        bail!("noncanonical profile identity UUID");
    }
    Ok(id)
}
pub(super) fn invitation(value: &str) -> anyhow::Result<&str> {
    let value = value.trim_matches(|c: char| c.is_ascii_whitespace());
    let hex = value
        .strip_prefix("ccp_join_")
        .context("invalid enrollment invitation")?;
    if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("invalid enrollment invitation");
    }
    Ok(value)
}
pub(super) fn pem(value: &str, label: &str) -> anyhow::Result<Vec<u8>> {
    if value.len() > 16 * 1024 {
        bail!("PEM exceeds profile limit");
    }
    let value = value.trim_matches(|c: char| c.is_ascii_whitespace());
    let mut lines = value.lines();
    let header = format!("-----BEGIN {label}-----");
    let footer = format!("-----END {label}-----");
    if lines.next() != Some(header.as_str()) || lines.next_back() != Some(footer.as_str()) {
        bail!("expected exactly one PEM block");
    }
    let (remaining, decoded) = x509_parser::pem::parse_x509_pem(value.as_bytes())
        .map_err(|_pem| anyhow::anyhow!("invalid profile PEM"))?;
    if decoded.label != label || !remaining.iter().all(u8::is_ascii_whitespace) {
        bail!("unexpected additional PEM data");
    }
    Ok(decoded.contents)
}
fn cert(der: &[u8]) -> anyhow::Result<X509Certificate<'_>> {
    let (remaining, parsed) = x509_parser::parse_x509_certificate(der)
        .map_err(|_der| anyhow::anyhow!("invalid certificate DER"))?;
    if !remaining.is_empty() {
        bail!("trailing certificate DER");
    }
    Ok(parsed)
}
pub(super) fn ca_der(value: &str) -> anyhow::Result<Vec<u8>> {
    let der = pem(value, "CERTIFICATE")?;
    let root = cert(&der)?;
    if !root.basic_constraints()?.is_some_and(|c| c.value.ca)
        || !root.key_usage()?.is_some_and(|k| k.value.key_cert_sign())
        || root.subject() != root.issuer()
        || root.validity().not_before >= root.validity().not_after
    {
        bail!("invalid instance CA profile");
    }
    root.verify_signature(None)
        .map_err(|_signature| anyhow::anyhow!("invalid instance CA signature"))?;
    Ok(der)
}
pub(super) fn root_time(value: &str, time: i64) -> anyhow::Result<()> {
    let der = ca_der(value)?;
    let root = cert(&der)?;
    if time < 0
        || time < root.validity().not_before.timestamp()
        || time >= root.validity().not_after.timestamp()
    {
        bail!("instance CA is not currently valid");
    }
    Ok(())
}
fn private_key(value: &str) -> anyhow::Result<KeyPair> {
    KeyPair::try_from(&PrivatePkcs8KeyDer::from(pem(value, "PRIVATE KEY")?))
        .map_err(|_key| anyhow::anyhow!("invalid saved client key"))
}
pub(super) fn pending(invitation: &str) -> anyhow::Result<Pending> {
    let key = KeyPair::generate()?;
    let csr_pem = CertificateParams::new(Vec::<String>::new())?
        .serialize_request(&key)?
        .pem()?;
    Ok(Pending {
        request_id: Uuid::new_v4().to_string(),
        invitation_hash: hash(invitation.as_bytes()),
        key_pem: key.serialize_pem(),
        csr_pem,
    })
}
pub(super) fn validate_pending(pending: &Pending) -> anyhow::Result<()> {
    canonical_id(&pending.request_id)?;
    if pending.invitation_hash.len() != 64
        || !pending
            .invitation_hash
            .bytes()
            .all(|b| b.is_ascii_hexdigit())
    {
        bail!("invalid pending enrollment reference");
    }
    let der = pem(&pending.csr_pem, "CERTIFICATE REQUEST")?;
    let (remaining, csr) =
        x509_parser::certification_request::X509CertificationRequest::from_der(&der)
            .map_err(|_csr| anyhow::anyhow!("invalid pending certificate request"))?;
    if !remaining.is_empty()
        || csr.certification_request_info.subject_pki.raw
            != private_key(&pending.key_pem)?.public_key_der()
    {
        bail!("pending certificate request key mismatch");
    }
    CertificateSigningRequestParams::from_der(&CertificateSigningRequestDer::from(der.as_slice()))
        .map_err(|_proof| anyhow::anyhow!("invalid pending key possession proof"))?;
    Ok(())
}

pub(super) fn credential(
    trust: &WorkspaceTrust,
    material: &Credential,
    time: i64,
) -> anyhow::Result<()> {
    let receipt = &material.receipt;
    let instance = canonical_id(&receipt.instance_id)?;
    let client = canonical_id(&receipt.client_id)?;
    let credential = canonical_id(&receipt.credential_id)?;
    if instance.to_string() != trust.instance_id {
        bail!("certificate receipt has wrong instance identity");
    }
    root_time(&trust.ca_pem, time)?;
    let der = pem(&receipt.certificate_pem, "CERTIFICATE")?;
    let parsed = cert(&der)?;
    let root_der = ca_der(&trust.ca_pem)?;
    let root = cert(&root_der)?;
    if parsed.validity().not_after.timestamp() != receipt.expires_at
        || time < parsed.validity().not_before.timestamp()
        || time >= receipt.expires_at
        || parsed.validity().not_after > root.validity().not_after
        || parsed.validity().not_before < root.validity().not_before
        || receipt
            .expires_at
            .saturating_sub(parsed.validity().not_before.timestamp())
            > 30 * 24 * 60 * 60 + 60
        || parsed.public_key().raw != private_key(&material.key_pem)?.public_key_der()
        || unsigned(parsed.raw_serial()) != unsigned(credential.as_bytes())
    {
        bail!("certificate receipt key, serial or validity mismatch");
    }
    capabilities(&parsed)?;
    let mut expected = BTreeSet::from([
        format!("ccp://instance/{instance}"),
        format!("ccp://client/{client}"),
        format!("ccp://credential/{credential}"),
    ]);
    let san = parsed
        .subject_alternative_name()?
        .context("missing certificate UUID identities")?;
    if san.value.general_names.len() != 3 {
        bail!("unexpected certificate identities");
    }
    for name in &san.value.general_names {
        let GeneralName::URI(uri) = name else {
            bail!("unexpected certificate identity name");
        };
        if !expected.remove(*uri) {
            bail!("wrong or duplicate certificate identity");
        }
    }
    let mut roots = RootCertStore::empty();
    roots.add(CertificateDer::from(root_der))?;
    WebPkiClientVerifier::builder_with_provider(
        Arc::new(roots),
        Arc::new(rustls::crypto::ring::default_provider()),
    )
    .build()?
    .verify_client_cert(
        &CertificateDer::from(der),
        &[],
        UnixTime::since_unix_epoch(Duration::from_secs(u64::try_from(time)?)),
    )?;
    Ok(())
}
fn unsigned(bytes: &[u8]) -> &[u8] {
    bytes
        .get(bytes.iter().position(|b| *b != 0).unwrap_or(bytes.len())..)
        .unwrap_or_default()
}
fn capabilities(cert: &X509Certificate<'_>) -> anyhow::Result<()> {
    let usage = cert.key_usage()?.context("missing client key usage")?;
    let eku = cert
        .extended_key_usage()?
        .context("missing client extended key usage")?;
    if usage.value.flags != 1
        || !eku.value.client_auth
        || eku.value.server_auth
        || eku.value.any
        || eku.value.code_signing
        || eku.value.email_protection
        || eku.value.time_stamping
        || eku.value.ocsp_signing
        || !eku.value.other.is_empty()
        || cert.basic_constraints()?.is_some_and(|c| c.value.ca)
    {
        bail!("unexpected client certificate capabilities");
    }
    Ok(())
}
