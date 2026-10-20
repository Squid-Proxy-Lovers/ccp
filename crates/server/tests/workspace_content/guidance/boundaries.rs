// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guidance_http_strict_requests_listener_separation_and_body_bounds() {
    let f = Fixture::new().await;
    let user = enrolled(&f, "read_write").await;
    let key = f.setup.owner_key.secret();
    let first = intent(local(&f), 0, "private");
    for (field, value) in [
        ("actor", json!("PRIVATE_GUIDANCE")),
        ("expected_revision", json!(true)),
        ("expected_revision", json!(-1)),
        ("expected_revision", json!(i64::MAX)),
        ("expected_revision", json!(u64::MAX)),
        ("operation_id", json!(Uuid::nil().to_string())),
        ("operation_id", json!("bad")),
        ("text", Value::Null),
        ("scope", json!({"kind":"workspace","workspace_id":"bad"})),
        (
            "scope",
            json!({"kind":"global","workspace_id":f.setup.workspace_id.to_string()}),
        ),
        (
            "scope",
            json!({"kind":"workspace","workspace_id":f.setup.workspace_id.to_string(),"publisher":"PRIVATE_GUIDANCE"}),
        ),
        ("scope", json!({"kind":"other"})),
    ] {
        let mut body = first.clone();
        body[field] = value;
        code(&save(&f, key, &body, 400).await, "invalid_request");
    }
    for body in [
        json!({"workspace_id":"bad"}),
        json!({"actor":"PRIVATE_GUIDANCE"}),
        json!({"workspace_id":true}),
    ] {
        code(
            &admin_call(&f, key, GET, &body, 400).await,
            "invalid_request",
        );
    }
    assert_routes(&f, &user, &first).await;
    assert_eq!(count(&f, "ccp_guidance_operations"), 0);
    f.finish().await;
}

async fn assert_routes(f: &Fixture, user: &User, first: &Value) {
    let key = f.setup.owner_key.secret();
    code(
        &json_reply(
            user.client
                .post(f.mtls(SAVE))
                .json(&first)
                .send()
                .await
                .unwrap(),
            404,
        )
        .await,
        "not_found",
    );
    code(
        &json_reply(
            user.client
                .post(f.mtls(GET))
                .bearer_auth(key)
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
        &json_reply(
            f.public_client()
                .post(f.url(GET))
                .json(&json!({}))
                .send()
                .await
                .unwrap(),
            401,
        )
        .await,
        "unauthorized",
    );
    code(
        &json_reply(
            f.public_client()
                .get(f.url(GET))
                .bearer_auth(key)
                .send()
                .await
                .unwrap(),
            405,
        )
        .await,
        "method_not_allowed",
    );
    code(
        &admin_call(f, key, &format!("{GET}?workspace_id=x"), &json!({}), 400).await,
        "invalid_request",
    );
    code(
        &json_reply(
            f.public_client()
                .post(f.url(SAVE))
                .bearer_auth(key)
                .header("content-type", "text/plain")
                .body("{}")
                .send()
                .await
                .unwrap(),
            415,
        )
        .await,
        "unsupported_media_type",
    );
    for path in [GET, SAVE] {
        let mut wire = Wire::connect(f, None).await;
        wire.send(&format!("POST {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: 65537\r\n\r\n")).await;
        let (status, reply) = wire.response().await;
        assert_eq!(status, 413);
        code(&reply, "request_too_large");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guidance_http_exact_utf8_controls_and_two_escaped_boards_fit_unchanged_limits() {
    let f = Fixture::new().await;
    let user = enrolled(&f, "read").await;
    let text = "\0".repeat(8192);
    save(
        &f,
        f.setup.instance_key.secret(),
        &intent(json!({"kind":"global"}), 0, &text),
        200,
    )
    .await;
    save(
        &f,
        f.setup.owner_key.secret(),
        &intent(local(&f), 0, &text),
        200,
    )
    .await;
    let bundle = get(&f, &user, Some(f.setup.workspace_id), 200).await;
    assert_eq!(bundle["data"]["global"]["text"], text);
    assert_eq!(bundle["data"]["workspace"]["text"], text);
    let encoded = serde_json::to_vec(&bundle).unwrap();
    assert!(encoded.len() > 98_000 && encoded.len() < 1024 * 1024);
    let unicode = "é".repeat(4096);
    save(
        &f,
        f.setup.owner_key.secret(),
        &intent(local(&f), 1, &unicode),
        200,
    )
    .await;
    assert_eq!(
        get(&f, &user, Some(f.setup.workspace_id), 200).await["data"]["workspace"]["text"],
        unicode
    );
    for text in ["x".repeat(8193), "é".repeat(4097)] {
        code(
            &save(
                &f,
                f.setup.owner_key.secret(),
                &intent(local(&f), 2, &text),
                400,
            )
            .await,
            "invalid_request",
        );
    }
    assert_eq!(count(&f, "ccp_guidance_versions"), 3);
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guidance_http_transaction_failure_and_corrupt_published_payload_are_sanitized() {
    let f = Fixture::new().await;
    let user = enrolled(&f, "read").await;
    let key = f.setup.owner_key.secret();
    save(&f, key, &intent(local(&f), 0, "PRIVATE_GUIDANCE"), 200).await;
    let next = intent(local(&f), 1, "PRIVATE_REPLACEMENT");
    f.db().execute_batch("CREATE TRIGGER guidance_http_fault BEFORE INSERT ON ccp_access_audit WHEN NEW.action='save_workspace_guidance' BEGIN SELECT RAISE(ABORT,'PRIVATE_SQL_DETAIL'); END;").unwrap();
    let failure = save(&f, key, &next, 503).await;
    code(&failure, "service_unavailable");
    assert!(!failure.to_string().contains("PRIVATE"));
    assert_eq!(count(&f, "ccp_guidance_versions"), 1);
    assert_eq!(count(&f, "ccp_guidance_operations"), 1);
    assert_eq!(
        get(&f, &user, Some(f.setup.workspace_id), 200).await["data"]["workspace"]["text"],
        "PRIVATE_GUIDANCE"
    );
    f.db()
        .execute_batch("DROP TRIGGER guidance_http_fault")
        .unwrap();
    save(&f, key, &next, 200).await;
    let db = f.db();
    db.pragma_update(None, "foreign_keys", false).unwrap();
    db.execute("DELETE FROM ccp_guidance_versions WHERE revision=2", [])
        .unwrap();
    drop(db);
    let failed = get(&f, &user, Some(f.setup.workspace_id), 503).await;
    code(&failed, "service_unavailable");
    assert!(!failed.to_string().contains("PRIVATE"));
    assert!(!failed.to_string().contains("text"));
    let global = get(&f, &user, None, 200).await;
    assert_eq!(global["data"]["global"]["revision"], 0);
    f.finish().await;
}
