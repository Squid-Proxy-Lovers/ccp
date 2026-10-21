// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
const LIST: &str = "/api/v1/work/status/list";
const GET: &str = "/api/v1/work/status/get";
const MUTATE: &str = "/api/v1/work/status/mutate";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn browser_reported_work_reads_keep_csrf_key_scope_and_client_publication_separate() {
    let f = Fixture::new().await;
    let (client, key) = f.enroll().await;
    let peer = f.private_client(client["certificate_pem"].as_str().unwrap(), &key);
    let id = Uuid::new_v4();
    let mutation = json!({"workspace_id":f.setup.workspace_id.to_string(),"status_id":id.to_string(),"operation_id":Uuid::new_v4().to_string(),"expected_revision":0,"change":{"action":"set","agent":"untrusted label","message":"Human-visible status"}});
    json_reply(
        peer.post(f.mtls(MUTATE))
            .json(&mutation)
            .send()
            .await
            .unwrap(),
        200,
    )
    .await;
    let body = json!({"workspace_id":f.setup.workspace_id.to_string()});
    let (cookie, session) = login(&f, f.setup.owner_key.secret(), None).await;
    let proof = session["csrf_token"].as_str().unwrap();
    post(&f, &cookie, "wrong", LIST, &body, 403).await;
    let page = post(&f, &cookie, proof, LIST, &body, 200).await;
    assert_eq!(page["data"]["items"][0]["client_id"], client["client_id"]);
    assert_eq!(page["data"]["items"][0]["message"], "Human-visible status");
    post(&f, &cookie, proof, MUTATE, &mutation, 404).await;
    let other = f.second();
    post(
        &f,
        &cookie,
        proof,
        LIST,
        &json!({"workspace_id":other.workspace_id.to_string()}),
        403,
    )
    .await;
    let (global, global_session) = login(&f, f.setup.instance_key.secret(), None).await;
    let global_proof = global_session["csrf_token"].as_str().unwrap();
    assert_eq!(
        post(&f, &global, global_proof, LIST, &body, 200).await["data"]["items"],
        page["data"]["items"]
    );
    post(&f, &global, global_proof, MUTATE, &mutation, 404).await;
    let target =
        json!({"workspace_id":f.setup.workspace_id.to_string(),"status_id":id.to_string()});
    post(&f, &cookie, proof, GET, &target, 200).await;
    f.db()
        .execute(
            "UPDATE ccp_admin_keys SET revoked_at=?2 WHERE id=?1",
            rusqlite::params![session["identity"]["key_id"].as_str().unwrap(), now()],
        )
        .unwrap();
    post(&f, &cookie, proof, LIST, &body, 401).await;
    assert_eq!(
        f.db()
            .query_row("SELECT count(*) FROM ccp_work_status_operations", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
}
