// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use crate::directory_mode::DirectoryLease;
use crate::foundation::{
    Action, BootstrappedInstance, DataRole, EnrollmentReceipt, InitialInvitationRequest,
    RedemptionRequest, authorize,
};
use rcgen::{
    CertificateParams, ExtendedKeyUsagePurpose, Ia5String, KeyPair, KeyUsagePurpose, SanType,
    SerialNumber,
};
use rustls::{
    ClientConfig, ClientConnection, ServerConfig,
    pki_types::{CertificateDer, PrivatePkcs8KeyDer, ServerName},
};
use std::path::PathBuf;
use std::sync::Arc;
use time::OffsetDateTime;

struct Fixture {
    directory: PathBuf,
    tls: InstanceTls,
    setup: BootstrappedInstance,
    invite: String,
    // Fields drop in declaration order: release the SQLite lease before cleanup
    // so this fixture also works on platforms that cannot unlink open files.
    _cleanup: Cleanup,
}
impl Fixture {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!("ccp-peer-{}", Uuid::new_v4()));
        let setup = crate::local_admin::bootstrap(&directory, "operator", "first").unwrap();
        super::super::initialize(&directory, &["localhost".into()]).unwrap();
        let lease = DirectoryLease::activate_workspace(&directory).unwrap();
        let tls = InstanceTls::load(&lease).unwrap();
        let mut connection = Connection::open(directory.join("ccp.sqlite3")).unwrap();
        let mut store = AccessStore::new(&mut connection).unwrap();
        let owner = store
            .authenticate_admin_key(setup.owner_key.secret(), now().unwrap())
            .unwrap();
        let invitation = store
            .issue_initial_invitation(
                owner,
                InitialInvitationRequest {
                    workspace_id: setup.workspace_id,
                    role: DataRole::ReadWrite,
                    max_clients: Some(10),
                    expires_at: Some(now().unwrap() + 3600),
                },
                now().unwrap(),
            )
            .unwrap();
        let cleanup = Cleanup(directory.clone());
        Self {
            directory,
            tls,
            setup,
            invite: invitation.secret().to_string(),
            _cleanup: cleanup,
        }
    }
    fn connection(&self) -> Connection {
        Connection::open(self.directory.join("ccp.sqlite3")).unwrap()
    }
    fn enroll(&self) -> (EnrollmentReceipt, KeyPair) {
        let key = KeyPair::generate().unwrap();
        let req = RedemptionRequest {
            request_id: Uuid::new_v4(),
            csr_pem: csr(&key),
            existing_credential: None,
        };
        let mut db = self.connection();
        let receipt = AccessStore::new(&mut db)
            .unwrap()
            .redeem_invitation(
                &self.invite,
                &req,
                &self.tls.issuing_authority().unwrap(),
                now().unwrap(),
            )
            .unwrap();
        (receipt, key)
    }
    fn client(&self, certificate: &str, key: &KeyPair, root_chain: bool) -> Arc<ClientConfig> {
        let mut roots = RootCertStore::empty();
        roots.add(self.tls.bundle.ca_der().unwrap()).unwrap();
        let mut chain = vec![CertificateDer::from(
            strict_pem_der(certificate, "CERTIFICATE").unwrap(),
        )];
        if root_chain {
            chain.push(self.tls.bundle.ca_der().unwrap());
        }
        let config = ClientConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_client_auth_cert(chain, PrivatePkcs8KeyDer::from(key.serialize_der()).into())
            .unwrap();
        Arc::new(config)
    }
    fn session(&self, receipt: &EnrollmentReceipt, key: &KeyPair) -> ServerConnection {
        handshake(
            self.tls.client_mtls_config().unwrap(),
            self.client(&receipt.certificate_pem, key, false),
        )
        .unwrap()
        .0
    }
    fn actor(&self, tls: &ServerConnection) -> anyhow::Result<Actor> {
        let mut db = self.connection();
        self.tls
            .authenticate_client(tls, &mut AccessStore::new(&mut db)?)
    }
    fn renew(
        &self,
        tls: &ServerConnection,
        request: &RenewalRequest,
    ) -> anyhow::Result<RenewalReceipt> {
        let mut db = self.connection();
        self.tls
            .renew_client(tls, &mut AccessStore::new(&mut db)?, request)
    }
    fn custom_certificate(&self, receipt: &EnrollmentReceipt, key: &KeyPair, kind: &str) -> String {
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        params.not_after = OffsetDateTime::from_unix_timestamp(receipt.expires_at).unwrap();
        params.serial_number = Some(SerialNumber::from_slice(receipt.credential_id.as_bytes()));
        let mut ids = vec![
            ("instance", self.setup.instance_id),
            ("client", receipt.client_id),
            ("credential", receipt.credential_id),
        ];
        match kind {
            "foreign_instance" => ids[0].1 = Uuid::new_v4(),
            "foreign_client" => ids[1].1 = Uuid::new_v4(),
            "duplicate" => ids[2].0 = "client",
            "unknown" => ids[2].0 = "unknown",
            "wrong_serial" => {
                params.serial_number = Some(SerialNumber::from_slice(Uuid::new_v4().as_bytes()))
            }
            "oversized" => params
                .distinguished_name
                .push(rcgen::DnType::CommonName, "x".repeat(17000)),
            "extra_eku" => params
                .extended_key_usages
                .push(ExtendedKeyUsagePurpose::ServerAuth),
            _ => (),
        }
        for (name, id) in ids {
            let mut value = format!("ccp://{name}/{id}");
            if kind == "uppercase" {
                value = value.to_uppercase();
            }
            if kind == "noncanonical_uuid" {
                value = format!("ccp://{name}/{}", id.simple());
            }
            params
                .subject_alt_names
                .push(SanType::URI(Ia5String::try_from(value).unwrap()));
        }
        if kind == "extra_san" {
            params
                .subject_alt_names
                .push(SanType::DnsName(Ia5String::try_from("localhost").unwrap()));
        }
        if kind == "missing" {
            params.subject_alt_names.pop();
        }
        if kind == "dns_instead" {
            params.subject_alt_names[2] =
                SanType::DnsName(Ia5String::try_from("localhost").unwrap());
        }
        let ca_key = KeyPair::from_pem(&self.tls.bundle.ca_key).unwrap();
        let ca = CertificateParams::from_ca_cert_pem(&self.tls.bundle.ca_certificate)
            .unwrap()
            .self_signed(&ca_key)
            .unwrap();
        params.signed_by(key, &ca, &ca_key).unwrap().pem()
    }
}
struct Cleanup(PathBuf);
impl Drop for Cleanup {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

fn csr(key: &KeyPair) -> String {
    CertificateParams::new(Vec::<String>::new())
        .unwrap()
        .serialize_request(key)
        .unwrap()
        .pem()
        .unwrap()
}
fn renewal() -> (RenewalRequest, KeyPair) {
    let key = KeyPair::generate().unwrap();
    (
        RenewalRequest {
            request_id: Uuid::new_v4(),
            csr_pem: csr(&key),
        },
        key,
    )
}

fn handshake(
    server: Arc<ServerConfig>,
    client: Arc<ClientConfig>,
) -> anyhow::Result<(ServerConnection, ClientConnection)> {
    let mut server = ServerConnection::new(server)?;
    let mut client = ClientConnection::new(client, ServerName::try_from("localhost")?)?;
    for _ in 0..20 {
        let mut bytes = Vec::new();
        client.write_tls(&mut bytes)?;
        let mut incoming = bytes.as_slice();
        while !incoming.is_empty() {
            if server.read_tls(&mut incoming)? == 0 {
                bail!("server stopped consuming TLS records");
            }
            server.process_new_packets()?;
        }
        let mut bytes = Vec::new();
        server.write_tls(&mut bytes)?;
        let mut incoming = bytes.as_slice();
        while !incoming.is_empty() {
            if client.read_tls(&mut incoming)? == 0 {
                bail!("client stopped consuming TLS records");
            }
            client.process_new_packets()?;
        }
        if !client.is_handshaking() && !server.is_handshaking() {
            return Ok((server, client));
        }
    }
    bail!("handshake did not finish")
}

#[test]
fn actual_enrolled_tls_peer_resolves_only_its_registered_credential_and_scope() {
    let f = Fixture::new();
    let (enrolled, key) = f.enroll();
    let session = f.session(&enrolled, &key);
    let actor = f.actor(&session).unwrap();
    assert_eq!(actor, Actor::Credential(enrolled.credential_id));
    assert!(
        authorize(
            &f.connection(),
            actor,
            Action::WriteContext,
            Some(f.setup.workspace_id),
            now().unwrap()
        )
        .is_ok()
    );
    assert!(
        authorize(
            &f.connection(),
            actor,
            Action::SaveGlobalBoard,
            None,
            now().unwrap()
        )
        .is_err()
    );
    assert!(
        authorize(
            &f.connection(),
            actor,
            Action::ReadContext,
            Some(Uuid::new_v4()),
            now().unwrap()
        )
        .is_err()
    );
    let mut db = f.connection();
    let visible = AccessStore::new(&mut db)
        .unwrap()
        .visible_workspaces(actor, now().unwrap())
        .unwrap();
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].id, f.setup.workspace_id);
    let with_root = handshake(
        f.tls.client_mtls_config().unwrap(),
        f.client(&enrolled.certificate_pem, &key, true),
    )
    .unwrap()
    .0;
    assert_eq!(f.actor(&with_root).unwrap(), actor);
}

#[test]
fn unestablished_and_https_without_client_certificate_cannot_supply_an_actor() {
    let f = Fixture::new();
    let unestablished = ServerConnection::new(f.tls.client_mtls_config().unwrap()).unwrap();
    assert!(f.actor(&unestablished).is_err());
    let mut roots = RootCertStore::empty();
    roots.add(f.tls.bundle.ca_der().unwrap()).unwrap();
    let config = Arc::new(
        ClientConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    );
    let session = handshake(f.tls.https_config().unwrap(), config.clone())
        .unwrap()
        .0;
    assert!(f.actor(&session).is_err());
    assert!(f.renew(&session, &renewal().0).is_err());
    assert!(handshake(f.tls.client_mtls_config().unwrap(), config).is_err());
}

#[test]
fn signed_but_unregistered_or_malformed_identity_never_becomes_registered_authority() {
    let f = Fixture::new();
    let (enrolled, key) = f.enroll();
    for kind in [
        "foreign_instance",
        "foreign_client",
        "duplicate",
        "unknown",
        "wrong_serial",
        "extra_eku",
        "uppercase",
        "noncanonical_uuid",
        "extra_san",
        "missing",
        "dns_instead",
        "unregistered",
        "oversized",
    ] {
        let cert = f.custom_certificate(&enrolled, &key, kind);
        let session = handshake(
            f.tls.client_mtls_config().unwrap(),
            f.client(&cert, &key, false),
        )
        .unwrap()
        .0;
        assert!(f.actor(&session).is_err(), "{kind}");
        assert!(f.renew(&session, &renewal().0).is_err(), "{kind}");
    }
    assert_eq!(
        f.actor(&f.session(&enrolled, &key)).unwrap(),
        Actor::Credential(enrolled.credential_id)
    );
}

#[test]
fn current_registry_must_match_exact_fingerprint_spki_certificate_expiry_and_client() {
    let f = Fixture::new();
    let (enrolled, key) = f.enroll();
    let session = f.session(&enrolled, &key);
    let db = f.connection();
    let original: (Vec<u8>, Vec<u8>) = db
        .query_row(
            "SELECT certificate_fingerprint, public_key_hash FROM ccp_credential_certificates",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    for sql in [
        "UPDATE ccp_credential_certificates SET certificate_fingerprint = zeroblob(32)",
        "UPDATE ccp_credential_certificates SET public_key_hash = zeroblob(32)",
        "UPDATE ccp_credential_certificates SET certificate_pem = 'corrupt'",
        "UPDATE ccp_credentials SET expires_at = expires_at + 1",
    ] {
        db.execute_batch(sql).unwrap();
        assert!(f.actor(&session).is_err(), "{sql}");
        assert!(f.renew(&session, &renewal().0).is_err(), "{sql}");
        db.execute("UPDATE ccp_credential_certificates SET certificate_fingerprint = ?1, public_key_hash = ?2, certificate_pem = ?3", params![original.0, original.1, enrolled.certificate_pem]).unwrap();
        db.execute(
            "UPDATE ccp_credentials SET expires_at = ?1, client_id = ?2",
            params![enrolled.expires_at, enrolled.client_id.to_string()],
        )
        .unwrap();
        db.execute(
            "UPDATE ccp_instance SET instance_id = ?1",
            [f.setup.instance_id.to_string()],
        )
        .unwrap();
        assert!(f.actor(&session).is_ok());
    }
    // A validly signed leaf with another client's URI must fail the client
    // join even when its exact fingerprint and PEM are registered to this ID.
    let wrong_client = f.custom_certificate(&enrolled, &key, "foreign_client");
    let wrong_der = strict_pem_der(&wrong_client, "CERTIFICATE").unwrap();
    db.execute(
        "UPDATE ccp_credential_certificates SET certificate_fingerprint = ?1, certificate_pem = ?2",
        params![Sha256::digest(&wrong_der).to_vec(), wrong_client],
    )
    .unwrap();
    let wrong_session = handshake(
        f.tls.client_mtls_config().unwrap(),
        f.client(&wrong_client, &key, false),
    )
    .unwrap()
    .0;
    assert!(f.tls.verify_peer(&wrong_session, now().unwrap()).is_ok());
    assert!(f.actor(&wrong_session).is_err());
    db.execute(
        "UPDATE ccp_credential_certificates SET certificate_fingerprint = ?1, certificate_pem = ?2",
        params![original.0, enrolled.certificate_pem],
    )
    .unwrap();
    // Separate committed changes are observed by the same established TLS peer.
    db.execute(
        "UPDATE ccp_credential_certificates SET certificate_pem = ?1",
        [&f.tls.trust.ca_certificate_pem],
    )
    .unwrap();
    assert!(f.actor(&session).is_err());
    db.execute(
        "UPDATE ccp_credential_certificates SET certificate_pem = ?1",
        [&enrolled.certificate_pem],
    )
    .unwrap();
    assert!(f.actor(&session).is_ok());
}

#[test]
fn revocation_disable_and_expiry_take_effect_on_an_already_open_connection() {
    let f = Fixture::new();
    let (enrolled, key) = f.enroll();
    let session = f.session(&enrolled, &key);
    let db = f.connection();
    for sql in [
        "UPDATE ccp_credentials SET revoked_at = 1",
        "UPDATE ccp_clients SET disabled = 1",
        "UPDATE ccp_credentials SET expires_at = 1",
    ] {
        db.execute_batch(sql).unwrap();
        assert!(f.actor(&session).is_err());
        assert!(f.renew(&session, &renewal().0).is_err());
        db.execute(
            "UPDATE ccp_credentials SET revoked_at = NULL, expires_at = ?1",
            [enrolled.expires_at],
        )
        .unwrap();
        db.execute_batch("UPDATE ccp_clients SET disabled = 0")
            .unwrap();
        assert!(f.actor(&session).is_ok());
    }
    assert!(f.tls.verify_peer(&session, enrolled.expires_at).is_err());
    assert!(
        f.tls
            .verify_peer(&session, f.tls.trust.ca_expires_at)
            .is_err()
    );
}

#[test]
fn grants_and_archive_stay_separate_from_identity_authentication() {
    let f = Fixture::new();
    let (enrolled, key) = f.enroll();
    let session = f.session(&enrolled, &key);
    let actor = f.actor(&session).unwrap();
    let db = f.connection();
    db.execute_batch("UPDATE ccp_workspaces SET state = 'archived'")
        .unwrap();
    assert!(
        authorize(
            &db,
            actor,
            Action::ReadContext,
            Some(f.setup.workspace_id),
            now().unwrap()
        )
        .is_ok()
    );
    assert!(
        authorize(
            &db,
            actor,
            Action::WriteContext,
            Some(f.setup.workspace_id),
            now().unwrap()
        )
        .is_err()
    );
    db.execute_batch("UPDATE ccp_grants SET revoked_at = 1")
        .unwrap();
    assert!(f.actor(&session).is_ok());
    assert!(
        authorize(
            &db,
            actor,
            Action::ReadContext,
            Some(f.setup.workspace_id),
            now().unwrap()
        )
        .is_err()
    );
    assert!(f.renew(&session, &renewal().0).is_err());
    let mut connection = f.connection();
    assert!(
        AccessStore::new(&mut connection)
            .unwrap()
            .visible_workspaces(actor, now().unwrap())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn superseded_peer_can_only_fetch_exact_current_renewal_receipt() {
    let f = Fixture::new();
    let (old, key) = f.enroll();
    let session = f.session(&old, &key);
    let (request, new_key) = renewal();
    let new = f.renew(&session, &request).unwrap();
    assert!(f.actor(&session).is_err());
    assert_eq!(f.renew(&session, &request).unwrap(), new);
    assert!(f.renew(&session, &renewal().0).is_err());
    let different_nonce = RenewalRequest {
        request_id: Uuid::new_v4(),
        csr_pem: request.csr_pem.clone(),
    };
    assert!(f.renew(&session, &different_nonce).is_err());
    let different_csr = RenewalRequest {
        request_id: request.request_id,
        csr_pem: renewal().0.csr_pem,
    };
    assert!(f.renew(&session, &different_csr).is_err());
    let new_session = handshake(
        f.tls.client_mtls_config().unwrap(),
        f.client(&new.certificate_pem, &new_key, false),
    )
    .unwrap()
    .0;
    assert_eq!(
        f.actor(&new_session).unwrap(),
        Actor::Credential(new.credential_id)
    );
    let old_reconnected = f.session(&old, &key);
    assert!(f.actor(&old_reconnected).is_err());
    assert_eq!(f.renew(&old_reconnected, &request).unwrap(), new);
    let count: i64 = f
        .connection()
        .query_row("SELECT COUNT(*) FROM ccp_credentials", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 2);
}

#[test]
fn retry_adapter_denies_old_or_new_revocation_disable_and_lost_captured_access() {
    for mutation in [
        "old_only",
        "new_only",
        "new_expired",
        "UPDATE ccp_credentials SET revoked_at = 1",
        "UPDATE ccp_clients SET disabled = 1",
        "UPDATE ccp_grants SET revoked_at = 1",
        "UPDATE ccp_workspaces SET state = 'purged'",
    ] {
        let f = Fixture::new();
        let (old, key) = f.enroll();
        let session = f.session(&old, &key);
        let (request, _) = renewal();
        let new = f.renew(&session, &request).unwrap();
        match mutation {
            "old_only" => {
                f.connection()
                    .execute(
                        "UPDATE ccp_credentials SET revoked_at = 1 WHERE id = ?1",
                        [old.credential_id.to_string()],
                    )
                    .unwrap();
            }
            "new_only" => {
                f.connection()
                    .execute(
                        "UPDATE ccp_credentials SET revoked_at = 1 WHERE id = ?1",
                        [new.credential_id.to_string()],
                    )
                    .unwrap();
            }
            "new_expired" => {
                f.connection()
                    .execute(
                        "UPDATE ccp_credentials SET expires_at = 1 WHERE id = ?1",
                        [new.credential_id.to_string()],
                    )
                    .unwrap();
            }
            _ => {
                f.connection().execute_batch(mutation).unwrap();
            }
        }
        assert!(f.renew(&session, &request).is_err(), "{mutation}");
        assert!(f.actor(&session).is_err());
        let count: i64 = f
            .connection()
            .query_row("SELECT COUNT(*) FROM ccp_credentials", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 2);
        assert_ne!(new.credential_id, old.credential_id);
    }
}

#[test]
fn resumed_tls_peer_still_rechecks_the_current_registry() {
    let f = Fixture::new();
    let (enrolled, key) = f.enroll();
    let server = f.tls.client_mtls_config().unwrap();
    let client = f.client(&enrolled.certificate_pem, &key, false);
    let first = handshake(server.clone(), client.clone()).unwrap();
    assert!(f.actor(&first.0).is_ok());
    drop(first);
    f.connection()
        .execute_batch("UPDATE ccp_clients SET disabled = 1")
        .unwrap();
    let resumed = handshake(server, client).unwrap();
    assert_eq!(
        resumed.0.handshake_kind(),
        Some(rustls::HandshakeKind::Resumed)
    );
    assert!(resumed.0.peer_certificates().is_some());
    assert!(f.actor(&resumed.0).is_err());
}

#[test]
fn a_live_tls_peer_cannot_authenticate_against_another_instance_database() {
    let a = Fixture::new();
    let (enrolled, key) = a.enroll();
    let session = a.session(&enrolled, &key);
    let b = Fixture::new();
    let mut other = b.connection();
    assert!(
        a.tls
            .authenticate_client(&session, &mut AccessStore::new(&mut other).unwrap())
            .is_err()
    );
    // Preserve all FK constraints while atomically changing the two instance
    // UUID references. A stale TLS instance must notice the new database UUID.
    let mut db = a.connection();
    let different = Uuid::new_v4().to_string();
    let tx = db.transaction().unwrap();
    tx.pragma_update(None, "defer_foreign_keys", true).unwrap();
    tx.execute("UPDATE ccp_instance SET instance_id = ?1", [&different])
        .unwrap();
    tx.execute("UPDATE ccp_instance_tls SET instance_id = ?1", [&different])
        .unwrap();
    tx.commit().unwrap();
    assert!(a.actor(&session).is_err());
    let invalid: i64 = db
        .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(invalid, 0);
}

#[test]
fn failed_adapter_renewal_preserves_working_old_peer_and_issues_no_orphan() {
    let f = Fixture::new();
    let (old, key) = f.enroll();
    let session = f.session(&old, &key);
    let (request, _) = renewal();
    f.connection().execute_batch("CREATE TRIGGER no_renew_audit BEFORE INSERT ON ccp_access_audit WHEN NEW.action = 'renew_credential' BEGIN SELECT RAISE(ABORT, 'audit unavailable'); END;").unwrap();
    assert!(f.renew(&session, &request).is_err());
    assert_eq!(
        f.actor(&session).unwrap(),
        Actor::Credential(old.credential_id)
    );
    let count: i64 = f
        .connection()
        .query_row("SELECT COUNT(*) FROM ccp_credentials", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);
    f.connection()
        .execute_batch("DROP TRIGGER no_renew_audit")
        .unwrap();
    let new = f.renew(&session, &request).unwrap();
    assert_eq!(f.renew(&session, &request).unwrap(), new);
    assert!(f.actor(&session).is_err());
}
