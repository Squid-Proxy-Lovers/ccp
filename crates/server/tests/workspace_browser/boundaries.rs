// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn browser_login_and_cookie_posts_require_origin_and_correct_csrf() {
    let f = Fixture::new().await;
    let key = f.setup.owner_key.secret();
    let public = f.public_client();
    for (site, proof) in [
        (None, Some("login")),
        (Some("https://foreign.invalid"), Some("login")),
        (Some(origin(&f).as_str()), None),
        (Some(origin(&f).as_str()), Some("wrong")),
    ] {
        let mut request = public.post(f.url(LOGIN)).json(&json!({"key":key}));
        if let Some(site) = site {
            request = request.header("origin", site);
        }
        if let Some(proof) = proof {
            request = request.header("x-ccp-csrf", proof);
        }
        let response = request.send().await.unwrap();
        assert!(!response.headers().contains_key("set-cookie"));
        code(&json_reply(response, 403).await, "forbidden");
    }
    let (cookie, session) = login(&f, key, None).await;
    let (_, other) = login(&f, key, None).await;
    let csrf = session["csrf_token"].as_str().unwrap();
    let here = origin(&f);
    for (site, proof) in [
        (None, Some(csrf)),
        (Some("https://foreign.invalid"), Some(csrf)),
        (Some(here.as_str()), None),
        (Some(here.as_str()), Some("bad")),
        (Some(here.as_str()), other["csrf_token"].as_str()),
    ] {
        let mut request = public
            .post(f.url(RETENTION))
            .header("cookie", &cookie)
            .json(&target(&f));
        if let Some(site) = site {
            request = request.header("origin", site);
        }
        if let Some(proof) = proof {
            request = request.header("x-ccp-csrf", proof);
        }
        code(
            &json_reply(request.send().await.unwrap(), 403).await,
            "forbidden",
        );
    }
    let elsewhere = format!("localhost:{}", f.startup.https_address.port());
    code(
        &json_reply(
            public
                .get(f.url(STATUS))
                .header("cookie", &cookie)
                .header("host", elsewhere)
                .send()
                .await
                .unwrap(),
            401,
        )
        .await,
        "unauthorized",
    );
    assert_eq!(status(&f, &cookie, 200).await["data"], session);
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn browser_credentials_are_unambiguous_and_old_bearer_clients_still_work() {
    let f = Fixture::new().await;
    let (cookie, session) = login(&f, f.setup.owner_key.secret(), None).await;
    let public = f.public_client();
    code(
        &json_reply(
            public
                .get(f.url("/api/v1/admin/workspaces"))
                .bearer_auth(f.setup.owner_key.secret())
                .header("cookie", &cookie)
                .send()
                .await
                .unwrap(),
            400,
        )
        .await,
        "invalid_request",
    );
    let duplicate = format!("{cookie}; {cookie}");
    code(&status(&f, &duplicate, 400).await, "invalid_request");
    let extra = format!("unrelated=other; {cookie}; another=value");
    assert_eq!(status(&f, &extra, 200).await["data"], session);
    let result = json_reply(
        public
            .post(f.url(RETENTION))
            .bearer_auth(f.setup.owner_key.secret())
            .header("cookie", "unrelated=1")
            .json(&target(&f))
            .send()
            .await
            .unwrap(),
        200,
    )
    .await;
    assert_eq!(result["data"]["revision"], 0);
    let (identity, key) = f.enroll().await;
    let client = f.private_client(identity["certificate_pem"].as_str().unwrap(), &key);
    for path in [LOGIN, STATUS, LOGOUT] {
        code(
            &json_reply(
                client
                    .get(f.mtls(path))
                    .header("cookie", &cookie)
                    .send()
                    .await
                    .unwrap(),
                404,
            )
            .await,
            "not_found",
        );
    }
    f.db()
        .execute(
            "UPDATE ccp_grants SET role='read' WHERE client_id=?1",
            [identity["client_id"].as_str().unwrap()],
        )
        .unwrap();
    mtls_ignores_browser_authority(
        &f,
        &client,
        &cookie,
        session["csrf_token"].as_str().unwrap(),
    )
    .await;
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn browser_input_is_strict_bounded_and_failed_login_keeps_existing_session() {
    let f = Fixture::new().await;
    let (cookie, session) = login(&f, f.setup.owner_key.secret(), None).await;
    let public = f.public_client();
    for (body, status) in [
        (
            json!({"key":f.setup.owner_key.secret(),"key_id":"PRIVATE_KEY"}),
            400,
        ),
        (json!({"key":123}), 400),
        (json!({"key":"PRIVATE_KEY"}), 401),
    ] {
        let response = public
            .post(f.url(LOGIN))
            .header("cookie", &cookie)
            .header("origin", origin(&f))
            .header("x-ccp-csrf", "login")
            .json(&body)
            .send()
            .await
            .unwrap();
        assert!(!response.headers().contains_key("set-cookie"));
        let value = json_reply(response, status).await;
        assert!(!value.to_string().contains("PRIVATE_KEY"));
    }
    let response = public
        .post(f.url(LOGIN))
        .header("origin", origin(&f))
        .header("x-ccp-csrf", "login")
        .header("content-type", "application/json")
        .body("x".repeat(64 * 1024 + 1))
        .send()
        .await
        .unwrap();
    code(&json_reply(response, 413).await, "request_too_large");
    code(
        &json_reply(
            public
                .post(f.url(LOGIN))
                .header("origin", origin(&f))
                .header("x-ccp-csrf", "login")
                .bearer_auth(f.setup.owner_key.secret())
                .json(&json!({"key":f.setup.owner_key.secret()}))
                .send()
                .await
                .unwrap(),
            400,
        )
        .await,
        "invalid_request",
    );
    assert_eq!(status(&f, &cookie, 200).await["data"], session);
    let (replaced, _) = login(&f, f.setup.owner_key.secret(), Some(&cookie)).await;
    code(&status(&f, &cookie, 401).await, "unauthorized");
    assert!(status(&f, &replaced, 200).await["data"]["identity"].is_object());
    f.finish().await;
}

async fn mtls_ignores_browser_authority(f: &Fixture, client: &Client, cookie: &str, csrf: &str) {
    // A malicious caller can forge HTTP Host/Origin; only the actual TLS
    // listener/peer selects the identity. The browser key must not be borrowed.
    let response=client.post(f.mtls("/api/v1/content/shelves/create"))
        .header("host",f.startup.https_address.to_string()).header("origin",origin(f))
        .header("cookie",cookie).header("x-ccp-csrf",csrf)
        .json(&json!({"workspace_id":f.setup.workspace_id.to_string(),"name":"must remain client-scoped","description":""}))
        .send().await.unwrap();
    code(&json_reply(response, 403).await, "forbidden");
}
