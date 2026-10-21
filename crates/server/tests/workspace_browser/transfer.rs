// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;
const TRANSFER: &str = "/api/v1/admin/workspaces/transfer-owner";
async fn recipient(f: &Fixture, cookie: &str, proof: &str) -> Value {
    post(f,cookie,proof,"/api/v1/admin/team/appoint",&json!({"workspace_id":f.setup.workspace_id.to_string(),"principal":{"kind":"new_principal","name":"transfer recipient"},"key_expires_at":null}),200).await["data"].clone()
}
fn transfer_body(f: &Fixture, recipient: &Value) -> Value {
    json!({"workspace_id":f.setup.workspace_id.to_string(),"new_owner_id":recipient["principal_id"]})
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn browser_transfer_revokes_all_former_owner_cookies_and_updates_recipient_existing_session()
{
    let f = Fixture::new().await;
    let (owner, session) = login(&f, f.setup.owner_key.secret(), None).await;
    let proof = session["csrf_token"].as_str().unwrap();
    let recipient = recipient(&f, &owner, proof).await;
    let (next, next_session) = login(&f, recipient["secret"].as_str().unwrap(), None).await;
    assert_eq!(next_session["identity"]["scope"]["role"], "admin");
    let extra=post(&f,&owner,proof,"/api/v1/admin/keys/issue",&json!({"workspace_id":f.setup.workspace_id.to_string(),"principal_id":session["identity"]["principal_id"],"name":"second owner key","expires_at":null}),200).await;
    let (extra_cookie, _) = login(&f, extra["data"]["secret"].as_str().unwrap(), None).await;
    let (global, _) = login(&f, f.setup.instance_key.secret(), None).await;
    let result = post(
        &f,
        &owner,
        proof,
        TRANSFER,
        &transfer_body(&f, &recipient),
        200,
    )
    .await;
    assert_eq!(result["data"], json!({"completed":true}));
    status(&f, &owner, 401).await;
    status(&f, &extra_cookie, 401).await;
    let current = status(&f, &next, 200).await;
    assert_eq!(current["data"]["identity"]["scope"]["role"], "owner");
    assert_eq!(current["data"]["csrf_token"], next_session["csrf_token"]);
    let inspect = post(
        &f,
        &next,
        next_session["csrf_token"].as_str().unwrap(),
        "/api/v1/admin/workspaces/inspect",
        &target(&f),
        200,
    )
    .await;
    assert_eq!(inspect["data"]["owner_id"], recipient["principal_id"]);
    assert_eq!(
        status(&f, &global, 200).await["data"]["identity"]["scope"]["kind"],
        "instance"
    );
    post(
        &f,
        &owner,
        proof,
        TRANSFER,
        &transfer_body(&f, &recipient),
        401,
    )
    .await;
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn browser_transfer_rejects_admin_global_foreign_and_archived_requests_without_changing_owner()
 {
    let f = Fixture::new().await;
    let (owner, session) = login(&f, f.setup.owner_key.secret(), None).await;
    let proof = session["csrf_token"].as_str().unwrap();
    let recipient = recipient(&f, &owner, proof).await;
    let body = transfer_body(&f, &recipient);
    let (admin, admin_session) = login(&f, recipient["secret"].as_str().unwrap(), None).await;
    post(
        &f,
        &admin,
        admin_session["csrf_token"].as_str().unwrap(),
        TRANSFER,
        &body,
        403,
    )
    .await;
    let (global, global_session) = login(&f, f.setup.instance_key.secret(), None).await;
    post(
        &f,
        &global,
        global_session["csrf_token"].as_str().unwrap(),
        TRANSFER,
        &body,
        403,
    )
    .await;
    let second = f.second();
    let foreign = json!({"workspace_id":second.workspace_id.to_string(),"new_owner_id":recipient["principal_id"]});
    post(&f, &owner, proof, TRANSFER, &foreign, 403).await;
    post(
        &f,
        &owner,
        proof,
        "/api/v1/admin/workspaces/archive",
        &target(&f),
        200,
    )
    .await;
    post(&f, &owner, proof, TRANSFER, &body, 403).await;
    let info = post(
        &f,
        &owner,
        proof,
        "/api/v1/admin/workspaces/inspect",
        &target(&f),
        200,
    )
    .await;
    assert_eq!(
        info["data"]["owner_id"],
        session["identity"]["principal_id"]
    );
    assert!(info["data"]["workspace"]["archived"].as_bool().unwrap());
    status(&f, &owner, 200).await;
    f.finish().await;
}
