// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

const GET: &str = "/api/v1/guidance/get";
const SAVE: &str = "/api/v1/admin/guidance/save";

fn save(scope: Value, revision: u64, text: &str) -> Value {
    json!({"scope":scope,"operation_id":Uuid::new_v4().to_string(),"expected_revision":revision,"text":text})
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn browser_guidance_cookie_live_save_exact_retry_and_conflict_keep_scoped_sources() {
    let f = Fixture::new().await;
    let (cookie, session) = login(&f, f.setup.owner_key.secret(), None).await;
    let csrf = session["csrf_token"].as_str().unwrap();
    let request = save(
        json!({"kind":"workspace","workspace_id":f.setup.workspace_id.to_string()}),
        0,
        "<script>literal guidance</script> é",
    );
    let receipt = post(&f, &cookie, csrf, SAVE, &request, 200).await;
    assert_eq!(receipt["data"]["revision"], 1);
    assert_eq!(post(&f, &cookie, csrf, SAVE, &request, 200).await, receipt);
    let mut stale = request.clone();
    stale["operation_id"] = json!(Uuid::new_v4().to_string());
    code(
        &post(&f, &cookie, csrf, SAVE, &stale, 409).await,
        "content_conflict",
    );
    let bundle = post(&f, &cookie, csrf, GET, &target(&f), 200).await;
    assert_eq!(bundle["data"]["precedence"], "global_first");
    assert_eq!(bundle["data"]["global"]["revision"], 0);
    assert_eq!(bundle["data"]["workspace"]["revision"], 1);
    assert_eq!(bundle["data"]["workspace"]["text"], request["text"]);
    assert_eq!(
        bundle["data"]["workspace"]["publisher"]["actor_id"],
        f.setup.owner_key.id.to_string()
    );
    let forbidden = save(json!({"kind":"global"}), 0, "no global power");
    post(&f, &cookie, csrf, SAVE, &forbidden, 403).await;
    let foreign = f.second();
    post(
        &f,
        &cookie,
        csrf,
        GET,
        &json!({"workspace_id":foreign.workspace_id.to_string()}),
        403,
    )
    .await;
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn browser_guidance_archive_freezes_workspace_saves_but_global_is_independent() {
    let f = Fixture::new().await;
    let (cookie, session) = login(&f, f.setup.owner_key.secret(), None).await;
    let csrf = session["csrf_token"].as_str().unwrap();
    let original = save(
        json!({"kind":"workspace","workspace_id":f.setup.workspace_id.to_string()}),
        0,
        "preserved",
    );
    post(&f, &cookie, csrf, SAVE, &original, 200).await;
    post(
        &f,
        &cookie,
        csrf,
        "/api/v1/admin/workspaces/archive",
        &target(&f),
        200,
    )
    .await;
    post(&f, &cookie, csrf, SAVE, &original, 403).await;
    let (global, session) = login(&f, f.setup.instance_key.secret(), None).await;
    let proof = session["csrf_token"].as_str().unwrap();
    let global_save = save(
        json!({"kind":"global"}),
        0,
        "global update while workspace archived",
    );
    assert_eq!(
        post(&f, &global, proof, SAVE, &global_save, 200).await["data"]["revision"],
        1
    );
    let denied = save(
        json!({"kind":"workspace","workspace_id":f.setup.workspace_id.to_string()}),
        1,
        "denied",
    );
    post(&f, &global, proof, SAVE, &denied, 403).await;
    let bundle = post(&f, &cookie, csrf, GET, &target(&f), 200).await;
    assert_eq!(bundle["data"]["global"]["text"], global_save["text"]);
    assert_eq!(bundle["data"]["workspace"]["text"], "preserved");
    post(
        &f,
        &cookie,
        csrf,
        "/api/v1/admin/workspaces/reactivate",
        &target(&f),
        200,
    )
    .await;
    let clear = save(original["scope"].clone(), 1, "");
    assert_eq!(
        post(&f, &cookie, csrf, SAVE, &clear, 200).await["data"]["revision"],
        2
    );
    let cleared = post(&f, &cookie, csrf, GET, &target(&f), 200).await;
    assert_eq!(cleared["data"]["workspace"]["text"], "");
    assert_eq!(cleared["data"]["workspace"]["revision"], 2);
    f.finish().await;
}
