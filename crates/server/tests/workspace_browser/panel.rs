// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn panel_assets_are_bundled_public_https_only_and_locked_down() {
    let f = Fixture::new().await;
    for (path, mime) in [
        ("/admin", "text/html"),
        ("/admin/", "text/html"),
        ("/admin/panel.css", "text/css"),
        ("/admin/app.js", "text/javascript"),
        ("/admin/model.js", "text/javascript"),
        ("/admin/view.js", "text/javascript"),
        ("/admin/context.js", "text/javascript"),
        ("/admin/access.js", "text/javascript"),
        ("/admin/guidance.js", "text/javascript"),
        ("/admin/settings.js", "text/javascript"),
        ("/admin/team.js", "text/javascript"),
        ("/admin/transfer.js", "text/javascript"),
        ("/admin/invitations.js", "text/javascript"),
        ("/admin/enrollment.js", "text/javascript"),
        ("/admin/reported.js", "text/javascript"),
        ("/admin/archives.js", "text/javascript"),
        ("/admin/client_access.js", "text/javascript"),
    ] {
        let response = f.public_client().get(f.url(path)).send().await.unwrap();
        assert_eq!(response.status(), 200);
        let headers = response.headers();
        assert!(headers["content-type"].to_str().unwrap().starts_with(mime));
        assert_eq!(headers["cache-control"], "no-store");
        assert_eq!(headers["x-content-type-options"], "nosniff");
        assert_eq!(headers["referrer-policy"], "no-referrer");
        assert_eq!(headers["x-frame-options"], "DENY");
        assert_eq!(headers["cross-origin-opener-policy"], "same-origin");
        assert!(!headers.contains_key("set-cookie"));
        assert!(!headers.contains_key("access-control-allow-origin"));
        let csp = headers["content-security-policy"].to_str().unwrap();
        for directive in [
            "default-src 'none'",
            "script-src 'self'",
            "style-src 'self'",
            "connect-src 'self'",
            "frame-ancestors 'none'",
            "base-uri 'none'",
            "form-action 'none'",
        ] {
            assert!(csp.contains(directive));
        }
        assert!(!csp.contains("unsafe-inline"));
        let body = response.text().await.unwrap();
        assert!(!body.is_empty());
        assert!(!body.contains(f.setup.owner_key.secret()));
        assert!(!body.contains(&f.setup.workspace_id.to_string()));
    }
    for path in ["/admin?key=PRIVATE_QUERY", "/admin/?key=PRIVATE_QUERY"] {
        let response = f.public_client().get(f.url(path)).send().await.unwrap();
        let value = json_reply(response, 400).await;
        assert!(!value.to_string().contains("PRIVATE_QUERY"));
    }
    json_reply(
        f.public_client()
            .get(f.url("/admin/missing.js"))
            .send()
            .await
            .unwrap(),
        404,
    )
    .await;
    json_reply(
        f.public_client()
            .post(f.url("/admin"))
            .json(&json!({}))
            .send()
            .await
            .unwrap(),
        405,
    )
    .await;
    let (receipt, key) = f.enroll().await;
    let private = f.private_client(receipt["certificate_pem"].as_str().unwrap(), &key);
    for path in ["/admin", "/admin/app.js"] {
        json_reply(private.get(f.mtls(path)).send().await.unwrap(), 404).await;
    }
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn panel_cookie_human_edits_history_conflicts_and_archive_privacy_use_real_store() {
    let f = Fixture::new().await;
    let (cookie, session) = login(&f, f.setup.owner_key.secret(), None).await;
    let csrf = session["csrf_token"].as_str().unwrap();
    let ws = f.setup.workspace_id.to_string();
    let shelf = post(
        &f,
        &cookie,
        csrf,
        "/api/v1/content/shelves/create",
        &json!({"workspace_id":ws,"name":"human","description":""}),
        200,
    )
    .await;
    let book = post(
        &f,
        &cookie,
        csrf,
        "/api/v1/content/books/create",
        &json!({"workspace_id":ws,"shelf_id":shelf["data"]["id"],"name":"notes","description":""}),
        200,
    )
    .await;
    let create = json!({"workspace_id":ws,"operation_id":Uuid::new_v4().to_string(),"agent":null,"change":{"action":"create","book_id":book["data"]["id"],"name":"entry","content":{"text":"first","description":"human entry","labels":["review"]}}});
    let first = post(
        &f,
        &cookie,
        csrf,
        "/api/v1/content/entries/mutate",
        &create,
        200,
    )
    .await;
    assert_eq!(
        post(
            &f,
            &cookie,
            csrf,
            "/api/v1/content/entries/mutate",
            &create,
            200
        )
        .await,
        first
    );
    let id = first["data"]["entry_id"].clone();
    let revise = json!({"workspace_id":ws,"operation_id":Uuid::new_v4().to_string(),"agent":null,"change":{"action":"revise","entry_id":id,"expected_revision":1,"content":{"text":"second","description":"revised","labels":[]}}});
    assert_eq!(
        post(
            &f,
            &cookie,
            csrf,
            "/api/v1/content/entries/mutate",
            &revise,
            200
        )
        .await["data"]["revision"],
        2
    );
    let mut stale = revise.clone();
    stale["operation_id"] = json!(Uuid::new_v4().to_string());
    code(
        &post(
            &f,
            &cookie,
            csrf,
            "/api/v1/content/entries/mutate",
            &stale,
            409,
        )
        .await,
        "content_conflict",
    );
    verify_history_and_archive(&f, &cookie, csrf, &ws, id).await;
    f.finish().await;
}

async fn verify_history_and_archive(f: &Fixture, cookie: &str, csrf: &str, ws: &str, id: Value) {
    let target = json!({"workspace_id":ws,"entry_id":id,"revision":1});
    let snapshot = post(f, cookie, csrf, "/api/v1/content/entries/get", &target, 200).await;
    assert_eq!(snapshot["data"]["content"]["text"], "first");
    assert!(snapshot["data"]["entry"]["owner_client_id"].is_null());
    assert_eq!(snapshot["data"]["revision"]["actor"]["kind"], "admin_key");
    assert_eq!(
        snapshot["data"]["revision"]["actor"]["id"],
        f.setup.owner_key.id.to_string()
    );
    let history = json!({"workspace_id":ws,"entry_id":id,"after":null,"limit":32});
    assert_eq!(
        post(
            f,
            cookie,
            csrf,
            "/api/v1/content/entries/history",
            &history,
            200
        )
        .await["data"]["items"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let archive = json!({"workspace_id":ws,"operation_id":Uuid::new_v4().to_string(),"agent":null,"change":{"action":"archive","entry_id":id,"expected_revision":2}});
    post(
        f,
        cookie,
        csrf,
        "/api/v1/content/entries/mutate",
        &archive,
        200,
    )
    .await;
    post(f, cookie, csrf, "/api/v1/content/entries/get", &target, 404).await;
    post(
        f,
        cookie,
        csrf,
        "/api/v1/content/entries/history",
        &history,
        404,
    )
    .await;
}
