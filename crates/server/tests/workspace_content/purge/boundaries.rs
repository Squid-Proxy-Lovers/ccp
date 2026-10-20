// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn entry_purge_http_strict_inputs_surface_separation_and_existing_request_cap() {
    let f = Fixture::new().await;
    let writer = enrolled(&f, "read_write").await;
    let reader = enrolled(&f, "read").await;
    let (_, _, request) = archived(&f, &writer).await;
    let key = f.setup.owner_key.secret();
    for (field, value) in [
        ("actor", json!("PRIVATE_BODY")),
        ("owner_client_id", writer.identity["client_id"].clone()),
        ("workspace_id", json!("bad")),
        ("entry_id", json!("bad")),
        ("entry_id", json!(Uuid::nil().to_string())),
        ("operation_id", json!(Uuid::nil().to_string())),
        ("expected_revision", json!(0)),
        ("expected_revision", json!(true)),
        ("expected_revision", json!(u64::MAX)),
        ("confirm_purge", json!("true")),
        ("confirm_purge", Value::Null),
    ] {
        let mut invalid = request.clone();
        invalid[field] = value;
        code(&change(&f, key, &invalid, 400).await, "invalid_request");
    }
    let mut missing = request.clone();
    missing.as_object_mut().unwrap().remove("confirm_purge");
    code(&change(&f, key, &missing, 400).await, "invalid_request");
    assert_listener_boundaries(&f, &writer, &reader, &request).await;
    assert_eq!(count(&f, "ccp_content_entries"), 1);
    assert_eq!(count(&f, "ccp_entry_purges"), 0);
    f.finish().await;
}

async fn assert_listener_boundaries(f: &Fixture, writer: &User, reader: &User, request: &Value) {
    let key = f.setup.owner_key.secret();
    for user in [writer, reader] {
        code(
            &json_reply(
                user.client
                    .post(f.mtls(PURGE))
                    .json(&request)
                    .send()
                    .await
                    .unwrap(),
                404,
            )
            .await,
            "not_found",
        );
    }
    code(
        &json_reply(
            writer
                .client
                .post(f.mtls(PURGE))
                .bearer_auth(key)
                .json(&request)
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
                .post(f.url(PURGE))
                .json(&request)
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
                .get(f.url(PURGE))
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
        &json_reply(
            f.public_client()
                .post(f.url(&format!("{PURGE}?confirm_purge=true")))
                .bearer_auth(key)
                .json(&request)
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
                .post(f.url(PURGE))
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
    let mut wire = Wire::connect(f, None).await;
    wire.send(&format!("POST {PURGE} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: 65537\r\n\r\n")).await;
    let (status, reply) = wire.response().await;
    assert_eq!(status, 413);
    code(&reply, "request_too_large");
    drop(wire);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn entry_purge_http_audit_abort_or_ignore_rolls_back_and_never_echoes_private_storage_detail()
{
    let f = Fixture::new().await;
    let writer = enrolled(&f, "read_write").await;
    let (_, _, request) = archived(&f, &writer).await;
    let key = f.setup.owner_key.secret();
    for statement in [
        "SELECT RAISE(ABORT,'PRIVATE_BODY_SQL_DETAIL');",
        "SELECT RAISE(IGNORE);",
    ] {
        f.db().execute_batch(&format!("CREATE TRIGGER purge_http_fault BEFORE INSERT ON ccp_access_audit WHEN NEW.action='purge_entry' BEGIN {statement} END;")).unwrap();
        let failed = change(&f, key, &request, 503).await;
        code(&failed, "service_unavailable");
        assert!(!failed.to_string().contains("PRIVATE"));
        assert_eq!(count(&f, "ccp_entry_purges"), 0);
        assert_eq!(count(&f, "ccp_content_entries"), 1);
        assert_eq!(count(&f, "ccp_content_versions"), 2);
        f.db()
            .execute_batch("DROP TRIGGER purge_http_fault")
            .unwrap();
    }
    change(&f, key, &request, 200).await;
    assert_eq!(count(&f, "ccp_content_versions"), 0);
    f.finish().await;
}
