// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Test the actual command, printed trust anchors and graceful restart, not only
//! its in-process server constructor. No setup secrets are printed by this test.
#[cfg(test)]
#[cfg(unix)]
mod tests {
    use serde_json::{Value, json};
    use sha2::{Digest, Sha256};
    use std::{path::Path, process::Stdio, time::Duration};
    use tokio::{
        io::{AsyncBufReadExt, BufReader},
        process::{Child, Command},
        time::timeout,
    };
    use uuid::Uuid;

    struct Started {
        child: Child,
        origin: String,
        code: Option<String>,
        ca: String,
        leaf: String,
    }
    async fn start(directory: &Path) -> Started {
        let mut child = Command::new(env!("CARGO_BIN_EXE_server"))
            .arg("up")
            .arg("--data-dir")
            .arg(directory)
            .args([
                "--https-listen",
                "127.0.0.1:0",
                "--mtls-listen",
                "127.0.0.1:0",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
        let mut output = Vec::new();
        for _ in 0..4 {
            output.push(
                timeout(Duration::from_secs(10), lines.next_line())
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap(),
            );
        }
        let url = reqwest::Url::parse(output[0].strip_prefix("Open: ").unwrap()).unwrap();
        let code = url
            .fragment()
            .map(|v| v.strip_prefix("code=").unwrap().to_owned());
        assert_eq!(
            url.path(),
            if code.is_some() {
                "/admin/setup"
            } else {
                "/admin"
            }
        );
        let ca = output[1]
            .strip_prefix("CA SHA-256: ")
            .unwrap()
            .replace(':', "");
        let leaf = output[2]
            .strip_prefix("Server leaf SHA-256: ")
            .unwrap()
            .replace(':', "");
        assert!(output[3].contains("not installed automatically"));
        Started {
            child,
            origin: url.origin().ascii_serialization(),
            code,
            ca,
            leaf,
        }
    }
    async fn stop(mut started: Started) {
        let id = started.child.id().unwrap().to_string();
        assert!(
            Command::new("/bin/kill")
                .args(["-INT", &id])
                .status()
                .await
                .unwrap()
                .success()
        );
        assert!(
            timeout(Duration::from_secs(10), started.child.wait())
                .await
                .unwrap()
                .unwrap()
                .success()
        );
    }
    fn fingerprint(pem: &str) -> String {
        let (rest, certificate) = x509_parser::pem::parse_x509_pem(pem.as_bytes()).unwrap();
        assert!(rest.iter().all(u8::is_ascii_whitespace));
        let der = certificate.contents;
        Sha256::digest(&der)
            .iter()
            .map(|v| format!("{v:02x}"))
            .collect()
    }
    fn client(directory: &Path, started: &Started) -> reqwest::Client {
        let bytes = std::fs::read(directory.join("ccp-instance-tls.json")).unwrap();
        let bundle: Value = serde_json::from_slice(&bytes).unwrap();
        let ca = bundle["ca_certificate"].as_str().unwrap();
        assert_eq!(fingerprint(ca), started.ca);
        assert_eq!(
            fingerprint(bundle["server_certificate"].as_str().unwrap()),
            started.leaf
        );
        reqwest::Client::builder()
            .no_proxy()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .tls_built_in_root_certs(false)
            .add_root_certificate(reqwest::Certificate::from_pem(ca.as_bytes()).unwrap())
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap()
    }
    #[tokio::test]
    async fn actual_up_command_claims_over_pinned_tls_and_restarts_without_a_new_setup_link() {
        let directory = std::env::temp_dir().join(format!("ccp-up-command-{}", Uuid::new_v4()));
        let first = start(&directory).await;
        let http = client(&directory, &first);
        let code = first.code.as_ref().unwrap();
        assert!(code.starts_with("ccp_setup_"));
        assert_eq!(code.len(), 74);
        let body = json!({"code":code,"operator":"CLI operator","workspace":"CLI workspace"});
        let response = http
            .post(format!("{}/api/v1/setup/claim", first.origin))
            .header("origin", &first.origin)
            .header("x-ccp-csrf", "setup")
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let receipt: Value = response.json().await.unwrap();
        let ca = first.ca.clone();
        stop(first).await;
        let second = start(&directory).await;
        assert!(second.code.is_none());
        assert_eq!(second.ca, ca);
        let http = client(&directory, &second);
        let response = http
            .post(format!("{}/api/v1/admin/session/login", second.origin))
            .header("origin", &second.origin)
            .header("x-ccp-csrf", "login")
            .json(&json!({"key":receipt["data"]["workspace_owner_key"]["secret"]}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        assert!(response.headers().contains_key("set-cookie"));
        stop(second).await;
        std::fs::remove_dir_all(directory).unwrap();
    }
}
