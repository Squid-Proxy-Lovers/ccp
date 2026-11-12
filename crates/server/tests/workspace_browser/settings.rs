// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
const GET: &str = "/api/v1/admin/content/retention/get";
const CHANGE: &str = "/api/v1/admin/content/retention/change";
const ARCHIVE: &str = "/api/v1/admin/workspaces/archive";
const REACTIVATE: &str = "/api/v1/admin/workspaces/reactivate";

fn change(f: &Fixture, revision: u64, seconds: Option<u64>) -> Value {
    json!({"workspace_id":f.setup.workspace_id.to_string(),"operation_id":Uuid::new_v4().to_string(),"expected_revision":revision,"retention_seconds":seconds,"confirm_timed_purge":seconds.is_some()})
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn browser_settings_cookie_policy_transition_receipts_and_global_read_scope() {
    let f = Fixture::new().await;
    let (cookie, session) = login(&f, f.setup.owner_key.secret(), None).await;
    let proof = session["csrf_token"].as_str().unwrap();
    let policy = post(&f, &cookie, proof, GET, &target(&f), 200).await;
    assert_eq!(policy["data"]["revision"], 0);
    assert!(policy["data"]["retention_seconds"].is_null());
    let enable = change(&f, 0, Some(3600));
    let first = post(&f, &cookie, proof, CHANGE, &enable, 200).await;
    assert_eq!(first["data"]["policy"]["revision"], 1);
    assert_eq!(first["data"]["policy"]["retention_seconds"], 3600);
    assert_eq!(post(&f, &cookie, proof, CHANGE, &enable, 200).await, first);
    post(&f, &cookie, proof, CHANGE, &change(&f, 1, Some(7200)), 409).await;
    let disable = change(&f, 1, None);
    assert_eq!(
        post(&f, &cookie, proof, CHANGE, &disable, 200).await["data"]["policy"]["revision"],
        2
    );
    assert_eq!(post(&f, &cookie, proof, CHANGE, &enable, 200).await, first);
    let current = post(&f, &cookie, proof, GET, &target(&f), 200).await;
    assert_eq!(current["data"]["revision"], 2);
    assert!(current["data"]["retention_seconds"].is_null());
    let (global, session) = login(&f, f.setup.instance_key.secret(), None).await;
    let global_proof = session["csrf_token"].as_str().unwrap();
    assert_eq!(
        post(&f, &global, global_proof, GET, &target(&f), 200).await,
        current
    );
    post(
        &f,
        &global,
        global_proof,
        CHANGE,
        &change(&f, 2, Some(3600)),
        403,
    )
    .await;
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn browser_settings_workspace_admin_configures_but_only_owner_or_instance_changes_lifecycle()
{
    let f = Fixture::new().await;
    let key = workspace_admin(&f);
    let (admin, session) = login(&f, &key, None).await;
    assert_eq!(session["identity"]["scope"]["role"], "admin");
    let proof = session["csrf_token"].as_str().unwrap();
    let enable = change(&f, 0, Some(3600));
    post(&f, &admin, proof, CHANGE, &enable, 200).await;
    post(&f, &admin, proof, ARCHIVE, &target(&f), 403).await;
    let (owner, owner_session) = login(&f, f.setup.owner_key.secret(), None).await;
    let owner_proof = owner_session["csrf_token"].as_str().unwrap();
    post(&f, &owner, owner_proof, ARCHIVE, &target(&f), 200).await;
    assert_eq!(
        post(&f, &admin, proof, GET, &target(&f), 200).await["data"]["revision"],
        1
    );
    post(&f, &admin, proof, CHANGE, &enable, 403).await;
    post(&f, &admin, proof, REACTIVATE, &target(&f), 403).await;
    let (global, global_session) = login(&f, f.setup.instance_key.secret(), None).await;
    post(
        &f,
        &global,
        global_session["csrf_token"].as_str().unwrap(),
        REACTIVATE,
        &target(&f),
        200,
    )
    .await;
    assert_eq!(
        post(&f, &admin, proof, CHANGE, &change(&f, 1, None), 200).await["data"]["policy"]["revision"],
        2
    );
    f.finish().await;
}

fn workspace_admin(f: &Fixture) -> String {
    let mut db = f.db();
    let mut store = AccessStore::new(&mut db).unwrap();
    let actor = store
        .authenticate_admin_key(f.setup.owner_key.secret(), now())
        .unwrap();
    store
        .appoint_workspace_admin_with_key(
            actor,
            f.setup.workspace_id,
            &server::foundation::AdminAppointment {
                principal: InitialOwner::NewPrincipal("settings admin".into()),
                key_expires_at: None,
            },
            now(),
        )
        .unwrap()
        .secret()
        .to_string()
}
