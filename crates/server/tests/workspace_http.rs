// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

#[cfg(test)]
mod tests {
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
            .unwrap();
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
    struct Wire {
        stream: TlsStream<TcpStream>,
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
}
