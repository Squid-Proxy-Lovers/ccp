// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use crate::foundation::{
    Action, Actor, BootstrappedInstance, DataRole, InitialInvitationRequest, RedemptionRequest,
    authorize,
};
use rcgen::{CertificateParams, KeyPair};
use rustls::{
    ClientConfig, ClientConnection, ServerConnection,
    pki_types::{CertificateDer, PrivatePkcs8KeyDer, ServerName},
};
use std::fs;
use std::path::PathBuf;

struct Fixture {
    directory: PathBuf,
    setup: BootstrappedInstance,
    initial: TrustInfo,
}
impl Fixture {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!("ccp-server-leaf-{}", Uuid::new_v4()));
        let setup = crate::local_admin::bootstrap(&directory, "operator", "first").unwrap();
        let initial = initialize(&directory, &["localhost".into(), "127.0.0.1".into()]).unwrap();
        Self {
            directory,
            setup,
            initial,
        }
    }
    fn db(&self) -> Connection {
        Connection::open(self.directory.join("ccp.sqlite3")).unwrap()
    }
    fn bytes(&self) -> Vec<u8> {
        let pin = anchor(&self.db()).unwrap().unwrap();
        files::read_pinned(&self.directory, &pin.hash).unwrap()
    }
    fn bundle(&self) -> PrivateBundle {
        decode_bundle(&self.bytes()).unwrap()
    }
    fn generation(&self) -> PathBuf {
        let pin = anchor(&self.db()).unwrap().unwrap();
        files::generation_path(&self.directory, &pin.hash).unwrap()
    }
    fn generations(&self) -> usize {
        fs::read_dir(&self.directory)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("ccp-instance-tls-") && n.ends_with(".json"))
            .count()
    }
    fn install_historical(&self, issued_at: i64) -> PrivateBundle {
        // Trusted synthetic installation fixture, not a production rotation API.
        let bundle = PrivateBundle::generate(
            self.setup.instance_id,
            self.initial.names.clone(),
            issued_at,
        )
        .unwrap();
        self.install(&bundle);
        bundle
    }
    fn install(&self, bundle: &PrivateBundle) {
        let bytes = serde_json::to_vec(bundle).unwrap();
        fs::write(self.directory.join(files::FILE_NAME), &bytes).unwrap();
        self.db()
            .execute(
                "UPDATE ccp_instance_tls SET bundle_hash = ?1, ca_fingerprint = ?2",
                params![
                    Sha256::digest(bytes).to_vec(),
                    Sha256::digest(bundle.ca_der().unwrap().as_ref()).to_vec()
                ],
            )
            .unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.directory).unwrap();
    }
}

fn client(ca: &str, credentials: Option<(&str, &KeyPair)>) -> Arc<ClientConfig> {
    let mut roots = RootCertStore::empty();
    roots
        .add(CertificateDer::from(
            crate::foundation::strict_pem_der(ca, "CERTIFICATE").unwrap(),
        ))
        .unwrap();
    let builder = ClientConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots);
    let config = match credentials {
        None => builder.with_no_client_auth(),
        Some((pem, key)) => builder
            .with_client_auth_cert(
                vec![CertificateDer::from(
                    crate::foundation::strict_pem_der(pem, "CERTIFICATE").unwrap(),
                )],
                PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
            )
            .unwrap(),
    };
    Arc::new(config)
}

fn handshake(
    server: Arc<ServerConfig>,
    client: Arc<ClientConfig>,
    name: &str,
) -> anyhow::Result<ServerConnection> {
    let mut server = ServerConnection::new(server)?;
    let mut client = ClientConnection::new(client, ServerName::try_from(name.to_string())?)?;
    for _ in 0..20 {
        let mut bytes = Vec::new();
        client.write_tls(&mut bytes)?;
        let mut incoming = bytes.as_slice();
        while !incoming.is_empty() {
            if server.read_tls(&mut incoming)? == 0 {
                bail!("server TLS did not consume input");
            }
            server.process_new_packets()?;
        }
        let mut bytes = Vec::new();
        server.write_tls(&mut bytes)?;
        let mut incoming = bytes.as_slice();
        while !incoming.is_empty() {
            if client.read_tls(&mut incoming)? == 0 {
                bail!("client TLS did not consume input");
            }
            client.process_new_packets()?;
        }
        if !server.is_handshaking() && !client.is_handshaking() {
            return Ok(server);
        }
    }
    bail!("TLS handshake did not complete")
}

#[test]
fn explicit_renewal_rotates_only_server_leaf_and_key_then_setup_and_restart_use_selected_pin() {
    let f = Fixture::new();
    let original = f.bytes();
    let old = f.bundle();
    let next = renew(&f.directory, None).unwrap();
    let bundle = f.bundle();
    assert_eq!(next.instance_id, f.initial.instance_id);
    assert_eq!(next.ca_sha256, f.initial.ca_sha256);
    assert_eq!(next.ca_certificate_pem, f.initial.ca_certificate_pem);
    assert_eq!(next.names, f.initial.names);
    assert_ne!(next.server_sha256, f.initial.server_sha256);
    assert_ne!(bundle.server_key, old.server_key);
    assert_eq!(bundle.ca_key, old.ca_key);
    assert_eq!(bundle.ca_certificate, old.ca_certificate);
    assert_eq!(
        fs::read(f.directory.join(files::FILE_NAME)).unwrap(),
        original
    );
    assert_eq!(f.generations(), 1);
    assert!(f.generation().is_file());
    assert_eq!(info(&f.directory).unwrap(), next);
    assert_eq!(initialize(&f.directory, &next.names).unwrap(), next);
    assert_eq!(f.generations(), 1);
    let audit: (String, String) = f
        .db()
        .query_row(
            "SELECT actor_kind, actor_id FROM ccp_access_audit WHERE action = 'renew_instance_tls'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(audit, ("local_operator".into(), "local".into()));
    let keys: i64 = f
        .db()
        .query_row(
            "SELECT COUNT(*) FROM ccp_admin_keys WHERE revoked_at IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(keys, 2);
    let lease = DirectoryLease::activate_workspace(&f.directory).unwrap();
    let loaded = InstanceTls::load(&lease).unwrap();
    assert_eq!(loaded.trust(), &next);
    assert!(
        handshake(
            loaded.https_config().unwrap(),
            client(&f.initial.ca_certificate_pem, None),
            "localhost"
        )
        .is_ok()
    );
}

#[test]
fn expired_leaf_can_be_renewed_but_expired_or_future_ca_never_rotates() {
    let f = Fixture::new();
    let timestamp = now().unwrap();
    let old = f.install_historical(timestamp - 366 * 86400);
    assert!(info(&f.directory).is_err());
    let next = renew(&f.directory, None).unwrap();
    assert_eq!(next.ca_certificate_pem, old.ca_certificate);
    assert_eq!(f.bundle().ca_key, old.ca_key);
    assert!(next.server_expires_at > timestamp);
    assert!(info(&f.directory).is_ok());
    for issued_at in [timestamp - 3651 * 86400, timestamp + 3600] {
        f.install_historical(issued_at);
        // The fixture pin selects the newly installed original bytes; earlier
        // unreferenced generations must not override that selection.
        let before = f.bytes();
        let pin = anchor(&f.db()).unwrap().unwrap().hash;
        assert!(renew(&f.directory, None).is_err());
        assert_eq!(f.bytes(), before);
        assert_eq!(anchor(&f.db()).unwrap().unwrap().hash, pin);
    }
}

#[test]
fn renewal_caps_leaf_at_current_ca_expiry_and_validates_old_key_before_replacement() {
    let f = Fixture::new();
    let timestamp = now().unwrap();
    let old = f.install_historical(timestamp - 3650 * 86400 + 3600);
    let next = renew(&f.directory, None).unwrap();
    assert_eq!(next.server_expires_at, next.ca_expires_at);
    assert_eq!(next.ca_certificate_pem, old.ca_certificate);
    assert!(next.ca_expires_at > timestamp);
    assert!(next.ca_expires_at <= timestamp + 3600);
    let mut bad = PrivateBundle::generate(
        f.setup.instance_id,
        f.initial.names.clone(),
        timestamp - 366 * 86400,
    )
    .unwrap();
    bad.server_key = KeyPair::generate().unwrap().serialize_pem();
    f.install(&bad);
    let hash = anchor(&f.db()).unwrap().unwrap().hash;
    assert!(renew(&f.directory, None).is_err());
    assert_eq!(anchor(&f.db()).unwrap().unwrap().hash, hash);
}

#[test]
fn failed_pin_or_audit_commit_preserves_old_active_identity_and_ignores_unreferenced_generation() {
    for trigger in [
        "CREATE TRIGGER fail_pin BEFORE UPDATE ON ccp_instance_tls BEGIN SELECT RAISE(ABORT, 'pin unavailable'); END;",
        "CREATE TRIGGER fail_audit BEFORE INSERT ON ccp_access_audit WHEN NEW.action = 'renew_instance_tls' BEGIN SELECT RAISE(ABORT, 'audit unavailable'); END;",
    ] {
        let f = Fixture::new();
        let bytes = f.bytes();
        f.db().execute_batch(trigger).unwrap();
        assert!(renew(&f.directory, None).is_err());
        assert_eq!(f.bytes(), bytes);
        assert_eq!(info(&f.directory).unwrap(), f.initial);
        assert_eq!(f.generations(), 1);
        let audits: i64 = f
            .db()
            .query_row(
                "SELECT COUNT(*) FROM ccp_access_audit WHERE action = 'renew_instance_tls'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(audits, 0);
    }
}

#[test]
fn missing_or_corrupt_selected_generation_never_falls_back_to_retired_valid_bundle() {
    let f = Fixture::new();
    let next = renew(&f.directory, None).unwrap();
    let bytes = f.bytes();
    let selected = f.generation();
    fs::remove_file(&selected).unwrap();
    assert!(info(&f.directory).is_err());
    assert!(initialize(&f.directory, &next.names).is_err());
    assert!(renew(&f.directory, None).is_err());
    assert!(f.directory.join(files::FILE_NAME).is_file());
    assert!(!selected.exists());
    files::publish_generation(&f.directory, &bytes).unwrap();
    assert_eq!(info(&f.directory).unwrap(), next);
    fs::write(&selected, b"corrupt").unwrap();
    assert!(info(&f.directory).is_err());
    assert!(renew(&f.directory, None).is_err());
    assert_eq!(fs::read(selected).unwrap(), b"corrupt");
}

#[test]
fn explicit_names_change_only_new_leaf_and_are_checked_by_actual_tls_verification() {
    let f = Fixture::new();
    let names = vec!["Renamed.Example".into()];
    let next = renew(&f.directory, Some(&names)).unwrap();
    assert_eq!(next.names, vec!["renamed.example"]);
    assert_eq!(next.ca_sha256, f.initial.ca_sha256);
    let lease = DirectoryLease::activate_workspace(&f.directory).unwrap();
    let loaded = InstanceTls::load(&lease).unwrap();
    assert!(
        handshake(
            loaded.https_config().unwrap(),
            client(&f.initial.ca_certificate_pem, None),
            "renamed.example"
        )
        .is_ok()
    );
    assert!(
        handshake(
            loaded.https_config().unwrap(),
            client(&f.initial.ca_certificate_pem, None),
            "localhost"
        )
        .is_err()
    );
    drop(loaded);
    drop(lease);
    assert_eq!(initialize(&f.directory, &next.names).unwrap(), next);
    assert!(initialize(&f.directory, &f.initial.names).is_err());
}

#[test]
fn renewal_requires_existing_pin_valid_names_and_exclusive_directory_access() {
    let f = Fixture::new();
    let before = f.bytes();
    for names in [
        vec![],
        vec!["*.example.com".into()],
        vec!["https://localhost".into()],
    ] {
        assert!(renew(&f.directory, Some(&names)).is_err());
    }
    let native = DirectoryLease::native(&f.directory).unwrap();
    assert!(renew(&f.directory, None).is_err());
    drop(native);
    let lease = DirectoryLease::activate_workspace(&f.directory).unwrap();
    let loaded = InstanceTls::load(&lease).unwrap();
    let config = loaded.https_config().unwrap();
    drop(loaded);
    drop(lease);
    assert!(renew(&f.directory, None).is_err());
    drop(config);
    assert_eq!(f.bytes(), before);
    let missing = f.directory.join("missing");
    assert!(renew(&missing, None).is_err());
    assert!(!missing.exists());
    f.db().execute("DELETE FROM ccp_instance_tls", []).unwrap();
    assert!(renew(&f.directory, None).is_err());
    assert_eq!(f.generations(), 0);
}

#[test]
fn schema13_original_layout_upgrades_without_changing_instance_or_owner_keys() {
    let f = Fixture::new();
    f.db().execute_batch("DELETE FROM schema_version WHERE version > 13; INSERT OR IGNORE INTO schema_version(version) VALUES (13);").unwrap();
    assert_eq!(info(&f.directory).unwrap(), f.initial);
    let next = renew(&f.directory, None).unwrap();
    assert_eq!(next.instance_id, f.initial.instance_id);
    assert_eq!(next.ca_sha256, f.initial.ca_sha256);
    let version: u32 = f
        .db()
        .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 14);
    let mut db = f.db();
    let mut store = crate::foundation::AccessStore::new(&mut db).unwrap();
    assert!(
        store
            .authenticate_admin_key(f.setup.owner_key.secret(), now().unwrap())
            .is_ok()
    );
    assert!(
        store
            .authenticate_admin_key(f.setup.instance_key.secret(), now().unwrap())
            .is_ok()
    );
}

#[test]
fn client_enrollment_and_registered_peer_authorization_survive_server_leaf_renewal() {
    let f = Fixture::new();
    let key = KeyPair::generate().unwrap();
    let receipt = {
        let lease = DirectoryLease::activate_workspace(&f.directory).unwrap();
        let loaded = InstanceTls::load(&lease).unwrap();
        let mut db = f.db();
        let mut store = crate::foundation::AccessStore::new(&mut db).unwrap();
        let owner = store
            .authenticate_admin_key(f.setup.owner_key.secret(), now().unwrap())
            .unwrap();
        let token = store
            .issue_initial_invitation(
                owner,
                InitialInvitationRequest {
                    workspace_id: f.setup.workspace_id,
                    role: DataRole::ReadWrite,
                    max_clients: Some(1),
                    expires_at: Some(now().unwrap() + 3600),
                },
                now().unwrap(),
            )
            .unwrap();
        let request = RedemptionRequest {
            request_id: Uuid::new_v4(),
            csr_pem: CertificateParams::new(Vec::<String>::new())
                .unwrap()
                .serialize_request(&key)
                .unwrap()
                .pem()
                .unwrap(),
            existing_credential: None,
        };
        store
            .redeem_invitation(
                token.secret(),
                &request,
                &loaded.issuing_authority().unwrap(),
                now().unwrap(),
            )
            .unwrap()
    };
    let next = renew(&f.directory, None).unwrap();
    assert_eq!(next.ca_sha256, f.initial.ca_sha256);
    let lease = DirectoryLease::activate_workspace(&f.directory).unwrap();
    let loaded = InstanceTls::load(&lease).unwrap();
    let peer = handshake(
        loaded.client_mtls_config().unwrap(),
        client(
            &f.initial.ca_certificate_pem,
            Some((&receipt.certificate_pem, &key)),
        ),
        "localhost",
    )
    .unwrap();
    let mut db = f.db();
    let actor = loaded
        .authenticate_client(
            &peer,
            &mut crate::foundation::AccessStore::new(&mut db).unwrap(),
        )
        .unwrap();
    assert_eq!(actor, Actor::Credential(receipt.credential_id));
    assert!(
        authorize(
            &db,
            actor,
            Action::WriteContext,
            Some(f.setup.workspace_id),
            now().unwrap()
        )
        .is_ok()
    );
    let closed: bool = db
        .query_row(
            "SELECT closed_at IS NOT NULL FROM ccp_initial_enrollment",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(closed);
}

#[test]
fn generation_publication_never_overwrites_and_uses_private_permissions() {
    let f = Fixture::new();
    let bytes = f.bytes();
    files::publish_generation(&f.directory, &bytes).unwrap();
    let path = f.generation();
    assert!(path.is_file());
    assert!(files::publish_generation(&f.directory, &bytes).is_err());
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(info(&f.directory).unwrap(), f.initial);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
