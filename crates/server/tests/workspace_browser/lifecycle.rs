// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn browser_logout_requires_proof_and_invalidates_only_that_session() {
    let f = Fixture::new().await;
    let (one, first) = login(&f, f.setup.owner_key.secret(), None).await;
    let (two, _) = login(&f, f.setup.owner_key.secret(), None).await;
    code(
        &post(&f, &one, "wrong", LOGOUT, &json!({}), 403).await,
        "forbidden",
    );
    code(
        &post(
            &f,
            &one,
            first["csrf_token"].as_str().unwrap(),
            LOGOUT,
            &json!({"actor":"PRIVATE_KEY"}),
            400,
        )
        .await,
        "invalid_request",
    );
    assert!(status(&f, &one, 200).await["data"].is_object());
    let response = f
        .public_client()
        .post(f.url(LOGOUT))
        .header("cookie", &one)
        .header("origin", origin(&f))
        .header("x-ccp-csrf", first["csrf_token"].as_str().unwrap())
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    let cleared = response
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(cleared.contains("; Max-Age=0"));
    assert!(cleared.contains("HttpOnly"));
    assert_eq!(json_reply(response, 200).await["data"]["signed_out"], true);
    code(&status(&f, &one, 401).await, "unauthorized");
    assert!(status(&f, &two, 200).await["data"].is_object());
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn browser_sessions_recheck_key_revocation_expiry_and_admin_role_each_request() {
    let f = Fixture::new().await;
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
                principal: InitialOwner::NewPrincipal("browser admin".into()),
                key_expires_at: None,
            },
            now(),
        )
        .unwrap();
    drop(db);
    f.db()
        .execute(
            "UPDATE ccp_workspaces SET owner_admin_id=NULL WHERE id=?1",
            [f.setup.workspace_id.to_string()],
        )
        .unwrap();
    let (cookie, view) = login(&f, admin.secret(), None).await;
    assert_eq!(view["identity"]["scope"]["role"], "admin");
    f.db()
        .execute(
            "UPDATE ccp_workspaces SET owner_admin_id=?1 WHERE id=?2",
            rusqlite::params![
                f.setup.owner_key.principal_id.to_string(),
                f.setup.workspace_id.to_string()
            ],
        )
        .unwrap();
    f.db()
        .execute(
            "DELETE FROM ccp_workspace_admins WHERE workspace_id=?1 AND admin_id=?2",
            rusqlite::params![
                f.setup.workspace_id.to_string(),
                admin.principal_id.to_string()
            ],
        )
        .unwrap();
    code(&status(&f, &cookie, 401).await, "unauthorized");
    let (cookie, view) = login(&f, f.setup.owner_key.secret(), None).await;
    f.db()
        .execute(
            "UPDATE ccp_admin_keys SET expires_at=?1 WHERE id=?2",
            rusqlite::params![now(), f.setup.owner_key.id.to_string()],
        )
        .unwrap();
    code(&status(&f, &cookie, 401).await, "unauthorized");
    code(
        &post(
            &f,
            &cookie,
            view["csrf_token"].as_str().unwrap(),
            RETENTION,
            &target(&f),
            401,
        )
        .await,
        "unauthorized",
    );
    f.db()
        .execute(
            "UPDATE ccp_admin_keys SET expires_at=NULL,revoked_at=?1 WHERE id=?2",
            rusqlite::params![now(), f.setup.owner_key.id.to_string()],
        )
        .unwrap();
    code(&status(&f, &cookie, 401).await, "unauthorized");
    assert_eq!(
        post(
            &f,
            &cookie,
            view["csrf_token"].as_str().unwrap(),
            LOGOUT,
            &json!({}),
            200
        )
        .await["data"]["signed_out"],
        true
    );
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn browser_restart_drops_sessions_and_same_underlying_key_can_log_in_again() {
    let f = Fixture::new().await;
    let (cookie, _) = login(&f, f.setup.owner_key.secret(), None).await;
    let Fixture {
        directory,
        setup,
        trust,
        stop,
        task,
        ..
    } = f;
    stop.send(()).unwrap();
    task.await.unwrap().unwrap();
    let server = WorkspaceServer::bind(Config {
        data_dir: directory.clone(),
        https_listen: "127.0.0.1:0".parse().unwrap(),
        mtls_listen: "127.0.0.1:0".parse().unwrap(),
        activate_workspace: false,
    })
    .await
    .unwrap();
    let startup = server.startup().unwrap();
    let (stop, signal) = oneshot::channel();
    let task = tokio::spawn(server.run_until(async { signal.await.context("test stop dropped") }));
    let f = Fixture {
        directory,
        setup,
        trust,
        startup,
        stop,
        task,
    };
    code(&status(&f, &cookie, 401).await, "unauthorized");
    let (_, view) = login(&f, f.setup.owner_key.secret(), Some(&cookie)).await;
    assert_eq!(view["identity"]["key_id"], f.setup.owner_key.id.to_string());
    f.finish().await;
}
