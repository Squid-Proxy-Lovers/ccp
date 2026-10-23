// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
mod worker;

const GET: &str = "/api/v1/admin/content/retention/get";
const CHANGE: &str = "/api/v1/admin/content/retention/change";

fn change(f: &Fixture, revision: u64, seconds: Option<u64>) -> Value {
    json!({"workspace_id":f.setup.workspace_id.to_string(),"operation_id":Uuid::new_v4().to_string(),"expected_revision":revision,"retention_seconds":seconds,"confirm_timed_purge":seconds.is_some()})
}
async fn request(f: &Fixture, key: &str, path: &str, body: &Value, status: u16) -> Value {
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
    .await
}
async fn policy(f: &Fixture) -> Value {
    request(
        f,
        f.setup.owner_key.secret(),
        GET,
        &json!({"workspace_id":f.setup.workspace_id.to_string()}),
        200,
    )
    .await["data"]
        .clone()
}
fn count(f: &Fixture, table: &str) -> u64 {
    f.db()
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retention_https_current_scope_cas_replay_and_archived_freeze() {
    let f = Fixture::new().await;
    let owner = f.setup.owner_key.secret();
    let initial = policy(&f).await;
    assert_eq!(initial["revision"], 0);
    assert!(initial["retention_seconds"].is_null());
    let enable = change(&f, 0, Some(3600));
    code(
        &request(&f, f.setup.instance_key.secret(), CHANGE, &enable, 403).await,
        "forbidden",
    );
    assert_eq!(
        request(
            &f,
            f.setup.instance_key.secret(),
            GET,
            &json!({"workspace_id":f.setup.workspace_id.to_string()}),
            200
        )
        .await["data"],
        initial
    );
    let saved = request(&f, owner, CHANGE, &enable, 200).await["data"].clone();
    assert_eq!(saved["operation_id"], enable["operation_id"]);
    assert_eq!(saved["policy"]["revision"], 1);
    assert_eq!(
        request(&f, owner, CHANGE, &enable, 200).await["data"],
        saved
    );
    let mut altered = enable.clone();
    altered["retention_seconds"] = json!(5);
    code(
        &request(&f, owner, CHANGE, &altered, 409).await,
        "content_conflict",
    );
    code(
        &request(&f, owner, CHANGE, &change(&f, 0, Some(3600)), 409).await,
        "content_conflict",
    );
    let disable = change(&f, 1, None);
    let manual = request(&f, owner, CHANGE, &disable, 200).await["data"]["policy"].clone();
    assert_eq!(manual["revision"], 2);
    assert!(manual["retention_seconds"].is_null());
    assert_eq!(
        request(&f, owner, CHANGE, &enable, 200).await["data"],
        saved
    );
    assert_eq!(policy(&f).await, manual);
    assert_eq!(count(&f, "ccp_entry_retention_clocks"), 0);
    request(
        &f,
        owner,
        "/api/v1/admin/workspaces/archive",
        &json!({"workspace_id":f.setup.workspace_id.to_string()}),
        200,
    )
    .await;
    assert_eq!(policy(&f).await, manual);
    code(
        &request(&f, owner, CHANGE, &disable, 403).await,
        "forbidden",
    );
    code(
        &request(&f, owner, CHANGE, &change(&f, 2, Some(1)), 403).await,
        "forbidden",
    );
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retention_routes_are_admin_https_only_strict_and_bounded() {
    let f = Fixture::new().await;
    let owner = f.setup.owner_key.secret();
    let user = enrolled(&f, "read_write").await;
    for route in [GET, CHANGE] {
        code(
            &json_reply(
                user.client
                    .post(f.mtls(route))
                    .json(&change(&f, 0, Some(10)))
                    .send()
                    .await
                    .unwrap(),
                404,
            )
            .await,
            "not_found",
        );
        code(
            &json_reply(
                user.client
                    .post(f.mtls(route))
                    .bearer_auth(owner)
                    .json(&json!({}))
                    .send()
                    .await
                    .unwrap(),
                400,
            )
            .await,
            "invalid_request",
        );
        code(
            &json_reply(
                f.public_client()
                    .get(f.url(route))
                    .bearer_auth(owner)
                    .send()
                    .await
                    .unwrap(),
                405,
            )
            .await,
            "method_not_allowed",
        );
        code(
            &json_reply(
                f.public_client()
                    .post(f.url(route))
                    .json(&json!({}))
                    .send()
                    .await
                    .unwrap(),
                401,
            )
            .await,
            "unauthorized",
        );
    }
    let mut bodies = Vec::new();
    for field in ["client_id", "actor_id", "profile_path", "admin_key"] {
        let mut body = change(&f, 0, Some(100));
        body[field] = json!("PRIVATE_BODY");
        bodies.push(body);
    }
    for (field, value) in [
        ("retention_seconds", json!(0)),
        ("retention_seconds", json!(-1)),
        ("retention_seconds", json!(true)),
        ("retention_seconds", json!("1")),
        ("confirm_timed_purge", json!(false)),
        ("expected_revision", json!(true)),
        ("operation_id", json!(Uuid::nil().to_string())),
    ] {
        let mut body = change(&f, 0, Some(100));
        body[field] = value;
        bodies.push(body);
    }
    let mut missing = change(&f, 0, None);
    missing.as_object_mut().unwrap().remove("retention_seconds");
    bodies.push(missing);
    for body in bodies {
        let value = request(&f, owner, CHANGE, &body, 400).await;
        code(&value, "invalid_request");
        assert!(!value.to_string().contains("PRIVATE_BODY"));
    }
    oversized_change(&f).await;
    assert_eq!(count(&f, "ccp_entry_retention_versions"), 0);
    f.finish().await;
}

async fn oversized_change(f: &Fixture) {
    let owner = f.setup.owner_key.secret();
    let huge = "x".repeat(64 * 1024 + 1);
    code(
        &json_reply(
            f.public_client()
                .post(f.url(CHANGE))
                .bearer_auth(owner)
                .header("content-type", "application/json")
                .body(huge)
                .send()
                .await
                .unwrap(),
            413,
        )
        .await,
        "request_too_large",
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retention_admin_foreign_scope_revocation_and_ignored_audit_are_enforced() {
    let f = Fixture::new().await;
    let second = f.second();
    code(
        &request(
            &f,
            second.owner_key.secret(),
            CHANGE,
            &change(&f, 0, Some(10)),
            403,
        )
        .await,
        "forbidden",
    );
    let mut db = f.db();
    let mut store = AccessStore::new(&mut db).unwrap();
    let owner = store
        .authenticate_admin_key(f.setup.owner_key.secret(), now())
        .unwrap();
    let admin = store
        .appoint_workspace_admin_with_key(
            owner,
            f.setup.workspace_id,
            &server::foundation::AdminAppointment {
                principal: InitialOwner::NewPrincipal("retention admin".into()),
                key_expires_at: None,
            },
            now(),
        )
        .unwrap();
    drop(db);
    let enable = change(&f, 0, Some(3600));
    request(&f, admin.secret(), CHANGE, &enable, 200).await;
    f.db()
        .execute(
            "DELETE FROM ccp_workspace_admins WHERE workspace_id=?1 AND admin_id=?2",
            rusqlite::params![
                f.setup.workspace_id.to_string(),
                admin.principal_id.to_string()
            ],
        )
        .unwrap();
    code(
        &request(&f, admin.secret(), CHANGE, &enable, 401).await,
        "unauthorized",
    );
    let manual = change(&f, 1, None);
    f.db().execute_batch("CREATE TRIGGER retention_http_fault BEFORE INSERT ON ccp_access_audit WHEN NEW.action='configure_entry_retention' BEGIN SELECT RAISE(IGNORE); END;").unwrap();
    code(
        &request(&f, f.setup.owner_key.secret(), CHANGE, &manual, 503).await,
        "service_unavailable",
    );
    assert_eq!(policy(&f).await["revision"], 1);
    assert_eq!(count(&f, "ccp_entry_retention_versions"), 1);
    f.db()
        .execute_batch("DROP TRIGGER retention_http_fault")
        .unwrap();
    request(&f, f.setup.owner_key.secret(), CHANGE, &manual, 200).await;
    f.db()
        .execute(
            "UPDATE ccp_admin_keys SET revoked_at=?1 WHERE id=?2",
            rusqlite::params![now(), f.setup.owner_key.id.to_string()],
        )
        .unwrap();
    code(
        &request(&f, f.setup.owner_key.secret(), CHANGE, &manual, 401).await,
        "unauthorized",
    );
    f.finish().await;
}
