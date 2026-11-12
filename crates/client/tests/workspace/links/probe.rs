// SPDX-License-Identifier: AGPL-3.0-or-later
//! Test-side TLS peer records actual requests; no production fault hooks.
use super::*;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[derive(Clone, Copy)]
enum Mode {
    Normal,
    Redirect,
    Encoded,
    Oversize,
    Chunked,
}
#[derive(Default, Clone)]
struct Seen {
    ca: usize,
    health: usize,
    post: usize,
    secret: usize,
    csr: usize,
    auth: usize,
}
struct Probe {
    origin: String,
    seen: Arc<Mutex<Seen>>,
    stop: oneshot::Sender<()>,
    task: tokio::task::JoinHandle<()>,
}
impl Probe {
    async fn start(f: &Fixture, secret: String, mode: Mode, wrong_leaf: bool) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("https://{}", listener.local_addr().unwrap());
        let (cert, key) = if wrong_leaf {
            let key = rcgen::KeyPair::generate().unwrap();
            let cert = rcgen::CertificateParams::new(vec!["127.0.0.1".into()])
                .unwrap()
                .self_signed(&key)
                .unwrap();
            (cert.der().to_vec(), key.serialize_der())
        } else {
            let bundle: Value = serde_json::from_slice(
                &fs::read(f.root.join("server/ccp-instance-tls.json")).unwrap(),
            )
            .unwrap();
            let (_, cert) = x509_parser::pem::parse_x509_pem(
                bundle["server_certificate"].as_str().unwrap().as_bytes(),
            )
            .unwrap();
            let (_, key) =
                x509_parser::pem::parse_x509_pem(bundle["server_key"].as_str().unwrap().as_bytes())
                    .unwrap();
            (cert.contents, key.contents)
        };
        let config = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![rustls::pki_types::CertificateDer::from(cert)],
            rustls::pki_types::PrivatePkcs8KeyDer::from(key).into(),
        )
        .unwrap();
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
        let (stop, mut stopped) = oneshot::channel();
        let seen = Arc::new(Mutex::new(Seen::default()));
        let observe = seen.clone();
        let ca = f.trust.ca_certificate_pem.clone();
        let instance = f.trust.instance_id.clone();
        let task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _=&mut stopped=>break,
                    accepted=listener.accept()=>{
                        let (tcp,_)=accepted.unwrap();
                        let Ok(Ok(mut tls))=tokio::time::timeout(std::time::Duration::from_secs(2),acceptor.accept(tcp)).await else {continue};
                        let mut bytes=Vec::new();let mut buffer=[0;4096];let mut complete=None;
                        loop {
                            let read=tokio::time::timeout(std::time::Duration::from_secs(2),tls.read(&mut buffer)).await.unwrap().unwrap();
                            if read==0 {break;}
                            bytes.extend_from_slice(&buffer[..read]);
                            assert!(bytes.len()<65536);
                            if let Some(end)=bytes.windows(4).position(|v|v==b"\r\n\r\n") {
                                let header=String::from_utf8_lossy(&bytes[..end]);
                                let length=header.lines().find_map(|line|line.to_ascii_lowercase().strip_prefix("content-length:").and_then(|v|v.trim().parse::<usize>().ok())).unwrap_or(0);
                                if bytes.len()>=end+4+length {complete=Some(end);break;}
                            }
                        }
                        let Some(end)=complete else {continue};
                        let text=String::from_utf8_lossy(&bytes);
                        let header=String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                        let is_ca=header.starts_with("get /api/v1/ca.pem ");
                        let is_health=header.starts_with("get /api/v1/health ");
                        let is_post=header.starts_with("post ");
                        {
                            let mut event=observe.lock().unwrap();
                            event.ca+=usize::from(is_ca);event.health+=usize::from(is_health);event.post+=usize::from(is_post);
                            event.secret+=usize::from(text.contains(&secret));
                            event.csr+=usize::from(text.contains("BEGIN CERTIFICATE REQUEST"));
                            event.auth+=usize::from(header.contains("authorization:")||header.contains("cookie:"));
                        }
                        let response=reply(mode,is_ca,is_health,&ca,&instance);
                        if tls.write_all(response.as_bytes()).await.is_err() {continue;}
                        if tls.shutdown().await.is_err() {continue;}
                    }
                }
            }
        });
        Self {
            origin,
            seen,
            stop,
            task,
        }
    }
    fn seen(&self) -> Seen {
        self.seen.lock().unwrap().clone()
    }
    async fn finish(self) {
        self.stop.send(()).unwrap();
        self.task.await.unwrap();
    }
}
fn reply(mode: Mode, is_ca: bool, is_health: bool, ca: &str, instance: &str) -> String {
    if is_ca && matches!(mode, Mode::Chunked) {
        let body = "a".repeat(16385);
        return format!(
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:X}\r\n{body}\r\n0\r\n\r\n",
            body.len()
        );
    }
    let (status, extra, body) = if is_ca {
        match mode {
            Mode::Redirect => ("302 Found", "Location: /api/v1/health\r\n", String::new()),
            Mode::Encoded => ("200 OK", "Content-Encoding: gzip\r\n", ca.to_owned()),
            Mode::Oversize => ("200 OK", "", "a".repeat(16385)),
            Mode::Normal | Mode::Chunked => ("200 OK", "", ca.to_owned()),
        }
    } else if is_health {
        (
            "200 OK",
            "",
            json!({"api_version":1,"data":{"instance_id":instance}}).to_string(),
        )
    } else {
        ("403 Forbidden", "", "{}".into())
    };
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn actual_transport_observes_no_secret_or_csr_before_pin_and_uuid_validation() {
    let f = Fixture::new().await;
    let secret = f.invitation();
    for kind in 0..3 {
        let probe = Probe::start(&f, secret.clone(), Mode::Normal, kind == 2).await;
        let mut target = link(&f, &secret, f.setup.workspace_id, "first")
            .target()
            .clone();
        target.https_origin = probe.origin.clone();
        if kind == 0 {
            target.ca_sha256[0] ^= 1;
        }
        if kind == 1 {
            target.instance_id = Uuid::new_v4();
        }
        let profiles = f.root.join(format!("probe-{kind}"));
        assert!(
            enroll_link(
                &EnrollmentLink::new(target, &secret).unwrap(),
                None,
                &profiles
            )
            .await
            .is_err()
        );
        let seen = probe.seen();
        assert_eq!(seen.ca, 1);
        if kind == 1 {
            assert_eq!(seen.health, 1);
        }
        assert_eq!(seen.post, 0);
        assert_eq!(seen.secret, 0);
        assert_eq!(seen.csr, 0);
        assert_eq!(seen.auth, 0);
        assert!(!profiles.exists());
        probe.finish().await;
    }
    // Positive control proves this peer observes a real secret-bearing request
    // when all trust checks succeed; its deliberate403 preserves pending state.
    let probe = Probe::start(&f, secret.clone(), Mode::Normal, false).await;
    let mut target = link(&f, &secret, f.setup.workspace_id, "first")
        .target()
        .clone();
    target.https_origin = probe.origin.clone();
    assert!(
        enroll_link(
            &EnrollmentLink::new(target, &secret).unwrap(),
            None,
            &f.root.join("positive")
        )
        .await
        .is_err()
    );
    let seen = probe.seen();
    assert_eq!(seen.post, 1);
    assert_eq!(seen.secret, 1);
    assert_eq!(seen.csr, 1);
    assert_eq!(seen.auth, 0);
    probe.finish().await;
    f.finish().await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn anonymous_ca_fetch_never_follows_redirects_or_accepts_encoding_and_oversize() {
    let f = Fixture::new().await;
    let secret = f.invitation();
    for (index, mode) in [Mode::Redirect, Mode::Encoded, Mode::Oversize, Mode::Chunked]
        .into_iter()
        .enumerate()
    {
        let probe = Probe::start(&f, secret.clone(), mode, false).await;
        let mut target = link(&f, &secret, f.setup.workspace_id, "first")
            .target()
            .clone();
        target.https_origin = probe.origin.clone();
        let profiles = f.root.join(format!("bounds-{index}"));
        assert!(
            enroll_link(
                &EnrollmentLink::new(target, &secret).unwrap(),
                None,
                &profiles
            )
            .await
            .is_err()
        );
        let seen = probe.seen();
        assert_eq!(seen.ca, 1);
        assert_eq!(seen.health, 0);
        assert_eq!(seen.post, 0);
        assert_eq!(seen.secret, 0);
        assert_eq!(seen.csr, 0);
        assert_eq!(seen.auth, 0);
        assert!(!profiles.exists());
        probe.finish().await;
    }
    f.finish().await;
}
