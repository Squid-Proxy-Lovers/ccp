// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;
const APPOINT: &str = "/api/v1/admin/team/appoint";
const LIST: &str = "/api/v1/admin/team/list";
const KEYS: &str = "/api/v1/admin/keys/list";
const ISSUE: &str = "/api/v1/admin/keys/issue";
const REVOKE: &str = "/api/v1/admin/keys/revoke";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn browser_team_owner_only_appointment_keys_and_revocation_recheck_live_sessions() {
    let f = Fixture::new().await;
    let (owner, session) = login(&f, f.setup.owner_key.secret(), None).await;
    let proof = session["csrf_token"].as_str().unwrap();
    let ws = f.setup.workspace_id.to_string();
    let appointed = post(&f, &owner, proof, APPOINT, &json!({"workspace_id":ws,"principal":{"kind":"new_principal","name":"browser admin"},"key_expires_at":null}), 200).await;
    let key = &appointed["data"];
    let (admin, admin_session) = login(&f, key["secret"].as_str().unwrap(), None).await;
    assert_eq!(admin_session["identity"]["scope"]["role"], "admin");
    post(
        &f,
        &admin,
        admin_session["csrf_token"].as_str().unwrap(),
        LIST,
        &target(&f),
        403,
    )
    .await;
    let (global, global_session) = login(&f, f.setup.instance_key.secret(), None).await;
    post(
        &f,
        &global,
        global_session["csrf_token"].as_str().unwrap(),
        KEYS,
        &target(&f),
        403,
    )
    .await;
    let roster = post(&f, &owner, proof, LIST, &target(&f), 200).await;
    assert_eq!(roster["data"]["admins"].as_array().unwrap().len(), 2);
    assert!(!roster.to_string().contains(key["secret"].as_str().unwrap()));
    let issued = post(&f, &owner, proof, ISSUE, &json!({"workspace_id":ws,"principal_id":key["principal_id"],"name":"rotation","expires_at":now()+3600}), 200).await;
    let issued = &issued["data"];
    assert_ne!(issued["key_id"], key["key_id"]);
    post(
        &f,
        &owner,
        proof,
        REVOKE,
        &json!({"workspace_id":ws,"key_id":key["key_id"]}),
        200,
    )
    .await;
    status(&f, &admin, 401).await;
    let (replacement, replacement_session) =
        login(&f, issued["secret"].as_str().unwrap(), None).await;
    assert_eq!(
        replacement_session["identity"]["principal_id"],
        key["principal_id"]
    );
    post(
        &f,
        &owner,
        proof,
        "/api/v1/admin/team/revoke",
        &json!({"workspace_id":ws,"principal_id":key["principal_id"]}),
        200,
    )
    .await;
    status(&f, &replacement, 401).await;
    let keys = post(&f, &owner, proof, KEYS, &target(&f), 200).await;
    assert!(
        !keys
            .to_string()
            .contains(issued["secret"].as_str().unwrap())
    );
    assert!(
        keys["data"]["keys"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|k| k["principal_id"] == key["principal_id"])
            .all(|k| !k["revoked_at"].is_null() && k["role_active"] == false)
    );
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn browser_team_archived_key_rotation_and_current_owner_key_revocation() {
    let f = Fixture::new().await;
    let (owner, session) = login(&f, f.setup.owner_key.secret(), None).await;
    let proof = session["csrf_token"].as_str().unwrap();
    let ws = f.setup.workspace_id.to_string();
    post(
        &f,
        &owner,
        proof,
        "/api/v1/admin/workspaces/archive",
        &target(&f),
        200,
    )
    .await;
    let roster = post(&f, &owner, proof, LIST, &target(&f), 200).await;
    assert_eq!(roster["data"]["admins"][0]["is_owner"], true);
    post(
        &f,
        &owner,
        proof,
        APPOINT,
        &json!({"workspace_id":ws,"principal":{"kind":"new_principal","name":"denied archived"}}),
        403,
    )
    .await;
    let issued = post(&f, &owner, proof, ISSUE, &json!({"workspace_id":ws,"principal_id":session["identity"]["principal_id"],"name":"owner replacement","expires_at":null}), 200).await;
    post(
        &f,
        &owner,
        proof,
        REVOKE,
        &json!({"workspace_id":ws,"key_id":session["identity"]["key_id"]}),
        200,
    )
    .await;
    status(&f, &owner, 401).await;
    let (new_cookie, new_session) =
        login(&f, issued["data"]["secret"].as_str().unwrap(), None).await;
    assert_eq!(new_session["identity"]["scope"]["role"], "owner");
    let keys = post(
        &f,
        &new_cookie,
        new_session["csrf_token"].as_str().unwrap(),
        KEYS,
        &target(&f),
        200,
    )
    .await;
    assert_eq!(keys["data"]["keys"].as_array().unwrap().len(), 2);
    f.finish().await;
}
