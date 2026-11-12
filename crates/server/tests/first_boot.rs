// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later
#[cfg(test)]
#[path = "."]
mod tests {
    #[path = "first_boot/links.rs"]
    mod links;
    use serde_json::{Value, json};
    use server::workspace_http::{UpConfig, WorkspaceServer};
    use std::{path::PathBuf, sync::Arc};
    use tokio::sync::oneshot;
    use uuid::Uuid;
    struct Fixture {
        root: PathBuf,
        base: String,
        mtls_port: u16,
        code: String,
        client: reqwest::Client,
        trust: server::instance_tls::TrustInfo,
        stop: oneshot::Sender<()>,
        task: tokio::task::JoinHandle<anyhow::Result<server::workspace_http::Report>>,
    }
    impl Fixture {
        async fn start(root: PathBuf, names: Vec<String>) -> Self {
            let (server, code) = WorkspaceServer::up(UpConfig {
                data_dir: root.clone(),
                names,
                https_listen: "127.0.0.1:0".parse().unwrap(),
                mtls_listen: "127.0.0.1:0".parse().unwrap(),
            })
            .await
            .unwrap();
            let trust = server.trust_info().clone();
            let client = reqwest::Client::builder()
                .https_only(true)
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .tls_built_in_root_certs(false)
                .add_root_certificate(
                    reqwest::Certificate::from_pem(trust.ca_certificate_pem.as_bytes()).unwrap(),
                )
                .timeout(std::time::Duration::from_secs(5))
                .build()
                .unwrap();
            let startup = server.startup().unwrap();
            let base = format!("https://{}", startup.https_address);
            let mtls_port = startup.mtls_address.port();
            let (stop, rx) = oneshot::channel();
            let task = tokio::spawn(server.run_until(async {
                rx.await?;
                Ok(())
            }));
            Self {
                root,
                base,
                mtls_port,
                code: code.unwrap_or_default(),
                client,
                trust,
                stop,
                task,
            }
        }
        async fn post(&self, path: &str, body: &Value) -> reqwest::Response {
            self.client
                .post(format!("{}{path}", self.base))
                .header("origin", &self.base)
                .header("x-ccp-csrf", "setup")
                .json(body)
                .send()
                .await
                .unwrap()
        }
        fn admins(&self) -> u64 {
            rusqlite::Connection::open(self.root.join("ccp.sqlite3"))
                .unwrap()
                .query_row("SELECT COUNT(*) FROM ccp_server_admins", [], |r| r.get(0))
                .unwrap()
        }
        async fn stop(self) -> (PathBuf, server::instance_tls::TrustInfo) {
            self.stop.send(()).unwrap();
            self.task.await.unwrap().unwrap();
            (self.root, self.trust)
        }
    }
    fn root() -> PathBuf {
        std::env::temp_dir().join(format!("ccp-first-boot-{}", Uuid::new_v4()))
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn empty_start_pinned_https_code_claim_login_and_restart_preserve_identity() {
        let f = Fixture::start(root(), Vec::new()).await;
        assert!(!f.code.is_empty());
        assert_eq!(f.admins(), 0);
        let ca = f
            .client
            .get(format!("{}/api/v1/ca.pem", f.base))
            .send()
            .await
            .unwrap();
        assert_eq!(ca.status(), 200);
        assert!(!ca.headers().contains_key("set-cookie"));
        assert_eq!(ca.text().await.unwrap(), f.trust.ca_certificate_pem);
        let verify = f
            .post("/api/v1/setup/verify", &json!({"code":f.code}))
            .await;
        assert_eq!(verify.status(), 200);
        assert!(!verify.headers().contains_key("set-cookie"));
        assert_eq!(f.admins(), 0);
        let claim = f
            .post(
                "/api/v1/setup/claim",
                &json!({"code":f.code,"operator":"owner","workspace":"first"}),
            )
            .await;
        assert_eq!(claim.status(), 200);
        assert!(!claim.headers().contains_key("set-cookie"));
        let keys: Value = claim.json().await.unwrap();
        assert_eq!(f.admins(), 1);
        let login = f
            .client
            .post(format!("{}/api/v1/admin/session/login", f.base))
            .header("origin", &f.base)
            .header("x-ccp-csrf", "login")
            .json(&json!({"key":keys["data"]["workspace_owner_key"]["secret"]}))
            .send()
            .await
            .unwrap();
        assert_eq!(login.status(), 200);
        assert!(login.headers().contains_key("set-cookie"));
        assert_eq!(
            f.post(
                "/api/v1/setup/claim",
                &json!({"code":f.code,"operator":"another","workspace":"another"})
            )
            .await
            .status(),
            409
        );
        let (directory, trust) = f.stop().await;
        let f = Fixture::start(directory, Vec::new()).await;
        assert!(f.code.is_empty());
        assert_eq!(f.trust, trust);
        assert_eq!(f.admins(), 1);
        assert_eq!(
            f.post("/api/v1/setup/verify", &json!({"code":"old"}))
                .await
                .status(),
            409
        );
        let (directory, _) = f.stop().await;
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn bad_cross_origin_and_throttled_requests_cannot_claim_or_lock_out_valid_code() {
        let f = Fixture::start(root(), Vec::new()).await;
        let body = json!({"code":f.code,"operator":"owner","workspace":"first"});
        let cross = f
            .client
            .post(format!("{}/api/v1/setup/claim", f.base))
            .header("origin", "https://evil.invalid")
            .header("x-ccp-csrf", "setup")
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(cross.status(), 403);
        assert_eq!(
            f.client
                .post(format!("{}/api/v1/setup/claim", f.base))
                .json(&body)
                .send()
                .await
                .unwrap()
                .status(),
            403
        );
        for _ in 0..16 {
            assert_eq!(
                f.post("/api/v1/setup/verify", &json!({"code":"invalid"}))
                    .await
                    .status(),
                401
            );
        }
        assert_eq!(
            f.post("/api/v1/setup/verify", &json!({"code":"invalid"}))
                .await
                .status(),
            429
        );
        assert_eq!(f.admins(), 0);
        let verify = f
            .post("/api/v1/setup/verify", &json!({"code":f.code}))
            .await;
        assert_eq!(verify.status(), 200);
        assert_eq!(f.post("/api/v1/setup/claim", &body).await.status(), 200);
        assert_eq!(f.admins(), 1);
        let (directory, _) = f.stop().await;
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_valid_claims_create_one_owner_and_bad_names_remain_retryable() {
        let f = Arc::new(Fixture::start(root(), Vec::new()).await);
        assert_eq!(
            f.post(
                "/api/v1/setup/claim",
                &json!({"code":f.code,"operator":"","workspace":"first"})
            )
            .await
            .status(),
            400
        );
        assert_eq!(f.admins(), 0);
        let tasks: Vec<_> = (0..8)
            .map(|n| {
                let f = f.clone();
                tokio::spawn(async move {
                    f.post(
                        "/api/v1/setup/claim",
                        &json!({"code":f.code,"operator":format!("owner{n}"),"workspace":"first"}),
                    )
                    .await
                    .status()
                    .as_u16()
                })
            })
            .collect();
        let mut successes = 0;
        for task in tasks {
            match task.await.unwrap() {
                200 => successes += 1,
                409 => {}
                code => panic!("unexpected status {code}"),
            }
        }
        assert_eq!(successes, 1);
        assert_eq!(f.admins(), 1);
        let f = Arc::try_unwrap(f).ok().unwrap();
        let (directory, _) = f.stop().await;
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pending_restart_rotates_only_code_and_keeps_custom_names() {
        let f = Fixture::start(root(), vec!["custom.example".into(), "127.0.0.1".into()]).await;
        let old = f.code.clone();
        let (directory, trust) = f.stop().await;
        let f = Fixture::start(directory, Vec::new()).await;
        assert_ne!(f.code, old);
        assert_eq!(f.trust, trust);
        assert_eq!(
            f.post("/api/v1/setup/verify", &json!({"code":old}))
                .await
                .status(),
            401
        );
        assert_eq!(
            f.post("/api/v1/setup/verify", &json!({"code":f.code}))
                .await
                .status(),
            200
        );
        let (directory, _) = f.stop().await;
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[tokio::test]
    async fn occupied_listeners_leave_missing_data_directory_missing() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        for https in [true, false] {
            let directory = root();
            let free = "127.0.0.1:0".parse().unwrap();
            let busy = listener.local_addr().unwrap();
            let result = WorkspaceServer::up(UpConfig {
                data_dir: directory.clone(),
                names: Vec::new(),
                https_listen: if https { busy } else { free },
                mtls_listen: if https { free } else { busy },
            })
            .await;
            assert!(result.is_err());
            assert!(!directory.exists());
        }
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn committed_setup_marker_prevents_reclaim_even_if_global_role_is_removed() {
        let f = Fixture::start(root(), Vec::new()).await;
        assert_eq!(
            f.post(
                "/api/v1/setup/claim",
                &json!({"code":f.code,"operator":"owner","workspace":"first"})
            )
            .await
            .status(),
            200
        );
        let db = rusqlite::Connection::open(f.root.join("ccp.sqlite3")).unwrap();
        assert!(
            db.query_row("SELECT claimed_at FROM ccp_setup_reservation", [], |row| {
                row.get::<_, Option<i64>>(0)
            })
            .unwrap()
            .is_some()
        );
        db.execute("DELETE FROM ccp_server_admins", []).unwrap();
        drop(db);
        let (directory, trust) = f.stop().await;
        let f = Fixture::start(directory, Vec::new()).await;
        assert!(f.code.is_empty());
        assert_eq!(f.trust, trust);
        assert_eq!(
            f.post(
                "/api/v1/setup/claim",
                &json!({"code":"new","operator":"attacker","workspace":"new"})
            )
            .await
            .status(),
            409
        );
        let (directory, _) = f.stop().await;
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn setup_request_shape_and_headers_are_enforced_without_owner_creation() {
        let f = Fixture::start(root(), Vec::new()).await;
        for (suffix, status) in [
            ("/api/v1/setup/verify", 400),
            ("/api/v1/setup/unknown", 404),
        ] {
            assert_eq!(
                f.client
                    .get(format!("{}{suffix}", f.base))
                    .send()
                    .await
                    .unwrap()
                    .status(),
                status
            );
        }
        let target = format!("{}/api/v1/setup/claim", f.base);
        let body = json!({"code":f.code,"operator":"owner","workspace":"first"});
        assert_eq!(
            f.client
                .post(&target)
                .header("origin", &f.base)
                .header("x-ccp-csrf", "wrong")
                .json(&body)
                .send()
                .await
                .unwrap()
                .status(),
            403
        );
        assert_eq!(
            f.client
                .post(&target)
                .header("origin", &f.base)
                .header("x-ccp-csrf", "setup")
                .bearer_auth("not-a-setup-code")
                .json(&body)
                .send()
                .await
                .unwrap()
                .status(),
            403
        );
        let extra =
            json!({"code":f.code,"operator":"owner","workspace":"first","role":"global_admin"});
        assert_eq!(f.post("/api/v1/setup/claim", &extra).await.status(), 400);
        assert_eq!(
            f.post("/api/v1/setup/claim?code=ignored", &body)
                .await
                .status(),
            400
        );
        let response = f
            .client
            .post(&target)
            .header("origin", &f.base)
            .header("x-ccp-csrf", "setup")
            .header("content-type", "text/plain")
            .body("{} ")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 415);
        let oversized = json!({"code":"x".repeat(65537),"operator":"owner","workspace":"first"});
        let response = f.post("/api/v1/setup/claim", &oversized).await;
        assert_eq!(response.status(), 413);
        assert_eq!(f.admins(), 0);
        assert_eq!(f.post("/api/v1/setup/claim", &body).await.status(), 200);
        let (directory, _) = f.stop().await;
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn claim_database_failures_are_unavailable_and_retryable_not_name_errors() {
        let f = Fixture::start(root(), Vec::new()).await;
        let db = rusqlite::Connection::open(f.root.join("ccp.sqlite3")).unwrap();
        db.execute_batch("CREATE TRIGGER fail_setup BEFORE INSERT ON ccp_admin_principals BEGIN SELECT RAISE(ABORT, 'PRIVATE_DATABASE_FAILURE'); END;").unwrap();
        let body = json!({"code":f.code,"operator":"owner","workspace":"first"});
        let failed = f.post("/api/v1/setup/claim", &body).await;
        assert_eq!(failed.status(), 503);
        let text = failed.text().await.unwrap();
        assert!(!text.contains("PRIVATE_DATABASE_FAILURE"));
        assert!(!text.contains(&f.code));
        assert_eq!(f.admins(), 0);
        assert_eq!(
            f.post("/api/v1/setup/verify", &json!({"code":f.code}))
                .await
                .status(),
            200
        );
        db.execute_batch("DROP TRIGGER fail_setup;").unwrap();
        drop(db);
        assert_eq!(f.post("/api/v1/setup/claim", &body).await.status(), 200);
        let (directory, _) = f.stop().await;
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[tokio::test]
    async fn advertised_origins_use_pinned_names_and_never_unspecified_addresses() {
        for (names, prefix) in [
            (vec!["127.0.0.1".to_owned()], "https://127.0.0.1:"),
            (vec!["::1".to_owned()], "https://[::1]:"),
            (
                vec!["custom.example".to_owned(), "localhost".to_owned()],
                "https://localhost:",
            ),
        ] {
            let directory = root();
            let (server, code) = WorkspaceServer::up(UpConfig {
                data_dir: directory.clone(),
                names,
                https_listen: "127.0.0.1:0".parse().unwrap(),
                mtls_listen: "127.0.0.1:0".parse().unwrap(),
            })
            .await
            .unwrap();
            assert!(code.is_some());
            assert!(server.panel_origin().unwrap().starts_with(prefix));
            drop(server);
            std::fs::remove_dir_all(directory).unwrap();
        }
        for name in ["0.0.0.0", "::"] {
            let directory = root();
            assert!(
                WorkspaceServer::up(UpConfig {
                    data_dir: directory.clone(),
                    names: vec![name.into()],
                    https_listen: "127.0.0.1:0".parse().unwrap(),
                    mtls_listen: "127.0.0.1:0".parse().unwrap(),
                })
                .await
                .is_err()
            );
            assert!(!directory.exists());
        }
    }
}
