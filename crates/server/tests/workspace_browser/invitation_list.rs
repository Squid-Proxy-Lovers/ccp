// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;
const LIST: &str = "/api/v1/admin/invitations/list";
const INDIVIDUAL: &str = "/api/v1/admin/invitations/individual";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn browser_invitation_list_pages_match_individual_status_and_never_return_secrets() {
    let f = Fixture::new().await;
    let (receipt, _) = f.enroll().await;
    let (cookie, session) = login(&f, f.setup.owner_key.secret(), None).await;
    let proof = session["csrf_token"].as_str().unwrap();
    let first = post(&f, &cookie, proof, LIST, &target(&f), 200).await;
    assert_eq!(first["data"]["invitations"].as_array().unwrap().len(), 1);
    assert_eq!(first["data"]["invitations"][0]["kind"], "initial_group");
    assert_eq!(first["data"]["invitations"][0]["status"]["uses"], 1);
    assert_eq!(
        first["data"]["invitations"][0]["status"]["initial_closed"],
        true
    );
    let mut expected = vec![
        first["data"]["invitations"][0]["status"]["invitation_id"]
            .as_str()
            .unwrap()
            .to_string(),
    ];
    let mut secrets = Vec::new();
    for role in ["read", "read_write", "read"] {
        let value=post(&f,&cookie,proof,INDIVIDUAL,&json!({"workspace_id":f.setup.workspace_id.to_string(),"role":role,"expires_at":now()+3600}),200).await;
        expected.push(value["data"]["invitation_id"].as_str().unwrap().to_string());
        secrets.push(value["data"]["secret"].as_str().unwrap().to_string());
    }
    expected.sort();
    let mut after: Option<String> = None;
    let mut found = Vec::new();
    loop {
        let value = post(
            &f,
            &cookie,
            proof,
            LIST,
            &json!({"workspace_id":f.setup.workspace_id.to_string(),"limit":2,"after":after}),
            200,
        )
        .await;
        assert_eq!(value["data"].as_object().unwrap().len(), 2);
        let rows = value["data"]["invitations"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        for row in rows {
            assert_eq!(row.as_object().unwrap().len(), 3);
            assert_eq!(row["status"].as_object().unwrap().len(), 9);
            let status=post(&f,&cookie,proof,"/api/v1/admin/invitations/status",&json!({"workspace_id":f.setup.workspace_id.to_string(),"invitation_id":row["status"]["invitation_id"]}),200).await;
            assert_eq!(row["status"], status["data"]);
            found.push(row["status"]["invitation_id"].as_str().unwrap().to_string());
        }
        let text = value.to_string();
        assert!(secrets.iter().all(|secret| !text.contains(secret)));
        assert!(!text.contains(f.setup.owner_key.secret()));
        assert!(!text.contains(receipt["certificate_pem"].as_str().unwrap()));
        after = value["data"]["next_cursor"].as_str().map(str::to_string);
        if after.is_none() {
            break;
        }
        assert_eq!(after.as_ref(), found.last());
    }
    assert_eq!(found, expected);
    let end = post(
        &f,
        &cookie,
        proof,
        LIST,
        &json!({"workspace_id":f.setup.workspace_id.to_string(),"after":found.last(),"limit":500}),
        200,
    )
    .await;
    assert!(end["data"]["invitations"].as_array().unwrap().is_empty());
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn browser_invitation_list_strict_bounds_scope_cookie_and_client_surface() {
    let f = Fixture::new().await;
    let other = f.second();
    let (cookie, session) = login(&f, f.setup.owner_key.secret(), None).await;
    let proof = session["csrf_token"].as_str().unwrap();
    for extra in [
        json!({"limit":0}),
        json!({"limit":501}),
        json!({"limit":-1}),
        json!({"limit":"1"}),
        json!({"after":"not UUID"}),
        json!({"actor":"owner"}),
    ] {
        let mut body = target(&f);
        body.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        post(&f, &cookie, proof, LIST, &body, 400).await;
    }
    post(
        &f,
        &cookie,
        proof,
        LIST,
        &json!({"workspace_id":other.workspace_id.to_string()}),
        403,
    )
    .await;
    post(&f, &cookie, "wrong proof", LIST, &target(&f), 403).await;
    json_reply(
        f.public_client()
            .post(f.url(LIST))
            .json(&target(&f))
            .send()
            .await
            .unwrap(),
        401,
    )
    .await;
    json_reply(
        f.public_client()
            .get(f.url(LIST))
            .bearer_auth(f.setup.owner_key.secret())
            .send()
            .await
            .unwrap(),
        405,
    )
    .await;
    json_reply(
        f.public_client()
            .post(f.url(LIST))
            .bearer_auth(f.setup.instance_key.secret())
            .json(&target(&f))
            .send()
            .await
            .unwrap(),
        403,
    )
    .await;
    let duplicate = format!(
        "{{\"workspace_id\":\"{}\",\"workspace_id\":\"{}\"}}",
        f.setup.workspace_id, f.setup.workspace_id
    );
    json_reply(
        f.public_client()
            .post(f.url(LIST))
            .bearer_auth(f.setup.owner_key.secret())
            .header("content-type", "application/json")
            .body(duplicate)
            .send()
            .await
            .unwrap(),
        400,
    )
    .await;
    let (receipt, key) = f.enroll().await;
    let private = f.private_client(receipt["certificate_pem"].as_str().unwrap(), &key);
    json_reply(
        private
            .post(f.mtls(LIST))
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
async fn browser_invitation_list_scoped_admin_archive_revocation_and_terminal_expiry() {
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
                principal: InitialOwner::NewPrincipal("list admin".into()),
                key_expires_at: None,
            },
            now(),
        )
        .unwrap();
    let (cookie, session) = login(&f, admin.secret(), None).await;
    let proof = session["csrf_token"].as_str().unwrap();
    let issued=post(&f,&cookie,proof,INDIVIDUAL,&json!({"workspace_id":f.setup.workspace_id.to_string(),"role":"read","expires_at":now()+3600}),200).await;
    let invitation = issued["data"]["invitation_id"].as_str().unwrap();
    db.execute(
        "UPDATE ccp_invitations SET expires_at=?1 WHERE id=?2",
        rusqlite::params![now() - 1, invitation],
    )
    .unwrap();
    let expired = post(&f, &cookie, proof, LIST, &target(&f), 200).await;
    assert_eq!(expired["data"]["invitations"][0]["status"]["expired"], true);
    db.execute(
        "UPDATE ccp_invitations SET expires_at=?1 WHERE id=?2",
        rusqlite::params![now() + 3600, invitation],
    )
    .unwrap();
    assert_eq!(
        post(&f, &cookie, proof, LIST, &target(&f), 200).await["data"]["invitations"][0]["status"]
            ["expired"],
        true
    );
    let (owner, owner_session) = login(&f, f.setup.owner_key.secret(), None).await;
    let owner_proof = owner_session["csrf_token"].as_str().unwrap();
    post(
        &f,
        &owner,
        owner_proof,
        "/api/v1/admin/workspaces/archive",
        &target(&f),
        200,
    )
    .await;
    let archived = post(&f, &cookie, proof, LIST, &target(&f), 200).await;
    assert_eq!(
        archived["data"]["invitations"][0]["status"]["issuer_authorized"],
        false
    );
    post(
        &f,
        &owner,
        owner_proof,
        "/api/v1/admin/keys/revoke",
        &json!({"workspace_id":f.setup.workspace_id.to_string(),"key_id":admin.id.to_string()}),
        200,
    )
    .await;
    post(&f, &cookie, proof, LIST, &target(&f), 401).await;
    let final_page = post(&f, &owner, owner_proof, LIST, &target(&f), 200).await;
    assert_eq!(
        final_page["data"]["invitations"][0]["status"]["issuer_authorized"],
        false
    );
    f.finish().await;
}
