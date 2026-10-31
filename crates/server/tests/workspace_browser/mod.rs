// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
mod boundaries;
mod deletion;
mod guidance;
mod initial_window;
mod invitation_list;
mod lifecycle;
mod overview;
mod panel;
mod recovery_list;
mod reported;
mod settings;
mod team;
mod transfer;

const LOGIN: &str = "/api/v1/admin/session/login";
const STATUS: &str = "/api/v1/admin/session";
const LOGOUT: &str = "/api/v1/admin/session/logout";
const RETENTION: &str = "/api/v1/admin/content/retention/get";

fn origin(f: &Fixture) -> String {
    format!("https://{}", f.startup.https_address)
}
async fn login(f: &Fixture, key: &str, previous: Option<&str>) -> (String, Value) {
    let mut request = f
        .public_client()
        .post(f.url(LOGIN))
        .header("origin", origin(f))
        .header("x-ccp-csrf", "login")
        .json(&json!({"key":key}));
    if let Some(previous) = previous {
        request = request.header("cookie", previous);
    }
    let response = request.send().await.unwrap();
    let header = response
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(header.starts_with(&format!("__Host-ccp_admin_{}=", f.trust.instance_id)));
    for flag in [
        "; Path=/",
        "; Secure",
        "; HttpOnly",
        "; SameSite=Strict",
        "; Max-Age=28800",
    ] {
        assert!(header.contains(flag));
    }
    assert!(!header.contains("Domain="));
    let cookie = header.split(';').next().unwrap().to_string();
    let result = json_reply(response, 200).await;
    assert!(!result.to_string().contains(key));
    assert!(
        !result
            .to_string()
            .contains(cookie.split_once('=').unwrap().1)
    );
    (cookie, result["data"].clone())
}
async fn status(f: &Fixture, cookie: &str, status: u16) -> Value {
    let response = f
        .public_client()
        .get(f.url(STATUS))
        .header("cookie", cookie)
        .send()
        .await
        .unwrap();
    assert!(!response.headers().contains_key("set-cookie"));
    assert!(
        !response
            .headers()
            .contains_key("access-control-allow-origin")
    );
    json_reply(response, status).await
}
async fn post(
    f: &Fixture,
    cookie: &str,
    csrf: &str,
    path: &str,
    body: &Value,
    status: u16,
) -> Value {
    json_reply(
        f.public_client()
            .post(f.url(path))
            .header("cookie", cookie)
            .header("origin", origin(f))
            .header("x-ccp-csrf", csrf)
            .json(body)
            .send()
            .await
            .unwrap(),
        status,
    )
    .await
}
fn target(f: &Fixture) -> Value {
    json!({"workspace_id":f.setup.workspace_id.to_string()})
}
fn code(value: &Value, wanted: &str) {
    assert_eq!(value["error"]["code"], wanted);
    assert_eq!(value.as_object().unwrap().len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn browser_owner_login_status_cookie_metadata_and_existing_write_routes_work() {
    let f = Fixture::new().await;
    let (cookie, session) = login(&f, f.setup.owner_key.secret(), None).await;
    assert_eq!(session["instance_id"], f.trust.instance_id);
    assert_eq!(
        session["identity"]["key_id"],
        f.setup.owner_key.id.to_string()
    );
    assert_eq!(
        session["identity"]["principal_id"],
        f.setup.owner_key.principal_id.to_string()
    );
    assert_eq!(session["identity"]["scope"]["kind"], "workspace");
    assert_eq!(session["identity"]["scope"]["role"], "owner");
    assert_eq!(
        session["identity"]["scope"]["workspace_id"],
        f.setup.workspace_id.to_string()
    );
    assert_eq!(session["csrf_token"].as_str().unwrap().len(), 64);
    assert!((now() + 28795..=now() + 28800).contains(&session["expires_at"].as_i64().unwrap()));
    assert_eq!(status(&f, &cookie, 200).await["data"], session);
    let list = json_reply(
        f.public_client()
            .get(f.url("/api/v1/admin/workspaces"))
            .header("cookie", &cookie)
            .send()
            .await
            .unwrap(),
        200,
    )
    .await;
    assert_eq!(list["data"]["workspaces"].as_array().unwrap().len(), 1);
    let csrf = session["csrf_token"].as_str().unwrap();
    assert_eq!(
        post(&f, &cookie, csrf, RETENTION, &target(&f), 200).await["data"]["revision"],
        0
    );
    let shelf=post(&f,&cookie,csrf,"/api/v1/content/shelves/create",&json!({"workspace_id":f.setup.workspace_id.to_string(),"name":"browser shelf","description":"human context"}),200).await;
    assert_eq!(shelf["data"]["name"], "browser shelf");
    let admin=f.db().query_row("SELECT actor_kind FROM ccp_access_audit WHERE action='create_shelf' ORDER BY rowid DESC LIMIT 1",[],|r|r.get::<_,String>(0)).unwrap();
    assert_eq!(admin, "admin");
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn browser_global_read_never_borrows_workspace_write_or_guidance_scope() {
    let f = Fixture::new().await;
    let second = f.second();
    let (cookie, session) = login(&f, f.setup.instance_key.secret(), None).await;
    assert_eq!(session["identity"]["scope"]["kind"], "instance");
    let csrf = session["csrf_token"].as_str().unwrap();
    let list = json_reply(
        f.public_client()
            .get(f.url("/api/v1/admin/workspaces"))
            .header("cookie", &cookie)
            .send()
            .await
            .unwrap(),
        200,
    )
    .await;
    assert_eq!(list["data"]["workspaces"].as_array().unwrap().len(), 2);
    code(&post(&f,&cookie,csrf,"/api/v1/content/shelves/create",&json!({"workspace_id":f.setup.workspace_id.to_string(),"name":"denied","description":""}),403).await,"forbidden");
    let save = json!({"scope":{"kind":"workspace","workspace_id":f.setup.workspace_id.to_string()},"operation_id":Uuid::new_v4().to_string(),"expected_revision":0,"text":"denied"});
    code(
        &post(&f, &cookie, csrf, "/api/v1/admin/guidance/save", &save, 403).await,
        "forbidden",
    );
    let global = json!({"scope":{"kind":"global"},"operation_id":Uuid::new_v4().to_string(),"expected_revision":0,"text":"global live"});
    assert_eq!(
        post(
            &f,
            &cookie,
            csrf,
            "/api/v1/admin/guidance/save",
            &global,
            200
        )
        .await["data"]["revision"],
        1
    );
    let (scoped, session) = login(&f, f.setup.owner_key.secret(), None).await;
    code(
        &post(
            &f,
            &scoped,
            session["csrf_token"].as_str().unwrap(),
            RETENTION,
            &json!({"workspace_id":second.workspace_id.to_string()}),
            403,
        )
        .await,
        "forbidden",
    );
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invalid_creation_names_refuse_input_without_invalidating_browser_sessions() {
    let f = Fixture::new().await;
    let (cookie, session) = login(&f, f.setup.instance_key.secret(), None).await;
    let proof = session["csrf_token"].as_str().unwrap();
    let mut responses = Vec::new();
    for owner_bad in [false, true] {
        for value in [
            " ".to_string(),
            "x".repeat(257),
            "🦑".repeat(65),
            "bad\0name".to_string(),
        ] {
            let body = if owner_bad {
                json!({"name":"new workspace","owner":{"kind":"new_principal","name":value},"key_expires_at":null})
            } else {
                json!({"name":value,"owner":{"kind":"new_principal","name":"new owner"},"key_expires_at":null})
            };
            let response = f
                .public_client()
                .post(f.url("/api/v1/admin/workspaces/create"))
                .header("cookie", &cookie)
                .header("origin", origin(&f))
                .header("x-ccp-csrf", proof)
                .json(&body)
                .send()
                .await
                .unwrap();
            responses.push(response.status().as_u16());
            let current = status(&f, &cookie, 200).await;
            assert_eq!(current["data"]["csrf_token"], session["csrf_token"]);
        }
    }
    let count: i64 = f
        .db()
        .query_row("SELECT COUNT(*) FROM ccp_workspaces", [], |row| row.get(0))
        .unwrap();
    let boundary = post(
        &f,
        &cookie,
        proof,
        "/api/v1/admin/workspaces/create",
        &json!({"name":"a".repeat(256),"owner":{"kind":"new_principal","name":"owner"},"key_expires_at":null}),
        200,
    )
    .await;
    assert!(boundary["data"]["workspace_id"].is_string());
    let (scoped, scoped_session) = login(&f, f.setup.owner_key.secret(), None).await;
    post(
        &f,
        &scoped,
        scoped_session["csrf_token"].as_str().unwrap(),
        "/api/v1/admin/workspaces/create",
        &json!({"name":"forbidden","owner":{"kind":"new_principal","name":"owner"},"key_expires_at":null}),
        403,
    )
    .await;
    f.finish().await;
    assert_eq!(count, 1);
    assert_eq!(responses, vec![400; 8]);
}
