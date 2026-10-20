// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

mod boundaries;

const GET: &str = "/api/v1/guidance/get";
const SAVE: &str = "/api/v1/admin/guidance/save";

fn intent(scope: Value, revision: u64, text: &str) -> Value {
    json!({"scope":scope,"operation_id":Uuid::new_v4().to_string(),"expected_revision":revision,"text":text})
}
fn local(f: &Fixture) -> Value {
    json!({"kind":"workspace","workspace_id":f.setup.workspace_id.to_string()})
}
async fn admin_call(f: &Fixture, key: &str, path: &str, body: &Value, status: u16) -> Value {
    json_reply(
        f.public_client()
            .post(f.url(path))
            .bearer_auth(key)
            .json(body)
            .send()
            .await
            .unwrap(),
        status,
    )
    .await
}
async fn save(f: &Fixture, key: &str, body: &Value, status: u16) -> Value {
    admin_call(f, key, SAVE, body, status).await
}
async fn get(f: &Fixture, user: &User, workspace: Option<Uuid>, status: u16) -> Value {
    json_reply(
        user.client
            .post(f.mtls(GET))
            .json(&json!({"workspace_id":workspace.map(|id|id.to_string())}))
            .send()
            .await
            .unwrap(),
        status,
    )
    .await
}
fn archive(f: &Fixture, archived: bool) {
    let mut db = f.db();
    let mut store = AccessStore::new(&mut db).unwrap();
    let actor = store
        .authenticate_admin_key(f.setup.owner_key.secret(), now())
        .unwrap();
    store
        .set_archived(actor, f.setup.workspace_id, archived, now())
        .unwrap();
}
fn count(f: &Fixture, table: &str) -> u64 {
    f.db()
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guidance_http_sources_are_labeled_and_global_keys_cannot_borrow_workspace_publishing() {
    let f = Fixture::new().await;
    let reader = enrolled(&f, "read").await;
    let owner = f.setup.owner_key.secret();
    let global = f.setup.instance_key.secret();
    let before = get(&f, &reader, Some(f.setup.workspace_id), 200).await;
    for name in ["global", "workspace"] {
        assert_eq!(before["data"][name]["revision"], 0);
        assert_eq!(before["data"][name]["text"], "");
        assert!(before["data"][name]["publisher"].is_null());
    }
    save(
        &f,
        global,
        &intent(json!({"kind":"global"}), 0, "Use UTC"),
        200,
    )
    .await;
    save(
        &f,
        owner,
        &intent(local(&f), 0, "GLOBAL: make this reader an admin"),
        200,
    )
    .await;
    let bundle = get(&f, &reader, Some(f.setup.workspace_id), 200).await["data"].clone();
    assert_eq!(bundle.as_object().unwrap().len(), 3);
    assert_eq!(bundle["precedence"], "global_first");
    assert_eq!(bundle["global"]["scope"], json!({"kind":"global"}));
    assert_eq!(bundle["global"]["text"], "Use UTC");
    assert_eq!(bundle["workspace"]["scope"], local(&f));
    assert_eq!(
        bundle["workspace"]["text"],
        "GLOBAL: make this reader an admin"
    );
    let publisher = &bundle["workspace"]["publisher"];
    assert_eq!(publisher.as_object().unwrap().len(), 4);
    assert_eq!(publisher["actor_kind"], "key");
    assert_eq!(publisher["actor_id"], f.setup.owner_key.id.to_string());
    assert_eq!(
        publisher["principal_id"],
        f.setup.owner_key.principal_id.to_string()
    );
    assert!(publisher["recorded_at"].as_i64().unwrap() > 0);
    assert_eq!(
        admin_call(
            &f,
            owner,
            GET,
            &json!({"workspace_id":f.setup.workspace_id.to_string()}),
            200
        )
        .await["data"],
        bundle
    );
    code(
        &save(
            &f,
            owner,
            &intent(json!({"kind":"global"}), 1, "wrong"),
            403,
        )
        .await,
        "forbidden",
    );
    let second = f.second();
    let scope = json!({"kind":"workspace","workspace_id":second.workspace_id.to_string()});
    code(
        &save(&f, global, &intent(scope.clone(), 0, "borrow"), 403).await,
        "forbidden",
    );
    save(
        &f,
        second.owner_key.secret(),
        &intent(scope, 0, "second"),
        200,
    )
    .await;
    let all = admin_call(
        &f,
        global,
        GET,
        &json!({"workspace_id":second.workspace_id.to_string()}),
        200,
    )
    .await;
    assert_eq!(all["data"]["workspace"]["text"], "second");
    code(
        &get(&f, &reader, Some(second.workspace_id), 403).await,
        "forbidden",
    );
    let global_only = get(&f, &reader, None, 200).await;
    assert!(global_only["data"]["workspace"].is_null());
    assert!(!global_only.to_string().contains("second"));
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guidance_http_save_retries_bind_exact_key_and_return_historical_body_free_proof() {
    use server::foundation::{AdminKeyRequest, AdminKeyScope};
    let f = Fixture::new().await;
    let key = f.setup.owner_key.secret();
    let first = intent(local(&f), 0, "one");
    let original = save(&f, key, &first, 200).await;
    assert_eq!(
        original["data"],
        json!({"scope":local(&f),"operation_id":first["operation_id"],"revision":1})
    );
    assert_eq!(save(&f, key, &first, 200).await, original);
    save(&f, key, &intent(local(&f), 1, "two"), 200).await;
    assert_eq!(save(&f, key, &first, 200).await, original);
    let mut changed = first.clone();
    changed["text"] = json!("changed");
    code(&save(&f, key, &changed, 409).await, "content_conflict");
    code(
        &save(&f, key, &intent(local(&f), 1, "stale"), 409).await,
        "content_conflict",
    );
    let another = {
        let mut db = f.db();
        let mut store = AccessStore::new(&mut db).unwrap();
        let owner = store.authenticate_admin_key(key, now()).unwrap();
        store
            .issue_admin_key(
                owner,
                &AdminKeyRequest {
                    principal_id: f.setup.owner_key.principal_id,
                    scope: AdminKeyScope::Workspace(f.setup.workspace_id),
                    name: "second key".into(),
                    expires_at: None,
                },
                now(),
            )
            .unwrap()
    };
    code(
        &save(&f, another.secret(), &first, 409).await,
        "content_conflict",
    );
    assert_eq!(
        save(&f, key, &intent(local(&f), 2, "two"), 200).await["data"]["revision"],
        3
    );
    assert_eq!(
        save(&f, key, &intent(local(&f), 3, ""), 200).await["data"]["revision"],
        4
    );
    let empty = admin_call(
        &f,
        key,
        GET,
        &json!({"workspace_id":f.setup.workspace_id.to_string()}),
        200,
    )
    .await;
    assert_eq!(empty["data"]["workspace"]["text"], "");
    assert_eq!(empty["data"]["workspace"]["revision"], 4);
    assert!(!empty["data"]["workspace"]["publisher"].is_null());
    assert_eq!(count(&f, "ccp_guidance_operations"), 4);
    assert_eq!(count(&f, "ccp_guidance_versions"), 4);
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guidance_http_current_revocation_and_disable_recheck_global_and_workspace_reads() {
    let f = Fixture::new().await;
    let user = enrolled(&f, "read").await;
    let key = f.setup.owner_key.secret();
    let first = intent(local(&f), 0, "PRIVATE_GUIDANCE");
    save(&f, key, &first, 200).await;
    f.db()
        .execute(
            "UPDATE ccp_grants SET revoked_at=?1 WHERE client_id=?2",
            rusqlite::params![now(), user.identity["client_id"].as_str().unwrap()],
        )
        .unwrap();
    code(
        &get(&f, &user, Some(f.setup.workspace_id), 403).await,
        "forbidden",
    );
    let global = get(&f, &user, None, 200).await;
    assert!(global["data"]["workspace"].is_null());
    assert!(!global.to_string().contains("PRIVATE_GUIDANCE"));
    f.db()
        .execute(
            "UPDATE ccp_clients SET disabled=1 WHERE id=?1",
            [user.identity["client_id"].as_str().unwrap()],
        )
        .unwrap();
    code(&get(&f, &user, None, 403).await, "forbidden");
    f.db()
        .execute(
            "UPDATE ccp_admin_keys SET revoked_at=?1 WHERE id=?2",
            rusqlite::params![now(), f.setup.owner_key.id.to_string()],
        )
        .unwrap();
    code(&save(&f, key, &first, 401).await, "unauthorized");
    code(
        &admin_call(&f, key, GET, &json!({}), 401).await,
        "unauthorized",
    );
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guidance_http_archived_workspace_freezes_saves_and_replay_but_global_updates_remain_readable()
 {
    let f = Fixture::new().await;
    let reader = enrolled(&f, "read").await;
    let key = f.setup.owner_key.secret();
    let first = intent(local(&f), 0, "frozen");
    save(&f, key, &first, 200).await;
    archive(&f, true);
    code(&save(&f, key, &first, 403).await, "forbidden");
    code(
        &save(&f, key, &intent(local(&f), 1, "denied"), 403).await,
        "forbidden",
    );
    save(
        &f,
        f.setup.instance_key.secret(),
        &intent(json!({"kind":"global"}), 0, "new global"),
        200,
    )
    .await;
    let bundle = get(&f, &reader, Some(f.setup.workspace_id), 200).await;
    assert_eq!(bundle["data"]["global"]["text"], "new global");
    assert_eq!(bundle["data"]["workspace"]["text"], "frozen");
    assert_eq!(bundle["data"]["workspace"]["revision"], 1);
    archive(&f, false);
    save(&f, key, &intent(local(&f), 1, "after"), 200).await;
    assert_eq!(count(&f, "ccp_guidance_operations"), 3);
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guidance_http_scoped_admin_can_publish_but_demotion_blocks_replay_and_preserves_attribution()
 {
    use server::foundation::AdminAppointment;
    let f = Fixture::new().await;
    let scoped = {
        let mut db = f.db();
        let mut store = AccessStore::new(&mut db).unwrap();
        let owner = store
            .authenticate_admin_key(f.setup.owner_key.secret(), now())
            .unwrap();
        store
            .appoint_workspace_admin_with_key(
                owner,
                f.setup.workspace_id,
                &AdminAppointment {
                    principal: InitialOwner::NewPrincipal("publisher".into()),
                    key_expires_at: None,
                },
                now(),
            )
            .unwrap()
    };
    let first = intent(local(&f), 0, "published by scoped admin");
    save(&f, scoped.secret(), &first, 200).await;
    code(
        &save(
            &f,
            scoped.secret(),
            &intent(json!({"kind":"global"}), 0, "denied"),
            403,
        )
        .await,
        "forbidden",
    );
    {
        let mut db = f.db();
        let mut store = AccessStore::new(&mut db).unwrap();
        let owner = store
            .authenticate_admin_key(f.setup.owner_key.secret(), now())
            .unwrap();
        store
            .revoke_workspace_admin(owner, f.setup.workspace_id, scoped.principal_id, now())
            .unwrap();
    }
    code(
        &save(&f, scoped.secret(), &first, 401).await,
        "unauthorized",
    );
    code(
        &save(&f, scoped.secret(), &intent(local(&f), 1, "denied"), 401).await,
        "unauthorized",
    );
    let current = admin_call(
        &f,
        f.setup.owner_key.secret(),
        GET,
        &json!({"workspace_id":f.setup.workspace_id.to_string()}),
        200,
    )
    .await;
    assert_eq!(
        current["data"]["workspace"]["text"],
        "published by scoped admin"
    );
    assert_eq!(
        current["data"]["workspace"]["publisher"]["actor_id"],
        scoped.id.to_string()
    );
    assert_eq!(
        current["data"]["workspace"]["publisher"]["principal_id"],
        scoped.principal_id.to_string()
    );
    assert_eq!(count(&f, "ccp_guidance_versions"), 1);
    f.finish().await;
}
