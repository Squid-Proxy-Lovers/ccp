// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;
const LIST: &str = "/api/v1/admin/recovery/list";
fn request(f: &Fixture, client: &Value) -> Value {
    json!({"client_id":client["client_id"],"scope":{"kind":"workspace","workspace_id":f.setup.workspace_id.to_string()},"limit":1})
}
async fn issue(f: &Fixture, key: &str, body: &Value) -> Value {
    json_reply(
        f.public_client()
            .post(f.url("/api/v1/admin/recovery/issue"))
            .bearer_auth(key)
            .json(body)
            .send()
            .await
            .unwrap(),
        200,
    )
    .await["data"]
        .clone()
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn browser_recovery_list_exact_scope_paging_status_parity_and_no_secrets() {
    let f = Fixture::new().await;
    let (client, _) = f.enroll().await;
    let (cookie, session) = login(&f, f.setup.owner_key.secret(), None).await;
    let proof = session["csrf_token"].as_str().unwrap();
    let body = json!({"client_id":client["client_id"],"scope":{"kind":"workspace","workspace_id":f.setup.workspace_id.to_string()},"expires_at":now()+3600});
    let a = issue(&f, f.setup.owner_key.secret(), &body).await;
    let b = issue(&f, f.setup.owner_key.secret(), &body).await;
    let global=issue(&f,f.setup.instance_key.secret(),&json!({"client_id":client["client_id"],"scope":{"kind":"instance"},"expires_at":now()+3600})).await;
    let mut body = request(&f, &client);
    let mut ids = Vec::new();
    loop {
        let page = post(&f, &cookie, proof, LIST, &body, 200).await;
        let entries = page["data"]["invitations"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        let item = &entries[0];
        assert_eq!(item.as_object().unwrap().len(), 2);
        assert_eq!(item["status"].as_object().unwrap().len(), 7);
        assert_ne!(item["invitation_id"], global["invitation_id"]);
        let status = post(
            &f,
            &cookie,
            proof,
            "/api/v1/admin/recovery/status",
            &json!({"invitation_id":item["invitation_id"]}),
            200,
        )
        .await;
        assert_eq!(item["status"], status["data"]);
        for secret in [&a["secret"], &b["secret"], &global["secret"]] {
            assert!(!page.to_string().contains(secret.as_str().unwrap()));
        }
        assert!(
            !page
                .to_string()
                .contains(client["certificate_pem"].as_str().unwrap())
        );
        ids.push(item["invitation_id"].as_str().unwrap().to_string());
        if page["data"]["next_cursor"].is_null() {
            break;
        }
        assert_eq!(page["data"]["next_cursor"], item["invitation_id"]);
        body["after"] = page["data"]["next_cursor"].clone();
    }
    let mut expected = vec![
        a["invitation_id"].as_str().unwrap().to_string(),
        b["invitation_id"].as_str().unwrap().to_string(),
    ];
    expected.sort();
    assert_eq!(ids, expected);
    let (g, gs) = login(&f, f.setup.instance_key.secret(), None).await;
    let globalpage = post(
        &f,
        &g,
        gs["csrf_token"].as_str().unwrap(),
        LIST,
        &json!({"client_id":client["client_id"],"scope":{"kind":"instance"},"limit":500}),
        200,
    )
    .await;
    assert_eq!(
        globalpage["data"]["invitations"].as_array().unwrap().len(),
        1
    );
    assert_eq!(
        globalpage["data"]["invitations"][0]["invitation_id"],
        global["invitation_id"]
    );
    post(
        &f,
        &g,
        gs["csrf_token"].as_str().unwrap(),
        LIST,
        &request(&f, &client),
        403,
    )
    .await;
    post(
        &f,
        &cookie,
        proof,
        LIST,
        &json!({"client_id":client["client_id"],"scope":{"kind":"instance"}}),
        403,
    )
    .await;
    f.finish().await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn browser_recovery_list_strict_request_foreign_client_surface_and_current_key() {
    let f = Fixture::new().await;
    let (client, key) = f.enroll().await;
    let other = f.second();
    let (cookie, session) = login(&f, f.setup.owner_key.secret(), None).await;
    let proof = session["csrf_token"].as_str().unwrap();
    for extra in [
        json!({"limit":0}),
        json!({"limit":501}),
        json!({"limit":"1"}),
        json!({"after":"bad"}),
        json!({"actor":"owner"}),
        json!({"scope":{"kind":"instance","workspace_id":f.setup.workspace_id.to_string()}}),
    ] {
        let mut body = request(&f, &client);
        body.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        post(&f, &cookie, proof, LIST, &body, 400).await;
    }
    post(&f,&cookie,proof,LIST,&json!({"client_id":client["client_id"],"scope":{"kind":"workspace","workspace_id":other.workspace_id.to_string()}}),403).await;
    post(&f, &cookie, "wrong", LIST, &request(&f, &client), 403).await;
    let private = f.private_client(client["certificate_pem"].as_str().unwrap(), &key);
    json_reply(
        private
            .post(f.mtls(LIST))
            .json(&request(&f, &client))
            .send()
            .await
            .unwrap(),
        404,
    )
    .await;
    let empty=post(&f,&cookie,proof,LIST,&json!({"client_id":Uuid::new_v4().to_string(),"scope":{"kind":"workspace","workspace_id":f.setup.workspace_id.to_string()}}),200).await;
    assert!(empty["data"]["invitations"].as_array().unwrap().is_empty());
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
                principal: InitialOwner::NewPrincipal("recovery list admin".into()),
                key_expires_at: None,
            },
            now(),
        )
        .unwrap();
    let (admin_cookie, asession) = login(&f, admin.secret(), None).await;
    let ap = asession["csrf_token"].as_str().unwrap();
    post(&f, &admin_cookie, ap, LIST, &request(&f, &client), 200).await;
    store
        .revoke_admin_key(
            owner,
            server::foundation::AdminKeyScope::Workspace(f.setup.workspace_id),
            admin.id,
            now(),
        )
        .unwrap();
    post(&f, &admin_cookie, ap, LIST, &request(&f, &client), 401).await;
    f.finish().await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn browser_recovery_list_terminal_expiry_and_disabled_archived_security_history() {
    let f = Fixture::new().await;
    let (client, _) = f.enroll().await;
    let (cookie, session) = login(&f, f.setup.owner_key.secret(), None).await;
    let proof = session["csrf_token"].as_str().unwrap();
    let token=issue(&f,f.setup.owner_key.secret(),&json!({"client_id":client["client_id"],"scope":{"kind":"workspace","workspace_id":f.setup.workspace_id.to_string()},"expires_at":now()+3600})).await;
    let id = token["invitation_id"].as_str().unwrap();
    let db = f.db();
    db.execute(
        "UPDATE ccp_recovery_invitations SET expires_at=?1 WHERE id=?2",
        rusqlite::params![now() - 1, id],
    )
    .unwrap();
    assert_eq!(
        post(&f, &cookie, proof, LIST, &request(&f, &client), 200).await["data"]["invitations"][0]
            ["status"]["expired"],
        true
    );
    db.execute(
        "UPDATE ccp_recovery_invitations SET expires_at=?1 WHERE id=?2",
        rusqlite::params![now() + 3600, id],
    )
    .unwrap();
    post(
        &f,
        &cookie,
        proof,
        "/api/v1/admin/workspaces/archive",
        &target(&f),
        200,
    )
    .await;
    let history = post(&f, &cookie, proof, LIST, &request(&f, &client), 200).await;
    assert_eq!(history["data"]["invitations"][0]["status"]["expired"], true);
    assert_eq!(
        history["data"]["invitations"][0]["status"]["issuer_authorized"],
        true
    );
    json_reply(
        f.public_client()
            .post(f.url("/api/v1/admin/clients/disable"))
            .bearer_auth(f.setup.instance_key.secret())
            .json(&json!({"client_id":client["client_id"]}))
            .send()
            .await
            .unwrap(),
        200,
    )
    .await;
    post(
        &f,
        &cookie,
        proof,
        "/api/v1/admin/recovery/revoke",
        &json!({"invitation_id":id}),
        200,
    )
    .await;
    assert_eq!(
        post(&f, &cookie, proof, LIST, &request(&f, &client), 200).await["data"]["invitations"][0]
            ["status"]["revoked"],
        true
    );
    f.finish().await;
}
