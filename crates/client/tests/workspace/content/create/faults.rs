// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use rcgen::{CertificateParams, ExtendedKeyUsagePurpose, KeyPair, KeyUsagePurpose};
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

// A trusted test TLS peer commits through the actual store, then corrupts only
// its reply. This counts create POSTs independently of transactional rollback.
fn tls(f: &Fixture) -> Arc<rustls::ServerConfig> {
    let bundle: Value =
        serde_json::from_slice(&fs::read(f.root.join("server/ccp-instance-tls.json")).unwrap())
            .unwrap();
    let ca_key = KeyPair::from_pem(bundle["ca_key"].as_str().unwrap()).unwrap();
    let ca = CertificateParams::from_ca_cert_pem(&f.trust.ca_certificate_pem)
        .unwrap()
        .self_signed(&ca_key)
        .unwrap();
    let key = KeyPair::generate().unwrap();
    let mut params = CertificateParams::new(vec!["127.0.0.1".into()]).unwrap();
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let cert = params.signed_by(&key, &ca, &ca_key).unwrap();
    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(CertificateDer::from(
            x509_parser::pem::parse_x509_pem(f.trust.ca_certificate_pem.as_bytes())
                .unwrap()
                .1
                .contents,
        ))
        .unwrap();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let verifier = rustls::server::WebPkiClientVerifier::builder_with_provider(
        Arc::new(roots),
        provider.clone(),
    )
    .build()
    .unwrap();
    Arc::new(
        rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_client_cert_verifier(verifier)
            .with_single_cert(
                vec![cert.der().clone()],
                PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
            )
            .unwrap(),
    )
}

async fn request(stream: &mut tokio_rustls::server::TlsStream<tokio::net::TcpStream>) -> String {
    let mut bytes = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        let size = stream.read(&mut buffer).await.unwrap();
        assert!(size > 0);
        bytes.extend_from_slice(&buffer[..size]);
        if let Some(end) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
            let header = String::from_utf8(bytes[..end].to_vec()).unwrap();
            let length = header
                .lines()
                .find_map(|line| {
                    let (key, value) = line.split_once(':')?;
                    key.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            if bytes.len() >= end + 4 + length {
                return header;
            }
        }
        assert!(bytes.len() <= 128 * 1024);
    }
}

struct Peer {
    count: Arc<AtomicUsize>,
    instance: String,
    workspace: Uuid,
    credential: Uuid,
    path: PathBuf,
    fault: &'static str,
}

impl Peer {
    fn create(&self) -> Value {
        self.count.fetch_add(1, Ordering::SeqCst);
        let mut db = Connection::open(&self.path).unwrap();
        if self.fault == "grant_loss" {
            db.execute("UPDATE ccp_grants SET revoked_at=1", [])
                .unwrap();
        }
        let shelf = AccessStore::new(&mut db).unwrap().create_shelf(
            Actor::Credential(self.credential),
            self.workspace,
            &NodeDraft {
                name: "created".into(),
                description: "exact\nmetadata\0é".into(),
            },
            clock(),
        );
        if matches!(self.fault, "write_fault" | "grant_loss") {
            assert!(shelf.is_err());
            Value::Null
        } else {
            let shelf = shelf.unwrap();
            json!({"id":shelf.id.to_string(),"workspace_id":self.workspace.to_string(),"name":shelf.name,"description":shelf.description,"created_at":shelf.created_at})
        }
    }

    fn reply(&self, header: &str) -> String {
        let mut body = if header.starts_with("GET /api/v1/health ") {
            json!({"instance_id":self.instance})
        } else if header.starts_with("POST /api/v1/content/shelves/list ") {
            json!({"items":[],"next_cursor":null})
        } else {
            assert!(header.starts_with("POST /api/v1/content/shelves/create "));
            self.create()
        };
        let creating = header.starts_with("POST /api/v1/content/shelves/create ");
        if creating && self.fault == "scope" {
            body["workspace_id"] = json!(Uuid::new_v4().to_string());
        }
        if creating && self.fault == "metadata" {
            body["description"] = json!("PRIVATE_VALUE");
        }
        let body = json!({"api_version":1,"data":body}).to_string();
        if creating && self.fault == "truncated" {
            "HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{}".to_owned()
        } else if creating && matches!(self.fault, "refused" | "write_fault" | "grant_loss") {
            "HTTP/1.1 503 Unavailable\r\nContent-Length: 13\r\nConnection: close\r\n\r\nPRIVATE_VALUE".to_owned()
        } else {
            format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
        }
    }

    async fn run(
        self,
        listener: tokio::net::TcpListener,
        acceptor: tokio_rustls::TlsAcceptor,
        mut signal: oneshot::Receiver<()>,
    ) {
        loop {
            let socket = tokio::select! { value = listener.accept() => value.unwrap().0, _ = &mut signal => break };
            let mut stream = acceptor.accept(socket).await.unwrap();
            let header = request(&mut stream).await;
            stream
                .write_all(self.reply(&header).as_bytes())
                .await
                .unwrap();
            stream.shutdown().await.unwrap();
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn committed_creation_with_truncated_refused_or_mismatched_reply_never_retries_the_post() {
    for fault in [
        "truncated",
        "refused",
        "scope",
        "metadata",
        "write_fault",
        "grant_loss",
    ] {
        let f = Fixture::new().await;
        let info = enroll_profile(&f).await;
        if fault == "write_fault" {
            f.db().execute_batch("CREATE TRIGGER fail_shelf BEFORE INSERT ON ccp_content_shelves BEGIN SELECT RAISE(ABORT,'PRIVATE_VALUE'); END;").unwrap();
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let acceptor = tokio_rustls::TlsAcceptor::from(tls(&f));
        let count = Arc::new(AtomicUsize::new(0));
        let peer = Peer {
            count: count.clone(),
            instance: f.trust.instance_id.clone(),
            workspace: f.setup.workspace_id,
            credential: Uuid::parse_str(info.credential_id.as_deref().unwrap()).unwrap(),
            path: f.root.join("server/ccp.sqlite3"),
            fault,
        };
        let (stop, signal) = oneshot::channel::<()>();
        let task = tokio::spawn(peer.run(listener, acceptor, signal));
        let db = Connection::open(f.profile.join("workspace-profile.sqlite3")).unwrap();
        let raw: String = db
            .query_row("SELECT trust FROM profile", [], |r| r.get(0))
            .unwrap();
        let mut trust: Value = serde_json::from_str(&raw).unwrap();
        trust["mtls_origin"] = json!(format!("https://{address}/"));
        db.execute("UPDATE profile SET trust=?1", [trust.to_string()])
            .unwrap();
        drop(db);
        let mut p = WorkspaceProfile::open(&f.profile).unwrap();
        let error = p
            .content_create_shelf(&shelf(&f, "created"))
            .await
            .err()
            .unwrap()
            .to_string();
        assert!(
            error.contains("inspect the explicit parent listing"),
            "{error}"
        );
        assert!(!error.contains("PRIVATE_VALUE"));
        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert_eq!(
            f.count("ccp_content_shelves"),
            if matches!(fault, "write_fault" | "grant_loss") {
                0
            } else {
                1
            }
        );
        assert!(p.info().unwrap().pending_content.is_none());
        assert_eq!(p.info().unwrap().credential_id, info.credential_id);
        drop(p);
        stop.send(()).unwrap();
        task.await.unwrap();
        f.finish().await;
    }
}
