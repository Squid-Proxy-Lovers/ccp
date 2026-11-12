// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;
const GET: &str = "/api/v1/admin/enrollment/status";
const OPEN: &str = "/api/v1/admin/invitations/initial";
const CLOSE: &str = "/api/v1/admin/enrollment/close";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn browser_initial_window_real_open_redemption_capacity_and_terminal_close() {
    let f = Fixture::new().await;
    let (cookie, session) = login(&f, f.setup.owner_key.secret(), None).await;
    let proof = session["csrf_token"].as_str().unwrap();
    let unused = post(&f, &cookie, proof, GET, &target(&f), 200).await;
    assert_eq!(unused["data"]["phase"], "unused");
    assert_eq!(unused["data"]["can_open"], true);
    let token=post(&f,&cookie,proof,OPEN,&json!({"workspace_id":f.setup.workspace_id.to_string(),"role":"read_write","max_clients":1,"expires_at":null}),200).await;
    let state = post(&f, &cookie, proof, GET, &target(&f), 200).await;
    assert_eq!(state["data"]["phase"], "open");
    assert_eq!(state["data"]["can_open"], false);
    assert_eq!(
        state["data"]["invitation"]["invitation_id"],
        token["data"]["invitation_id"]
    );
    assert!(
        !state
            .to_string()
            .contains(token["data"]["secret"].as_str().unwrap())
    );
    let key = KeyPair::generate().unwrap();
    let receipt=json_reply(f.public_client().post(f.url("/api/v1/enroll")).json(&json!({"invitation":token["data"]["secret"],"request_id":Uuid::new_v4().to_string(),"csr_pem":csr(&key)})).send().await.unwrap(),200).await;
    assert!(!receipt["data"]["client_id"].as_str().unwrap().is_empty());
    let closed = post(&f, &cookie, proof, GET, &target(&f), 200).await;
    assert_eq!(closed["data"]["phase"], "closed");
    assert_eq!(closed["data"]["has_admission_history"], true);
    assert_eq!(closed["data"]["invitation"]["uses"], 1);
    assert_eq!(closed["data"]["can_open"], false);
    post(&f,&cookie,proof,OPEN,&json!({"workspace_id":f.setup.workspace_id.to_string(),"role":"read","max_clients":2,"expires_at":null}),403).await;
    post(&f, &cookie, proof, CLOSE, &target(&f), 200).await;
    f.finish().await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn browser_initial_window_status_security_scope_archive_and_closed_without_invitation() {
    let f = Fixture::new().await;
    let second = f.second();
    let (cookie, session) = login(&f, f.setup.owner_key.secret(), None).await;
    let proof = session["csrf_token"].as_str().unwrap();
    post(
        &f,
        &cookie,
        proof,
        GET,
        &json!({"workspace_id":second.workspace_id.to_string()}),
        403,
    )
    .await;
    let (global, g) = login(&f, f.setup.instance_key.secret(), None).await;
    post(
        &f,
        &global,
        g["csrf_token"].as_str().unwrap(),
        GET,
        &target(&f),
        403,
    )
    .await;
    post(
        &f,
        &cookie,
        proof,
        GET,
        &json!({"workspace_id":f.setup.workspace_id.to_string(),"after":null}),
        400,
    )
    .await;
    post(&f, &cookie, "wrong", GET, &target(&f), 403).await;
    post(
        &f,
        &cookie,
        proof,
        "/api/v1/admin/workspaces/archive",
        &target(&f),
        200,
    )
    .await;
    let archived = post(&f, &cookie, proof, GET, &target(&f), 200).await;
    assert_eq!(archived["data"]["phase"], "unused");
    assert_eq!(archived["data"]["workspace_archived"], true);
    assert_eq!(archived["data"]["can_open"], false);
    post(&f, &cookie, proof, CLOSE, &target(&f), 200).await;
    let closed = post(&f, &cookie, proof, GET, &target(&f), 200).await;
    assert_eq!(closed["data"]["phase"], "closed");
    assert!(closed["data"]["invitation"].is_null());
    assert!(closed["data"]["closed_at"].is_i64());
    let token = f
        .initial(second.owner_key.secret(), second.workspace_id)
        .await;
    let key = KeyPair::generate().unwrap();
    let receipt=json_reply(f.public_client().post(f.url("/api/v1/enroll")).json(&json!({"invitation":token["secret"],"request_id":Uuid::new_v4().to_string(),"csr_pem":csr(&key)})).send().await.unwrap(),200).await["data"].clone();
    let private = f.private_client(receipt["certificate_pem"].as_str().unwrap(), &key);
    json_reply(
        private
            .post(f.mtls(GET))
            .json(&target(&f))
            .send()
            .await
            .unwrap(),
        404,
    )
    .await;
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn browser_initial_window_scoped_admin_current_key_and_terminal_deadline() {
    let f = Fixture::new().await;
    let mut db = f.db();
    let mut store = AccessStore::new(&mut db).unwrap();
    let actor = store
        .authenticate_admin_key(f.setup.owner_key.secret(), now())
        .unwrap();
    let admin = store
        .appoint_workspace_admin_with_key(
            actor,
            f.setup.workspace_id,
            &server::foundation::AdminAppointment {
                principal: InitialOwner::NewPrincipal("window admin".into()),
                key_expires_at: None,
            },
            now(),
        )
        .unwrap();
    let (cookie, session) = login(&f, admin.secret(), None).await;
    let proof = session["csrf_token"].as_str().unwrap();
    assert_eq!(
        post(&f, &cookie, proof, GET, &target(&f), 200).await["data"]["can_open"],
        true
    );
    let token=post(&f,&cookie,proof,OPEN,&json!({"workspace_id":f.setup.workspace_id.to_string(),"role":"read","max_clients":null,"expires_at":now()+3600}),200).await;
    let id = token["data"]["invitation_id"].as_str().unwrap();
    db.execute(
        "UPDATE ccp_invitations SET expires_at=?1 WHERE id=?2",
        rusqlite::params![now() - 1, id],
    )
    .unwrap();
    let closed = post(&f, &cookie, proof, GET, &target(&f), 200).await;
    assert_eq!(closed["data"]["phase"], "closed");
    assert_eq!(closed["data"]["invitation"]["expired"], true);
    db.execute(
        "UPDATE ccp_invitations SET expires_at=?1 WHERE id=?2",
        rusqlite::params![now() + 3600, id],
    )
    .unwrap();
    assert_eq!(
        post(&f, &cookie, proof, GET, &target(&f), 200).await["data"]["phase"],
        "closed"
    );
    let (owner, owner_session) = login(&f, f.setup.owner_key.secret(), None).await;
    post(
        &f,
        &owner,
        owner_session["csrf_token"].as_str().unwrap(),
        "/api/v1/admin/keys/revoke",
        &json!({"workspace_id":f.setup.workspace_id.to_string(),"key_id":admin.id.to_string()}),
        200,
    )
    .await;
    post(&f, &cookie, proof, GET, &target(&f), 401).await;
    f.finish().await;
}
