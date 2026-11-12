// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
const CONTEXT: &str = "/api/v1/guidance/context";

async fn delivery(f: &Fixture, user: &User, workspace: Option<Uuid>, status: u16) -> Value {
    json_reply(
        user.client
            .post(f.mtls(CONTEXT))
            .json(&json!({"workspace_id":workspace.map(|id|id.to_string())}))
            .send()
            .await
            .unwrap(),
        status,
    )
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guidance_delivery_http_context_is_authenticated_client_snapshot_only() {
    let f = Fixture::new().await;
    let user = enrolled(&f, "read").await;
    let ws = f.setup.workspace_id;
    let value = delivery(&f, &user, Some(ws), 200).await;
    let context = &value["data"]["context"];
    assert_eq!(value["data"]["format"], 1);
    assert_eq!(context.as_object().unwrap().len(), 4);
    assert_eq!(context["instance_id"], f.trust.instance_id);
    assert_eq!(context["client_id"], user.identity["client_id"]);
    assert_eq!(context["credential_id"], user.identity["credential_id"]);
    let grant: String = f
        .db()
        .query_row(
            "SELECT id FROM ccp_grants WHERE workspace_id=?1 AND client_id=?2",
            rusqlite::params![ws.to_string(), user.identity["client_id"].as_str().unwrap()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(context["workspace_grant_id"], grant);
    assert_eq!(
        value["data"]["guidance"],
        get(&f, &user, Some(ws), 200).await["data"]
    );
    assert!(
        delivery(&f, &user, None, 200).await["data"]["context"]["workspace_grant_id"].is_null()
    );
    code(
        &json_reply(
            f.public_client()
                .post(f.url(CONTEXT))
                .bearer_auth(f.setup.owner_key.secret())
                .json(&json!({}))
                .send()
                .await
                .unwrap(),
            404,
        )
        .await,
        "not_found",
    );
    for body in [
        json!({"client_id":user.identity["client_id"]}),
        json!({"workspace_grant_id":grant}),
        json!({"workspace_id":"bad"}),
    ] {
        code(
            &json_reply(
                user.client
                    .post(f.mtls(CONTEXT))
                    .json(&body)
                    .send()
                    .await
                    .unwrap(),
                400,
            )
            .await,
            "invalid_request",
        );
    }
    code(
        &json_reply(
            user.client
                .post(f.mtls(CONTEXT))
                .bearer_auth(f.setup.owner_key.secret())
                .json(&json!({}))
                .send()
                .await
                .unwrap(),
            400,
        )
        .await,
        "invalid_request",
    );
    code(
        &json_reply(user.client.get(f.mtls(CONTEXT)).send().await.unwrap(), 405).await,
        "method_not_allowed",
    );
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guidance_delivery_http_re_admission_changes_binding_even_when_board_and_credential_do_not()
{
    use server::foundation::{Admission, DataRole};
    let f = Fixture::new().await;
    let user = enrolled(&f, "read").await;
    let ws = f.setup.workspace_id;
    let old = delivery(&f, &user, Some(ws), 200).await["data"].clone();
    {
        let mut db = f.db();
        let mut store = AccessStore::new(&mut db).unwrap();
        let owner = store
            .authenticate_admin_key(f.setup.owner_key.secret(), now())
            .unwrap();
        store
            .revoke_grant(
                owner,
                Uuid::parse_str(old["context"]["workspace_grant_id"].as_str().unwrap()).unwrap(),
                now(),
            )
            .unwrap();
    }
    code(&delivery(&f, &user, Some(ws), 403).await, "forbidden");
    assert!(
        delivery(&f, &user, None, 200).await["data"]["context"]["workspace_grant_id"].is_null()
    );
    let grant = {
        let mut db = f.db();
        let mut store = AccessStore::new(&mut db).unwrap();
        let owner = store
            .authenticate_admin_key(f.setup.owner_key.secret(), now())
            .unwrap();
        store
            .admit_existing_client(
                owner,
                Admission {
                    workspace_id: ws,
                    presented_credential: Uuid::parse_str(
                        user.identity["credential_id"].as_str().unwrap(),
                    )
                    .unwrap(),
                    role: DataRole::Read,
                },
                now(),
            )
            .unwrap()
    };
    let fresh = delivery(&f, &user, Some(ws), 200).await["data"].clone();
    assert_eq!(fresh["context"]["workspace_grant_id"], grant.to_string());
    assert_ne!(
        fresh["context"]["workspace_grant_id"],
        old["context"]["workspace_grant_id"]
    );
    assert_eq!(
        fresh["context"]["credential_id"],
        old["context"]["credential_id"]
    );
    assert_eq!(fresh["guidance"], old["guidance"]);
    f.finish().await;
}
