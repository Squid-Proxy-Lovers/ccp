// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

mod boundaries;
mod lifecycle;
mod ownership;
mod resolution;
mod shelves;

struct User {
    client: Client,
    identity: Value,
}

async fn enrolled(f: &Fixture, role: &str) -> User {
    let invitation = json_reply(f.public_client()
        .post(f.url("/api/v1/admin/invitations/individual"))
        .bearer_auth(f.setup.owner_key.secret())
        .json(&json!({"workspace_id":f.setup.workspace_id.to_string(),"role":role,"expires_at":now()+3600}))
        .send().await.unwrap(),200).await;
    let key = KeyPair::generate().unwrap();
    let result = json_reply(f.public_client().post(f.url("/api/v1/enroll"))
        .json(&json!({"invitation":invitation["data"]["secret"],"request_id":Uuid::new_v4().to_string(),"csr_pem":csr(&key)}))
        .send().await.unwrap(),200).await;
    User {
        client: f.private_client(result["data"]["certificate_pem"].as_str().unwrap(), &key),
        identity: result["data"].clone(),
    }
}

async fn call(f: &Fixture, user: &User, path: &str, body: &Value, status: u16) -> Value {
    json_reply(
        user.client
            .post(f.mtls(&format!("/api/v1/content/{path}")))
            .json(body)
            .send()
            .await
            .unwrap(),
        status,
    )
    .await
}

async fn admin(f: &Fixture, key: &str, path: &str, body: &Value, status: u16) -> Value {
    json_reply(
        f.public_client()
            .post(f.url(&format!("/api/v1/content/{path}")))
            .bearer_auth(key)
            .json(body)
            .send()
            .await
            .unwrap(),
        status,
    )
    .await
}

fn mutation(f: &Fixture, change: Value) -> Value {
    json!({"workspace_id":f.setup.workspace_id.to_string(),"operation_id":Uuid::new_v4().to_string(),"agent":"same reported name","change":change})
}

fn content_body(text: &str) -> Value {
    json!({"text":text,"description":"old description","labels":["shared"]})
}

fn create_entry(f: &Fixture, book: &Value, name: &str, text: &str) -> Value {
    mutation(
        f,
        json!({"action":"create","book_id":book,"name":name,"content":content_body(text)}),
    )
}

async fn hierarchy(f: &Fixture, user: &User) -> (Value, Value) {
    let shelf = call(f,user,"shelves/create",&json!({"workspace_id":f.setup.workspace_id.to_string(),"name":"notes","description":"shelf\nmetadata"}),200).await;
    let book = call(f,user,"books/create",&json!({"workspace_id":f.setup.workspace_id.to_string(),"shelf_id":shelf["data"]["id"],"name":"book","description":"book metadata"}),200).await;
    (shelf["data"].clone(), book["data"].clone())
}

async fn snapshot(f: &Fixture, user: &User, entry: &Value, revision: Option<u64>) -> Value {
    call(f,user,"entries/get",&json!({"workspace_id":f.setup.workspace_id.to_string(),"entry_id":entry,"revision":revision}),200).await["data"].clone()
}

fn entry_target(f: &Fixture, id: &Value) -> Value {
    json!({"workspace_id":f.setup.workspace_id.to_string(),"entry_id":id})
}

fn code(value: &Value, expected: &str) {
    assert_eq!(value["error"]["code"], expected);
    assert_eq!(value.as_object().unwrap().len(), 2);
    assert_eq!(value["error"].as_object().unwrap().len(), 1);
}
