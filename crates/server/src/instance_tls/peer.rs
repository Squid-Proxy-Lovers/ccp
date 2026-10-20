// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::time::Duration;

use anyhow::{Context, bail};
use rusqlite::{Connection, OptionalExtension, params};
use rustls::{RootCertStore, ServerConnection, pki_types::UnixTime, server::WebPkiClientVerifier};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use x509_parser::extensions::GeneralName;

use super::{InstanceTls, now, provider};
use crate::foundation::{AccessStore, Actor, RenewalReceipt, RenewalRequest, strict_pem_der};

/// Constructed only from a completed TLS connection, never a request field.
/// Kept crate-private so raw certificate bytes cannot become an external
/// authentication mechanism without TLS proof of private-key possession.
pub(crate) struct VerifiedPeer {
    instance: Uuid,
    client: Uuid,
    credential: Uuid,
    fingerprint: Vec<u8>,
    key_hash: Vec<u8>,
    expires_at: i64,
}

impl InstanceTls {
    /// Authenticate the actual established peer for this request. Never cache
    /// the returned actor as a substitute for authenticating later requests.
    /// Protected operations must still recheck current authorization in their
    /// own transaction. Superseded peers never receive an ordinary actor.
    pub fn authenticate_client(
        &self,
        tls: &ServerConnection,
        store: &mut AccessStore<'_>,
    ) -> anyhow::Result<Actor> {
        let timestamp = now()?;
        let peer = self.verify_peer(tls, timestamp)?;
        store.authenticate_peer(&peer, timestamp)
    }

    /// The only superseded-peer adapter: verify TLS proof, then revalidate the
    /// registry within the atomic renewal/retry transaction. No actor escapes.
    pub fn renew_client(
        &self,
        tls: &ServerConnection,
        store: &mut AccessStore<'_>,
        request: &RenewalRequest,
    ) -> anyhow::Result<RenewalReceipt> {
        let timestamp = now()?;
        let peer = self.verify_peer(tls, timestamp)?;
        store.renew_verified_peer(&peer, request, &self.issuing_authority()?, timestamp)
    }

    fn verify_peer(&self, tls: &ServerConnection, timestamp: i64) -> anyhow::Result<VerifiedPeer> {
        if tls.is_handshaking() {
            bail!("client TLS handshake is not complete");
        }
        let chain = tls
            .peer_certificates()
            .context("client TLS certificate required")?;
        self.verify_chain(chain, timestamp)
    }

    pub(crate) fn capture_peer(&self, tls: &ServerConnection) -> anyhow::Result<CapturedPeer> {
        if tls.is_handshaking() {
            bail!("client TLS handshake is not complete");
        }
        let chain = tls
            .peer_certificates()
            .context("client TLS certificate required")?;
        if chain.is_empty()
            || chain.len() > 2
            || chain.first().is_some_and(|leaf| leaf.len() > 16 * 1024)
        {
            bail!("unexpected client certificate chain");
        }
        Ok(CapturedPeer {
            chain: chain.to_vec(),
        })
    }

    fn verify_chain(
        &self,
        chain: &[rustls::pki_types::CertificateDer<'static>],
        timestamp: i64,
    ) -> anyhow::Result<VerifiedPeer> {
        let leaf = chain.first().context("empty client certificate chain")?;
        if leaf.len() > 16 * 1024 {
            bail!("client certificate is too large");
        }
        let ca = self.bundle.ca_der()?;
        if chain.len() > 2 || chain.get(1).is_some_and(|cert| cert != &ca) {
            bail!("unexpected client certificate chain");
        }
        let (_, root) = x509_parser::parse_x509_certificate(&ca)
            .map_err(|_invalid| anyhow::anyhow!("invalid instance CA"))?;
        if timestamp < root.validity().not_before.timestamp()
            || timestamp >= root.validity().not_after.timestamp()
        {
            bail!("instance CA is not currently valid");
        }
        let mut roots = RootCertStore::empty();
        roots.add(ca.clone())?;
        let verifier =
            WebPkiClientVerifier::builder_with_provider(std::sync::Arc::new(roots), provider())
                .build()?;
        verifier.verify_client_cert(
            leaf,
            &[],
            UnixTime::since_unix_epoch(Duration::from_secs(u64::try_from(timestamp)?)),
        )?;
        let (remaining, cert) = x509_parser::parse_x509_certificate(leaf)
            .map_err(|_invalid| anyhow::anyhow!("invalid client certificate"))?;
        if !remaining.is_empty() {
            bail!("trailing client certificate data");
        }
        // CCP's credential expiry is exclusive; WebPKI accepts the exact
        // notAfter instant, so enforce the same boundary as registry queries.
        if timestamp < cert.validity().not_before.timestamp()
            || timestamp >= cert.validity().not_after.timestamp()
            || cert.validity().not_after > root.validity().not_after
        {
            bail!("client certificate is outside its allowed validity interval");
        }
        client_capabilities(&cert)?;
        let (instance, client, credential) = subject_ids(&cert)?;
        if instance.to_string() != self.trust.instance_id
            || unsigned(cert.raw_serial()) != unsigned(credential.as_bytes())
        {
            bail!("client certificate has wrong instance or serial identity");
        }
        Ok(VerifiedPeer {
            instance,
            client,
            credential,
            fingerprint: Sha256::digest(leaf.as_ref()).to_vec(),
            key_hash: Sha256::digest(cert.public_key().raw).to_vec(),
            expires_at: cert.validity().not_after.timestamp(),
        })
    }
}

impl VerifiedPeer {
    pub(crate) fn credential(&self) -> Uuid {
        self.credential
    }

    pub(crate) fn require_registered(
        &self,
        connection: &Connection,
        timestamp: i64,
        allow_superseded: bool,
    ) -> anyhow::Result<()> {
        let row = connection.query_row(
            "SELECT k.certificate_fingerprint, k.public_key_hash, k.certificate_pem,
             c.expires_at, EXISTS(SELECT 1 FROM ccp_credential_renewals r WHERE r.previous_credential_id = c.id)
             FROM ccp_credentials c JOIN ccp_clients p ON p.id = c.client_id
             JOIN ccp_credential_certificates k ON k.credential_id = c.id
             JOIN ccp_instance i ON i.singleton = 1
             WHERE c.id = ?1 AND c.client_id = ?2 AND p.disabled = 0 AND c.revoked_at IS NULL
             AND c.expires_at > ?3 AND i.instance_id = ?4",
            params![self.credential.to_string(), self.client.to_string(), timestamp, self.instance.to_string()],
            |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?, row.get::<_, String>(2)?, row.get::<_, i64>(3)?, row.get::<_, bool>(4)?)),
        ).optional()?.context("client certificate is not currently registered and enabled")?;
        if row.0 != self.fingerprint
            || row.1 != self.key_hash
            || row.3 != self.expires_at
            || (row.4 && !allow_superseded)
        {
            bail!("client certificate registry mismatch or superseded credential");
        }
        let stored = strict_pem_der(&row.2, "CERTIFICATE")?;
        if Sha256::digest(stored).as_slice() != self.fingerprint {
            bail!("stored certificate differs from registered TLS peer");
        }
        Ok(())
    }
}

fn client_capabilities(cert: &x509_parser::certificate::X509Certificate<'_>) -> anyhow::Result<()> {
    let key = cert.key_usage()?.context("missing client key usage")?;
    let eku = cert
        .extended_key_usage()?
        .context("missing client extended key usage")?;
    if key.value.flags != 1
        || !eku.value.client_auth
        || eku.value.server_auth
        || eku.value.any
        || eku.value.code_signing
        || eku.value.email_protection
        || eku.value.time_stamping
        || eku.value.ocsp_signing
        || !eku.value.other.is_empty()
        || cert
            .basic_constraints()?
            .is_some_and(|value| value.value.ca)
    {
        bail!("unexpected client certificate capabilities");
    }
    Ok(())
}

fn subject_ids(
    cert: &x509_parser::certificate::X509Certificate<'_>,
) -> anyhow::Result<(Uuid, Uuid, Uuid)> {
    let san = cert
        .subject_alternative_name()?
        .context("missing client certificate identities")?;
    if san.value.general_names.len() != 3 {
        bail!("expected exactly three client identity URIs");
    }
    let mut instance = None;
    let mut client = None;
    let mut credential = None;
    for name in &san.value.general_names {
        let GeneralName::URI(uri) = name else {
            bail!("unexpected client identity name");
        };
        let Some((kind, value)) = uri.strip_prefix("ccp://").and_then(|v| v.split_once('/')) else {
            bail!("invalid client identity URI");
        };
        let id = Uuid::parse_str(value).context("invalid client identity UUID")?;
        if id.to_string() != value {
            bail!("noncanonical client identity UUID");
        }
        let slot = match kind {
            "instance" => &mut instance,
            "client" => &mut client,
            "credential" => &mut credential,
            _ => bail!("unknown client identity URI"),
        };
        if slot.replace(id).is_some() {
            bail!("duplicate client identity URI");
        }
    }
    Ok((
        instance.context("missing instance URI")?,
        client.context("missing client URI")?,
        credential.context("missing credential URI")?,
    ))
}

fn unsigned(bytes: &[u8]) -> &[u8] {
    let offset = bytes
        .iter()
        .position(|byte| *byte != 0)
        .unwrap_or(bytes.len());
    bytes.get(offset..).unwrap_or_default()
}

#[cfg(test)]
mod tests;

// Opaque proof source captured only from the completed mandatory-mTLS stream.
// The HTTP service owns it for that exact connection; each request revalidates
// time/profile/chain and the current registry, never caches an Actor.
pub(crate) struct CapturedPeer {
    chain: Vec<rustls::pki_types::CertificateDer<'static>>,
}
impl CapturedPeer {
    pub(crate) fn verify(&self, identity: &InstanceTls, now: i64) -> anyhow::Result<VerifiedPeer> {
        identity.verify_chain(&self.chain, now)
    }
}
