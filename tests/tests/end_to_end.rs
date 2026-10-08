// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use reqwest::StatusCode;
use serde_json::Value;

use ccp_tests::harness::{LoadOperation, TestServer, run_persistent_load};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn enrollment_tokens_are_reusable_until_expiry_and_issue_distinct_client_certs()
-> anyhow::Result<()> {
    let server = TestServer::start().await?;

    let reused_token = server.issue_token("read")?;
    let read_one = server.redeem_token(&reused_token.token).await?;
    let read_two_from_same_token = server.redeem_token(&reused_token.token).await?;
    assert_ne!(read_one.client_cn, read_two_from_same_token.client_cn);

    let read_two = server.enroll_read().await?;
    let write_one = server.enroll_read_write().await?;

    assert_eq!(read_one.session_name, server.session_name);
    assert_eq!(read_one.session_id, server.session_id);
    assert_eq!(read_two.session_id, server.session_id);
    assert_eq!(write_one.session_id, server.session_id);
    assert_eq!(read_one.access, "read");
    assert_eq!(write_one.access, "read_write");
    assert_ne!(read_one.client_cn, read_two.client_cn);
    assert_ne!(read_one.client_cn, write_one.client_cn);

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn message_flow_enforces_access_and_records_history() -> anyhow::Result<()> {
    let server = TestServer::start().await?;
    let write_client = server.enroll_read_write().await?;
    write_client
        .add_with_labels(
            "alpha",
            "seed description",
            &["rust".to_string(), "protocol".to_string()],
            "seed context",
        )
        .await?;
    let read_client = server.enroll_read().await?;

    let summaries = read_client.list().await?;
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].name, "alpha");
    assert_eq!(summaries[0].description, "seed description");
    assert_eq!(summaries[0].labels, vec!["rust", "protocol"]);

    let entry = read_client.get("alpha").await?;
    assert_eq!(entry.context, "seed context");
    assert_eq!(entry.labels, vec!["rust", "protocol"]);

    let (status, body) = read_client
        .append_response("alpha", "blocked append")
        .await?;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let error: Value = serde_json::from_str(&body)?;
    assert_eq!(error["message"].as_str(), Some("access denied"));

    let append = write_client.append("alpha", "new context line").await?;
    assert_eq!(append.name, "alpha");
    assert_eq!(append.appended_bytes, "new context line".len());

    let updated = read_client.get("alpha").await?;
    assert_eq!(updated.context, "seed context\nnew context line");

    let history = read_client.history("alpha").await?;
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].client_common_name, write_client.client_cn);
    assert_eq!(history[0].appended_content, "new context line");

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn search_delete_restore_and_nonsense_queries_work_end_to_end() -> anyhow::Result<()> {
    let server = TestServer::start().await?;
    let write_client = server.enroll_read_write().await?;
    let read_client = server.enroll_read().await?;

    write_client
        .add_with_labels(
            "atlas",
            "system architecture benchmark",
            &[
                "benchmark".to_string(),
                "protocol".to_string(),
                "design".to_string(),
            ],
            "The atlas entry contains glorbax zenthar context for tls framing and search validation.",
        )
        .await?;
    write_client
        .append("atlas", "Additional quoril vexant metadata line.")
        .await?;

    let simple_entries = read_client.search_entries("benchmark").await?;
    assert_eq!(simple_entries.len(), 1);
    assert_eq!(simple_entries[0].name, "atlas");

    let complex_entries = read_client.search_entries("benchmark protocol").await?;
    assert_eq!(complex_entries.len(), 1);
    assert_eq!(complex_entries[0].name, "atlas");

    let nonsense_entries = read_client.search_entries("voidneedleabsent").await?;
    assert!(nonsense_entries.is_empty());

    let simple_context = read_client.search_context("tls framing").await?;
    assert_eq!(simple_context.len(), 1);
    assert_eq!(simple_context[0].name, "atlas");
    assert!(
        simple_context[0]
            .snippets
            .iter()
            .any(|snippet| snippet.contains("tls framing"))
    );

    let complex_context = read_client.search_context("glorbax zenthar").await?;
    assert_eq!(complex_context.len(), 1);

    let nonsense_context = read_client.search_context("voidneedleabsent").await?;
    assert!(nonsense_context.is_empty());

    let deleted = write_client.delete("atlas").await?;
    let deleted_entries = read_client.search_deleted("atlas").await?;
    assert_eq!(deleted_entries.len(), 1);
    assert_eq!(deleted_entries[0].entry_key, deleted.entry_key);
    assert_eq!(
        deleted_entries[0].labels,
        vec!["benchmark", "protocol", "design"]
    );

    let restored = write_client.restore(&deleted.entry_key).await?;
    assert_eq!(restored.restored_entry.name, "atlas");
    assert_eq!(
        restored.restored_entry.labels,
        vec!["benchmark", "protocol", "design"]
    );

    let restored_entry = read_client.get("atlas").await?;
    assert!(
        restored_entry
            .context
            .contains("Additional quoril vexant metadata line.")
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn duplicate_chapter_names_can_exist_in_different_books() -> anyhow::Result<()> {
    let server = TestServer::start().await?;
    let write_client = server.enroll_read_write().await?;
    let read_client = server.enroll_read().await?;

    write_client
        .add_with_labels_in_location(
            "atlas",
            "volume a chapter",
            &["systems".to_string()],
            "first copy",
            Some("engineering"),
            Some("volume-a"),
        )
        .await?;
    write_client
        .add_with_labels_in_location(
            "atlas",
            "volume b chapter",
            &["systems".to_string()],
            "second copy",
            Some("engineering"),
            Some("volume-b"),
        )
        .await?;

    let summaries = read_client.list().await?;
    assert_eq!(summaries.len(), 2);
    assert!(summaries.iter().any(|entry| {
        entry.name == "atlas" && entry.shelf_name == "engineering" && entry.book_name == "volume-a"
    }));
    assert!(summaries.iter().any(|entry| {
        entry.name == "atlas" && entry.shelf_name == "engineering" && entry.book_name == "volume-b"
    }));

    let book_a = read_client
        .get_in_location("atlas", Some("engineering"), Some("volume-a"))
        .await?;
    let book_b = read_client
        .get_in_location("atlas", Some("engineering"), Some("volume-b"))
        .await?;
    assert_eq!(book_a.context, "first copy");
    assert_eq!(book_b.context, "second copy");

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shelf_and_book_search_support_descriptions_and_fuzzy_queries() -> anyhow::Result<()> {
    let server = TestServer::start().await?;
    let write_client = server.enroll_read_write().await?;
    let read_client = server.enroll_read().await?;

    write_client
        .add_with_labels_and_library_metadata_in_location(
            "atlas",
            "ops reference",
            &["systems".to_string()],
            "catalog entry",
            Some("engineering"),
            Some("volume-a"),
            Some("Platform engineering shelf"),
            Some("Incident response runbook"),
        )
        .await?;

    let entry = read_client
        .get_in_location("atlas", Some("engineering"), Some("volume-a"))
        .await?;
    assert_eq!(entry.shelf_description, "Platform engineering shelf");
    assert_eq!(entry.book_description, "Incident response runbook");

    let shelves = read_client.search_shelves("enginering").await?;
    assert_eq!(shelves.len(), 1);
    assert_eq!(shelves[0].shelf_name, "engineering");
    assert_eq!(shelves[0].description, "Platform engineering shelf");

    let books = read_client.search_books("runbok").await?;
    assert_eq!(books.len(), 1);
    assert_eq!(books[0].book_name, "volume-a");
    assert_eq!(books[0].description, "Incident response runbook");
    assert_eq!(books[0].shelf_description, "Platform engineering shelf");

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mtls_endpoint_rejects_requests_without_client_certificates() -> anyhow::Result<()> {
    let server = TestServer::start().await?;
    let result = server.unauthenticated_list_status().await?;
    assert!(result.is_err());
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "load test for many concurrent clients"]
async fn throughput_handles_many_concurrent_clients() -> anyhow::Result<()> {
    let server = TestServer::start().await?;
    let writer = server.enroll_read_write().await?;
    writer
        .add("load-entry", "throughput target", "baseline")
        .await?;

    let mut clients = Vec::new();
    for _ in 0..48 {
        clients.push(server.enroll_read().await?);
    }

    let result = run_persistent_load(clients, 20, LoadOperation::List).await?;

    println!(
        "load_test total_requests={} elapsed_ms={:.2} req_per_sec={:.2} p50_ms={:.2} p95_ms={:.2}",
        result.total_requests,
        result.elapsed.as_secs_f64() * 1000.0,
        result.requests_per_second,
        result.p50_ms,
        result.p95_ms
    );

    assert_eq!(result.total_requests, 960);
    assert!(result.requests_per_second > 0.0);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "write load test for many concurrent clients"]
async fn append_throughput_handles_many_concurrent_clients() -> anyhow::Result<()> {
    let server = TestServer::start().await?;
    let seed_writer = server.enroll_read_write().await?;
    seed_writer
        .add("append-entry", "throughput target", "baseline")
        .await?;

    let mut clients = Vec::new();
    for _ in 0..16 {
        clients.push(server.enroll_read_write().await?);
    }

    let result = run_persistent_load(
        clients,
        20,
        LoadOperation::Append {
            entry_name: "append-entry".to_string(),
            prefix: "load".to_string(),
        },
    )
    .await?;

    println!(
        "append_load_test total_requests={} elapsed_ms={:.2} req_per_sec={:.2} p50_ms={:.2} p95_ms={:.2}",
        result.total_requests,
        result.elapsed.as_secs_f64() * 1000.0,
        result.requests_per_second,
        result.p50_ms,
        result.p95_ms
    );

    assert_eq!(result.total_requests, 320);
    assert!(result.requests_per_second > 0.0);
    Ok(())
}
