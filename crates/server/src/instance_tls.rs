// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Stable instance TLS material. These are local setup/runtime primitives, not
//! listeners or remote authorization. Client TLS verification must additionally
//! be bound to current registered credentials and grants before serving data.

mod bundle;
mod files;
mod peer;
mod renewal;
pub(crate) use peer::{CapturedPeer, VerifiedPeer};
pub use renewal::renew;

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, bail};
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use rustls::{RootCertStore, ServerConfig, server::WebPkiClientVerifier};
use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::directory_mode::DirectoryLease;
use crate::foundation::IssuingAuthority;
use bundle::PrivateBundle;

/// Public trust information only. Convey it over an independently trusted
/// channel before sending enrollment/admin secrets to a remote CCP instance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TrustInfo {
    pub instance_id: String,
    pub ca_certificate_pem: String,
    pub ca_sha256: String,
    pub server_sha256: String,
    pub names: Vec<String>,
    pub ca_expires_at: i64,
    pub server_expires_at: i64,
}

/// Private material deliberately has no Debug/Serialize/Clone implementation.
/// A retained workspace lease covers every future listener using this object.
pub struct InstanceTls {
    bundle: PrivateBundle,
    trust: TrustInfo,
    _lease: DirectoryLease,
}

impl InstanceTls {
    pub fn load(lease: &DirectoryLease) -> anyhow::Result<Self> {
        if !lease.allows_workspace() {
            bail!("instance TLS runtime requires a workspace runtime lease");
        }
        let connection = database(lease, false)?;
        let (bundle, trust) = load_pinned(lease, &connection, now()?)?;
        Ok(Self {
            bundle,
            trust,
            _lease: lease.clone(),
        })
    }

    pub fn trust(&self) -> &TrustInfo {
        &self.trust
    }

    pub fn issuing_authority(&self) -> anyhow::Result<IssuingAuthority> {
        IssuingAuthority::from_pem(&self.bundle.ca_certificate, &self.bundle.ca_key)
    }

    /// HTTPS for browser administration and invitation redemption. Application
    /// authentication is separate; this config never authorizes an admin key.
    pub fn https_config(&self) -> anyhow::Result<Arc<ServerConfig>> {
        let config = ServerConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()?
            .with_no_client_auth()
            .with_cert_resolver(self.resolver()?);
        Ok(http_config(config))
    }

    fn resolver(&self) -> anyhow::Result<Arc<LeasedResolver>> {
        Ok(Arc::new(LeasedResolver {
            key: Arc::new(rustls::sign::CertifiedKey::from_der(
                self.bundle.server_chain()?,
                self.bundle.server_private_key()?,
                &provider(),
            )?),
            _lease: self._lease.clone(),
        }))
    }

    /// Mandatory cryptographic client authentication. A valid chain alone does
    /// not authorize any workspace or establish a registered client identity.
    pub fn client_mtls_config(&self) -> anyhow::Result<Arc<ServerConfig>> {
        let mut roots = RootCertStore::empty();
        roots.add(self.bundle.ca_der()?)?;
        let verifier =
            WebPkiClientVerifier::builder_with_provider(Arc::new(roots), provider()).build()?;
        let config = ServerConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()?
            .with_client_cert_verifier(verifier)
            .with_cert_resolver(self.resolver()?);
        Ok(http_config(config))
    }
}

// Configs and connections can outlive InstanceTls. The resolver keeps the
// exclusive directory lease in the actual rustls configuration's lifetime.
struct LeasedResolver {
    key: Arc<rustls::sign::CertifiedKey>,
    _lease: DirectoryLease,
}

impl std::fmt::Debug for LeasedResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LeasedInstanceCertificate")
    }
}

impl rustls::server::ResolvesServerCert for LeasedResolver {
    fn resolve(
        &self,
        _hello: rustls::server::ClientHello<'_>,
    ) -> Option<Arc<rustls::sign::CertifiedKey>> {
        Some(self.key.clone())
    }
}

fn http_config(mut config: ServerConfig) -> Arc<ServerConfig> {
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    config.max_early_data_size = 0;
    Arc::new(config)
}

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// Explicit OS-operator operation. Existing material is never overwritten or
/// silently rotated; a failed anchor commit can reconcile the same bundle.
pub fn initialize(directory: &Path, names: &[String]) -> anyhow::Result<TrustInfo> {
    let names = bundle::normalize_names(names)?;
    require_database(directory)?;
    let lease = DirectoryLease::maintenance(directory)?;
    initialize_leased(&lease, &names, false)
}

pub(crate) fn initialize_setup(directory: &Path, names: &[String]) -> anyhow::Result<TrustInfo> {
    require_database(directory)?;
    let lease = DirectoryLease::maintenance(directory)?;
    initialize_setup_leased(&lease, names)
}

pub(crate) fn initialize_setup_leased(
    lease: &DirectoryLease,
    names: &[String],
) -> anyhow::Result<TrustInfo> {
    if !lease.allows_maintenance() {
        bail!("setup TLS requires an exclusive maintenance lease");
    }
    let defaults;
    let names = if names.is_empty() {
        let connection = database(lease, true)?;
        defaults = if let Some(pin) = anchor(&connection)? {
            let bytes = files::read_pinned(lease.directory(), &pin.hash)?;
            validate(&bytes, pinnable_instance(&connection)?, now()?)?
                .1
                .names
        } else if files::exists(lease.directory())? {
            validate(
                &files::read(lease.directory())?,
                pinnable_instance(&connection)?,
                now()?,
            )?
            .1
            .names
        } else {
            vec!["localhost".to_owned(), "127.0.0.1".to_owned()]
        };
        &defaults
    } else {
        names
    };
    initialize_leased(lease, names, true)
}

fn initialize_leased(
    lease: &DirectoryLease,
    names: &[String],
    setup: bool,
) -> anyhow::Result<TrustInfo> {
    let names = bundle::normalize_names(names)?;
    let mut connection = database(lease, true)?;
    connection.pragma_update(None, "synchronous", "FULL")?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let instance = if setup {
        pinnable_instance(&transaction)?
    } else {
        initialized_instance(&transaction)?
    };
    let pinned = anchor(&transaction)?;
    let timestamp = now()?;
    let bytes = if let Some(pin) = &pinned {
        files::read_pinned(lease.directory(), &pin.hash)?
    } else if files::exists(lease.directory())? {
        files::read(lease.directory())?
    } else {
        let bundle = PrivateBundle::generate(instance, names.clone(), timestamp)?;
        let bytes = serde_json::to_vec(&bundle)?;
        files::publish(lease.directory(), &bytes)?;
        bytes
    };
    let (bundle, trust) = validate(&bytes, instance, timestamp)?;
    if bundle.names != names {
        bail!("TLS names differ; existing identity is never overwritten");
    }
    let hash = Sha256::digest(&bytes).to_vec();
    let ca_hash = Sha256::digest(bundle.ca_der()?.as_ref()).to_vec();
    if let Some(pin) = pinned {
        check_pin(&pin, instance, &hash, &ca_hash)?;
    } else {
        transaction.execute("INSERT INTO ccp_instance_tls(singleton, instance_id, bundle_hash, ca_fingerprint) VALUES (1, ?1, ?2, ?3)", params![instance.to_string(), hash, ca_hash])?;
        transaction.execute("INSERT INTO ccp_access_audit(actor_kind, actor_id, action, target_id, recorded_at) VALUES ('local_operator', 'local', 'provision_instance_tls', ?1, ?2)", params![instance.to_string(), timestamp])?;
    }
    transaction
        .commit()
        .context("failed to pin TLS identity; rerun explicit setup with the same names")?;
    Ok(trust)
}

pub fn info(directory: &Path) -> anyhow::Result<TrustInfo> {
    require_database(directory)?;
    let lease = DirectoryLease::maintenance(directory)?;
    let connection = database(&lease, false)?;
    let (_, trust) = load_pinned(&lease, &connection, now()?)?;
    Ok(trust)
}

fn require_database(directory: &Path) -> anyhow::Result<()> {
    if !directory.join("ccp.sqlite3").is_file() {
        bail!("initialized instance database required; run workspace-init");
    }
    Ok(())
}

fn database(lease: &DirectoryLease, upgrade: bool) -> anyhow::Result<Connection> {
    require_database(lease.directory())?;
    let path = lease.directory().join("ccp.sqlite3");
    if upgrade {
        crate::init::init_sqlite(&path)?;
    }
    let flags = if upgrade {
        OpenFlags::SQLITE_OPEN_READ_WRITE
    } else {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    };
    let connection = Connection::open_with_flags(path, flags)?;
    crate::init::reject_unsupported_schema(&connection)?;
    connection.pragma_update(None, "foreign_keys", true)?;
    Ok(connection)
}

fn initialized_instance(connection: &Connection) -> anyhow::Result<Uuid> {
    let globals: u64 =
        connection.query_row("SELECT COUNT(*) FROM ccp_server_admins", [], |row| {
            row.get(0)
        })?;
    if globals == 0 {
        bail!("local administration must be initialized first");
    }
    let id: String = connection.query_row(
        "SELECT instance_id FROM ccp_instance WHERE singleton = 1",
        [],
        |row| row.get(0),
    )?;
    Uuid::parse_str(&id).context("invalid stored instance UUID")
}

fn pinnable_instance(connection: &Connection) -> anyhow::Result<Uuid> {
    let admins: u64 =
        connection.query_row("SELECT COUNT(*) FROM ccp_server_admins", [], |row| {
            row.get(0)
        })?;
    if admins > 0 {
        return initialized_instance(connection);
    }
    let id: String = connection.query_row(
        "SELECT r.instance_id FROM ccp_setup_reservation r JOIN ccp_instance i ON i.instance_id=r.instance_id JOIN ccp_runtime_mode m ON m.singleton=1 AND m.mode='workspace' WHERE r.singleton=1 AND i.singleton=1", [], |row|row.get(0),
    ).context("local administration or an explicit setup reservation is required")?;
    Uuid::parse_str(&id).context("invalid reserved instance UUID")
}

struct Pin {
    instance: String,
    hash: Vec<u8>,
    ca_hash: Vec<u8>,
}

fn anchor(connection: &Connection) -> anyhow::Result<Option<Pin>> {
    Ok(connection.query_row("SELECT instance_id, bundle_hash, ca_fingerprint FROM ccp_instance_tls WHERE singleton = 1", [], |row| {
        Ok(Pin { instance: row.get(0)?, hash: row.get(1)?, ca_hash: row.get(2)? })
    }).optional()?)
}

fn check_pin(pin: &Pin, instance: Uuid, hash: &[u8], ca_hash: &[u8]) -> anyhow::Result<()> {
    if pin.instance != instance.to_string() || pin.hash != hash || pin.ca_hash != ca_hash {
        bail!("TLS bundle does not match the pinned instance identity");
    }
    Ok(())
}

fn load_pinned(
    lease: &DirectoryLease,
    connection: &Connection,
    timestamp: i64,
) -> anyhow::Result<(PrivateBundle, TrustInfo)> {
    let instance = pinnable_instance(connection)?;
    let pin = anchor(connection)?.context("TLS identity is not pinned; run workspace-tls-init")?;
    let bytes = files::read_pinned(lease.directory(), &pin.hash)?;
    let (bundle, trust) = validate(&bytes, instance, timestamp)?;
    check_pin(
        &pin,
        instance,
        &Sha256::digest(bytes),
        &Sha256::digest(bundle.ca_der()?.as_ref()),
    )?;
    Ok((bundle, trust))
}

fn validate(
    bytes: &[u8],
    instance: Uuid,
    timestamp: i64,
) -> anyhow::Result<(PrivateBundle, TrustInfo)> {
    let bundle = decode_bundle(bytes)?;
    let trust = bundle.validate(instance, timestamp)?;
    Ok((bundle, trust))
}

fn decode_bundle(bytes: &[u8]) -> anyhow::Result<PrivateBundle> {
    // Unknown field errors can include private input; never attach the source.
    serde_json::from_slice(bytes)
        .map_err(|_invalid_input| anyhow::anyhow!("invalid TLS bundle format"))
}

pub(crate) fn now() -> anyhow::Result<i64> {
    Ok(i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs(),
    )?)
}

#[cfg(test)]
mod tests;
