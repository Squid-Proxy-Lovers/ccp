// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use rcgen::{CertificateParams, ExtendedKeyUsagePurpose, KeyPair, KeyUsagePurpose};
use rustls::{ClientConfig, ClientConnection, ServerConnection, pki_types::ServerName};
use std::path::PathBuf;
use time::OffsetDateTime;

struct Fixture {
    directory: PathBuf,
    instance: Uuid,
}
impl Fixture {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!("ccp-instance-tls-{}", Uuid::new_v4()));
        let setup = crate::local_admin::bootstrap(&directory, "operator", "first").unwrap();
        Self {
            directory,
            instance: setup.instance_id,
        }
    }
    fn names(&self) -> Vec<String> {
        vec!["localhost".into(), "127.0.0.1".into(), "::1".into()]
    }
    fn init(&self) -> TrustInfo {
        initialize(&self.directory, &self.names()).unwrap()
    }
    fn bytes(&self) -> Vec<u8> {
        files::read(&self.directory).unwrap()
    }
    fn bundle(&self) -> PrivateBundle {
        serde_json::from_slice(&self.bytes()).unwrap()
    }
    fn connection(&self) -> Connection {
        Connection::open(self.directory.join("ccp.sqlite3")).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.directory).unwrap();
    }
}

#[test]
fn setup_pins_stable_ca_names_and_local_audit_without_touching_native_material() {
    let f = Fixture::new();
    let native = f.directory.join("ccp_ca_key.pem");
    std::fs::write(&native, b"untouched native fixture").unwrap();
    let first = f.init();
    let bytes = f.bytes();
    let second = initialize(
        &f.directory,
        &[
            "LOCALHOST".into(),
            "::1".into(),
            "127.0.0.1".into(),
            "localhost".into(),
        ],
    )
    .unwrap();
    assert_eq!(first, second);
    assert_eq!(bytes, f.bytes());
    assert_eq!(info(&f.directory).unwrap(), first);
    assert_eq!(first.instance_id, f.instance.to_string());
    assert_eq!(first.names, vec!["127.0.0.1", "::1", "localhost"]);
    assert_eq!(first.ca_sha256.len(), 64);
    assert_ne!(first.server_sha256, first.ca_sha256);
    assert!(first.ca_expires_at > first.server_expires_at);
    let audit: i64 = f.connection().query_row("SELECT COUNT(*) FROM ccp_access_audit WHERE action = 'provision_instance_tls' AND actor_kind = 'local_operator' AND actor_id = 'local' AND admin_key_id IS NULL", [], |r| r.get(0)).unwrap();
    assert_eq!(audit, 1);
    assert_eq!(std::fs::read(native).unwrap(), b"untouched native fixture");
    let output = serde_json::to_string(&first).unwrap();
    assert!(!output.contains("PRIVATE KEY"));
    assert!(!output.contains(&f.bundle().server_key));
}

#[test]
fn missing_uninitialized_database_and_invalid_names_do_not_publish_any_identity() {
    let missing = std::env::temp_dir().join(format!("ccp-tls-missing-{}", Uuid::new_v4()));
    assert!(initialize(&missing, &["localhost".into()]).is_err());
    assert!(info(&missing).is_err());
    assert!(!missing.exists());
    let f = Fixture::new();
    for name in [
        "",
        "*.example.com",
        "https://example.com",
        "bad name",
        "a..b",
        "-a",
        "a-",
        "a.",
        "é.com",
        "[::1]",
    ] {
        assert!(initialize(&f.directory, &[name.into()]).is_err(), "{name}");
    }
    assert!(initialize(&f.directory, &[]).is_err());
    assert!(initialize(&f.directory, &vec!["localhost".into(); 33]).is_err());
    assert!(!files::exists(&f.directory).unwrap());
    f.connection()
        .execute("DELETE FROM ccp_server_admins", [])
        .unwrap();
    assert!(initialize(&f.directory, &f.names()).is_err());
    assert!(!files::exists(&f.directory).unwrap());
}

#[test]
fn existing_pin_prevents_replacement_after_bundle_loss_or_changed_names() {
    let f = Fixture::new();
    f.init();
    let bytes = f.bytes();
    assert!(initialize(&f.directory, &["elsewhere.example".into()]).is_err());
    assert_eq!(f.bytes(), bytes);
    std::fs::remove_file(f.directory.join(files::FILE_NAME)).unwrap();
    assert!(initialize(&f.directory, &f.names()).is_err());
    assert!(info(&f.directory).is_err());
    assert!(!files::exists(&f.directory).unwrap());
    files::publish(&f.directory, &bytes).unwrap();
    assert!(info(&f.directory).is_ok());
}

#[test]
fn failed_anchor_audit_rolls_back_pin_and_reconciles_exact_published_bundle() {
    let f = Fixture::new();
    f.connection().execute_batch("CREATE TRIGGER no_tls_audit BEFORE INSERT ON ccp_access_audit WHEN NEW.action = 'provision_instance_tls' BEGIN SELECT RAISE(ABORT, 'audit unavailable'); END;").unwrap();
    assert!(initialize(&f.directory, &f.names()).is_err());
    let bytes = f.bytes();
    let count: i64 = f
        .connection()
        .query_row("SELECT COUNT(*) FROM ccp_instance_tls", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
    assert!(info(&f.directory).is_err());
    f.connection()
        .execute_batch("DROP TRIGGER no_tls_audit")
        .unwrap();
    let trust = f.init();
    assert_eq!(bytes, f.bytes());
    assert_eq!(trust, info(&f.directory).unwrap());
}

#[test]
fn foreign_instance_or_changed_bytes_fail_even_when_certificate_is_valid() {
    let a = Fixture::new();
    a.init();
    let b = Fixture::new();
    b.init();
    std::fs::write(b.directory.join(files::FILE_NAME), a.bytes()).unwrap();
    assert!(info(&b.directory).is_err());
    assert!(initialize(&b.directory, &b.names()).is_err());
    let mut bytes = a.bytes();
    bytes.push(b' ');
    std::fs::write(a.directory.join(files::FILE_NAME), &bytes).unwrap();
    assert!(info(&a.directory).is_err());
    assert!(initialize(&a.directory, &a.names()).is_err());
    assert_eq!(a.bytes(), bytes);
}

#[test]
fn private_format_rejects_corruption_unknown_version_and_duplicate_fields_without_leaking_input() {
    let f = Fixture::new();
    f.init();
    let timestamp = now().unwrap();
    for bytes in [
        b"truncated".to_vec(),
        b"{\"format\":1,\"format\":1}".to_vec(),
        b"{\"SECRET_VALUE\": \"do not echo\"}".to_vec(),
    ] {
        let err = validate(&bytes, f.instance, timestamp)
            .err()
            .unwrap()
            .to_string();
        assert_eq!(err, "invalid TLS bundle format");
        assert!(!err.contains("SECRET_VALUE"));
    }
    let mut bundle = f.bundle();
    bundle.format = 2;
    assert!(bundle.validate(f.instance, timestamp).is_err());
    bundle.format = 1;
    bundle.names.reverse();
    assert!(bundle.validate(f.instance, timestamp).is_err());
}

#[test]
fn certificate_validation_enforces_time_names_key_pair_and_strict_pem() {
    let f = Fixture::new();
    f.init();
    let mut bundle = f.bundle();
    let timestamp = now().unwrap();
    assert!(bundle.validate(f.instance, 0).is_err());
    assert!(
        bundle
            .validate(f.instance, timestamp + 366 * 86400)
            .is_err()
    );
    assert!(
        bundle
            .validate(f.instance, timestamp + 3651 * 86400)
            .is_err()
    );
    assert!(bundle.validate(f.instance, -1).is_err());
    bundle.names = vec!["wrong.example".into()];
    assert!(bundle.validate(f.instance, timestamp).is_err());
    bundle = f.bundle();
    bundle.server_key = KeyPair::generate().unwrap().serialize_pem();
    assert!(bundle.validate(f.instance, timestamp).is_err());
    bundle = f.bundle();
    bundle.ca_key = KeyPair::generate().unwrap().serialize_pem();
    assert!(bundle.validate(f.instance, timestamp).is_err());
    bundle = f.bundle();
    bundle
        .ca_certificate
        .push_str(&bundle.ca_certificate.clone());
    assert!(bundle.validate(f.instance, timestamp).is_err());
    bundle = f.bundle();
    bundle.server_key.push_str(&bundle.server_key.clone());
    assert!(bundle.validate(f.instance, timestamp).is_err());
}

#[test]
fn publication_never_overwrites_existing_material_and_ignores_crash_staging_files() {
    let f = Fixture::new();
    std::fs::write(
        f.directory.join(".ccp-instance-tls-crash.tmp"),
        b"partial fixture",
    )
    .unwrap();
    f.init();
    let bytes = f.bytes();
    assert!(files::publish(&f.directory, b"replacement").is_err());
    assert_eq!(bytes, f.bytes());
    assert!(files::publish(&f.directory, &vec![0; 65537]).is_err());
    assert_eq!(bytes, f.bytes());
    std::fs::write(f.directory.join(files::FILE_NAME), vec![0; 65537]).unwrap();
    assert!(info(&f.directory).is_err());
}

#[cfg(unix)]
#[test]
fn unix_private_permissions_and_symlinks_are_checked() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let f = Fixture::new();
    f.init();
    let path = f.directory.join(files::FILE_NAME);
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(info(&f.directory).is_err());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(info(&f.directory).is_ok());
    let real = f.directory.join("other.json");
    std::fs::rename(&path, &real).unwrap();
    symlink(&real, &path).unwrap();
    assert!(info(&f.directory).is_err());
    assert!(initialize(&f.directory, &f.names()).is_err());
    assert!(
        std::fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn maintenance_and_runtime_lock_boundaries_retain_exclusive_lease() {
    let f = Fixture::new();
    let native = DirectoryLease::native(&f.directory).unwrap();
    assert!(initialize(&f.directory, &f.names()).is_err());
    assert!(info(&f.directory).is_err());
    assert!(InstanceTls::load(&native).is_err());
    drop(native);
    f.init();
    let maintenance = DirectoryLease::maintenance(&f.directory).unwrap();
    assert!(InstanceTls::load(&maintenance).is_err());
    drop(maintenance);
    let lease = DirectoryLease::activate_workspace(&f.directory).unwrap();
    let identity = InstanceTls::load(&lease).unwrap();
    drop(lease);
    assert!(DirectoryLease::maintenance(&f.directory).is_err());
    assert!(initialize(&f.directory, &f.names()).is_err());
    assert_eq!(identity.trust().instance_id, f.instance.to_string());
    assert!(identity.issuing_authority().is_ok());
    drop(identity);
    assert!(info(&f.directory).is_ok());
    assert!(DirectoryLease::native(&f.directory).is_err());
}

fn client_config(bundle: &PrivateBundle, client: bool) -> Arc<ClientConfig> {
    let mut roots = RootCertStore::empty();
    roots.add(bundle.ca_der().unwrap()).unwrap();
    let builder = ClientConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots);
    let mut config = if client {
        let key = KeyPair::generate().unwrap();
        let ca_key = KeyPair::from_pem(&bundle.ca_key).unwrap();
        let ca = CertificateParams::from_ca_cert_pem(&bundle.ca_certificate)
            .unwrap()
            .self_signed(&ca_key)
            .unwrap();
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        let cert = params.signed_by(&key, &ca, &ca_key).unwrap();
        builder
            .with_client_auth_cert(
                vec![cert.der().clone()],
                rustls::pki_types::PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
            )
            .unwrap()
    } else {
        builder.with_no_client_auth()
    };
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Arc::new(config)
}

fn handshake(
    server: Arc<ServerConfig>,
    client: Arc<ClientConfig>,
    name: &str,
) -> anyhow::Result<()> {
    let mut server = ServerConnection::new(server)?;
    let mut client = ClientConnection::new(client, ServerName::try_from(name.to_string())?)?;
    for _ in 0..20 {
        let mut bytes = Vec::new();
        client.write_tls(&mut bytes)?;
        if !bytes.is_empty() {
            server.read_tls(&mut bytes.as_slice())?;
            server.process_new_packets()?;
        }
        let mut bytes = Vec::new();
        server.write_tls(&mut bytes)?;
        if !bytes.is_empty() {
            client.read_tls(&mut bytes.as_slice())?;
            client.process_new_packets()?;
        }
        if !client.is_handshaking() && !server.is_handshaking() {
            assert_eq!(client.alpn_protocol(), Some(b"http/1.1".as_slice()));
            return Ok(());
        }
    }
    anyhow::bail!("handshake did not complete")
}

#[test]
fn actual_rustls_handshakes_require_trusted_name_and_client_certificate_on_mtls() {
    let f = Fixture::new();
    f.init();
    let bundle = f.bundle();
    let lease = DirectoryLease::activate_workspace(&f.directory).unwrap();
    let identity = InstanceTls::load(&lease).unwrap();
    assert!(
        handshake(
            identity.https_config().unwrap(),
            client_config(&bundle, false),
            "localhost"
        )
        .is_ok()
    );
    assert!(
        handshake(
            identity.https_config().unwrap(),
            client_config(&bundle, false),
            "127.0.0.1"
        )
        .is_ok()
    );
    assert!(
        handshake(
            identity.https_config().unwrap(),
            client_config(&bundle, false),
            "wrong.example"
        )
        .is_err()
    );
    assert!(
        handshake(
            identity.client_mtls_config().unwrap(),
            client_config(&bundle, false),
            "localhost"
        )
        .is_err()
    );
    assert!(
        handshake(
            identity.client_mtls_config().unwrap(),
            client_config(&bundle, true),
            "localhost"
        )
        .is_ok()
    );
    let other =
        PrivateBundle::generate(Uuid::new_v4(), vec!["localhost".into()], now().unwrap()).unwrap();
    assert!(
        handshake(
            identity.https_config().unwrap(),
            client_config(&other, false),
            "localhost"
        )
        .is_err()
    );
}

#[test]
fn schema12_upgrade_preserves_instance_roles_keys_and_mode() {
    let f = Fixture::new();
    f.connection().execute_batch("DROP TABLE ccp_instance_tls; DELETE FROM schema_version WHERE version > 12; INSERT OR IGNORE INTO schema_version(version) VALUES (12);").unwrap();
    let trust = f.init();
    assert_eq!(trust.instance_id, f.instance.to_string());
    let connection = f.connection();
    let count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM ccp_admin_keys WHERE revoked_at IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 2);
    let version: u32 = connection
        .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 13);
    assert!(DirectoryLease::native(&f.directory).is_ok());
}

#[test]
fn escaped_tls_configs_and_connections_keep_the_directory_exclusively_leased() {
    let f = Fixture::new();
    f.init();
    let lease = DirectoryLease::activate_workspace(&f.directory).unwrap();
    let identity = InstanceTls::load(&lease).unwrap();
    let https = identity.https_config().unwrap();
    let mtls = identity.client_mtls_config().unwrap();
    let connection = ServerConnection::new(https.clone()).unwrap();
    drop(identity);
    drop(lease);
    assert!(DirectoryLease::maintenance(&f.directory).is_err());
    drop(https);
    assert!(DirectoryLease::maintenance(&f.directory).is_err());
    drop(mtls);
    assert!(DirectoryLease::maintenance(&f.directory).is_err());
    drop(connection);
    assert!(DirectoryLease::maintenance(&f.directory).is_ok());
}

#[test]
fn server_leaf_constraints_and_exact_pem_are_enforced_before_pin() {
    let f = Fixture::new();
    f.init();
    let mut bundle = f.bundle();
    let ca_key = KeyPair::from_pem(&bundle.ca_key).unwrap();
    let ca = CertificateParams::from_ca_cert_pem(&bundle.ca_certificate)
        .unwrap()
        .self_signed(&ca_key)
        .unwrap();
    let server_key = KeyPair::from_pem(&bundle.server_key).unwrap();
    for kind in [
        "extra_client_auth",
        "ca_leaf",
        "other_san",
        "missing_key_usage",
        "wrong_issuer",
    ] {
        let mut params = CertificateParams::new(bundle.names.clone()).unwrap();
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        params.not_after = OffsetDateTime::from_unix_timestamp(now().unwrap() + 86400).unwrap();
        match kind {
            "extra_client_auth" => params
                .extended_key_usages
                .push(ExtendedKeyUsagePurpose::ClientAuth),
            "ca_leaf" => params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained),
            "other_san" => params.subject_alt_names.push(rcgen::SanType::URI(
                rcgen::Ia5String::try_from("ccp://unexpected").unwrap(),
            )),
            "missing_key_usage" => params.key_usages.clear(),
            _ => (),
        }
        bundle.server_certificate = if kind == "wrong_issuer" {
            params.self_signed(&server_key).unwrap().pem()
        } else {
            params.signed_by(&server_key, &ca, &ca_key).unwrap().pem()
        };
        assert!(
            bundle.validate(f.instance, now().unwrap()).is_err(),
            "{kind}"
        );
    }
    bundle = f.bundle();
    bundle
        .server_certificate
        .push_str(&f.bundle().ca_certificate);
    assert!(bundle.validate(f.instance, now().unwrap()).is_err());
}
