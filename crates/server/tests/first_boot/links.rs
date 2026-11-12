// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;
use protocol::enrollment_link::EnrollmentLink;
const PATH: &str = "/api/v1/admin/invitations/link";
async fn setup(f: &Fixture) -> Value {
    let response = f
        .post(
            "/api/v1/setup/claim",
            &json!({"code":f.code,"operator":"link owner","workspace":"Link workspace"}),
        )
        .await;
    assert_eq!(response.status(), 200);
    response.json::<Value>().await.unwrap()["data"].clone()
}
async fn issue(f: &Fixture, key: &str, body: Value) -> reqwest::Response {
    f.client
        .post(format!("{}{PATH}", f.base))
        .bearer_auth(key)
        .json(&body)
        .send()
        .await
        .unwrap()
}
fn count(f: &Fixture) -> i64 {
    rusqlite::Connection::open(f.root.join("ccp.sqlite3"))
        .unwrap()
        .query_row("SELECT COUNT(*) FROM ccp_invitations", [], |row| row.get(0))
        .unwrap()
}
#[tokio::test]
async fn defaults_and_custom_link_routing_pin_actual_instance_workspace_and_ca() {
    let f = Fixture::start(root(), Vec::new()).await;
    let keys = setup(&f).await;
    let owner = keys["workspace_owner_key"]["secret"].as_str().unwrap();
    let before = i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    )
    .unwrap();
    let response = issue(
        &f,
        owner,
        json!({"workspace_id":keys["workspace_id"],"role":"read_write"}),
    )
    .await;
    assert_eq!(response.status(), 200);
    assert!(!response.headers().contains_key("set-cookie"));
    assert_eq!(response.headers()["cache-control"], "no-store");
    let value = response.json::<Value>().await.unwrap()["data"].clone();
    let parsed = EnrollmentLink::parse(value["enrollment_link"].as_str().unwrap()).unwrap();
    assert_eq!(parsed.target().https_origin, format!("{}/", f.base));
    assert_eq!(
        parsed.target().mtls_origin,
        format!("https://127.0.0.1:{}/", f.mtls_port)
    );
    assert_eq!(parsed.target().instance_id.to_string(), f.trust.instance_id);
    assert_eq!(
        parsed.target().workspace_id.to_string(),
        keys["workspace_id"]
    );
    assert_eq!(parsed.target().workspace_name, "Link workspace");
    let pin: String = parsed
        .target()
        .ca_sha256
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(pin, f.trust.ca_sha256);
    assert!(value["expires_at"].as_i64().unwrap() >= before + 1800);
    assert!(value["expires_at"].as_i64().unwrap() <= before + 1805);
    assert_eq!(value["max_uses"], 1);
    let custom = issue(
        &f,
        owner,
        json!({
            "workspace_id":keys["workspace_id"],"role":"read",
            "lifetime":{"kind":"at","expires_at":before+90*86400,"confirm_long_lived":true},
            "mtls_origin":"https://localhost:9444",
        }),
    )
    .await;
    assert_eq!(custom.status(), 200);
    let custom = custom.json::<Value>().await.unwrap();
    let parsed =
        EnrollmentLink::parse(custom["data"]["enrollment_link"].as_str().unwrap()).unwrap();
    assert_eq!(parsed.target().mtls_origin, "https://localhost:9444/");
    assert_eq!(custom["data"]["expires_at"], before + 90 * 86400);
    let (directory, _) = f.stop().await;
    std::fs::remove_dir_all(directory).unwrap();
}
#[tokio::test]
async fn missing_confirmation_invalid_policy_and_invalid_targets_never_mint_links() {
    let f = Fixture::start(root(), Vec::new()).await;
    let keys = setup(&f).await;
    let owner = keys["workspace_owner_key"]["secret"].as_str().unwrap();
    let base = json!({"workspace_id":keys["workspace_id"],"role":"read"});
    for extra in [
        json!({"lifetime":{"kind":"never","confirm_no_expiry":false}}),
        json!({"lifetime":{"kind":"never"}}),
        json!({"lifetime":{"kind":"short","confirm_no_expiry":true}}),
        json!({"lifetime":{"kind":"at","expires_at":0}}),
        json!({"mtls_origin":"http://localhost:8444"}),
        json!({"mtls_origin":"https://user@localhost:8444"}),
        json!({"mtls_origin":"https://xn--zz.example"}),
        json!({"mtls_origin":"https://[::ffff:0.0.0.0]"}),
        json!({"unrecognized":true}),
        json!({"lifetime":{"kind":"at","expires_at":4102444800_i64}}),
        json!({"admission":{"kind":"initial_group"},"lifetime":{"kind":"at","expires_at":4102444800_i64,"confirm_long_lived":true}}),
        json!({"admission":{"kind":"individual","max_clients":100}}),
        json!({"admission":{"kind":"initial_group","max_clients":0}}),
        json!({"admission":{"kind":"initial_group","max_clients":null},"lifetime":{"kind":"never","confirm_no_expiry":true}}),
    ] {
        let mut body = base.clone();
        body.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        assert_eq!(
            issue(&f, owner, body.clone()).await.status(),
            400,
            "rejected policy: {body}"
        );
        assert_eq!(count(&f), 0);
    }
    let response=issue(&f,owner,json!({"workspace_id":keys["workspace_id"],"role":"read","lifetime":{"kind":"never","confirm_no_expiry":true}})).await;
    assert_eq!(response.status(), 200);
    let value = response.json::<Value>().await.unwrap();
    assert!(value["data"]["expires_at"].is_null());
    assert_eq!(value["data"]["max_uses"], 1);
    let (directory, _) = f.stop().await;
    std::fs::remove_dir_all(directory).unwrap();
}
#[tokio::test]
async fn global_key_cannot_borrow_scoped_issuance_and_archiving_blocks_new_links() {
    let f = Fixture::start(root(), Vec::new()).await;
    let keys = setup(&f).await;
    let body = json!({"workspace_id":keys["workspace_id"],"role":"read"});
    let global = keys["instance_admin_key"]["secret"].as_str().unwrap();
    assert_eq!(issue(&f, global, body.clone()).await.status(), 403);
    assert_eq!(issue(&f, "invalid", body.clone()).await.status(), 401);
    assert_eq!(count(&f), 0);
    let owner = keys["workspace_owner_key"]["secret"].as_str().unwrap();
    let archived = f
        .client
        .post(format!("{}/api/v1/admin/workspaces/archive", f.base))
        .bearer_auth(owner)
        .json(&json!({"workspace_id":keys["workspace_id"]}))
        .send()
        .await
        .unwrap();
    assert_eq!(archived.status(), 200);
    assert_eq!(issue(&f, owner, body).await.status(), 403);
    assert_eq!(count(&f), 0);
    let (directory, _) = f.stop().await;
    std::fs::remove_dir_all(directory).unwrap();
}

#[tokio::test]
async fn issued_uri_redeems_exactly_once_and_certificate_reads_only_its_workspace() {
    let f = Fixture::start(root(), Vec::new()).await;
    let keys = setup(&f).await;
    let issued = issue(
        &f,
        keys["workspace_owner_key"]["secret"].as_str().unwrap(),
        json!({
            "workspace_id":keys["workspace_id"],"role":"read",
            "lifetime":{"kind":"never","confirm_no_expiry":true},
        }),
    )
    .await;
    assert_eq!(issued.status(), 200);
    let value = issued.json::<Value>().await.unwrap();
    let link = EnrollmentLink::parse(value["data"]["enrollment_link"].as_str().unwrap()).unwrap();
    let key = rcgen::KeyPair::generate().unwrap();
    let csr = rcgen::CertificateParams::new(Vec::<String>::new())
        .unwrap()
        .serialize_request(&key)
        .unwrap()
        .pem()
        .unwrap();
    let body = json!({"invitation":link.invitation(),"request_id":Uuid::new_v4().to_string(),"csr_pem":csr});
    let target = format!("{}api/v1/enroll", link.target().https_origin);
    let response = f.client.post(&target).json(&body).send().await.unwrap();
    assert_eq!(response.status(), 200);
    let receipt = response.json::<Value>().await.unwrap()["data"].clone();
    let retry = f.client.post(&target).json(&body).send().await.unwrap();
    assert_eq!(retry.status(), 200);
    assert_eq!(retry.json::<Value>().await.unwrap()["data"], receipt);
    let cert = receipt["certificate_pem"].as_str().unwrap();
    let enrolled = reqwest::Client::builder()
        .https_only(true)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .tls_built_in_root_certs(false)
        .add_root_certificate(
            reqwest::Certificate::from_pem(f.trust.ca_certificate_pem.as_bytes()).unwrap(),
        )
        .identity(
            reqwest::Identity::from_pem(format!("{}{}", key.serialize_pem(), cert).as_bytes())
                .unwrap(),
        )
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap();
    let read = enrolled
        .get(format!("{}api/v1/workspaces", link.target().mtls_origin))
        .send()
        .await
        .unwrap();
    assert_eq!(read.status(), 200);
    let page = read.json::<Value>().await.unwrap();
    assert_eq!(page["data"]["workspaces"].as_array().unwrap().len(), 1);
    assert_eq!(page["data"]["workspaces"][0]["id"], keys["workspace_id"]);
    let join_denied = enrolled
        .get(format!("{}join", link.target().mtls_origin))
        .send()
        .await
        .unwrap();
    assert_eq!(join_denied.status(), 404);
    let admin_denied = enrolled
        .post(format!(
            "{}api/v1/admin/invitations/link",
            link.target().mtls_origin
        ))
        .json(&json!({"workspace_id":keys["workspace_id"],"role":"read"}))
        .send()
        .await
        .unwrap();
    assert_eq!(admin_denied.status(), 404);
    let fresh_key = rcgen::KeyPair::generate().unwrap();
    let fresh_csr = rcgen::CertificateParams::new(Vec::<String>::new())
        .unwrap()
        .serialize_request(&fresh_key)
        .unwrap()
        .pem()
        .unwrap();
    let denied=f.client.post(&target).json(&json!({"invitation":link.invitation(),"request_id":Uuid::new_v4().to_string(),"csr_pem":fresh_csr})).send().await.unwrap();
    assert_eq!(denied.status(), 403);
    let db = rusqlite::Connection::open(f.root.join("ccp.sqlite3")).unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM ccp_clients", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM ccp_grants", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    drop(db);
    let (directory, _) = f.stop().await;
    std::fs::remove_dir_all(directory).unwrap();
}
#[tokio::test]
async fn invalid_existing_display_metadata_is_refused_before_minting_a_bearer() {
    let f = Fixture::start(root(), Vec::new()).await;
    let keys = setup(&f).await;
    let db = rusqlite::Connection::open(f.root.join("ccp.sqlite3")).unwrap();
    db.execute(
        "UPDATE ccp_workspaces SET name=?1 WHERE id=?2",
        rusqlite::params!["x".repeat(257), keys["workspace_id"].as_str().unwrap()],
    )
    .unwrap();
    let before: i64 = db
        .query_row("SELECT COUNT(*) FROM ccp_access_audit", [], |r| r.get(0))
        .unwrap();
    let response = issue(
        &f,
        keys["workspace_owner_key"]["secret"].as_str().unwrap(),
        json!({"workspace_id":keys["workspace_id"],"role":"read"}),
    )
    .await;
    assert_eq!(response.status(), 400);
    assert_eq!(count(&f), 0);
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM ccp_initial_enrollment", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM ccp_access_audit", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        before
    );
    drop(db);
    let (directory, _) = f.stop().await;
    std::fs::remove_dir_all(directory).unwrap();
}

#[tokio::test]
async fn public_origin_override_keeps_pins_and_does_not_inherit_an_admin_forward() {
    let f = Fixture::start(root(), Vec::new()).await;
    let keys = setup(&f).await;
    let response=issue(&f,keys["workspace_owner_key"]["secret"].as_str().unwrap(),json!({
        "workspace_id":keys["workspace_id"],"role":"read",
        "https_origin":"https://public.example:9443","mtls_origin":"https://clients.example:9444",
    })).await;
    assert_eq!(response.status(), 200);
    let value = response.json::<Value>().await.unwrap();
    let link = EnrollmentLink::parse(value["data"]["enrollment_link"].as_str().unwrap()).unwrap();
    assert_eq!(link.target().https_origin, "https://public.example:9443/");
    assert_eq!(link.target().mtls_origin, "https://clients.example:9444/");
    assert_eq!(link.target().instance_id.to_string(), f.trust.instance_id);
    let bad=issue(&f,keys["workspace_owner_key"]["secret"].as_str().unwrap(),json!({
        "workspace_id":keys["workspace_id"],"role":"read","https_origin":"https://user@public.example",
    })).await;
    assert_eq!(bad.status(), 400);
    assert_eq!(count(&f), 1);
    let (directory, _) = f.stop().await;
    std::fs::remove_dir_all(directory).unwrap();
}

#[tokio::test]
async fn public_join_shell_is_https_only_no_store_and_rejects_queries_and_posts() {
    let f = Fixture::start(root(), Vec::new()).await;
    let response = f
        .client
        .get(format!("{}/join", f.base))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(response.headers()["referrer-policy"], "no-referrer");
    assert!(
        response.headers()["content-security-policy"]
            .to_str()
            .unwrap()
            .contains("form-action 'none'")
    );
    let body = response.text().await.unwrap();
    assert!(body.contains("/admin/app.js"));
    assert!(!body.contains("ccp_join_"));
    assert_eq!(
        f.client
            .get(format!("{}/join?t=private", f.base))
            .send()
            .await
            .unwrap()
            .status(),
        400
    );
    assert_eq!(
        f.client
            .post(format!("{}/join", f.base))
            .json(&json!({}))
            .send()
            .await
            .unwrap()
            .status(),
        405
    );
    assert_eq!(count(&f), 0);
    let (directory, _) = f.stop().await;
    std::fs::remove_dir_all(directory).unwrap();
}
