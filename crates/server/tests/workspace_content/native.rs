// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;
use rusqlite::params;

const PATH: &str = "entries/native-history";
fn seed(f: &Fixture, entry: &Value, large: bool, sealed: bool) -> Value {
    let db = f.db();
    let ws = f.setup.workspace_id.to_string();
    db.execute("INSERT INTO ccp_native_imports(workspace_id,source_sha256,source_schema,legacy_session_id,recorded_by,recorded_at) VALUES(?1,?2,2,-99,?3,?4)",params![ws,"a".repeat(64),f.setup.owner_key.principal_id.to_string(),now()]).unwrap();
    let mut raw = json!({"columns":["id","context","created_at","negative_zero","integer_max","text_bytes","blob","null"],"values":[{"type":"integer","value":"42"},{"type":"text","value":"PRIVATE_SOURCE"},{"type":"text","value":"arbitrary old time"},{"type":"real","value":"8000000000000000"},{"type":"integer","value":"9223372036854775807"},{"type":"text_bytes","value":[0,255]},{"type":"blob","value":[0,255]},{"type":"null"}]});
    raw["values"][6]["value"] = json!((0_u8..=255).collect::<Vec<_>>());
    db.execute("INSERT INTO ccp_native_entry_rows(workspace_id,entry_id,ordinal,source_table,source_key,row_json) VALUES(?1,?2,1,'message_packs','42',?3)",params![ws,entry.as_str().unwrap(),raw.to_string()]).unwrap();
    if large {
        for n in 2..=5 {
            let row = json!({"columns":["id","appended_content"],"values":[{"type":"integer","value":n.to_string()},{"type":"text","value":"x".repeat(1_200_000)}]});
            db.execute("INSERT INTO ccp_native_entry_rows(workspace_id,entry_id,ordinal,source_table,source_key,row_json) VALUES(?1,?2,?3,'message_history',?4,?5)",params![ws,entry.as_str().unwrap(),n,n.to_string(),row.to_string()]).unwrap();
        }
    }
    db.execute("INSERT INTO ccp_native_workspace_rows(workspace_id,ordinal,source_table,source_key,row_json) VALUES(?1,1,'auth_tokens','77',?2)",params![ws,json!({"columns":["token_value"],"values":[{"type":"text","value":"PRIVATE_NATIVE_TOKEN"}]}).to_string()]).unwrap();
    if sealed {
        db.execute(
            "UPDATE ccp_native_imports SET sealed=1 WHERE workspace_id=?1",
            [ws],
        )
        .unwrap();
    }
    raw
}

async fn entry(f: &Fixture, user: &User) -> Value {
    let (_, book) = hierarchy(f, user).await;
    call(
        f,
        user,
        "entries/mutate",
        &create_entry(f, &book["id"], "entry", "CURRENT_BODY"),
        200,
    )
    .await["data"]["entry_id"]
        .clone()
}
fn page(f: &Fixture, entry: &Value) -> Value {
    json!({"workspace_id":f.setup.workspace_id.to_string(),"entry_id":entry,"after":null,"limit":32})
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_history_tls_is_exact_scoped_private_and_budget_paged() {
    let f = Fixture::new().await;
    let writer = enrolled(&f, "read_write").await;
    let reader = enrolled(&f, "read").await;
    let entry = entry(&f, &writer).await;
    let body = page(&f, &entry);
    let fresh = call(&f, &reader, PATH, &body, 200).await;
    assert!(fresh["data"]["source"].is_null());
    assert_eq!(fresh["data"]["items"], json!([]));
    let raw = seed(&f, &entry, true, true);
    let mut first = body.clone();
    first["limit"] = json!(1);
    let value = call(&f, &reader, PATH, &first, 200).await["data"].clone();
    assert_eq!(value["items"][0]["row"], raw);
    assert_eq!(value["next_cursor"], 1);
    assert_eq!(value["workspace_id"], body["workspace_id"]);
    assert_eq!(value["entry_id"], entry);
    assert_eq!(value["source"].as_object().unwrap().len(), 3);
    assert_eq!(value["source"]["schema"], 2);
    assert_eq!(value["source"]["database_sha256"], "a".repeat(64));
    for key in [f.setup.owner_key.secret(), f.setup.instance_key.secret()] {
        assert_eq!(admin(&f, key, PATH, &first, 200).await["data"], value);
    }
    let large = call(&f, &reader, PATH, &body, 200).await["data"].clone();
    assert_eq!(large["items"].as_array().unwrap().len(), 4);
    assert_eq!(large["next_cursor"], 4);
    assert!(large.to_string().len() > 1024 * 1024);
    assert!(!large.to_string().contains("PRIVATE_NATIVE_TOKEN"));
    let mut tail = body.clone();
    tail["after"] = large["next_cursor"].clone();
    let last = call(&f, &reader, PATH, &tail, 200).await;
    assert_eq!(last["data"]["items"][0]["ordinal"], 5);
    assert!(last["data"]["next_cursor"].is_null());
    assert_eq!(last["data"]["items"].as_array().unwrap().len(), 1);
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_history_strict_routes_and_visibility_follow_current_entry_authority() {
    let f = Fixture::new().await;
    let writer = enrolled(&f, "read_write").await;
    let reader = enrolled(&f, "read").await;
    let entry = entry(&f, &writer).await;
    seed(&f, &entry, false, true);
    let body = page(&f, &entry);
    for (field, bad) in [
        ("limit", json!(0)),
        ("limit", json!(33)),
        ("after", json!(u64::MAX)),
        ("claimed_actor", json!("owner")),
    ] {
        let mut invalid = body.clone();
        invalid[field] = bad;
        code(
            &call(&f, &reader, PATH, &invalid, 400).await,
            "invalid_request",
        );
    }
    json_reply(
        reader
            .client
            .get(f.mtls(&format!("/api/v1/content/{PATH}")))
            .send()
            .await
            .unwrap(),
        405,
    )
    .await;
    json_reply(
        f.public_client()
            .post(f.url(&format!("/api/v1/content/{PATH}")))
            .json(&body)
            .send()
            .await
            .unwrap(),
        401,
    )
    .await;
    let second = f.second();
    let mut foreign = body.clone();
    foreign["workspace_id"] = json!(second.workspace_id.to_string());
    call(&f, &reader, PATH, &foreign, 403).await;
    let archive = mutation(
        &f,
        json!({"action":"archive","entry_id":entry,"expected_revision":1}),
    );
    call(&f, &writer, "entries/mutate", &archive, 200).await;
    for user in [&writer, &reader] {
        call(&f, user, PATH, &body, 404).await;
    }
    admin(&f, f.setup.owner_key.secret(), PATH, &body, 404).await;
    let restore = mutation(
        &f,
        json!({"action":"restore","entry_id":entry,"expected_revision":2}),
    );
    call(&f, &writer, "entries/mutate", &restore, 200).await;
    f.db()
        .execute(
            "UPDATE ccp_workspaces SET state='archived' WHERE id=?1",
            [f.setup.workspace_id.to_string()],
        )
        .unwrap();
    call(&f, &reader, PATH, &body, 200).await;
    f.db()
        .execute(
            "UPDATE ccp_clients SET disabled=1 WHERE id=?1",
            [reader.identity["client_id"].as_str().unwrap()],
        )
        .unwrap();
    code(&call(&f, &reader, PATH, &body, 403).await, "forbidden");
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_history_unsealed_and_corrupt_storage_never_release_source_payload() {
    let f = Fixture::new().await;
    let writer = enrolled(&f, "read_write").await;
    let entry = entry(&f, &writer).await;
    seed(&f, &entry, false, false);
    let body = page(&f, &entry);
    let value = call(&f, &writer, PATH, &body, 200).await;
    assert!(value["data"]["source"].is_null());
    assert_eq!(value["data"]["items"], json!([]));
    f.db()
        .execute(
            "UPDATE ccp_native_imports SET sealed=1 WHERE workspace_id=?1",
            [f.setup.workspace_id.to_string()],
        )
        .unwrap();
    f.db().execute_batch("DROP TRIGGER ccp_immutable_native_entry_row; UPDATE ccp_native_entry_rows SET row_json='PRIVATE_BROKEN_JSON';").unwrap();
    let broken = call(&f, &writer, PATH, &body, 503).await;
    assert_eq!(broken["error"]["code"], "service_unavailable");
    assert!(!broken.to_string().contains("PRIVATE"));
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_history_rechecks_grants_shelf_visibility_and_legacy_conversion_gate() {
    let f = Fixture::new().await;
    let writer = enrolled(&f, "read_write").await;
    let reader = enrolled(&f, "read").await;
    let entry = entry(&f, &writer).await;
    seed(&f, &entry, false, true);
    let body = page(&f, &entry);
    f.db()
        .execute(
            "UPDATE ccp_grants SET revoked_at=?1 WHERE client_id=?2",
            params![now(), reader.identity["client_id"].as_str().unwrap()],
        )
        .unwrap();
    call(&f, &reader, PATH, &body, 403).await;
    f.db()
        .execute(
            "UPDATE ccp_grants SET revoked_at=NULL WHERE client_id=?1",
            [reader.identity["client_id"].as_str().unwrap()],
        )
        .unwrap();
    call(&f, &reader, PATH, &body, 200).await;
    f.db()
        .execute(
            "UPDATE ccp_content_shelves SET archived=1 WHERE workspace_id=?1",
            [f.setup.workspace_id.to_string()],
        )
        .unwrap();
    call(&f, &reader, PATH, &body, 404).await;
    f.db()
        .execute(
            "UPDATE ccp_content_shelves SET archived=0 WHERE workspace_id=?1",
            [f.setup.workspace_id.to_string()],
        )
        .unwrap();
    call(&f, &reader, PATH, &body, 200).await;
    f.db()
        .execute_batch("INSERT INTO sessions(id,name) VALUES(99,'legacy')")
        .unwrap();
    f.db()
        .execute(
            "UPDATE ccp_workspaces SET legacy_session_id=99 WHERE id=?1",
            [f.setup.workspace_id.to_string()],
        )
        .unwrap();
    code(
        &call(&f, &reader, PATH, &body, 409).await,
        "content_conversion_required",
    );
    f.finish().await;
}
