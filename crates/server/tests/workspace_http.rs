// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

#[cfg(test)]
#[path = "."]
mod tests {
    #[path = "workspace_browser/mod.rs"]
    mod browser;
    #[path = "workspace_content/mod.rs"]
    mod content;
    #[path = "workspace_preflight/mod.rs"]
    mod preflight;
    #[path = "workspace_tunnel/mod.rs"]
    mod tunnel;
    use anyhow::Context;
    use rcgen::{CertificateParams, KeyPair};
    use reqwest::Client;
    use rusqlite::Connection;
    use rustls::{
        ClientConfig, RootCertStore,
        pki_types::{CertificateDer, PrivatePkcs8KeyDer, ServerName, pem::PemObject},
    };
    use serde_json::{Value, json};
    use server::foundation::{
        AccessStore, BootstrappedInstance, InitialOwner, OwnerProvisioning, ProvisionedWorkspace,
    };
    use server::instance_tls::TrustInfo;
    use server::workspace_http::{Config, Report, Startup, WorkspaceServer};
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;
    use tokio::sync::oneshot;
    use tokio_rustls::{TlsConnector, client::TlsStream};
    use uuid::Uuid;

    fn now() -> i64 {
        i64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        )
        .unwrap()
    }
    fn csr(key: &KeyPair) -> String {
        CertificateParams::new(Vec::<String>::new())
            .unwrap()
            .serialize_request(key)
            .unwrap()
            .pem()
            .unwrap()
    }
    fn tls_config(ca: Option<&str>, identity: Option<(&str, &KeyPair)>) -> ClientConfig {
        let mut roots = RootCertStore::empty();
        if let Some(ca) = ca {
            roots
                .add(CertificateDer::from_pem_slice(ca.as_bytes()).unwrap())
                .unwrap();
        }
        let builder =
            ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_root_certificates(roots);
        let mut config = match identity {
            Some((pem, key)) => builder
                .with_client_auth_cert(
                    vec![CertificateDer::from_pem_slice(pem.as_bytes()).unwrap()],
                    PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
                )
                .unwrap(),
            None => builder.with_no_client_auth(),
        };
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        config
    }
    fn client(config: ClientConfig) -> Client {
        Client::builder()
            .no_proxy()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(5))
            .use_preconfigured_tls(config)
            .build()
            .unwrap()
    }
    async fn json_reply(response: reqwest::Response, status: u16) -> Value {
        assert_eq!(response.status().as_u16(), status);
        assert_eq!(response.headers().get("cache-control").unwrap(), "no-store");
        let value: Value = response.json().await.unwrap();
        assert_eq!(value["api_version"], 1);
        value
    }

    struct Fixture {
        directory: PathBuf,
        setup: BootstrappedInstance,
        trust: TrustInfo,
        startup: Startup,
        stop: oneshot::Sender<()>,
        task: tokio::task::JoinHandle<anyhow::Result<Report>>,
    }
    impl Fixture {
        async fn new() -> Self {
            Self::with_tunnels(false).await
        }
        async fn with_tunnels(enabled: bool) -> Self {
            let directory = std::env::temp_dir().join(format!("ccp-http-{}", Uuid::new_v4()));
            let setup = server::local_admin::bootstrap(&directory, "operator", "first").unwrap();
            let trust = server::instance_tls::initialize(
                &directory,
                &["localhost".into(), "127.0.0.1".into()],
            )
            .unwrap();
            let server = WorkspaceServer::bind(Config {
                data_dir: directory.clone(),
                https_listen: "127.0.0.1:0".parse().unwrap(),
                mtls_listen: "127.0.0.1:0".parse().unwrap(),
                activate_workspace: true,
            })
            .await
            .unwrap()
            .with_websocket_tunnels(enabled);
            let startup = server.startup().unwrap();
            let (stop, signal) = oneshot::channel();
            let task = tokio::spawn(
                server.run_until(async { signal.await.context("test shutdown sender dropped") }),
            );
            Self {
                directory,
                setup,
                trust,
                startup,
                stop,
                task,
            }
        }
        fn public_client(&self) -> Client {
            client(tls_config(Some(&self.trust.ca_certificate_pem), None))
        }
        fn private_client(&self, cert: &str, key: &KeyPair) -> Client {
            client(tls_config(
                Some(&self.trust.ca_certificate_pem),
                Some((cert, key)),
            ))
        }
        fn url(&self, path: &str) -> String {
            format!("https://{}{path}", self.startup.https_address)
        }
        fn mtls(&self, path: &str) -> String {
            format!("https://{}{path}", self.startup.mtls_address)
        }
        fn db(&self) -> Connection {
            Connection::open(self.directory.join("ccp.sqlite3")).unwrap()
        }
        async fn initial(&self, key: &str, workspace: Uuid) -> Value {
            let response=self.public_client().post(self.url("/api/v1/admin/invitations/initial")).bearer_auth(key).json(&json!({"workspace_id":workspace.to_string(),"role":"read_write","max_clients":1,"expires_at":now()+3600})).send().await.unwrap();
            json_reply(response, 200).await["data"].clone()
        }
        async fn enroll(&self) -> (Value, KeyPair) {
            let token = self
                .initial(self.setup.owner_key.secret(), self.setup.workspace_id)
                .await;
            let key = KeyPair::generate().unwrap();
            let body = json!({"invitation":token["secret"],"request_id":Uuid::new_v4().to_string(),"csr_pem":csr(&key)});
            let receipt = json_reply(
                self.public_client()
                    .post(self.url("/api/v1/enroll"))
                    .json(&body)
                    .send()
                    .await
                    .unwrap(),
                200,
            )
            .await;
            (receipt["data"].clone(), key)
        }
        fn second(&self) -> ProvisionedWorkspace {
            let mut db = self.db();
            let mut store = AccessStore::new(&mut db).unwrap();
            let actor = store
                .authenticate_admin_key(self.setup.instance_key.secret(), now())
                .unwrap();
            store
                .create_workspace_with_owner_key(
                    actor,
                    "second",
                    &OwnerProvisioning {
                        owner: InitialOwner::Existing(self.setup.instance_key.principal_id),
                        key_expires_at: None,
                    },
                    now(),
                )
                .unwrap()
        }
        async fn finish(self) -> Report {
            self.stop.send(()).unwrap();
            let report = self.task.await.unwrap().unwrap();
            assert!(server::directory_mode::DirectoryLease::native(&self.directory).is_err());
            {
                let lease =
                    server::directory_mode::DirectoryLease::maintenance(&self.directory).unwrap();
                assert_eq!(
                    lease.mode(),
                    server::directory_mode::DirectoryMode::Workspace
                );
            }
            std::fs::remove_dir_all(self.directory).unwrap();
            report
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn real_https_scope_invitation_enrollment_and_accessible_only_mtls_discovery() {
        let f = Fixture::new().await;
        let public = f.public_client();
        let health = json_reply(
            public.get(f.url("/api/v1/health")).send().await.unwrap(),
            200,
        )
        .await;
        assert_eq!(health["data"]["instance_id"], f.trust.instance_id);
        json_reply(
            public
                .get(f.url("/api/v1/admin/workspaces"))
                .send()
                .await
                .unwrap(),
            401,
        )
        .await;
        let second = f.second();
        let owned = json_reply(
            public
                .get(f.url("/api/v1/admin/workspaces"))
                .bearer_auth(f.setup.owner_key.secret())
                .send()
                .await
                .unwrap(),
            200,
        )
        .await;
        assert_eq!(owned["data"]["workspaces"].as_array().unwrap().len(), 1);
        let global = json_reply(
            public
                .get(f.url("/api/v1/admin/workspaces"))
                .bearer_auth(f.setup.instance_key.secret())
                .send()
                .await
                .unwrap(),
            200,
        )
        .await;
        assert_eq!(global["data"]["workspaces"].as_array().unwrap().len(), 2);
        json_reply(public.post(f.url("/api/v1/admin/invitations/initial")).bearer_auth(f.setup.instance_key.secret()).json(&json!({"workspace_id":second.workspace_id.to_string(),"role":"read","max_clients":1,"expires_at":null})).send().await.unwrap(),403).await;
        let (receipt, key) = f.enroll().await;
        assert!(!receipt.to_string().contains("PRIVATE KEY"));
        let private = f.private_client(receipt["certificate_pem"].as_str().unwrap(), &key);
        let list = json_reply(
            private
                .get(f.mtls("/api/v1/workspaces"))
                .send()
                .await
                .unwrap(),
            200,
        )
        .await;
        assert_eq!(list["data"]["workspaces"].as_array().unwrap().len(), 1);
        assert_eq!(
            list["data"]["workspaces"][0]["id"],
            f.setup.workspace_id.to_string()
        );
        assert!(
            public
                .get(f.mtls("/api/v1/workspaces"))
                .send()
                .await
                .is_err()
        );
        json_reply(
            public
                .get(f.url("/api/v1/workspaces"))
                .send()
                .await
                .unwrap(),
            404,
        )
        .await;
        drop(private);
        drop(public);
        f.finish().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn existing_client_admission_reuses_identity_only_through_actual_mtls_peer() {
        let f = Fixture::new().await;
        let (receipt, key) = f.enroll().await;
        let second = f.second();
        let token = f
            .initial(second.owner_key.secret(), second.workspace_id)
            .await;
        let private = f.private_client(receipt["certificate_pem"].as_str().unwrap(), &key);
        let body = json!({"invitation":token["secret"],"request_id":Uuid::new_v4().to_string(),"csr_pem":csr(&key)});
        let mut spoof = body.clone();
        spoof["existing_credential"] = receipt["credential_id"].clone();
        json_reply(
            f.public_client()
                .post(f.url("/api/v1/enroll"))
                .json(&spoof)
                .send()
                .await
                .unwrap(),
            400,
        )
        .await;
        json_reply(
            private
                .post(f.mtls("/api/v1/admissions"))
                .json(&spoof)
                .send()
                .await
                .unwrap(),
            400,
        )
        .await;
        let admitted = json_reply(
            private
                .post(f.mtls("/api/v1/admissions"))
                .json(&body)
                .send()
                .await
                .unwrap(),
            200,
        )
        .await;
        assert_eq!(admitted["data"]["client_id"], receipt["client_id"]);
        assert_eq!(admitted["data"]["credential_id"], receipt["credential_id"]);
        let list = json_reply(
            private
                .get(f.mtls("/api/v1/workspaces"))
                .send()
                .await
                .unwrap(),
            200,
        )
        .await;
        assert_eq!(list["data"]["workspaces"].as_array().unwrap().len(), 2);
        let clients: i64 = f
            .db()
            .query_row("SELECT COUNT(*) FROM ccp_clients", [], |r| r.get(0))
            .unwrap();
        assert_eq!(clients, 1);
        drop(private);
        f.finish().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn network_renewal_is_exact_retry_only_for_the_superseded_peer() {
        let f = Fixture::new().await;
        let (old, key) = f.enroll().await;
        let private = f.private_client(old["certificate_pem"].as_str().unwrap(), &key);
        let nextkey = KeyPair::generate().unwrap();
        let body = json!({"request_id":Uuid::new_v4().to_string(),"csr_pem":csr(&nextkey)});
        let next = json_reply(
            private
                .post(f.mtls("/api/v1/credentials/renew"))
                .json(&body)
                .send()
                .await
                .unwrap(),
            200,
        )
        .await;
        json_reply(
            private
                .get(f.mtls("/api/v1/workspaces"))
                .send()
                .await
                .unwrap(),
            403,
        )
        .await;
        let retry = json_reply(
            private
                .post(f.mtls("/api/v1/credentials/renew"))
                .json(&body)
                .send()
                .await
                .unwrap(),
            200,
        )
        .await;
        assert_eq!(retry, next);
        assert_eq!(next["data"]["client_id"], old["client_id"]);
        let wrong = json!({"request_id":Uuid::new_v4().to_string(),"csr_pem":csr(&nextkey)});
        json_reply(
            private
                .post(f.mtls("/api/v1/credentials/renew"))
                .json(&wrong)
                .send()
                .await
                .unwrap(),
            403,
        )
        .await;
        let newclient =
            f.private_client(next["data"]["certificate_pem"].as_str().unwrap(), &nextkey);
        json_reply(
            newclient
                .get(f.mtls("/api/v1/workspaces"))
                .send()
                .await
                .unwrap(),
            200,
        )
        .await;
        f.db()
            .execute("UPDATE ccp_clients SET disabled=1", [])
            .unwrap();
        json_reply(
            private
                .post(f.mtls("/api/v1/credentials/renew"))
                .json(&body)
                .send()
                .await
                .unwrap(),
            403,
        )
        .await;
        json_reply(
            newclient
                .get(f.mtls("/api/v1/workspaces"))
                .send()
                .await
                .unwrap(),
            403,
        )
        .await;
        drop(private);
        drop(newclient);
        f.finish().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn tls_ca_and_name_are_verified_before_enrollment_secrets_are_sent() {
        let f = Fixture::new().await;
        let token = f
            .initial(f.setup.owner_key.secret(), f.setup.workspace_id)
            .await;
        let key = KeyPair::generate().unwrap();
        let body = json!({"invitation":token["secret"],"request_id":Uuid::new_v4().to_string(),"csr_pem":csr(&key)});
        let untrusted = client(tls_config(None, None));
        assert!(
            untrusted
                .post(f.url("/api/v1/enroll"))
                .json(&body)
                .send()
                .await
                .is_err()
        );
        let wrong = Client::builder()
            .no_proxy()
            .https_only(true)
            .use_preconfigured_tls(tls_config(Some(&f.trust.ca_certificate_pem), None))
            .resolve("wrong.example", f.startup.https_address)
            .build()
            .unwrap();
        let wrongurl = format!(
            "https://wrong.example:{}/api/v1/enroll",
            f.startup.https_address.port()
        );
        assert!(wrong.post(wrongurl).json(&body).send().await.is_err());
        let clients: i64 = f
            .db()
            .query_row("SELECT COUNT(*) FROM ccp_clients", [], |r| r.get(0))
            .unwrap();
        assert_eq!(clients, 0);
        json_reply(
            f.public_client()
                .post(f.url("/api/v1/enroll"))
                .json(&body)
                .send()
                .await
                .unwrap(),
            200,
        )
        .await;
        drop(untrusted);
        drop(wrong);
        f.finish().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn body_origin_role_and_secret_errors_are_bounded_uniform_and_do_not_consume_invites() {
        let f = Fixture::new().await;
        let public = f.public_client();
        let path = f.url("/api/v1/admin/invitations/initial");
        let body = json!({"workspace_id":f.setup.workspace_id.to_string(),"role":"admin","max_clients":1,"expires_at":null});
        json_reply(
            public
                .post(&path)
                .bearer_auth(f.setup.owner_key.secret())
                .json(&body)
                .send()
                .await
                .unwrap(),
            400,
        )
        .await;
        json_reply(
            public
                .post(&path)
                .bearer_auth(f.setup.owner_key.secret())
                .header("origin", "https://attacker.example")
                .json(&body)
                .send()
                .await
                .unwrap(),
            403,
        )
        .await;
        json_reply(
            public
                .post(&path)
                .bearer_auth(f.setup.owner_key.secret())
                .header("content-type", "text/plain")
                .body("{}")
                .send()
                .await
                .unwrap(),
            415,
        )
        .await;
        json_reply(
            public
                .post(&path)
                .bearer_auth(f.setup.owner_key.secret())
                .header("content-type", "application/json")
                .body("x".repeat(65537))
                .send()
                .await
                .unwrap(),
            413,
        )
        .await;
        let invalid = json_reply(
            public
                .get(f.url("/api/v1/admin/workspaces"))
                .bearer_auth("PRIVATE_SENTINEL")
                .send()
                .await
                .unwrap(),
            401,
        )
        .await;
        assert_eq!(
            invalid,
            json!({"api_version":1,"error":{"code":"unauthorized"}})
        );
        let rows: i64 = f
            .db()
            .query_row("SELECT COUNT(*) FROM ccp_invitations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 0);
        drop(public);
        f.finish().await;
    }
    struct Wire<S = TlsStream<TcpStream>> {
        stream: S,
        buffer: Vec<u8>,
    }
    impl Wire {
        async fn connect(f: &Fixture, identity: Option<(&str, &KeyPair)>) -> Self {
            let address = if identity.is_some() {
                f.startup.mtls_address
            } else {
                f.startup.https_address
            };
            let socket = TcpStream::connect(address).await.unwrap();
            let stream = TlsConnector::from(Arc::new(tls_config(
                Some(&f.trust.ca_certificate_pem),
                identity,
            )))
            .connect(ServerName::try_from("localhost").unwrap(), socket)
            .await
            .unwrap();
            Self {
                stream,
                buffer: Vec::new(),
            }
        }
    }
    impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin> Wire<S> {
        async fn send(&mut self, request: &str) {
            self.stream.write_all(request.as_bytes()).await.unwrap();
            self.stream.flush().await.unwrap();
        }
        async fn response(&mut self) -> (u16, Value) {
            self.response_timeout(Duration::from_secs(3)).await
        }
        async fn response_timeout(&mut self, deadline: Duration) -> (u16, Value) {
            let end = loop {
                if let Some(end) = self.buffer.windows(4).position(|v| v == b"\r\n\r\n") {
                    break end + 4;
                }
                let mut bytes = [0; 4096];
                let count = tokio::time::timeout(deadline, self.stream.read(&mut bytes))
                    .await
                    .unwrap()
                    .unwrap();
                assert!(count > 0);
                self.buffer.extend_from_slice(&bytes[..count]);
            };
            let (status, length) = {
                let mut headers = [httparse::EMPTY_HEADER; 32];
                let mut parsed = httparse::Response::new(&mut headers);
                assert!(parsed.parse(&self.buffer[..end]).unwrap().is_complete());
                let length = parsed
                    .headers
                    .iter()
                    .find(|h| h.name.eq_ignore_ascii_case("content-length"))
                    .map_or(0, |h| {
                        std::str::from_utf8(h.value)
                            .unwrap()
                            .parse::<usize>()
                            .unwrap()
                    });
                (parsed.code.unwrap(), length)
            };
            while self.buffer.len() < end + length {
                let mut bytes = [0; 4096];
                let count = tokio::time::timeout(deadline, self.stream.read(&mut bytes))
                    .await
                    .unwrap()
                    .unwrap();
                assert!(count > 0);
                self.buffer.extend_from_slice(&bytes[..count]);
            }
            let body = if length == 0 {
                Value::Null
            } else {
                serde_json::from_slice(&self.buffer[end..end + length]).unwrap()
            };
            self.buffer.drain(..end + length);
            (status, body)
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn same_connection_rechecks_client_disable_and_admin_key_revocation_each_request() {
        let f = Fixture::new().await;
        let (receipt, key) = f.enroll().await;
        let mut wire = Wire::connect(
            &f,
            Some((receipt["certificate_pem"].as_str().unwrap(), &key)),
        )
        .await;
        let get = "GET /api/v1/workspaces HTTP/1.1\r\nHost: localhost\r\n\r\n";
        wire.send(get).await;
        assert_eq!(wire.response().await.0, 200);
        f.db()
            .execute("UPDATE ccp_clients SET disabled=1", [])
            .unwrap();
        wire.send(get).await;
        assert_eq!(wire.response().await.0, 403);
        drop(wire);
        let mut admin = Wire::connect(&f, None).await;
        let get = format!(
            "GET /api/v1/admin/workspaces HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {}\r\n\r\n",
            f.setup.owner_key.secret()
        );
        admin.send(&get).await;
        assert_eq!(admin.response().await.0, 200);
        f.db()
            .execute(
                "UPDATE ccp_admin_keys SET revoked_at=1 WHERE id=?1",
                [f.setup.owner_key.id.to_string()],
            )
            .unwrap();
        admin.send(&get).await;
        assert_eq!(admin.response().await.0, 401);
        drop(admin);
        f.finish().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn hyper_handles_pipelining_chunked_json_and_rejects_conflicting_framing_without_mutation()
     {
        let f = Fixture::new().await;
        let mut wire = Wire::connect(&f, None).await;
        let get = "GET /api/v1/health HTTP/1.1\r\nHost: localhost\r\n\r\n";
        wire.send(&format!("{get}{get}")).await;
        assert_eq!(wire.response().await.0, 200);
        assert_eq!(wire.response().await.0, 200);
        let body=json!({"workspace_id":f.setup.workspace_id.to_string(),"role":"read","max_clients":1,"expires_at":null}).to_string();
        let request = format!(
            "POST /api/v1/admin/invitations/initial HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{}\r\n0\r\n\r\n",
            f.setup.owner_key.secret(),
            body.len(),
            body
        );
        wire.send(&request).await;
        assert_eq!(wire.response().await.0, 200);
        drop(wire);
        let mut bad = Wire::connect(&f, None).await;
        bad.send("POST /api/v1/enroll HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: 1\r\nContent-Length: 2\r\n\r\n{}").await;
        assert_eq!(bad.response().await.0, 400);
        drop(bad);
        let clients: i64 = f
            .db()
            .query_row("SELECT COUNT(*) FROM ccp_clients", [], |r| r.get(0))
            .unwrap();
        assert_eq!(clients, 0);
        let mut header = Wire::connect(&f, None).await;
        header
            .send(&format!(
                "GET /api/v1/health HTTP/1.1\r\nHost: localhost\r\nX-Padding: {}\r\n\r\n",
                "x".repeat(8200)
            ))
            .await;
        assert_eq!(header.response().await.0, 431);
        drop(header);
        f.finish().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn lost_enrollment_response_replays_one_registration_and_raw_plaintext_is_not_http() {
        let f = Fixture::new().await;
        let token = f
            .initial(f.setup.owner_key.secret(), f.setup.workspace_id)
            .await;
        let key = KeyPair::generate().unwrap();
        let body = json!({"invitation":token["secret"],"request_id":Uuid::new_v4().to_string(),"csr_pem":csr(&key)});
        let bytes = body.to_string();
        let mut wire = Wire::connect(&f, None).await;
        wire.send(&format!("POST /api/v1/enroll HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",bytes.len(),bytes)).await;
        drop(wire);
        let client = f.public_client();
        let first = json_reply(
            client
                .post(f.url("/api/v1/enroll"))
                .json(&body)
                .send()
                .await
                .unwrap(),
            200,
        )
        .await;
        let second = json_reply(
            client
                .post(f.url("/api/v1/enroll"))
                .json(&body)
                .send()
                .await
                .unwrap(),
            200,
        )
        .await;
        assert_eq!(first, second);
        let clients: i64 = f
            .db()
            .query_row("SELECT COUNT(*) FROM ccp_clients", [], |r| r.get(0))
            .unwrap();
        assert_eq!(clients, 1);
        let mut plain = TcpStream::connect(f.startup.https_address).await.unwrap();
        plain
            .write_all(b"GET /api/v1/health HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await
            .unwrap();
        let mut bytes = [0; 64];
        let response = tokio::time::timeout(Duration::from_secs(2), plain.read(&mut bytes))
            .await
            .unwrap();
        if let Ok(count) = response {
            assert!(!bytes[..count].starts_with(b"HTTP/1.1 200"));
        }
        drop(plain);
        drop(client);
        f.finish().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn first_workspace_activation_is_explicit_and_failed_bind_preserves_native_mode() {
        let directory =
            std::env::temp_dir().join(format!("ccp-http-activation-{}", Uuid::new_v4()));
        server::local_admin::bootstrap(&directory, "operator", "first").unwrap();
        server::instance_tls::initialize(&directory, &["localhost".into()]).unwrap();
        let config = |activate| Config {
            data_dir: directory.clone(),
            https_listen: "127.0.0.1:0".parse().unwrap(),
            mtls_listen: "127.0.0.1:0".parse().unwrap(),
            activate_workspace: activate,
        };
        assert!(WorkspaceServer::bind(config(false)).await.is_err());
        {
            let native = server::directory_mode::DirectoryLease::native(&directory).unwrap();
            drop(native);
        }
        let occupied = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut blocked = config(true);
        blocked.https_listen = occupied.local_addr().unwrap();
        assert!(WorkspaceServer::bind(blocked).await.is_err());
        drop(occupied);
        {
            let native = server::directory_mode::DirectoryLease::native(&directory).unwrap();
            drop(native);
        }
        let first = WorkspaceServer::bind(config(true)).await.unwrap();
        first.run_until(async { Ok(()) }).await.unwrap();
        assert!(server::directory_mode::DirectoryLease::native(&directory).is_err());
        let resumed = WorkspaceServer::bind(config(false)).await.unwrap();
        resumed.run_until(async { Ok(()) }).await.unwrap();
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn connection_admission_is_bounded_and_recovers_after_incomplete_handshakes_close() {
        let f = Fixture::new().await;
        let mut pending = Vec::new();
        for _ in 0..64 {
            pending.push(TcpStream::connect(f.startup.https_address).await.unwrap());
        }
        tokio::time::sleep(Duration::from_millis(40)).await;
        let blocked = f.public_client().get(f.url("/api/v1/health")).send().await;
        assert!(blocked.is_err());
        drop(pending);
        let client = f.public_client();
        let mut recovered = false;
        for _ in 0..20 {
            if let Ok(response) = client.get(f.url("/api/v1/health")).send().await {
                assert_eq!(response.status().as_u16(), 200);
                recovered = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(recovered);
        drop(client);
        let report = f.finish().await;
        assert!(report.failed_connections >= 64);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn individual_invites_and_quota_races_use_current_scope_and_retain_one_registration() {
        let f = Fixture::new().await;
        let token = f
            .initial(f.setup.owner_key.secret(), f.setup.workspace_id)
            .await;
        let a = KeyPair::generate().unwrap();
        let b = KeyPair::generate().unwrap();
        let one = json!({"invitation":token["secret"],"request_id":Uuid::new_v4().to_string(),"csr_pem":csr(&a)});
        let two = json!({"invitation":token["secret"],"request_id":Uuid::new_v4().to_string(),"csr_pem":csr(&b)});
        let client = f.public_client();
        let (one, two) = tokio::join!(
            client.post(f.url("/api/v1/enroll")).json(&one).send(),
            client.post(f.url("/api/v1/enroll")).json(&two).send()
        );
        let mut statuses = vec![
            one.unwrap().status().as_u16(),
            two.unwrap().status().as_u16(),
        ];
        statuses.sort();
        assert_eq!(statuses, vec![200, 403]);
        let invitation=json_reply(client.post(f.url("/api/v1/admin/invitations/individual")).bearer_auth(f.setup.owner_key.secret()).json(&json!({"workspace_id":f.setup.workspace_id.to_string(),"role":"read","expires_at":now()+3600})).send().await.unwrap(),200).await;
        let third = json!({"invitation":invitation["data"]["secret"],"request_id":Uuid::new_v4().to_string(),"csr_pem":csr(&KeyPair::generate().unwrap())});
        json_reply(
            client
                .post(f.url("/api/v1/enroll"))
                .json(&third)
                .send()
                .await
                .unwrap(),
            200,
        )
        .await;
        let clients: i64 = f
            .db()
            .query_row("SELECT COUNT(*) FROM ccp_clients", [], |r| r.get(0))
            .unwrap();
        assert_eq!(clients, 2);
        drop(client);
        f.finish().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn framing_authority_and_request_budget_are_enforced_on_real_tls_connections() {
        let f = Fixture::new().await;
        for request in [
            "GET /api/v1/health HTTP/1.1\r\n\r\n",
            "GET /api/v1/health HTTP/1.1\r\nHost: localhost\r\nHost: duplicate\r\n\r\n",
            "GET /api/v1/health HTTP/1.1\r\nHost: localhost:bad\r\n\r\n",
            "GET /api/v1/health?key=PRIVATE_SENTINEL HTTP/1.1\r\nHost: localhost\r\n\r\n",
        ] {
            let mut wire = Wire::connect(&f, None).await;
            wire.send(request).await;
            assert_eq!(wire.response().await.0, 400);
        }
        let mut budget = Wire::connect(&f, None).await;
        for _ in 0..100 {
            budget
                .send("GET /api/v1/health HTTP/1.1\r\nHost: localhost\r\n\r\n")
                .await;
            assert_eq!(budget.response().await.0, 200);
        }
        let mut byte = [0];
        let eof = tokio::time::timeout(Duration::from_secs(2), budget.stream.read(&mut byte))
            .await
            .unwrap();
        assert!(eof.is_err() || eof.unwrap() == 0);
        drop(budget);
        let mut large = Wire::connect(&f, None).await;
        large.send("POST /api/v1/enroll HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: 99999999\r\n\r\n").await;
        assert_eq!(large.response().await.0, 413);
        drop(large);
        f.finish().await;
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn real_cli_starts_tls_only_workspace_listeners_and_stops_without_printing_secrets() {
        use tokio::io::AsyncBufReadExt;
        let directory = std::env::temp_dir().join(format!("ccp-http-cli-{}", Uuid::new_v4()));
        server::local_admin::bootstrap(&directory, "operator", "first").unwrap();
        let trust =
            server::instance_tls::initialize(&directory, &["localhost".into(), "127.0.0.1".into()])
                .unwrap();
        let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_server"))
            .arg("workspace-serve")
            .arg("--data-dir")
            .arg(&directory)
            .args([
                "--activate-workspace",
                "--https-listen",
                "127.0.0.1:0",
                "--mtls-listen",
                "127.0.0.1:0",
            ])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let mut reader = tokio::io::BufReader::new(child.stdout.take().unwrap());
        let mut startup = String::new();
        let data = loop {
            let count =
                tokio::time::timeout(Duration::from_secs(5), reader.read_line(&mut startup))
                    .await
                    .unwrap()
                    .unwrap();
            assert!(count > 0);
            if let Ok(value) = serde_json::from_str::<Value>(&startup) {
                break value;
            }
        };
        assert!(!startup.contains("PRIVATE KEY"));
        assert!(!startup.contains("ccp_admin_"));
        let client = client(tls_config(Some(&trust.ca_certificate_pem), None));
        json_reply(
            client
                .get(format!(
                    "https://{}/api/v1/health",
                    data["https_address"].as_str().unwrap()
                ))
                .send()
                .await
                .unwrap(),
            200,
        )
        .await;
        let status = tokio::process::Command::new("kill")
            .arg("-INT")
            .arg(child.id().unwrap().to_string())
            .status()
            .await
            .unwrap();
        assert!(status.success());
        assert!(
            tokio::time::timeout(Duration::from_secs(5), child.wait())
                .await
                .unwrap()
                .unwrap()
                .success()
        );
        let mut remaining = String::new();
        reader.read_to_string(&mut remaining).await.unwrap();
        assert!(!remaining.contains("ccp_admin_"));
        assert!(!remaining.contains("PRIVATE KEY"));
        drop(client);
        drop(reader);
        drop(child);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn incomplete_bodies_time_out_without_dispatching_and_te_cl_closes_before_pipeline() {
        let f = Fixture::new().await;
        let mut slow = Wire::connect(&f, None).await;
        slow.send("POST /api/v1/enroll HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: 20\r\n\r\n{").await;
        assert_eq!(slow.response_timeout(Duration::from_secs(15)).await.0, 408);
        drop(slow);
        let clients: i64 = f
            .db()
            .query_row("SELECT COUNT(*) FROM ccp_clients", [], |r| r.get(0))
            .unwrap();
        assert_eq!(clients, 0);
        let body=json!({"workspace_id":f.setup.workspace_id.to_string(),"role":"read","max_clients":1,"expires_at":null}).to_string();
        let appended = format!(
            "POST /api/v1/admin/invitations/initial HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            f.setup.owner_key.secret(),
            body.len(),
            body
        );
        let mut ambiguous = Wire::connect(&f, None).await;
        ambiguous.send(&format!("GET /api/v1/health HTTP/1.1\r\nHost: localhost\r\nContent-Length: 999\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n{appended}")).await;
        assert_eq!(ambiguous.response().await.0, 200);
        let mut byte = [0];
        let eof = tokio::time::timeout(Duration::from_secs(2), ambiguous.stream.read(&mut byte))
            .await
            .unwrap();
        assert!(eof.is_err() || eof.unwrap() == 0);
        drop(ambiguous);
        let invitations: i64 = f
            .db()
            .query_row("SELECT COUNT(*) FROM ccp_invitations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(invitations, 0);
        f.finish().await;
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn paged_discovery_keeps_cursor_scope_and_limits_without_global_role_borrowing() {
        let f = Fixture::new().await;
        {
            let mut db = f.db();
            let mut store = AccessStore::new(&mut db).unwrap();
            let actor = store
                .authenticate_admin_key(f.setup.instance_key.secret(), now())
                .unwrap();
            for name in ["zeta", "γ", "aardvark"] {
                store
                    .create_workspace_with_owner_key(
                        actor,
                        name,
                        &OwnerProvisioning {
                            owner: InitialOwner::Existing(f.setup.instance_key.principal_id),
                            key_expires_at: None,
                        },
                        now(),
                    )
                    .unwrap();
            }
        }
        let public = f.public_client();
        let mut cursor = None;
        let mut all = Vec::new();
        loop {
            let mut request = public
                .get(f.url("/api/v1/admin/workspaces"))
                .bearer_auth(f.setup.instance_key.secret())
                .query(&[("limit", "2")]);
            if let Some(after) = &cursor {
                request = request.query(&[("after", after)]);
            }
            let response = json_reply(request.send().await.unwrap(), 200).await;
            let page = response["data"]["workspaces"].as_array().unwrap();
            assert!(page.len() <= 2);
            all.extend(page.iter().map(|w| w["id"].as_str().unwrap().to_string()));
            match response["data"]["next_cursor"].as_str() {
                Some(value) => cursor = Some(value.to_string()),
                None => break,
            }
        }
        all.sort();
        all.dedup();
        assert_eq!(all.len(), 4);
        for query in [
            "limit=0",
            "limit=501",
            "limit=1&limit=2",
            "actor=global",
            "after=invalid",
        ] {
            json_reply(
                public
                    .get(format!("{}?{query}", f.url("/api/v1/admin/workspaces")))
                    .bearer_auth(f.setup.instance_key.secret())
                    .send()
                    .await
                    .unwrap(),
                400,
            )
            .await;
        }
        let scoped = json_reply(
            public
                .get(f.url("/api/v1/admin/workspaces"))
                .bearer_auth(f.setup.owner_key.secret())
                .query(&[("limit", "1")])
                .send()
                .await
                .unwrap(),
            200,
        )
        .await;
        assert_eq!(scoped["data"]["workspaces"].as_array().unwrap().len(), 1);
        assert!(scoped["data"]["next_cursor"].is_null());
        drop(public);
        f.finish().await;
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shutdown_waits_for_disconnected_atomic_database_work_and_retains_the_directory_guard()
    {
        let f = Fixture::new().await;
        let token = f
            .initial(f.setup.owner_key.secret(), f.setup.workspace_id)
            .await;
        let key = KeyPair::generate().unwrap();
        let body=json!({"invitation":token["secret"],"request_id":Uuid::new_v4().to_string(),"csr_pem":csr(&key)}).to_string();
        let lock = f.db();
        lock.execute_batch("BEGIN IMMEDIATE").unwrap();
        let mut wire = Wire::connect(&f, None).await;
        wire.send(&format!("POST /api/v1/enroll HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",body.len(),body)).await;
        // The completed body is available to the other runtime worker while
        // SQLite's write lock deliberately holds the atomic job in progress.
        tokio::time::sleep(Duration::from_millis(250)).await;
        drop(wire);
        let Fixture {
            directory,
            stop,
            task,
            ..
        } = f;
        stop.send(()).unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(!task.is_finished());
        assert!(server::directory_mode::DirectoryLease::maintenance(&directory).is_err());
        lock.execute_batch("ROLLBACK").unwrap();
        drop(lock);
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        {
            let guard = server::directory_mode::DirectoryLease::maintenance(&directory).unwrap();
            let db = Connection::open(directory.join("ccp.sqlite3")).unwrap();
            let clients: i64 = db
                .query_row("SELECT COUNT(*) FROM ccp_clients", [], |r| r.get(0))
                .unwrap();
            assert_eq!(clients, 1);
            drop(db);
            drop(guard);
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
    async fn admin_action(f: &Fixture, key: &str, path: &str, body: &Value, status: u16) -> Value {
        json_reply(
            f.public_client()
                .post(f.url(path))
                .bearer_auth(key)
                .json(body)
                .send()
                .await
                .unwrap(),
            status,
        )
        .await["data"]
            .clone()
    }
    async fn public_action(f: &Fixture, path: &str, body: &Value, status: u16) -> Value {
        json_reply(
            f.public_client()
                .post(f.url(path))
                .json(body)
                .send()
                .await
                .unwrap(),
            status,
        )
        .await
    }
    async fn joined_pair(f: &Fixture) -> (Value, KeyPair, ProvisionedWorkspace) {
        let (old, key) = f.enroll().await;
        let second = f.second();
        let invite = f
            .initial(second.owner_key.secret(), second.workspace_id)
            .await;
        let private = f.private_client(old["certificate_pem"].as_str().unwrap(), &key);
        json_reply(private.post(f.mtls("/api/v1/admissions"))
            .json(&json!({"invitation":invite["secret"],"request_id":Uuid::new_v4().to_string(),"csr_pem":csr(&key)}))
            .send().await.unwrap(),200).await;
        (old, key, second)
    }
    async fn workspace_ids(f: &Fixture, receipt: &Value, key: &KeyPair) -> Vec<String> {
        let reply = json_reply(
            f.private_client(receipt["certificate_pem"].as_str().unwrap(), key)
                .get(f.mtls("/api/v1/workspaces"))
                .send()
                .await
                .unwrap(),
            200,
        )
        .await;
        reply["data"]["workspaces"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["id"].as_str().unwrap().to_string())
            .collect()
    }
    async fn recovery_ticket(f: &Fixture, admin_key: &str, old: &Value, scope: Value) -> Value {
        admin_action(
            f,
            admin_key,
            "/api/v1/admin/recovery/issue",
            &json!({"client_id":old["client_id"],"scope":scope,"expires_at":now()+3600}),
            200,
        )
        .await
    }
    fn redemption(ticket: &Value, key: &KeyPair) -> Value {
        json!({"invitation":ticket["secret"],"request_id":Uuid::new_v4().to_string(),"csr_pem":csr(key)})
    }
    fn revoke_first_grant(f: &Fixture) {
        let mut db = f.db();
        let grant: String = db
            .query_row(
                "SELECT id FROM ccp_grants WHERE workspace_id=?1 AND revoked_at IS NULL",
                [f.setup.workspace_id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        let mut store = AccessStore::new(&mut db).unwrap();
        let actor = store
            .authenticate_admin_key(f.setup.owner_key.secret(), now())
            .unwrap();
        store
            .revoke_grant(actor, Uuid::parse_str(&grant).unwrap(), now())
            .unwrap();
    }
    async fn readmission_ticket(f: &Fixture, old: &Value) -> Value {
        admin_action(f, f.setup.owner_key.secret(), "/api/v1/admin/invitations/readmission",
            &json!({"workspace_id":f.setup.workspace_id.to_string(),"client_id":old["client_id"],"role":"read","expires_at":now()+3600}),200).await
    }

    async fn reject_cross_scope_invitation_actions(
        f: &Fixture,
        second: &ProvisionedWorkspace,
        target: &Value,
    ) {
        for wrong in [f.setup.instance_key.secret(), second.owner_key.secret()] {
            for action in ["status", "revoke"] {
                admin_action(
                    f,
                    wrong,
                    &format!("/api/v1/admin/invitations/{action}"),
                    target,
                    403,
                )
                .await;
            }
            admin_action(
                f,
                wrong,
                "/api/v1/admin/enrollment/close",
                &json!({"workspace_id":f.setup.workspace_id.to_string()}),
                403,
            )
            .await;
        }
    }

    async fn reject_invalid_recovery_scopes(f: &Fixture, old: &Value) {
        for scope in [
            Value::Null,
            json!({"kind":"global"}),
            json!({"kind":"instance","workspace_id":f.setup.workspace_id.to_string()}),
            json!({"kind":"workspace"}),
            json!({"kind":"workspace","workspace_id":"bad"}),
            json!({"kind":"workspace","workspace_id":f.setup.workspace_id.to_string(),"actor":"admin"}),
        ] {
            admin_action(
                f,
                f.setup.instance_key.secret(),
                "/api/v1/admin/recovery/issue",
                &json!({"client_id":old["client_id"],"scope":scope,"expires_at":now()+3600}),
                400,
            )
            .await;
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn invitation_lifecycle_closure_is_terminal_and_status_is_scoped_secret_free() {
        let f = Fixture::new().await;
        let second = f.second();
        let ticket = f
            .initial(f.setup.owner_key.secret(), f.setup.workspace_id)
            .await;
        let target = json!({"workspace_id":f.setup.workspace_id.to_string(),"invitation_id":ticket["invitation_id"]});
        reject_cross_scope_invitation_actions(&f, &second, &target).await;
        let status = admin_action(
            &f,
            f.setup.owner_key.secret(),
            "/api/v1/admin/invitations/status",
            &target,
            200,
        )
        .await;
        assert_eq!(status["uses"], 0);
        assert_eq!(status["initial_closed"], false);
        assert_eq!(status["issuer_authorized"], true);
        assert_eq!(status["target_client_id"], Value::Null);
        assert!(
            !status
                .to_string()
                .contains(ticket["secret"].as_str().unwrap())
        );
        assert!(status.get("token_hash").is_none());
        for _ in 0..2 {
            let done = admin_action(
                &f,
                f.setup.owner_key.secret(),
                "/api/v1/admin/enrollment/close",
                &json!({"workspace_id":f.setup.workspace_id.to_string()}),
                200,
            )
            .await;
            assert_eq!(done, json!({"completed":true}));
        }
        let status = admin_action(
            &f,
            f.setup.owner_key.secret(),
            "/api/v1/admin/invitations/status",
            &target,
            200,
        )
        .await;
        assert_eq!(status["initial_closed"], true);
        public_action(
            &f,
            "/api/v1/enroll",
            &redemption(&ticket, &KeyPair::generate().unwrap()),
            403,
        )
        .await;
        admin_action(&f,f.setup.owner_key.secret(),"/api/v1/admin/invitations/initial",
            &json!({"workspace_id":f.setup.workspace_id.to_string(),"role":"read","max_clients":2,"expires_at":null}),403).await;
        let individual = admin_action(
            &f,
            f.setup.owner_key.secret(),
            "/api/v1/admin/invitations/individual",
            &json!({"workspace_id":f.setup.workspace_id.to_string(),"role":"read","expires_at":now()+3600}),
            200,
        )
        .await;
        let individual_target = json!({"workspace_id":f.setup.workspace_id.to_string(),"invitation_id":individual["invitation_id"]});
        admin_action(
            &f,
            f.setup.owner_key.secret(),
            "/api/v1/admin/invitations/revoke",
            &individual_target,
            200,
        )
        .await;
        let status = admin_action(
            &f,
            f.setup.owner_key.secret(),
            "/api/v1/admin/invitations/status",
            &individual_target,
            200,
        )
        .await;
        assert_eq!(status["revoked"], true);
        public_action(
            &f,
            "/api/v1/enroll",
            &redemption(&individual, &KeyPair::generate().unwrap()),
            403,
        )
        .await;
        assert_eq!(
            f.db()
                .query_row("SELECT COUNT(*) FROM ccp_clients", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        f.finish().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn scoped_recovery_uses_fresh_key_exact_retry_and_preserves_other_workspace_binding() {
        let f = Fixture::new().await;
        let (old, key, second) = joined_pair(&f).await;
        let ticket = recovery_ticket(
            &f,
            f.setup.owner_key.secret(),
            &old,
            json!({"kind":"workspace","workspace_id":f.setup.workspace_id.to_string()}),
        )
        .await;
        public_action(
            &f,
            "/api/v1/enroll",
            &redemption(&ticket, &KeyPair::generate().unwrap()),
            403,
        )
        .await;
        public_action(&f, "/api/v1/recover", &redemption(&ticket, &key), 403).await;
        let newkey = KeyPair::generate().unwrap();
        let body = redemption(&ticket, &newkey);
        let recovered = public_action(&f, "/api/v1/recover", &body, 200).await;
        assert_eq!(recovered["data"]["client_id"], old["client_id"]);
        assert_ne!(recovered["data"]["credential_id"], old["credential_id"]);
        assert_eq!(recovered["data"]["instance_id"], f.trust.instance_id);
        assert!(!recovered.to_string().contains("PRIVATE KEY"));
        assert!(recovered["data"].get("grant_ids").is_none());
        assert_eq!(
            public_action(&f, "/api/v1/recover", &body, 200).await,
            recovered
        );
        let mut wrong = body.clone();
        wrong["request_id"] = json!(Uuid::new_v4().to_string());
        public_action(&f, "/api/v1/recover", &wrong, 403).await;
        let wrong_key = redemption(&ticket, &KeyPair::generate().unwrap());
        public_action(&f, "/api/v1/recover", &wrong_key, 403).await;
        assert_eq!(
            workspace_ids(&f, &old, &key).await,
            vec![second.workspace_id.to_string()]
        );
        assert_eq!(
            workspace_ids(&f, &recovered["data"], &newkey).await,
            vec![f.setup.workspace_id.to_string()]
        );
        let status = admin_action(
            &f,
            f.setup.owner_key.secret(),
            "/api/v1/admin/recovery/status",
            &json!({"invitation_id":ticket["invitation_id"]}),
            200,
        )
        .await;
        assert_eq!(status["redeemed"], true);
        assert_eq!(status["client_id"], old["client_id"]);
        assert_eq!(
            status["scope"],
            json!({"kind":"workspace","workspace_id":f.setup.workspace_id.to_string()})
        );
        assert!(status.get("secret").is_none());
        assert_eq!(
            f.db()
                .query_row("SELECT COUNT(*) FROM ccp_clients", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            f.db()
                .query_row("SELECT COUNT(*) FROM ccp_grants", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            2
        );
        f.finish().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn global_recovery_captures_live_grants_and_never_restores_revoked_access() {
        let f = Fixture::new().await;
        let (old, key, second) = joined_pair(&f).await;
        let scope = json!({"kind":"instance"});
        admin_action(
            &f,
            f.setup.owner_key.secret(),
            "/api/v1/admin/recovery/issue",
            &json!({"client_id":old["client_id"],"scope":scope,"expires_at":now()+3600}),
            403,
        )
        .await;
        admin_action(&f,f.setup.instance_key.secret(),"/api/v1/admin/recovery/issue",
            &json!({"client_id":old["client_id"],"scope":{"kind":"workspace","workspace_id":f.setup.workspace_id.to_string()},"expires_at":now()+3600}),403).await;
        let ticket = recovery_ticket(&f, f.setup.instance_key.secret(), &old, scope).await;
        let target = json!({"invitation_id":ticket["invitation_id"]});
        admin_action(
            &f,
            f.setup.owner_key.secret(),
            "/api/v1/admin/recovery/status",
            &target,
            403,
        )
        .await;
        let status = admin_action(
            &f,
            f.setup.instance_key.secret(),
            "/api/v1/admin/recovery/status",
            &target,
            200,
        )
        .await;
        assert_eq!(status["scope"], json!({"kind":"instance"}));
        revoke_first_grant(&f);
        let newkey = KeyPair::generate().unwrap();
        let body = redemption(&ticket, &newkey);
        let recovered = public_action(&f, "/api/v1/recover", &body, 200).await;
        assert_eq!(recovered["data"]["client_id"], old["client_id"]);
        assert_eq!(
            public_action(&f, "/api/v1/recover", &body, 200).await,
            recovered
        );
        assert_eq!(
            workspace_ids(&f, &recovered["data"], &newkey).await,
            vec![second.workspace_id.to_string()]
        );
        assert!(workspace_ids(&f, &old, &key).await.is_empty());
        assert_eq!(
            f.db()
                .query_row(
                    "SELECT COUNT(*) FROM ccp_grants WHERE revoked_at IS NOT NULL",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
        f.finish().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn recovery_revocation_and_current_issuer_apply_to_public_redemption() {
        let f = Fixture::new().await;
        let (old, _key, second) = joined_pair(&f).await;
        let scope = json!({"kind":"workspace","workspace_id":f.setup.workspace_id.to_string()});
        let ticket = recovery_ticket(&f, f.setup.owner_key.secret(), &old, scope.clone()).await;
        let target = json!({"invitation_id":ticket["invitation_id"]});
        for key in [second.owner_key.secret(), f.setup.instance_key.secret()] {
            for action in ["status", "revoke"] {
                admin_action(
                    &f,
                    key,
                    &format!("/api/v1/admin/recovery/{action}"),
                    &target,
                    403,
                )
                .await;
            }
        }
        admin_action(
            &f,
            f.setup.owner_key.secret(),
            "/api/v1/admin/recovery/revoke",
            &target,
            200,
        )
        .await;
        let status = admin_action(
            &f,
            f.setup.owner_key.secret(),
            "/api/v1/admin/recovery/status",
            &target,
            200,
        )
        .await;
        assert_eq!(status["revoked"], true);
        assert_eq!(status["redeemed"], false);
        public_action(
            &f,
            "/api/v1/recover",
            &redemption(&ticket, &KeyPair::generate().unwrap()),
            403,
        )
        .await;
        let fresh = recovery_ticket(&f, f.setup.owner_key.secret(), &old, scope).await;
        f.db()
            .execute(
                "UPDATE ccp_admin_keys SET revoked_at=?1 WHERE id=?2",
                rusqlite::params![now(), f.setup.owner_key.id.to_string()],
            )
            .unwrap();
        public_action(
            &f,
            "/api/v1/recover",
            &redemption(&fresh, &KeyPair::generate().unwrap()),
            403,
        )
        .await;
        let status=admin_action(&f,second.owner_key.secret(),"/api/v1/admin/recovery/issue",
            &json!({"client_id":old["client_id"],"scope":{"kind":"workspace","workspace_id":second.workspace_id.to_string()},"expires_at":now()+3600}),200).await;
        assert!(
            status["secret"]
                .as_str()
                .unwrap()
                .starts_with("ccp_recover_")
        );
        assert_eq!(
            f.db()
                .query_row("SELECT COUNT(*) FROM ccp_recovery_receipts", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        f.finish().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn globally_disabled_client_cannot_issue_redeem_or_retry_recovery() {
        let f = Fixture::new().await;
        let (old, _key) = f.enroll().await;
        let scope = json!({"kind":"instance"});
        let ticket = recovery_ticket(&f, f.setup.instance_key.secret(), &old, scope.clone()).await;
        let body = redemption(&ticket, &KeyPair::generate().unwrap());
        let recovered = public_action(&f, "/api/v1/recover", &body, 200).await;
        assert_eq!(recovered["data"]["client_id"], old["client_id"]);
        let pending = recovery_ticket(&f, f.setup.instance_key.secret(), &old, scope.clone()).await;
        {
            let mut db = f.db();
            let mut store = AccessStore::new(&mut db).unwrap();
            let actor = store
                .authenticate_admin_key(f.setup.instance_key.secret(), now())
                .unwrap();
            store
                .disable_client(
                    actor,
                    Uuid::parse_str(old["client_id"].as_str().unwrap()).unwrap(),
                    now(),
                )
                .unwrap();
        }
        public_action(&f, "/api/v1/recover", &body, 403).await;
        public_action(
            &f,
            "/api/v1/recover",
            &redemption(&pending, &KeyPair::generate().unwrap()),
            403,
        )
        .await;
        admin_action(
            &f,
            f.setup.instance_key.secret(),
            "/api/v1/admin/recovery/issue",
            &json!({"client_id":old["client_id"],"scope":scope,"expires_at":now()+3600}),
            403,
        )
        .await;
        assert_eq!(
            f.db()
                .query_row("SELECT COUNT(*) FROM ccp_recovery_receipts", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        f.finish().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn targeted_readmission_returns_same_client_new_grant_and_preserves_other_access() {
        let f = Fixture::new().await;
        let (old, key, second) = joined_pair(&f).await;
        revoke_first_grant(&f);
        let body = json!({"workspace_id":f.setup.workspace_id.to_string(),"client_id":old["client_id"],"role":"read","expires_at":now()+3600});
        for wrong in [f.setup.instance_key.secret(), second.owner_key.secret()] {
            admin_action(
                &f,
                wrong,
                "/api/v1/admin/invitations/readmission",
                &body,
                403,
            )
            .await;
        }
        let ticket = readmission_ticket(&f, &old).await;
        let target = json!({"workspace_id":f.setup.workspace_id.to_string(),"invitation_id":ticket["invitation_id"]});
        let status = admin_action(
            &f,
            f.setup.owner_key.secret(),
            "/api/v1/admin/invitations/status",
            &target,
            200,
        )
        .await;
        assert_eq!(status["target_client_id"], old["client_id"]);
        public_action(
            &f,
            "/api/v1/recover",
            &redemption(&ticket, &KeyPair::generate().unwrap()),
            403,
        )
        .await;
        public_action(&f, "/api/v1/enroll", &redemption(&ticket, &key), 403).await;
        let newkey = KeyPair::generate().unwrap();
        let body = redemption(&ticket, &newkey);
        let mut spoof = body.clone();
        spoof["client_id"] = old["client_id"].clone();
        public_action(&f, "/api/v1/enroll", &spoof, 400).await;
        let readmitted = public_action(&f, "/api/v1/enroll", &body, 200).await;
        assert_eq!(readmitted["data"]["client_id"], old["client_id"]);
        assert_ne!(readmitted["data"]["credential_id"], old["credential_id"]);
        assert_eq!(
            public_action(&f, "/api/v1/enroll", &body, 200).await,
            readmitted
        );
        assert_eq!(
            workspace_ids(&f, &old, &key).await,
            vec![second.workspace_id.to_string()]
        );
        assert_eq!(
            workspace_ids(&f, &readmitted["data"], &newkey).await,
            vec![f.setup.workspace_id.to_string()]
        );
        let status = admin_action(
            &f,
            f.setup.owner_key.secret(),
            "/api/v1/admin/invitations/status",
            &target,
            200,
        )
        .await;
        assert_eq!(status["uses"], 1);
        assert_eq!(
            f.db()
                .query_row("SELECT COUNT(*) FROM ccp_clients", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            f.db()
                .query_row("SELECT COUNT(*) FROM ccp_grants", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            3
        );
        f.finish().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn readmission_ticket_stays_stale_after_intervening_admission_is_revoked() {
        let f = Fixture::new().await;
        let (old, _key, _second) = joined_pair(&f).await;
        revoke_first_grant(&f);
        let stale = readmission_ticket(&f, &old).await;
        let current = readmission_ticket(&f, &old).await;
        public_action(
            &f,
            "/api/v1/enroll",
            &redemption(&current, &KeyPair::generate().unwrap()),
            200,
        )
        .await;
        revoke_first_grant(&f);
        public_action(
            &f,
            "/api/v1/enroll",
            &redemption(&stale, &KeyPair::generate().unwrap()),
            403,
        )
        .await;
        let status = admin_action(
            &f,
            f.setup.owner_key.secret(),
            "/api/v1/admin/invitations/status",
            &json!({"workspace_id":f.setup.workspace_id.to_string(),"invitation_id":stale["invitation_id"]}),
            200,
        )
        .await;
        assert_eq!(status["uses"], 0);
        assert_eq!(
            f.db()
                .query_row(
                    "SELECT COUNT(*) FROM ccp_grants WHERE revoked_at IS NULL",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
        f.finish().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn recovery_atomic_rollback_and_concurrent_exact_retry_are_visible_over_https() {
        let f = Fixture::new().await;
        let (old, key) = f.enroll().await;
        let ticket = recovery_ticket(
            &f,
            f.setup.owner_key.secret(),
            &old,
            json!({"kind":"workspace","workspace_id":f.setup.workspace_id.to_string()}),
        )
        .await;
        let newkey = KeyPair::generate().unwrap();
        let body = redemption(&ticket, &newkey);
        f.db().execute_batch("CREATE TRIGGER fail_http_recovery BEFORE INSERT ON ccp_access_audit WHEN NEW.action='redeem_client_recovery' BEGIN SELECT RAISE(ABORT,'injected audit failure'); END;").unwrap();
        public_action(&f, "/api/v1/recover", &body, 503).await;
        assert_eq!(
            workspace_ids(&f, &old, &key).await,
            vec![f.setup.workspace_id.to_string()]
        );
        assert_eq!(
            f.db()
                .query_row("SELECT COUNT(*) FROM ccp_recovery_receipts", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            f.db()
                .query_row("SELECT COUNT(*) FROM ccp_credentials", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        f.db()
            .execute_batch("DROP TRIGGER fail_http_recovery")
            .unwrap();
        let (a, b) = tokio::join!(
            public_action(&f, "/api/v1/recover", &body, 200),
            public_action(&f, "/api/v1/recover", &body, 200)
        );
        assert_eq!(a, b);
        assert_eq!(
            f.db()
                .query_row("SELECT COUNT(*) FROM ccp_recovery_receipts", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            f.db()
                .query_row("SELECT COUNT(*) FROM ccp_credentials", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            2
        );
        assert_eq!(
            workspace_ids(&f, &a["data"], &newkey).await,
            vec![f.setup.workspace_id.to_string()]
        );
        f.finish().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn archived_workspace_keeps_security_recovery_but_blocks_new_admission() {
        let f = Fixture::new().await;
        let (old, _key) = f.enroll().await;
        {
            let mut db = f.db();
            let mut store = AccessStore::new(&mut db).unwrap();
            let actor = store
                .authenticate_admin_key(f.setup.owner_key.secret(), now())
                .unwrap();
            store
                .set_archived(actor, f.setup.workspace_id, true, now())
                .unwrap();
        }
        let ticket = recovery_ticket(
            &f,
            f.setup.owner_key.secret(),
            &old,
            json!({"kind":"workspace","workspace_id":f.setup.workspace_id.to_string()}),
        )
        .await;
        let newkey = KeyPair::generate().unwrap();
        let recovered =
            public_action(&f, "/api/v1/recover", &redemption(&ticket, &newkey), 200).await;
        assert_eq!(
            workspace_ids(&f, &recovered["data"], &newkey).await,
            vec![f.setup.workspace_id.to_string()]
        );
        admin_action(
            &f,
            f.setup.owner_key.secret(),
            "/api/v1/admin/invitations/individual",
            &json!({"workspace_id":f.setup.workspace_id.to_string(),"role":"read","expires_at":now()+3600}),
            403,
        )
        .await;
        revoke_first_grant(&f);
        admin_action(&f,f.setup.owner_key.secret(),"/api/v1/admin/invitations/readmission",
            &json!({"workspace_id":f.setup.workspace_id.to_string(),"client_id":old["client_id"],"role":"read","expires_at":now()+3600}),403).await;
        admin_action(
            &f,
            f.setup.owner_key.secret(),
            "/api/v1/admin/enrollment/close",
            &json!({"workspace_id":f.setup.workspace_id.to_string()}),
            200,
        )
        .await;
        f.finish().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn recovery_dtos_listener_method_query_and_headers_cannot_select_authority() {
        let f = Fixture::new().await;
        let (old, key) = f.enroll().await;
        let scope = json!({"kind":"workspace","workspace_id":f.setup.workspace_id.to_string()});
        let ticket = recovery_ticket(&f, f.setup.owner_key.secret(), &old, scope).await;
        let body = redemption(&ticket, &KeyPair::generate().unwrap());
        for field in ["client_id", "credential_id", "actor", "grant_ids"] {
            let mut spoof = body.clone();
            spoof[field] = json!(old["client_id"]);
            public_action(&f, "/api/v1/recover", &spoof, 400).await;
        }
        let public = f.public_client();
        json_reply(
            public
                .post(f.url("/api/v1/recover"))
                .bearer_auth(f.setup.owner_key.secret())
                .json(&body)
                .send()
                .await
                .unwrap(),
            400,
        )
        .await;
        json_reply(
            public.get(f.url("/api/v1/recover")).send().await.unwrap(),
            405,
        )
        .await;
        public_action(&f, "/api/v1/recover?client_id=claimed", &body, 400).await;
        reject_invalid_recovery_scopes(&f, &old).await;
        json_reply(
            public
                .post(f.url("/api/v1/admin/recovery/status"))
                .header("content-type", "application/json")
                .bearer_auth(f.setup.owner_key.secret())
                .body(format!(
                    "{{\"invitation_id\":{},\"invitation_id\":{}}}",
                    ticket["invitation_id"], ticket["invitation_id"]
                ))
                .send()
                .await
                .unwrap(),
            400,
        )
        .await;
        json_reply(
            public
                .get(f.url("/api/v1/admin/recovery/status"))
                .bearer_auth(f.setup.owner_key.secret())
                .send()
                .await
                .unwrap(),
            405,
        )
        .await;
        let private = f.private_client(old["certificate_pem"].as_str().unwrap(), &key);
        json_reply(
            private
                .post(f.mtls("/api/v1/recover"))
                .json(&body)
                .send()
                .await
                .unwrap(),
            404,
        )
        .await;
        json_reply(
            private
                .post(f.mtls("/api/v1/admin/recovery/issue"))
                .json(&json!({}))
                .send()
                .await
                .unwrap(),
            404,
        )
        .await;
        assert_eq!(
            f.db()
                .query_row("SELECT COUNT(*) FROM ccp_recovery_invitations", [], |r| r
                    .get::<_, i64>(
                    0
                ))
                .unwrap(),
            1
        );
        assert_eq!(
            f.db()
                .query_row("SELECT COUNT(*) FROM ccp_recovery_receipts", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        public_action(&f, "/api/v1/recover", &body, 200).await;
        drop(private);
        drop(public);
        f.finish().await;
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn global_recovery_concurrent_different_requests_issue_one_credential_for_both_workspaces()
     {
        let f = Fixture::new().await;
        let (old, key, second) = joined_pair(&f).await;
        let ticket = recovery_ticket(
            &f,
            f.setup.instance_key.secret(),
            &old,
            json!({"kind":"instance"}),
        )
        .await;
        let key_a = KeyPair::generate().unwrap();
        let key_b = KeyPair::generate().unwrap();
        let body_a = redemption(&ticket, &key_a);
        let body_b = redemption(&ticket, &key_b);
        let a = f
            .public_client()
            .post(f.url("/api/v1/recover"))
            .json(&body_a);
        let b = f
            .public_client()
            .post(f.url("/api/v1/recover"))
            .json(&body_b);
        let (a, b) = tokio::join!(a.send(), b.send());
        let a = a.unwrap();
        let b = b.unwrap();
        let codes = [a.status().as_u16(), b.status().as_u16()];
        assert!(codes == [200, 403] || codes == [403, 200]);
        let (success, newkey) = if codes[0] == 200 {
            json_reply(b, 403).await;
            (json_reply(a, 200).await, key_a)
        } else {
            json_reply(a, 403).await;
            (json_reply(b, 200).await, key_b)
        };
        assert_eq!(success["data"]["client_id"], old["client_id"]);
        assert_eq!(
            workspace_ids(&f, &success["data"], &newkey).await,
            vec![
                f.setup.workspace_id.to_string(),
                second.workspace_id.to_string()
            ]
        );
        assert!(workspace_ids(&f, &old, &key).await.is_empty());
        assert_eq!(
            f.db()
                .query_row("SELECT COUNT(*) FROM ccp_recovery_receipts", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            f.db()
                .query_row("SELECT COUNT(*) FROM ccp_credentials", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            2
        );
        f.finish().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn status_observation_persists_invitation_and_recovery_expiry_without_returning_secrets()
    {
        let f = Fixture::new().await;
        let (old, _key) = f.enroll().await;
        let invitation=admin_action(&f,f.setup.owner_key.secret(),"/api/v1/admin/invitations/individual",
            &json!({"workspace_id":f.setup.workspace_id.to_string(),"role":"read","expires_at":now()+3600}),200).await;
        f.db()
            .execute(
                "UPDATE ccp_invitations SET expires_at=?1 WHERE id=?2",
                rusqlite::params![now() - 1, invitation["invitation_id"].as_str().unwrap()],
            )
            .unwrap();
        let target = json!({"workspace_id":f.setup.workspace_id.to_string(),"invitation_id":invitation["invitation_id"]});
        let status = admin_action(
            &f,
            f.setup.owner_key.secret(),
            "/api/v1/admin/invitations/status",
            &target,
            200,
        )
        .await;
        assert_eq!(status["expired"], true);
        assert!(status.get("secret").is_none());
        f.db()
            .execute(
                "UPDATE ccp_invitations SET expires_at=?1 WHERE id=?2",
                rusqlite::params![now() + 3600, invitation["invitation_id"].as_str().unwrap()],
            )
            .unwrap();
        public_action(
            &f,
            "/api/v1/enroll",
            &redemption(&invitation, &KeyPair::generate().unwrap()),
            403,
        )
        .await;
        let recovery = recovery_ticket(
            &f,
            f.setup.owner_key.secret(),
            &old,
            json!({"kind":"workspace","workspace_id":f.setup.workspace_id.to_string()}),
        )
        .await;
        f.db()
            .execute(
                "UPDATE ccp_recovery_invitations SET expires_at=?1 WHERE id=?2",
                rusqlite::params![now() - 1, recovery["invitation_id"].as_str().unwrap()],
            )
            .unwrap();
        let status = admin_action(
            &f,
            f.setup.owner_key.secret(),
            "/api/v1/admin/recovery/status",
            &json!({"invitation_id":recovery["invitation_id"]}),
            200,
        )
        .await;
        assert_eq!(status["expired"], true);
        assert_eq!(status["redeemed"], false);
        assert!(
            !status
                .to_string()
                .contains(recovery["secret"].as_str().unwrap())
        );
        f.db()
            .execute(
                "UPDATE ccp_recovery_invitations SET expires_at=?1 WHERE id=?2",
                rusqlite::params![now() + 3600, recovery["invitation_id"].as_str().unwrap()],
            )
            .unwrap();
        public_action(
            &f,
            "/api/v1/recover",
            &redemption(&recovery, &KeyPair::generate().unwrap()),
            403,
        )
        .await;
        assert_eq!(
            f.db()
                .query_row("SELECT COUNT(*) FROM ccp_recovery_receipts", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        f.finish().await;
    }
    fn managed_admin(f: &Fixture) -> server::foundation::IssuedAdminKey {
        use server::foundation::{AdminKeyRequest, AdminKeyScope};
        let mut db = f.db();
        let mut store = AccessStore::new(&mut db).unwrap();
        let actor = store
            .authenticate_admin_key(f.setup.owner_key.secret(), now())
            .unwrap();
        let admin = store
            .appoint_workspace_admin(actor, f.setup.workspace_id, "scoped admin", now())
            .unwrap();
        store
            .issue_admin_key(
                actor,
                &AdminKeyRequest {
                    principal_id: admin,
                    scope: AdminKeyScope::Workspace(f.setup.workspace_id),
                    name: "admin".into(),
                    expires_at: None,
                },
                now(),
            )
            .unwrap()
    }
    async fn inspect_workspace(f: &Fixture, key: &str, workspace: &Value, status: u16) -> Value {
        admin_action(
            f,
            key,
            "/api/v1/admin/workspaces/inspect",
            &json!({"workspace_id":workspace}),
            status,
        )
        .await
    }
    async fn grant_page(
        f: &Fixture,
        key: &str,
        workspace: Uuid,
        after: Option<&str>,
        limit: u32,
        status: u16,
    ) -> Value {
        admin_action(
            f,
            key,
            "/api/v1/admin/workspaces/grants",
            &json!({"workspace_id":workspace.to_string(),"after":after,"limit":limit}),
            status,
        )
        .await
    }
    fn management_counts(f: &Fixture) -> Vec<i64> {
        [
            "ccp_workspaces",
            "ccp_admin_principals",
            "ccp_admin_keys",
            "ccp_access_audit",
        ]
        .iter()
        .map(|table| {
            f.db()
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                .unwrap()
        })
        .collect()
    }
    async fn create_workspace(
        f: &Fixture,
        name: &str,
        owner: Value,
        expires: Option<i64>,
        status: u16,
    ) -> Value {
        admin_action(
            f,
            f.setup.instance_key.secret(),
            "/api/v1/admin/workspaces/create",
            &json!({"name":name,"owner":owner,"key_expires_at":expires}),
            status,
        )
        .await
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn workspace_creation_delivers_initial_owner_key_once_and_preserves_scope() {
        let f = Fixture::new().await;
        let owner = json!({"kind":"new_principal","name":"new owner"});
        admin_action(
            &f,
            f.setup.owner_key.secret(),
            "/api/v1/admin/workspaces/create",
            &json!({"name":"forbidden","owner":owner}),
            403,
        )
        .await;
        let created = create_workspace(&f, "created", owner, None, 200).await;
        assert_ne!(
            created["owner_id"],
            f.setup.instance_key.principal_id.to_string()
        );
        let secret = created["owner_key"].as_str().unwrap();
        assert!(secret.starts_with("ccp_admin_"));
        let info = inspect_workspace(&f, secret, &created["workspace_id"], 200).await;
        assert_eq!(info["owner_id"], created["owner_id"]);
        assert_eq!(info["workspace"]["name"], "created");
        assert_eq!(info["workspace"]["archived"], false);
        assert!(info.get("owner_key").is_none());
        inspect_workspace(
            &f,
            f.setup.owner_key.secret(),
            &created["workspace_id"],
            403,
        )
        .await;
        let global = inspect_workspace(
            &f,
            f.setup.instance_key.secret(),
            &created["workspace_id"],
            200,
        )
        .await;
        assert_eq!(global, info);
        admin_action(
            &f,
            f.setup.instance_key.secret(),
            "/api/v1/admin/invitations/individual",
            &json!({"workspace_id":created["workspace_id"],"role":"read","expires_at":now()+3600}),
            403,
        )
        .await;
        let before = management_counts(&f);
        admin_action(&f,f.setup.instance_key.secret(),"/api/v1/admin/workspaces/first-owner",
            &json!({"workspace_id":created["workspace_id"],"owner":{"kind":"new_principal","name":"retry"}}),403).await;
        assert_eq!(management_counts(&f), before);
        let expiry: Option<i64> = f
            .db()
            .query_row(
                "SELECT expires_at FROM ccp_admin_keys WHERE id=?1",
                [created["owner_key_id"].as_str().unwrap()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(expiry, None);
        let audit:String=f.db().query_row("SELECT admin_key_id FROM ccp_access_audit WHERE action='create_workspace' AND workspace_id=?1",
            [created["workspace_id"].as_str().unwrap()],|r|r.get(0)).unwrap();
        assert_eq!(audit, f.setup.instance_key.id.to_string());
        assert_eq!(
            f.db()
                .query_row(
                    "SELECT COUNT(*) FROM ccp_server_admins WHERE admin_id=?1",
                    [created["owner_id"].as_str().unwrap()],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
        f.finish().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn first_owner_lost_response_has_metadata_but_no_remote_secret_reissue() {
        let f = Fixture::new().await;
        let workspace = Uuid::new_v4().to_string();
        f.db()
            .execute(
                "INSERT INTO ccp_workspaces(id,name,state) VALUES (?1,'unowned','archived')",
                [&workspace],
            )
            .unwrap();
        let info =
            inspect_workspace(&f, f.setup.instance_key.secret(), &json!(workspace), 200).await;
        assert_eq!(info["owner_id"], Value::Null);
        let body = json!({"workspace_id":workspace,"owner":{"kind":"existing","principal_id":f.setup.instance_key.principal_id.to_string()},"key_expires_at":now()+3600});
        let response = f
            .public_client()
            .post(f.url("/api/v1/admin/workspaces/first-owner"))
            .bearer_auth(f.setup.instance_key.secret())
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 200);
        drop(response);
        let info =
            inspect_workspace(&f, f.setup.instance_key.secret(), &json!(workspace), 200).await;
        assert_eq!(
            info["owner_id"],
            f.setup.instance_key.principal_id.to_string()
        );
        assert_eq!(info["workspace"]["archived"], true);
        assert!(info.get("owner_key").is_none());
        let before = management_counts(&f);
        admin_action(
            &f,
            f.setup.instance_key.secret(),
            "/api/v1/admin/workspaces/first-owner",
            &body,
            403,
        )
        .await;
        admin_action(
            &f,
            f.setup.instance_key.secret(),
            "/api/v1/admin/workspaces/reset-owner-key",
            &body,
            404,
        )
        .await;
        assert_eq!(management_counts(&f), before);
        let expiry: i64 = f
            .db()
            .query_row(
                "SELECT expires_at FROM ccp_admin_keys WHERE workspace_id=?1",
                [&workspace],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(expiry, body["key_expires_at"].as_i64().unwrap());
        f.finish().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn archive_and_reactivate_require_owner_or_global_while_scoped_security_stays_available()
    {
        let f = Fixture::new().await;
        let (_old, _key) = f.enroll().await;
        let admin = managed_admin(&f);
        let second = f.second();
        let target = json!({"workspace_id":f.setup.workspace_id.to_string()});
        for bad in [admin.secret(), second.owner_key.secret()] {
            admin_action(&f, bad, "/api/v1/admin/workspaces/archive", &target, 403).await;
        }
        admin_action(
            &f,
            f.setup.owner_key.secret(),
            "/api/v1/admin/workspaces/archive",
            &target,
            200,
        )
        .await;
        let info = inspect_workspace(&f, admin.secret(), &target["workspace_id"], 200).await;
        assert_eq!(info["workspace"]["archived"], true);
        admin_action(
            &f,
            admin.secret(),
            "/api/v1/admin/workspaces/reactivate",
            &target,
            403,
        )
        .await;
        admin_action(&f,admin.secret(),"/api/v1/admin/invitations/individual",
            &json!({"workspace_id":f.setup.workspace_id.to_string(),"role":"read","expires_at":now()+3600}),403).await;
        let grants = grant_page(&f, admin.secret(), f.setup.workspace_id, None, 10, 200).await;
        admin_action(
            &f,
            admin.secret(),
            "/api/v1/admin/grants/revoke",
            &json!({"grant_id":grants["grants"][0]["id"]}),
            200,
        )
        .await;
        admin_action(
            &f,
            f.setup.instance_key.secret(),
            "/api/v1/admin/workspaces/reactivate",
            &target,
            200,
        )
        .await;
        admin_action(
            &f,
            f.setup.instance_key.secret(),
            "/api/v1/admin/workspaces/archive",
            &target,
            200,
        )
        .await;
        admin_action(
            &f,
            f.setup.owner_key.secret(),
            "/api/v1/admin/workspaces/reactivate",
            &target,
            200,
        )
        .await;
        assert_eq!(
            inspect_workspace(&f, f.setup.owner_key.secret(), &target["workspace_id"], 200).await["workspace"]
                ["archived"],
            false
        );
        assert_eq!(
            f.db()
                .query_row("SELECT COUNT(*) FROM ccp_grants", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        f.finish().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn grant_inspection_and_revocation_preserve_history_and_unrelated_workspace_access() {
        let f = Fixture::new().await;
        let (old, key, second) = joined_pair(&f).await;
        let grants = grant_page(
            &f,
            f.setup.owner_key.secret(),
            f.setup.workspace_id,
            None,
            10,
            200,
        )
        .await;
        assert_eq!(grants["grants"].as_array().unwrap().len(), 1);
        assert_eq!(grants["grants"][0]["client_id"], old["client_id"]);
        assert_eq!(grants["grants"][0]["role"], "read_write");
        assert_eq!(grants["grants"][0]["revoked_at"], Value::Null);
        assert_eq!(grants["grants"][0]["client_disabled"], false);
        assert!(grants["grants"][0].get("certificate_pem").is_none());
        assert!(grants["grants"][0].get("credential_id").is_none());
        let global = grant_page(
            &f,
            f.setup.instance_key.secret(),
            f.setup.workspace_id,
            None,
            10,
            200,
        )
        .await;
        assert_eq!(global, grants);
        grant_page(
            &f,
            second.owner_key.secret(),
            f.setup.workspace_id,
            None,
            10,
            403,
        )
        .await;
        let target = json!({"grant_id":grants["grants"][0]["id"]});
        for bad in [f.setup.instance_key.secret(), second.owner_key.secret()] {
            admin_action(&f, bad, "/api/v1/admin/grants/revoke", &target, 403).await;
        }
        admin_action(
            &f,
            f.setup.owner_key.secret(),
            "/api/v1/admin/grants/revoke",
            &target,
            200,
        )
        .await;
        assert_eq!(
            workspace_ids(&f, &old, &key).await,
            vec![second.workspace_id.to_string()]
        );
        let after = grant_page(
            &f,
            f.setup.owner_key.secret(),
            f.setup.workspace_id,
            None,
            10,
            200,
        )
        .await;
        assert!(after["grants"][0]["revoked_at"].as_i64().is_some());
        assert_eq!(after["grants"][0]["id"], grants["grants"][0]["id"]);
        assert_eq!(
            f.db()
                .query_row("SELECT COUNT(*) FROM ccp_grants", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            2
        );
        let renewkey = KeyPair::generate().unwrap();
        let next = json_reply(
            f.private_client(old["certificate_pem"].as_str().unwrap(), &key)
                .post(f.mtls("/api/v1/credentials/renew"))
                .json(&json!({"request_id":Uuid::new_v4().to_string(),"csr_pem":csr(&renewkey)}))
                .send()
                .await
                .unwrap(),
            200,
        )
        .await;
        assert_eq!(
            workspace_ids(&f, &next["data"], &renewkey).await,
            vec![second.workspace_id.to_string()]
        );
        f.finish().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn global_disable_endpoint_takes_effect_on_same_tls_connection_and_preserves_history() {
        let f = Fixture::new().await;
        let (old, key, _second) = joined_pair(&f).await;
        let target = json!({"client_id":old["client_id"]});
        admin_action(
            &f,
            f.setup.owner_key.secret(),
            "/api/v1/admin/clients/disable",
            &target,
            403,
        )
        .await;
        let mut wire =
            Wire::connect(&f, Some((old["certificate_pem"].as_str().unwrap(), &key))).await;
        wire.send("GET /api/v1/workspaces HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await;
        assert_eq!(wire.response().await.0, 200);
        admin_action(
            &f,
            f.setup.instance_key.secret(),
            "/api/v1/admin/clients/disable",
            &target,
            200,
        )
        .await;
        wire.send("GET /api/v1/workspaces HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await;
        assert_eq!(wire.response().await.0, 403);
        let private = f.private_client(old["certificate_pem"].as_str().unwrap(), &key);
        for path in ["/api/v1/credentials/renew", "/api/v1/admissions"] {
            json_reply(private.post(f.mtls(path)).json(&json!({"request_id":Uuid::new_v4().to_string(),"csr_pem":csr(&KeyPair::generate().unwrap())})).send().await.unwrap(),403).await;
        }
        let page = grant_page(
            &f,
            f.setup.owner_key.secret(),
            f.setup.workspace_id,
            None,
            10,
            200,
        )
        .await;
        assert_eq!(page["grants"][0]["client_disabled"], true);
        assert_eq!(
            f.db()
                .query_row("SELECT COUNT(*) FROM ccp_clients", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            f.db()
                .query_row(
                    "SELECT COUNT(*) FROM ccp_grants WHERE revoked_at IS NULL",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            2
        );
        drop(wire);
        drop(private);
        f.finish().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn creation_and_first_owner_audit_failures_roll_back_keys_principals_and_workspace() {
        let f = Fixture::new().await;
        let before = management_counts(&f);
        f.db().execute_batch("CREATE TRIGGER fail_http_owner BEFORE INSERT ON ccp_access_audit WHEN NEW.action='issue_initial_owner_key' BEGIN SELECT RAISE(ABORT,'injected owner audit failure'); END;").unwrap();
        create_workspace(
            &f,
            "rollback",
            json!({"kind":"new_principal","name":"rollback owner"}),
            None,
            503,
        )
        .await;
        assert_eq!(management_counts(&f), before);
        let unowned = Uuid::new_v4().to_string();
        f.db()
            .execute(
                "INSERT INTO ccp_workspaces(id,name) VALUES (?1,'rollback first owner')",
                [&unowned],
            )
            .unwrap();
        let unowned_before = management_counts(&f);
        admin_action(&f,f.setup.instance_key.secret(),"/api/v1/admin/workspaces/first-owner",
            &json!({"workspace_id":unowned,"owner":{"kind":"new_principal","name":"rollback initial owner"}}),503).await;
        assert_eq!(management_counts(&f), unowned_before);
        assert_eq!(
            inspect_workspace(&f, f.setup.instance_key.secret(), &json!(unowned), 200).await["owner_id"],
            Value::Null
        );
        f.db()
            .execute_batch("DROP TRIGGER fail_http_owner")
            .unwrap();
        let created = create_workspace(
            &f,
            "rollback",
            json!({"kind":"new_principal","name":"rollback owner"}),
            None,
            200,
        )
        .await;
        assert!(
            created["owner_key"]
                .as_str()
                .unwrap()
                .starts_with("ccp_admin_")
        );
        let before = management_counts(&f);
        create_workspace(
            &f,
            "rollback",
            json!({"kind":"new_principal","name":"duplicate owner"}),
            None,
            503,
        )
        .await;
        assert_eq!(management_counts(&f), before);
        let missing = Uuid::new_v4().to_string();
        admin_action(
            &f,
            f.setup.instance_key.secret(),
            "/api/v1/admin/workspaces/first-owner",
            &json!({"workspace_id":missing,"owner":{"kind":"new_principal","name":"orphan"}}),
            403,
        )
        .await;
        assert_eq!(management_counts(&f), before);
        f.finish().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn first_owner_concurrent_network_appointments_create_exactly_one_principal_and_key() {
        let f = Fixture::new().await;
        let workspace = Uuid::new_v4().to_string();
        f.db()
            .execute(
                "INSERT INTO ccp_workspaces(id,name) VALUES (?1,'raced')",
                [&workspace],
            )
            .unwrap();
        let before = management_counts(&f);
        let a = f
            .public_client()
            .post(f.url("/api/v1/admin/workspaces/first-owner"))
            .bearer_auth(f.setup.instance_key.secret())
            .json(
                &json!({"workspace_id":workspace,"owner":{"kind":"new_principal","name":"alice"}}),
            );
        let b = f
            .public_client()
            .post(f.url("/api/v1/admin/workspaces/first-owner"))
            .bearer_auth(f.setup.instance_key.secret())
            .json(&json!({"workspace_id":workspace,"owner":{"kind":"new_principal","name":"bob"}}));
        let (a, b) = tokio::join!(a.send(), b.send());
        let a = a.unwrap();
        let b = b.unwrap();
        let codes = [a.status().as_u16(), b.status().as_u16()];
        assert!(codes == [200, 403] || codes == [403, 200]);
        let win = if codes[0] == 200 {
            json_reply(b, 403).await;
            json_reply(a, 200).await
        } else {
            json_reply(a, 403).await;
            json_reply(b, 200).await
        };
        let counts = management_counts(&f);
        assert_eq!(counts[0], before[0]);
        assert_eq!(counts[1], before[1] + 1);
        assert_eq!(counts[2], before[2] + 1);
        assert_eq!(
            inspect_workspace(&f, f.setup.instance_key.secret(), &json!(workspace), 200).await["owner_id"],
            win["data"]["owner_id"]
        );
        f.finish().await;
    }

    async fn reject_owner_selectors(f: &Fixture) {
        for owner in [
            Value::Null,
            json!({"kind":"instance"}),
            json!({"kind":"existing","principal_id":"bad"}),
            json!({"kind":"existing","principal_id":f.setup.instance_key.principal_id.to_string(),"name":"claim"}),
            json!({"kind":"new_principal","name":"claim","principal_id":f.setup.instance_key.principal_id.to_string()}),
        ] {
            create_workspace(f, "invalid", owner, None, 400).await;
        }
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn management_requests_reject_ambiguous_identity_claims_and_wrong_surfaces() {
        let f = Fixture::new().await;
        let (old, key) = f.enroll().await;
        let before = management_counts(&f);
        reject_owner_selectors(&f).await;
        let target = json!({"workspace_id":f.setup.workspace_id.to_string()});
        for path in [
            "workspaces/inspect",
            "workspaces/archive",
            "workspaces/reactivate",
            "workspaces/grants",
        ] {
            let path = format!("/api/v1/admin/{path}");
            let mut spoof = target.clone();
            spoof["actor"] = json!("global");
            admin_action(&f, f.setup.instance_key.secret(), &path, &spoof, 400).await;
            json_reply(
                f.public_client()
                    .get(f.url(&path))
                    .bearer_auth(f.setup.instance_key.secret())
                    .send()
                    .await
                    .unwrap(),
                405,
            )
            .await;
            admin_action(
                &f,
                f.setup.instance_key.secret(),
                &format!("{path}?workspace_id=claimed"),
                &target,
                400,
            )
            .await;
            json_reply(
                f.private_client(old["certificate_pem"].as_str().unwrap(), &key)
                    .post(f.mtls(&path))
                    .json(&target)
                    .send()
                    .await
                    .unwrap(),
                404,
            )
            .await;
        }
        for limit in [0, 501] {
            grant_page(
                &f,
                f.setup.instance_key.secret(),
                f.setup.workspace_id,
                None,
                limit,
                400,
            )
            .await;
        }
        grant_page(
            &f,
            f.setup.instance_key.secret(),
            f.setup.workspace_id,
            Some("bad"),
            10,
            400,
        )
        .await;
        assert_eq!(management_counts(&f), before);
        f.finish().await;
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn grant_history_paginates_through_authenticated_https_without_cursor_authority() {
        let f = Fixture::new().await;
        let (_old, _key) = f.enroll().await;
        for _ in 0..3 {
            let invite=admin_action(&f,f.setup.owner_key.secret(),"/api/v1/admin/invitations/individual",
                &json!({"workspace_id":f.setup.workspace_id.to_string(),"role":"read","expires_at":now()+3600}),200).await;
            public_action(
                &f,
                "/api/v1/enroll",
                &redemption(&invite, &KeyPair::generate().unwrap()),
                200,
            )
            .await;
        }
        let second = f.second();
        let mut after = None;
        let mut found = Vec::new();
        loop {
            let page = grant_page(
                &f,
                f.setup.owner_key.secret(),
                f.setup.workspace_id,
                after.as_deref(),
                2,
                200,
            )
            .await;
            let grants = page["grants"].as_array().unwrap();
            assert!(grants.len() <= 2);
            found.extend(grants.iter().map(|g| g["id"].as_str().unwrap().to_string()));
            after = page["next_cursor"].as_str().map(str::to_string);
            if after.is_none() {
                break;
            }
        }
        assert_eq!(found.len(), 4);
        let mut sorted = found.clone();
        sorted.sort();
        assert_eq!(found, sorted);
        sorted.dedup();
        assert_eq!(sorted.len(), 4);
        grant_page(
            &f,
            f.setup.owner_key.secret(),
            second.workspace_id,
            Some(&found[0]),
            2,
            403,
        )
        .await;
        let end = grant_page(
            &f,
            f.setup.owner_key.secret(),
            f.setup.workspace_id,
            Some(&Uuid::max().to_string()),
            2,
            200,
        )
        .await;
        assert!(end["grants"].as_array().unwrap().is_empty());
        assert_eq!(end["next_cursor"], Value::Null);
        let defaults = admin_action(
            &f,
            f.setup.instance_key.secret(),
            "/api/v1/admin/workspaces/grants",
            &json!({"workspace_id":f.setup.workspace_id.to_string()}),
            200,
        )
        .await;
        assert_eq!(defaults["grants"].as_array().unwrap().len(), 4);
        f.finish().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn owner_key_expiry_and_disabled_initial_owner_are_current_network_decisions() {
        let f = Fixture::new().await;
        let created = create_workspace(
            &f,
            "expiring",
            json!({"kind":"new_principal","name":"limited owner"}),
            Some(now() + 3600),
            200,
        )
        .await;
        let secret = created["owner_key"].as_str().unwrap();
        inspect_workspace(&f, secret, &created["workspace_id"], 200).await;
        f.db()
            .execute(
                "UPDATE ccp_admin_keys SET expires_at=?1 WHERE id=?2",
                rusqlite::params![now() - 1, created["owner_key_id"].as_str().unwrap()],
            )
            .unwrap();
        inspect_workspace(&f, secret, &created["workspace_id"], 401).await;
        let info = inspect_workspace(
            &f,
            f.setup.instance_key.secret(),
            &created["workspace_id"],
            200,
        )
        .await;
        assert_eq!(info["owner_id"], created["owner_id"]);
        f.db()
            .execute(
                "UPDATE ccp_admin_principals SET enabled=0 WHERE id=?1",
                [created["owner_id"].as_str().unwrap()],
            )
            .unwrap();
        let before = management_counts(&f);
        create_workspace(
            &f,
            "disabled owner",
            json!({"kind":"existing","principal_id":created["owner_id"]}),
            None,
            403,
        )
        .await;
        assert_eq!(management_counts(&f), before);
        create_workspace(
            &f,
            "invalid expiry",
            json!({"kind":"new_principal","name":"early"}),
            Some(now() - 1),
            403,
        )
        .await;
        assert_eq!(management_counts(&f), before);
        f.finish().await;
    }
    async fn team_member(f: &Fixture, name: &str) -> Value {
        admin_action(f,f.setup.owner_key.secret(),"/api/v1/admin/team/appoint",
            &json!({"workspace_id":f.setup.workspace_id.to_string(),"principal":{"kind":"new_principal","name":name}}),200).await
    }
    async fn team_page(
        f: &Fixture,
        key: &str,
        action: &str,
        after: Option<&str>,
        limit: u32,
        status: u16,
    ) -> Value {
        admin_action(
            f,
            key,
            &format!("/api/v1/admin/{action}/list"),
            &json!({"workspace_id":f.setup.workspace_id.to_string(),"after":after,"limit":limit}),
            status,
        )
        .await
    }
    async fn transfer_owner(f: &Fixture, key: &str, target: &Value, status: u16) -> Value {
        admin_action(
            f,
            key,
            "/api/v1/admin/workspaces/transfer-owner",
            &json!({"workspace_id":f.setup.workspace_id.to_string(),"new_owner_id":target}),
            status,
        )
        .await
    }
    async fn owner_key(f: &Fixture, key: &str, principal: &Value, status: u16) -> Value {
        admin_action(f,key,"/api/v1/admin/keys/issue",
            &json!({"workspace_id":f.setup.workspace_id.to_string(),"principal_id":principal,"name":"rotated key","expires_at":now()+3600}),status).await
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn owner_appoints_admin_and_rotates_keys_without_granting_team_or_global_powers() {
        let f = Fixture::new().await;
        let admin = team_member(&f, "personal admin").await;
        let secret = admin["secret"].as_str().unwrap();
        let info =
            inspect_workspace(&f, secret, &json!(f.setup.workspace_id.to_string()), 200).await;
        assert_eq!(info["owner_id"], f.setup.owner_key.principal_id.to_string());
        let before = management_counts(&f);
        for key in [secret, f.setup.instance_key.secret()] {
            admin_action(&f,key,"/api/v1/admin/team/appoint",
                &json!({"workspace_id":f.setup.workspace_id.to_string(),"principal":{"kind":"new_principal","name":"forbidden"}}),403).await;
            team_page(&f, key, "team", None, 10, 403).await;
            team_page(&f, key, "keys", None, 10, 403).await;
            owner_key(&f, key, &admin["principal_id"], 403).await;
            transfer_owner(&f, key, &admin["principal_id"], 403).await;
        }
        assert_eq!(management_counts(&f), before);
        let rotated = owner_key(&f, f.setup.owner_key.secret(), &admin["principal_id"], 200).await;
        assert_eq!(rotated["principal_id"], admin["principal_id"]);
        assert_ne!(rotated["key_id"], admin["key_id"]);
        admin_action(
            &f,
            f.setup.owner_key.secret(),
            "/api/v1/admin/keys/revoke",
            &json!({"workspace_id":f.setup.workspace_id.to_string(),"key_id":admin["key_id"]}),
            200,
        )
        .await;
        inspect_workspace(&f, secret, &json!(f.setup.workspace_id.to_string()), 401).await;
        inspect_workspace(
            &f,
            rotated["secret"].as_str().unwrap(),
            &json!(f.setup.workspace_id.to_string()),
            200,
        )
        .await;
        let pages = team_page(&f, f.setup.owner_key.secret(), "keys", None, 10, 200).await;
        assert!(!pages.to_string().contains(secret));
        assert!(
            !pages
                .to_string()
                .contains(rotated["secret"].as_str().unwrap())
        );
        assert!(
            pages["keys"]
                .as_array()
                .unwrap()
                .iter()
                .any(|k| k["id"] == admin["key_id"] && k["revoked_at"].is_i64())
        );
        f.finish().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn revoked_admin_reappointment_preserves_identity_but_never_resurrects_old_keys() {
        let f = Fixture::new().await;
        let admin = team_member(&f, "returning admin").await;
        let second = f.second();
        let other=admin_action(&f,second.owner_key.secret(),"/api/v1/admin/team/appoint",
            &json!({"workspace_id":second.workspace_id.to_string(),"principal":{"kind":"existing","principal_id":admin["principal_id"]}}),200).await;
        let target = json!({"workspace_id":f.setup.workspace_id.to_string(),"principal_id":admin["principal_id"]});
        admin_action(
            &f,
            f.setup.owner_key.secret(),
            "/api/v1/admin/team/revoke",
            &target,
            200,
        )
        .await;
        inspect_workspace(
            &f,
            admin["secret"].as_str().unwrap(),
            &json!(f.setup.workspace_id.to_string()),
            401,
        )
        .await;
        let fresh=admin_action(&f,f.setup.owner_key.secret(),"/api/v1/admin/team/appoint",
            &json!({"workspace_id":f.setup.workspace_id.to_string(),"principal":{"kind":"existing","principal_id":admin["principal_id"]}}),200).await;
        assert_eq!(fresh["principal_id"], admin["principal_id"]);
        inspect_workspace(
            &f,
            fresh["secret"].as_str().unwrap(),
            &json!(f.setup.workspace_id.to_string()),
            200,
        )
        .await;
        inspect_workspace(
            &f,
            admin["secret"].as_str().unwrap(),
            &json!(f.setup.workspace_id.to_string()),
            401,
        )
        .await;
        inspect_workspace(
            &f,
            other["secret"].as_str().unwrap(),
            &json!(second.workspace_id.to_string()),
            200,
        )
        .await;
        f.finish().await;
    }

    async fn collect_owner_page(f: &Fixture, action: &str, field: &str) -> Vec<Value> {
        let mut after = None;
        let mut found = Vec::new();
        loop {
            let page = team_page(
                f,
                f.setup.owner_key.secret(),
                action,
                after.as_deref(),
                2,
                200,
            )
            .await;
            let entries = page[field].as_array().unwrap();
            assert!(entries.len() <= 2);
            found.extend(entries.iter().cloned());
            after = page["next_cursor"].as_str().map(str::to_string);
            if after.is_none() {
                break;
            }
        }
        found
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn owner_roster_and_key_pages_are_bounded_private_and_scope_exact() {
        let f = Fixture::new().await;
        let a = team_member(&f, "alice").await;
        let b = team_member(&f, "bob").await;
        let second = f.second();
        let admins = collect_owner_page(&f, "team", "admins").await;
        assert_eq!(admins.len(), 3);
        assert_eq!(admins.iter().filter(|a| a["is_owner"] == true).count(), 1);
        let keys = collect_owner_page(&f, "keys", "keys").await;
        assert_eq!(keys.len(), 3);
        for entries in [&admins, &keys] {
            let ids: Vec<_> = entries
                .iter()
                .map(|v| v["id"].as_str().unwrap().to_string())
                .collect();
            let mut sorted = ids.clone();
            sorted.sort();
            assert_eq!(ids, sorted);
            sorted.dedup();
            assert_eq!(sorted.len(), 3);
            let text = serde_json::to_string(entries).unwrap();
            assert!(!text.contains(a["secret"].as_str().unwrap()));
            assert!(!text.contains(b["secret"].as_str().unwrap()));
            assert!(!text.contains("token_hash"));
        }
        for action in ["team", "keys"] {
            team_page(&f, second.owner_key.secret(), action, None, 2, 403).await;
            let end = team_page(
                &f,
                f.setup.owner_key.secret(),
                action,
                Some(&Uuid::max().to_string()),
                2,
                200,
            )
            .await;
            assert_eq!(end["next_cursor"], Value::Null);
            assert!(
                end[if action == "team" { "admins" } else { "keys" }]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
        }
        f.finish().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn owner_transfer_changes_authority_on_same_connection_and_preserves_unrelated_scopes() {
        let f = Fixture::new().await;
        let second = f.second();
        let admin = team_member(&f, "successor").await;
        let secret = admin["secret"].as_str().unwrap();
        let mut wire = Wire::connect(&f, None).await;
        let request = format!(
            "GET /api/v1/admin/workspaces HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {}\r\n\r\n",
            f.setup.owner_key.secret()
        );
        wire.send(&request).await;
        assert_eq!(wire.response().await.0, 200);
        transfer_owner(&f, f.setup.owner_key.secret(), &admin["principal_id"], 200).await;
        wire.send(&request).await;
        assert_eq!(wire.response().await.0, 401);
        let info = inspect_workspace(
            &f,
            f.setup.instance_key.secret(),
            &json!(f.setup.workspace_id.to_string()),
            200,
        )
        .await;
        assert_eq!(info["owner_id"], admin["principal_id"]);
        let team = team_page(&f, secret, "team", None, 10, 200).await;
        assert_eq!(team["admins"].as_array().unwrap().len(), 1);
        assert_eq!(team["admins"][0]["is_owner"], true);
        owner_key(&f, secret, &admin["principal_id"], 200).await;
        owner_key(
            &f,
            secret,
            &json!(f.setup.owner_key.principal_id.to_string()),
            403,
        )
        .await;
        inspect_workspace(
            &f,
            second.owner_key.secret(),
            &json!(second.workspace_id.to_string()),
            200,
        )
        .await;
        admin_action(&f,f.setup.instance_key.secret(),"/api/v1/admin/workspaces/first-owner",
            &json!({"workspace_id":f.setup.workspace_id.to_string(),"owner":{"kind":"existing","principal_id":f.setup.owner_key.principal_id.to_string()}}),403).await;
        transfer_owner(
            &f,
            f.setup.owner_key.secret(),
            &json!(f.setup.owner_key.principal_id.to_string()),
            401,
        )
        .await;
        let audit: String = f
            .db()
            .query_row(
                "SELECT admin_key_id FROM ccp_access_audit WHERE action='transfer_owner'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(audit, f.setup.owner_key.id.to_string());
        drop(wire);
        f.finish().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn competing_owner_transfers_have_one_winner_and_no_ownerless_state() {
        let f = Fixture::new().await;
        let a = team_member(&f, "one").await;
        let b = team_member(&f, "two").await;
        let request = |target: &Value| {
            f.public_client()
                .post(f.url("/api/v1/admin/workspaces/transfer-owner"))
                .bearer_auth(f.setup.owner_key.secret())
                .json(
                    &json!({"workspace_id":f.setup.workspace_id.to_string(),"new_owner_id":target}),
                )
        };
        let (ra, rb) = tokio::join!(
            request(&a["principal_id"]).send(),
            request(&b["principal_id"]).send()
        );
        let ra = ra.unwrap();
        let rb = rb.unwrap();
        let codes = [ra.status().as_u16(), rb.status().as_u16()];
        assert_eq!(codes.iter().filter(|code| **code == 200).count(), 1);
        assert!(codes.iter().all(|code| [200, 401, 403].contains(code)));
        let winner = if codes[0] == 200 {
            json_reply(rb, codes[1]).await;
            json_reply(ra, 200).await;
            a
        } else {
            json_reply(ra, codes[0]).await;
            json_reply(rb, 200).await;
            b
        };
        assert_eq!(
            inspect_workspace(
                &f,
                f.setup.instance_key.secret(),
                &json!(f.setup.workspace_id.to_string()),
                200
            )
            .await["owner_id"],
            winner["principal_id"]
        );
        team_page(
            &f,
            winner["secret"].as_str().unwrap(),
            "keys",
            None,
            10,
            200,
        )
        .await;
        assert_eq!(
            f.db()
                .query_row(
                    "SELECT COUNT(*) FROM ccp_access_audit WHERE action='transfer_owner'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
        f.finish().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn owner_transfer_audit_failure_keeps_old_key_owner_and_recipient_role_working() {
        let f = Fixture::new().await;
        let admin = team_member(&f, "successor").await;
        let before = management_counts(&f);
        f.db().execute_batch("CREATE TRIGGER fail_owner_transfer BEFORE INSERT ON ccp_access_audit WHEN NEW.action='transfer_owner' BEGIN SELECT RAISE(ABORT,'injected transfer audit'); END;").unwrap();
        transfer_owner(&f, f.setup.owner_key.secret(), &admin["principal_id"], 503).await;
        assert_eq!(management_counts(&f), before);
        assert_eq!(
            inspect_workspace(
                &f,
                f.setup.owner_key.secret(),
                &json!(f.setup.workspace_id.to_string()),
                200
            )
            .await["owner_id"],
            f.setup.owner_key.principal_id.to_string()
        );
        team_page(&f, admin["secret"].as_str().unwrap(), "team", None, 10, 403).await;
        f.db()
            .execute_batch("DROP TRIGGER fail_owner_transfer")
            .unwrap();
        transfer_owner(&f, f.setup.owner_key.secret(), &admin["principal_id"], 200).await;
        f.finish().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn archived_owner_can_rotate_or_revoke_existing_admin_keys_without_expanding_team() {
        let f = Fixture::new().await;
        let admin = team_member(&f, "existing admin").await;
        admin_action(
            &f,
            f.setup.owner_key.secret(),
            "/api/v1/admin/workspaces/archive",
            &json!({"workspace_id":f.setup.workspace_id.to_string()}),
            200,
        )
        .await;
        admin_action(&f,f.setup.owner_key.secret(),"/api/v1/admin/team/appoint",
            &json!({"workspace_id":f.setup.workspace_id.to_string(),"principal":{"kind":"new_principal","name":"frozen"}}),403).await;
        transfer_owner(&f, f.setup.owner_key.secret(), &admin["principal_id"], 403).await;
        let rotated = owner_key(&f, f.setup.owner_key.secret(), &admin["principal_id"], 200).await;
        team_page(&f, f.setup.owner_key.secret(), "team", None, 10, 200).await;
        admin_action(&f,f.setup.owner_key.secret(),"/api/v1/admin/team/revoke",
            &json!({"workspace_id":f.setup.workspace_id.to_string(),"principal_id":admin["principal_id"]}),200).await;
        inspect_workspace(
            &f,
            rotated["secret"].as_str().unwrap(),
            &json!(f.setup.workspace_id.to_string()),
            401,
        )
        .await;
        inspect_workspace(
            &f,
            admin["secret"].as_str().unwrap(),
            &json!(f.setup.workspace_id.to_string()),
            401,
        )
        .await;
        f.finish().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn team_appointment_failure_and_lost_response_have_no_orphan_or_secret_retrieval_route() {
        let f = Fixture::new().await;
        let before = management_counts(&f);
        let body = json!({"workspace_id":f.setup.workspace_id.to_string(),"principal":{"kind":"new_principal","name":"lost admin"}});
        f.db().execute_batch("CREATE TRIGGER fail_appointment BEFORE INSERT ON ccp_access_audit WHEN NEW.action='issue_admin_key' BEGIN SELECT RAISE(ABORT,'injected appointment audit'); END;").unwrap();
        admin_action(
            &f,
            f.setup.owner_key.secret(),
            "/api/v1/admin/team/appoint",
            &body,
            503,
        )
        .await;
        assert_eq!(management_counts(&f), before);
        f.db()
            .execute_batch("DROP TRIGGER fail_appointment")
            .unwrap();
        let response = f
            .public_client()
            .post(f.url("/api/v1/admin/team/appoint"))
            .bearer_auth(f.setup.owner_key.secret())
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 200);
        drop(response);
        let page = team_page(&f, f.setup.owner_key.secret(), "team", None, 10, 200).await;
        let principal = page["admins"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["name"] == "lost admin")
            .unwrap()["id"]
            .clone();
        admin_action(&f,f.setup.owner_key.secret(),"/api/v1/admin/team/appoint",
            &json!({"workspace_id":f.setup.workspace_id.to_string(),"principal":{"kind":"existing","principal_id":principal}}),403).await;
        let fresh = owner_key(&f, f.setup.owner_key.secret(), &principal, 200).await;
        inspect_workspace(
            &f,
            fresh["secret"].as_str().unwrap(),
            &json!(f.setup.workspace_id.to_string()),
            200,
        )
        .await;
        assert!(page.get("secret").is_none());
        f.finish().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn owner_endpoints_reject_scope_inference_identity_claims_methods_and_client_surface() {
        let f = Fixture::new().await;
        let (old, key) = f.enroll().await;
        let before = management_counts(&f);
        for action in [
            "team/list",
            "keys/list",
            "team/revoke",
            "keys/revoke",
            "keys/issue",
            "team/appoint",
            "workspaces/transfer-owner",
        ] {
            let path = format!("/api/v1/admin/{action}");
            let body = json!({"workspace_id":f.setup.workspace_id.to_string(),"actor":"global","scope":"instance"});
            admin_action(&f, f.setup.owner_key.secret(), &path, &body, 400).await;
            json_reply(
                f.public_client()
                    .get(f.url(&path))
                    .bearer_auth(f.setup.owner_key.secret())
                    .send()
                    .await
                    .unwrap(),
                405,
            )
            .await;
            admin_action(
                &f,
                f.setup.owner_key.secret(),
                &format!("{path}?scope=instance"),
                &body,
                400,
            )
            .await;
            json_reply(
                f.private_client(old["certificate_pem"].as_str().unwrap(), &key)
                    .post(f.mtls(&path))
                    .json(&body)
                    .send()
                    .await
                    .unwrap(),
                404,
            )
            .await;
        }
        for action in ["team", "keys"] {
            for limit in [0, 501] {
                team_page(&f, f.setup.owner_key.secret(), action, None, limit, 400).await;
            }
            team_page(
                &f,
                f.setup.owner_key.secret(),
                action,
                Some("invalid"),
                10,
                400,
            )
            .await;
        }
        admin_action(&f,f.setup.owner_key.secret(),"/api/v1/admin/keys/issue",
            &json!({"workspace_id":Value::Null,"principal_id":f.setup.owner_key.principal_id.to_string(),"name":"global"}),400).await;
        assert_eq!(management_counts(&f), before);
        f.finish().await;
    }
}
