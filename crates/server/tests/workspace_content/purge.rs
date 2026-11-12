// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

mod boundaries;

const PURGE: &str = "/api/v1/admin/content/entries/purge";
fn intent(f: &Fixture, entry: &Value, revision: u64) -> Value {
    json!({"workspace_id":f.setup.workspace_id.to_string(),"entry_id":entry,"operation_id":Uuid::new_v4().to_string(),"expected_revision":revision,"confirm_purge":true})
}
async fn change(f: &Fixture, key: &str, body: &Value, status: u16) -> Value {
    json_reply(
        f.public_client()
            .post(f.url(PURGE))
            .bearer_auth(key)
            .json(body)
            .send()
            .await
            .unwrap(),
        status,
    )
    .await
}
async fn archived(f: &Fixture, writer: &User) -> (Value, Value, Value) {
    let (_, book) = hierarchy(f, writer).await;
    let created = call(
        f,
        writer,
        "entries/mutate",
        &create_entry(f, &book["id"], "entry", "PRIVATE_BODY"),
        200,
    )
    .await;
    call(f,writer,"entries/mutate",&mutation(f,json!({"action":"archive","entry_id":created["data"]["entry_id"],"expected_revision":1})),200).await;
    let request = intent(f, &created["data"]["entry_id"], 2);
    (book, created, request)
}
fn count(f: &Fixture, table: &str) -> u64 {
    f.db()
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn entry_purge_http_removes_payload_but_retains_original_client_outcomes_and_exact_admin_proof()
 {
    let f = Fixture::new().await;
    let writer = enrolled(&f, "read_write").await;
    let (_, book) = hierarchy(&f, &writer).await;
    let original = create_entry(&f, &book["id"], "entry", "PRIVATE_BODY");
    let created = call(&f, &writer, "entries/mutate", &original, 200).await;
    let id = created["data"]["entry_id"].clone();
    call(
        &f,
        &writer,
        "entries/mutate",
        &mutation(
            &f,
            json!({"action":"archive","entry_id":id,"expected_revision":1}),
        ),
        200,
    )
    .await;
    let sibling = call(
        &f,
        &writer,
        "entries/mutate",
        &create_entry(&f, &book["id"], "sibling", "kept"),
        200,
    )
    .await;
    let key = f.setup.owner_key.secret();
    let request = intent(&f, &id, 2);
    let result = change(&f, key, &request, 200).await;
    assert_eq!(result["data"].as_object().unwrap().len(), 5);
    for field in ["workspace_id", "entry_id", "operation_id"] {
        assert_eq!(result["data"][field], request[field]);
    }
    assert_eq!(result["data"]["revision"], 2);
    assert!(result["data"]["purged_at"].as_i64().unwrap() > 0);
    assert!(!result.to_string().contains("PRIVATE_BODY"));
    assert_eq!(change(&f, key, &request, 200).await, result);
    assert_eq!(
        call(&f, &writer, "entries/mutate", &original, 200).await,
        created
    );
    let target = original_resolution(&f, &original, &writer.identity["credential_id"]);
    let resolved = call(&f, &writer, "entries/resolve-operation", &target, 200).await;
    assert_eq!(
        resolved["data"]["outcome"],
        json!({"state":"committed","entry_id":id,"revision":1})
    );
    code(
        &call(&f, &writer, "entries/get", &entry_target(&f, &id), 404).await,
        "not_found",
    );
    assert_eq!(
        snapshot(&f, &writer, &sibling["data"]["entry_id"], None).await["content"]["text"],
        "kept"
    );
    assert_eq!(count(&f, "ccp_content_entries"), 1);
    assert_eq!(count(&f, "ccp_content_versions"), 1);
    assert_eq!(count(&f, "ccp_entry_purges"), 1);
    assert_eq!(count(&f, "ccp_content_operations"), 3);
    let mut changed = request.clone();
    changed["expected_revision"] = json!(1);
    code(&change(&f, key, &changed, 409).await, "content_conflict");
    let second = intent(&f, &id, 2);
    code(&change(&f, key, &second, 404).await, "not_found");
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn entry_purge_http_d064_archive_current_scope_and_key_revocation_remain_authoritative() {
    let f = Fixture::new().await;
    let writer = enrolled(&f, "read_write").await;
    let (_, _, request) = archived(&f, &writer).await;
    code(
        &change(&f, f.setup.instance_key.secret(), &request, 403).await,
        "forbidden",
    );
    let second = f.second();
    let mut foreign = request.clone();
    foreign["workspace_id"] = json!(second.workspace_id.to_string());
    code(
        &change(&f, f.setup.owner_key.secret(), &foreign, 403).await,
        "forbidden",
    );
    f.db()
        .execute(
            "UPDATE ccp_workspaces SET state='archived' WHERE id=?1",
            [f.setup.workspace_id.to_string()],
        )
        .unwrap();
    let key = f.setup.owner_key.secret();
    let proof = change(&f, key, &request, 200).await;
    assert_eq!(change(&f, key, &request, 200).await, proof);
    assert_eq!(count(&f, "ccp_content_entries"), 0);
    f.db()
        .execute(
            "UPDATE ccp_admin_keys SET revoked_at=?1 WHERE id=?2",
            rusqlite::params![now(), f.setup.owner_key.id.to_string()],
        )
        .unwrap();
    code(&change(&f, key, &request, 401).await, "unauthorized");
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn entry_purge_http_conflict_and_failed_confirmation_cannot_delete_active_or_changed_entries()
{
    let f = Fixture::new().await;
    let writer = enrolled(&f, "read_write").await;
    let (_, book) = hierarchy(&f, &writer).await;
    let created = call(
        &f,
        &writer,
        "entries/mutate",
        &create_entry(&f, &book["id"], "entry", "kept"),
        200,
    )
    .await;
    let id = &created["data"]["entry_id"];
    let key = f.setup.owner_key.secret();
    code(
        &change(&f, key, &intent(&f, id, 1), 409).await,
        "content_conflict",
    );
    call(
        &f,
        &writer,
        "entries/mutate",
        &mutation(
            &f,
            json!({"action":"archive","entry_id":id,"expected_revision":1}),
        ),
        200,
    )
    .await;
    let mut unconfirmed = intent(&f, id, 2);
    unconfirmed["confirm_purge"] = json!(false);
    code(&change(&f, key, &unconfirmed, 400).await, "invalid_request");
    let stale = intent(&f, id, 2);
    call(
        &f,
        &writer,
        "entries/mutate",
        &mutation(
            &f,
            json!({"action":"restore","entry_id":id,"expected_revision":2}),
        ),
        200,
    )
    .await;
    call(
        &f,
        &writer,
        "entries/mutate",
        &mutation(
            &f,
            json!({"action":"archive","entry_id":id,"expected_revision":3}),
        ),
        200,
    )
    .await;
    code(&change(&f, key, &stale, 409).await, "content_conflict");
    assert_eq!(count(&f, "ccp_entry_purges"), 0);
    assert_eq!(count(&f, "ccp_content_versions"), 4);
    change(&f, key, &intent(&f, id, 4), 200).await;
    f.finish().await;
}

fn original_resolution(f: &Fixture, request: &Value, credential: &Value) -> Value {
    use sha2::{Digest, Sha256};
    let typed: protocol::workspace::content::EntryMutation =
        serde_json::from_value(request.clone()).unwrap();
    let encoded =
        protocol::workspace::content::fingerprint_bytes(&typed.change, typed.agent.as_deref())
            .unwrap();
    let hash = Sha256::digest(encoded)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    json!({"workspace_id":f.setup.workspace_id.to_string(),"operation_id":request["operation_id"],"credential_id":credential,"request_hash":hash})
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn entry_purge_http_scoped_admin_can_purge_while_archived_but_demotion_blocks_replay() {
    use server::foundation::AdminAppointment;
    let f = Fixture::new().await;
    let writer = enrolled(&f, "read_write").await;
    let (_, _, request) = archived(&f, &writer).await;
    let scoped = {
        let mut db = f.db();
        let mut store = AccessStore::new(&mut db).unwrap();
        let owner = store
            .authenticate_admin_key(f.setup.owner_key.secret(), now())
            .unwrap();
        let scoped = store
            .appoint_workspace_admin_with_key(
                owner,
                f.setup.workspace_id,
                &AdminAppointment {
                    principal: InitialOwner::NewPrincipal("purger".into()),
                    key_expires_at: None,
                },
                now(),
            )
            .unwrap();
        store
            .set_archived(owner, f.setup.workspace_id, true, now())
            .unwrap();
        scoped
    };
    let proof = change(&f, scoped.secret(), &request, 200).await;
    assert_eq!(change(&f, scoped.secret(), &request, 200).await, proof);
    code(
        &change(&f, f.setup.owner_key.secret(), &request, 409).await,
        "content_conflict",
    );
    let row: (String, String) = f
        .db()
        .query_row(
            "SELECT actor_id,principal_id FROM ccp_entry_purges",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        row,
        (scoped.id.to_string(), scoped.principal_id.to_string())
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
        &change(&f, scoped.secret(), &request, 401).await,
        "unauthorized",
    );
    assert_eq!(count(&f, "ccp_entry_purges"), 1);
    assert_eq!(count(&f, "ccp_content_entries"), 0);
    f.finish().await;
}
