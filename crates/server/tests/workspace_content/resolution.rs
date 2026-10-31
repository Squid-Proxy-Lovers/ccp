// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use sha2::{Digest, Sha256};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recovered_read_only_client_resolves_its_expired_original_operations_without_regaining_write()
 {
    let f = Fixture::new().await;
    let user = enrolled(&f, "read_write").await;
    let (_, book) = hierarchy(&f, &user).await;
    let first = call(
        &f,
        &user,
        "entries/mutate",
        &create_entry(&f, &book["id"], "entry", "base"),
        200,
    )
    .await;
    let id = &first["data"]["entry_id"];
    let committed = mutation(
        &f,
        json!({"action":"append","entry_id":id,"expected_revision":1,"text":"+once"}),
    );
    let committed_target = resolution(&f, &committed, &user.identity["credential_id"]);
    call(&f, &user, "entries/mutate", &committed, 200).await;
    let pending = mutation(
        &f,
        json!({"action":"append","entry_id":id,"expected_revision":2,"text":"+late"}),
    );
    let pending_target = resolution(&f, &pending, &user.identity["credential_id"]);
    let token = admin_action(&f,f.setup.owner_key.secret(),"/api/v1/admin/recovery/issue",&json!({"client_id":user.identity["client_id"],"scope":{"kind":"workspace","workspace_id":f.setup.workspace_id.to_string()},"expires_at":now()+3600}),200).await;
    let key = KeyPair::generate().unwrap();
    let reply = json_reply(f.public_client().post(f.url("/api/v1/recover")).json(&json!({"invitation":token["secret"],"request_id":Uuid::new_v4().to_string(),"csr_pem":csr(&key)})).send().await.unwrap(),200).await;
    let recovered = User {
        client: f.private_client(reply["data"]["certificate_pem"].as_str().unwrap(), &key),
        identity: reply["data"].clone(),
    };
    assert_eq!(recovered.identity["client_id"], user.identity["client_id"]);
    f.db()
        .execute(
            "UPDATE ccp_credentials SET expires_at=?1 WHERE id=?2",
            rusqlite::params![now(), user.identity["credential_id"].as_str().unwrap()],
        )
        .unwrap();
    f.db()
        .execute("UPDATE ccp_grants SET role='read'", [])
        .unwrap();
    assert_eq!(
        call(
            &f,
            &recovered,
            "entries/resolve-operation",
            &committed_target,
            200
        )
        .await["data"]["outcome"]["state"],
        "committed"
    );
    assert_eq!(
        call(
            &f,
            &recovered,
            "entries/resolve-operation",
            &pending_target,
            200
        )
        .await["data"]["outcome"]["state"],
        "cancelled"
    );
    code(
        &call(&f, &user, "entries/resolve-operation", &pending_target, 403).await,
        "forbidden",
    );
    code(
        &call(&f, &recovered, "entries/mutate", &pending, 403).await,
        "forbidden",
    );
    assert_eq!(
        snapshot(&f, &recovered, id, None).await["content"]["text"],
        "base+once"
    );
    f.finish().await;
}

fn resolution(f: &Fixture, request: &Value, credential: &Value) -> Value {
    let change = &request["change"];
    let encoded = serde_json::to_vec(&json!([
        "entry-mutation-1",
        "append",
        request["agent"],
        [
            change["entry_id"],
            change["expected_revision"],
            change["text"]
        ]
    ]))
    .unwrap();
    let hash = Sha256::digest(encoded)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    json!({"workspace_id":f.setup.workspace_id.to_string(),"operation_id":request["operation_id"],"credential_id":credential,"request_hash":hash})
}

async fn renew_user(f: &Fixture, user: &User) -> User {
    let key = KeyPair::generate().unwrap();
    let response = json_reply(
        user.client
            .post(f.mtls("/api/v1/credentials/renew"))
            .json(&json!({"request_id":Uuid::new_v4().to_string(),"csr_pem":csr(&key)}))
            .send()
            .await
            .unwrap(),
        200,
    )
    .await;
    User {
        client: f.private_client(response["data"]["certificate_pem"].as_str().unwrap(), &key),
        identity: response["data"].clone(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn committed_operation_can_be_resolved_after_renewal_and_archive_without_replaying_or_reading_body()
 {
    let f = Fixture::new().await;
    let user = enrolled(&f, "read_write").await;
    let (_, book) = hierarchy(&f, &user).await;
    let first = call(
        &f,
        &user,
        "entries/mutate",
        &create_entry(&f, &book["id"], "entry", "private original"),
        200,
    )
    .await;
    let id = &first["data"]["entry_id"];
    let append = mutation(
        &f,
        json!({"action":"append","entry_id":id,"expected_revision":1,"text":"+once"}),
    );
    let target = resolution(&f, &append, &user.identity["credential_id"]);
    let receipt = call(&f, &user, "entries/mutate", &append, 200).await;
    call(
        &f,
        &user,
        "entries/mutate",
        &mutation(
            &f,
            json!({"action":"archive","entry_id":id,"expected_revision":2}),
        ),
        200,
    )
    .await;
    let renewed = renew_user(&f, &user).await;
    assert_eq!(renewed.identity["client_id"], user.identity["client_id"]);
    assert_ne!(
        renewed.identity["credential_id"],
        user.identity["credential_id"]
    );
    code(
        &call(&f, &user, "entries/resolve-operation", &target, 403).await,
        "forbidden",
    );
    let found = call(&f, &renewed, "entries/resolve-operation", &target, 200).await;
    assert_eq!(
        found["data"]["outcome"],
        json!({"state":"committed","entry_id":id,"revision":receipt["data"]["revision"]})
    );
    assert_eq!(found["data"]["credential_id"], target["credential_id"]);
    assert_eq!(found["data"]["request_hash"], target["request_hash"]);
    assert!(!found.to_string().contains("private original"));
    code(
        &call(&f, &renewed, "entries/get", &entry_target(&f, id), 404).await,
        "not_found",
    );
    let mut changed = target.clone();
    changed["request_hash"] = json!("0".repeat(64));
    code(
        &call(&f, &renewed, "entries/resolve-operation", &changed, 409).await,
        "content_conflict",
    );
    assert_cancel_after_rotation(&f, &renewed, &user.identity["credential_id"], id).await;
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn read_only_resolution_is_scoped_to_own_client_and_cannot_preempt_a_foreign_writer() {
    let f = Fixture::new().await;
    let user = enrolled(&f, "read_write").await;
    let reader = enrolled(&f, "read").await;
    let (_, book) = hierarchy(&f, &user).await;
    let first = call(
        &f,
        &user,
        "entries/mutate",
        &create_entry(&f, &book["id"], "entry", "base"),
        200,
    )
    .await;
    let id = &first["data"]["entry_id"];
    let append = mutation(
        &f,
        json!({"action":"append","entry_id":id,"expected_revision":1,"text":"+once"}),
    );
    let target = resolution(&f, &append, &user.identity["credential_id"]);
    code(
        &call(&f, &reader, "entries/resolve-operation", &target, 403).await,
        "forbidden",
    );
    let mut own = target.clone();
    own["credential_id"] = reader.identity["credential_id"].clone();
    assert_eq!(
        call(&f, &reader, "entries/resolve-operation", &own, 200).await["data"]["outcome"]["state"],
        "cancelled"
    );
    call(&f, &user, "entries/mutate", &append, 200).await;
    assert_eq!(
        call(&f, &reader, "entries/resolve-operation", &own, 200).await["data"]["outcome"]["state"],
        "cancelled"
    );
    assert_eq!(
        call(&f, &user, "entries/resolve-operation", &target, 200).await["data"]["outcome"]["state"],
        "committed"
    );
    let late = mutation(
        &f,
        json!({"action":"append","entry_id":id,"expected_revision":2,"text":"+twice"}),
    );
    let target = resolution(&f, &late, &user.identity["credential_id"]);
    let cancel = call(&f, &user, "entries/resolve-operation", &target, 200).await;
    assert_eq!(cancel["data"]["outcome"].as_object().unwrap().len(), 1);
    code(
        &call(&f, &user, "entries/mutate", &late, 409).await,
        "operation_cancelled",
    );
    let mut changed = late;
    changed["agent"] = Value::Null;
    code(
        &call(&f, &user, "entries/mutate", &changed, 409).await,
        "content_conflict",
    );
    assert_eq!(
        snapshot(&f, &user, id, None).await["content"]["text"],
        "base+once"
    );
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn operation_resolution_rejects_spoofed_fields_admin_impersonation_bad_hashes_and_current_disable()
 {
    let f = Fixture::new().await;
    let user = enrolled(&f, "read_write").await;
    let request = mutation(
        &f,
        json!({"action":"append","entry_id":Uuid::new_v4().to_string(),"expected_revision":1,"text":"once"}),
    );
    let target = resolution(&f, &request, &user.identity["credential_id"]);
    for hash in [
        String::new(),
        "a".repeat(63),
        "A".repeat(64),
        "g".repeat(64),
        "é".repeat(32),
    ] {
        let mut body = target.clone();
        body["request_hash"] = json!(hash);
        code(
            &call(&f, &user, "entries/resolve-operation", &body, 400).await,
            "invalid_request",
        );
    }
    let mut bad = target.clone();
    bad["operation_id"] = json!("invalid");
    code(
        &call(&f, &user, "entries/resolve-operation", &bad, 400).await,
        "invalid_request",
    );
    let mut bad = target.clone();
    bad["client_id"] = user.identity["client_id"].clone();
    code(
        &call(&f, &user, "entries/resolve-operation", &bad, 400).await,
        "invalid_request",
    );
    let mut bad = target.clone();
    bad["credential_id"] = json!(Uuid::new_v4().to_string());
    code(
        &call(&f, &user, "entries/resolve-operation", &bad, 403).await,
        "forbidden",
    );
    for key in [f.setup.owner_key.secret(), f.setup.instance_key.secret()] {
        code(
            &admin(&f, key, "entries/resolve-operation", &target, 403).await,
            "forbidden",
        );
    }
    code(
        &json_reply(
            f.public_client()
                .post(f.url("/api/v1/content/entries/resolve-operation"))
                .json(&target)
                .send()
                .await
                .unwrap(),
            401,
        )
        .await,
        "unauthorized",
    );
    code(
        &call(&f, &user, "entries/resolve-operation?extra=1", &target, 400).await,
        "invalid_request",
    );
    code(
        &json_reply(
            user.client
                .get(f.mtls("/api/v1/content/entries/resolve-operation"))
                .send()
                .await
                .unwrap(),
            405,
        )
        .await,
        "method_not_allowed",
    );
    admin_action(
        &f,
        f.setup.owner_key.secret(),
        "/api/v1/admin/workspaces/archive",
        &json!({"workspace_id":f.setup.workspace_id.to_string()}),
        200,
    )
    .await;
    assert_eq!(
        call(&f, &user, "entries/resolve-operation", &target, 200).await["data"]["outcome"]["state"],
        "cancelled"
    );
    admin_action(
        &f,
        f.setup.instance_key.secret(),
        "/api/v1/admin/clients/disable",
        &json!({"client_id":user.identity["client_id"]}),
        200,
    )
    .await;
    code(
        &call(&f, &user, "entries/resolve-operation", &target, 403).await,
        "forbidden",
    );
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn operation_resolution_audit_fault_rolls_back_and_legacy_conversion_is_never_bypassed() {
    let f = Fixture::new().await;
    let user = enrolled(&f, "read_write").await;
    let request = mutation(
        &f,
        json!({"action":"append","entry_id":Uuid::new_v4().to_string(),"expected_revision":1,"text":"once"}),
    );
    let target = resolution(&f, &request, &user.identity["credential_id"]);
    f.db().execute_batch("CREATE TRIGGER resolution_fault BEFORE INSERT ON ccp_access_audit WHEN NEW.action='cancel_entry_operation' BEGIN SELECT RAISE(ABORT,'private resolution sql'); END;").unwrap();
    code(
        &call(&f, &user, "entries/resolve-operation", &target, 503).await,
        "service_unavailable",
    );
    let count: u64 = f
        .db()
        .query_row("SELECT COUNT(*) FROM ccp_content_cancellations", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 0);
    f.db()
        .execute_batch("DROP TRIGGER resolution_fault")
        .unwrap();
    assert_eq!(
        call(&f, &user, "entries/resolve-operation", &target, 200).await["data"]["outcome"]["state"],
        "cancelled"
    );
    code(
        &call(&f, &user, "entries/mutate", &request, 409).await,
        "operation_cancelled",
    );
    f.db()
        .execute_batch("INSERT INTO sessions(id,name) VALUES(99,'old');")
        .unwrap();
    f.db()
        .execute(
            "UPDATE ccp_workspaces SET legacy_session_id=99 WHERE id=?1",
            [f.setup.workspace_id.to_string()],
        )
        .unwrap();
    code(
        &call(&f, &user, "entries/resolve-operation", &target, 409).await,
        "content_conversion_required",
    );
    f.finish().await;
}

async fn assert_cancel_after_rotation(f: &Fixture, current: &User, original: &Value, id: &Value) {
    call(
        f,
        current,
        "entries/mutate",
        &mutation(
            f,
            json!({"action":"restore","entry_id":id,"expected_revision":3}),
        ),
        200,
    )
    .await;
    let unresolved = mutation(
        f,
        json!({"action":"append","entry_id":id,"expected_revision":4,"text":"late"}),
    );
    let target = resolution(f, &unresolved, original);
    assert_eq!(
        call(f, current, "entries/resolve-operation", &target, 200).await["data"]["outcome"]["state"],
        "cancelled"
    );
    code(
        &call(f, current, "entries/mutate", &unresolved, 409).await,
        "content_conflict",
    );
    assert_eq!(
        snapshot(f, current, id, None).await["content"]["text"],
        "private original+once"
    );
}
