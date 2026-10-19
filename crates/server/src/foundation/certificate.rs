// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use anyhow::{Context, bail};
use rcgen::{
    Certificate, CertificateParams, CertificateSigningRequestParams, DnType,
    ExtendedKeyUsagePurpose, Ia5String, IsCa, KeyPair, KeyUsagePurpose, SanType, SerialNumber,
};
use rustls::pki_types::{CertificateDer, CertificateSigningRequestDer};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use uuid::Uuid;
use x509_parser::prelude::FromDer;

const CLIENT_LIFETIME_SECONDS: i64 = 30 * 24 * 60 * 60;
const MAX_CSR_BYTES: usize = 16 * 1024;

/// Holds server-private signing material. Loading it is a trusted local server
/// operation; no Debug/Serialize/Clone implementation exposes the private key.
pub struct IssuingAuthority {
    certificate: Certificate,
    key: KeyPair,
    not_before: i64,
    not_after: i64,
}

pub(super) struct VerifiedCsr {
    csr: CertificateSigningRequestParams,
    pub key_hash: Vec<u8>,
}

pub(super) struct SignedClientCertificate {
    pub pem: String,
    pub fingerprint: Vec<u8>,
    pub key_hash: Vec<u8>,
    pub expires_at: i64,
}

#[derive(Clone, Debug)]
pub struct RedemptionRequest {
    pub request_id: Uuid,
    pub csr_pem: String,
    /// A claim only: reuse also requires an authenticated mTLS actor, a live
    /// stored credential and its matching CSR key, never this UUID by itself.
    pub existing_credential: Option<Uuid>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnrollmentReceipt {
    pub client_id: Uuid,
    pub credential_id: Uuid,
    pub grant_id: Uuid,
    pub certificate_pem: String,
    pub expires_at: i64,
}

impl IssuingAuthority {
    pub fn from_pem(certificate_pem: &str, private_key_pem: &str) -> anyhow::Result<Self> {
        let key = KeyPair::from_pem(private_key_pem).context("invalid issuing private key")?;
        let der = strict_pem_der(certificate_pem, "CERTIFICATE")?;
        let (remaining, parsed) = x509_parser::parse_x509_certificate(&der)
            .map_err(|error| anyhow::anyhow!("invalid issuing certificate DER: {error}"))?;
        if !remaining.is_empty() {
            bail!("issuing certificate has trailing DER data");
        }
        let params = CertificateParams::from_ca_cert_der(&CertificateDer::from(der.as_slice()))
            .context("invalid issuing CA certificate")?;
        if !matches!(params.is_ca, IsCa::Ca(_)) {
            bail!("issuing certificate must be a CA");
        }
        if parsed.public_key().raw != key.public_key_der() {
            bail!("issuing private key does not match CA certificate");
        }
        if parsed
            .key_usage()?
            .is_some_and(|usage| !usage.value.key_cert_sign())
        {
            bail!("issuing CA key usage does not allow certificate signing");
        }
        let not_before = params.not_before.unix_timestamp();
        let not_after = params.not_after.unix_timestamp();
        let certificate = params
            .self_signed(&key)
            .context("invalid issuing certificate parameters")?;
        Ok(Self {
            certificate,
            key,
            not_before,
            not_after,
        })
    }

    pub(super) fn issue(
        &self,
        proof: &VerifiedCsr,
        instance: Uuid,
        client: Uuid,
        credential: Uuid,
        now: i64,
    ) -> anyhow::Result<SignedClientCertificate> {
        if now < 0 || now < self.not_before || now >= self.not_after {
            bail!("issuing CA is not currently valid");
        }
        let expires_at = now
            .checked_add(CLIENT_LIFETIME_SECONDS)
            .context("credential expiry overflow")?
            .min(self.not_after);
        let mut params = CertificateParams::new(Vec::<String>::new())?;
        params.not_before = OffsetDateTime::from_unix_timestamp(
            now.saturating_sub(60).max(0).max(self.not_before),
        )?;
        params.not_after = OffsetDateTime::from_unix_timestamp(expires_at)?;
        params.serial_number = Some(SerialNumber::from_slice(credential.as_bytes()));
        params
            .distinguished_name
            .push(DnType::CommonName, credential.to_string());
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        for (kind, id) in [
            ("instance", instance),
            ("client", client),
            ("credential", credential),
        ] {
            params
                .subject_alt_names
                .push(SanType::URI(Ia5String::try_from(format!(
                    "ccp://{kind}/{id}"
                ))?));
        }
        let cert = params
            .signed_by(&proof.csr.public_key, &self.certificate, &self.key)
            .context("client certificate signing failed")?;
        let (remaining, parsed) = x509_parser::parse_x509_certificate(cert.der().as_ref())
            .map_err(|error| anyhow::anyhow!("invalid issued certificate DER: {error}"))?;
        let key_hash = Sha256::digest(parsed.public_key().raw).to_vec();
        if !remaining.is_empty() || key_hash != proof.key_hash {
            bail!("issued certificate key differs from the verified certificate request");
        }
        Ok(SignedClientCertificate {
            pem: cert.pem(),
            key_hash,
            fingerprint: Sha256::digest(cert.der().as_ref()).to_vec(),
            expires_at,
        })
    }
}

impl VerifiedCsr {
    pub fn parse(pem: &str) -> anyhow::Result<Self> {
        if pem.len() > MAX_CSR_BYTES {
            bail!("certificate request is too large");
        }
        // rcgen verifies the PKCS#10 signature, not merely its syntax. Caller
        // requested names, SANs and privileges are never copied to our leaf.
        let der = strict_pem_der(pem, "CERTIFICATE REQUEST")?;
        let (remaining, parsed) =
            x509_parser::certification_request::X509CertificationRequest::from_der(&der)
                .map_err(|error| anyhow::anyhow!("invalid certificate request DER: {error}"))?;
        if !remaining.is_empty() {
            bail!("certificate request has trailing DER data");
        }
        let csr = CertificateSigningRequestParams::from_der(&CertificateSigningRequestDer::from(
            der.as_slice(),
        ))
        .context("invalid certificate request or proof of key possession")?;
        let key_hash = Sha256::digest(parsed.certification_request_info.subject_pki.raw).to_vec();
        Ok(Self { csr, key_hash })
    }
}

/// Only outer ASCII whitespace is allowed. Decode one exact-label PEM block
/// once, then give both DER consumers the identical bytes; never let separate
/// PEM parsers select different certificates or requests from one string.
fn strict_pem_der(input: &str, label: &str) -> anyhow::Result<Vec<u8>> {
    let input = input.trim_matches(|character: char| character.is_ascii_whitespace());
    let header = format!("-----BEGIN {label}-----");
    let footer = format!("-----END {label}-----");
    let mut lines = input.lines();
    if lines.next() != Some(header.as_str()) || lines.next_back() != Some(footer.as_str()) {
        bail!("expected exactly one {label} PEM block");
    }
    let (remaining, decoded) = x509_parser::pem::parse_x509_pem(input.as_bytes())
        .map_err(|error| anyhow::anyhow!("invalid {label} PEM: {error}"))?;
    if decoded.label != label || !remaining.iter().all(u8::is_ascii_whitespace) {
        bail!("expected exactly one {label} PEM block without additional data");
    }
    Ok(decoded.contents)
}
