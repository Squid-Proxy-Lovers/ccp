// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use crate::workspace::{store, tests as fixture};
use serde_json::{Value, json};
use std::{fs, path::PathBuf};

fn profile() -> (PathBuf, WorkspaceProfile) {
    let (ca, key) = fixture::authority();
    let trust = fixture::trust(&ca);
    let dir = fixture::directory();
    store::create(&dir, &trust).unwrap();
    let credential = fixture::material(&trust, &ca, &key, |p| {
        let elapsed = crypto::now().unwrap() - p.not_before.unix_timestamp() - 60;
        p.not_before += std::time::Duration::from_secs(u64::try_from(elapsed).unwrap());
        p.not_after = p.not_before + std::time::Duration::from_secs(30 * 24 * 60 * 60);
    });
    Connection::open(dir.join("workspace-profile.sqlite3"))
        .unwrap()
        .execute(
            "UPDATE profile SET credential=?1",
            [serde_json::to_string(&credential).unwrap()],
        )
        .unwrap();
    (dir.clone(), WorkspaceProfile::open(&dir).unwrap())
}

fn request() -> wire::EntryMutation {
    wire::EntryMutation {
        workspace_id: Uuid::new_v4().to_string(),
        operation_id: Uuid::new_v4().to_string(),
        agent: Some("agent".into()),
        change: wire::EntryChange::Append {
            entry_id: Uuid::new_v4().to_string(),
            expected_revision: 7,
            text: "PRIVATE_BODY\0é".into(),
        },
    }
}

#[test]
fn durable_pending_preserves_exact_intent_and_blocks_every_credential_transition() {
    let (dir, mut p) = profile();
    let request = request();
    let original = serde_json::to_string(&request).unwrap();
    p.prepare_content(
        request,
        input::validate(&serde_json::from_str(&original).unwrap()).unwrap(),
    )
    .unwrap();
    let raw = p.content_state.pending_record.clone();
    let credential = p.credential_record.clone();
    let info = serde_json::to_string(&p.info().unwrap()).unwrap();
    assert!(!info.contains("PRIVATE_BODY"));
    assert!(!info.contains("PRIVATE KEY"));
    assert!(p.prepare_operation(Operation::Renewal, None).is_err());
    assert!(
        p.prepare_operation(Operation::Admission, Some(&fixture::invite()))
            .is_err()
    );
    assert!(p.prepare(&fixture::invite()).is_err());
    assert!(
        p.prepare_recovery(&Uuid::new_v4().to_string(), "invalid")
            .is_err()
    );
    assert!(p.prepare_content(self::request(), "0".repeat(64)).is_err());
    drop(p);
    let mut p = WorkspaceProfile::open(&dir).unwrap();
    assert_eq!(p.content_state.pending_record, raw);
    assert_eq!(p.credential_record, credential);
    assert_eq!(
        serde_json::to_string(&p.content_state.pending.as_ref().unwrap().request).unwrap(),
        original
    );
    p.finish_content(Terminal::Cancelled).unwrap();
    assert!(p.pending_entry_operation().is_none());
    assert_eq!(p.info().unwrap().last_content.unwrap().state, "cancelled");
    assert!(
        !p.content_state
            .last_record
            .as_ref()
            .unwrap()
            .contains("PRIVATE_BODY")
    );
    drop(p);
    let p = WorkspaceProfile::open(&dir).unwrap();
    assert!(p.content_state.pending.is_none());
    assert_eq!(p.info().unwrap().last_content.unwrap().state, "cancelled");
    drop(p);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn prepare_and_finish_failures_leave_memory_and_original_intent_unchanged() {
    let (dir, mut p) = profile();
    let request = request();
    let hash = input::validate(&request).unwrap();
    assert!(
        p.prepare_content(
            serde_json::from_value(serde_json::to_value(&request).unwrap()).unwrap(),
            "0".repeat(64)
        )
        .is_err()
    );
    p.connection.execute_batch("CREATE TRIGGER fail_content BEFORE UPDATE ON content_state BEGIN SELECT RAISE(ABORT,'PRIVATE_BODY'); END;").unwrap();
    assert!(
        !p.prepare_content(
            serde_json::from_value(serde_json::to_value(&request).unwrap()).unwrap(),
            hash.clone()
        )
        .unwrap_err()
        .to_string()
        .contains("PRIVATE_BODY")
    );
    assert!(p.content_state.pending.is_none());
    p.connection
        .execute_batch("DROP TRIGGER fail_content")
        .unwrap();
    p.prepare_content(request, hash).unwrap();
    let raw = p.content_state.pending_record.clone();
    p.connection.execute_batch("CREATE TRIGGER fail_finish BEFORE UPDATE ON content_state BEGIN SELECT RAISE(ABORT,'PRIVATE_BODY'); END;").unwrap();
    assert!(
        !p.finish_content(Terminal::Cancelled)
            .unwrap_err()
            .to_string()
            .contains("PRIVATE_BODY")
    );
    assert_eq!(p.content_state.pending_record, raw);
    assert!(p.content_state.last.is_none());
    p.connection
        .execute_batch("DROP TRIGGER fail_finish; UPDATE content_state SET last='{}'")
        .unwrap();
    assert!(p.finish_content(Terminal::Cancelled).is_err());
    assert_eq!(p.content_state.pending_record, raw);
    drop(p);
    assert!(WorkspaceProfile::open(&dir).is_err());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn malformed_saved_bindings_payloads_and_oversize_records_fail_closed_without_echo() {
    let (dir, mut p) = profile();
    let request = request();
    let hash = input::validate(&request).unwrap();
    p.prepare_content(request, hash).unwrap();
    let raw = p.content_state.pending_record.clone().unwrap();
    drop(p);
    for (path, value) in [
        ("/binding/client_id", json!(Uuid::new_v4().to_string())),
        ("/binding/credential_id", json!(Uuid::new_v4().to_string())),
        ("/binding/instance_id", json!(Uuid::new_v4().to_string())),
        ("/binding/workspace_id", json!(Uuid::new_v4().to_string())),
        ("/binding/operation_id", json!(Uuid::new_v4().to_string())),
        ("/binding/request_hash", json!("f".repeat(64))),
        ("/request/change/text", json!("PRIVATE_BODY_CHANGED")),
    ] {
        let mut changed: Value = serde_json::from_str(&raw).unwrap();
        *changed.pointer_mut(path).unwrap() = value;
        let db = Connection::open(dir.join("workspace-profile.sqlite3")).unwrap();
        db.execute("UPDATE content_state SET pending=?1", [changed.to_string()])
            .unwrap();
        drop(db);
        let error = WorkspaceProfile::open(&dir).err().unwrap().to_string();
        assert!(!error.contains("PRIVATE_BODY"));
    }
    let db = Connection::open(dir.join("workspace-profile.sqlite3")).unwrap();
    assert!(
        db.execute(
            "UPDATE content_state SET pending=?1",
            ["x".repeat(MAX_PENDING + 1)]
        )
        .is_err()
    );
    assert!(
        db.execute("UPDATE content_state SET last=?1", ["x".repeat(4097)])
            .is_err()
    );
    drop(db);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn input_and_receipts_bind_exact_targets_revisions_bounds_and_strict_json() {
    let original = serde_json::to_value(request()).unwrap();
    for (path, value) in [
        ("/workspace_id", json!("invalid")),
        ("/operation_id", json!(Uuid::nil().to_string())),
        ("/agent", json!("bad\nagent")),
        ("/change/expected_revision", json!(0)),
        ("/change/expected_revision", json!(i64::MAX)),
        ("/change/text", json!("")),
        ("/change/text", json!("x".repeat(256 * 1024 + 1))),
    ] {
        let mut v = original.clone();
        *v.pointer_mut(path).unwrap() = value;
        let request = serde_json::from_value(v).unwrap();
        assert!(input::validate(&request).is_err());
    }
    let mut unknown = original.clone();
    unknown["credential_id"] = json!(Uuid::new_v4().to_string());
    assert!(serde_json::from_value::<wire::EntryMutation>(unknown).is_err());
    let request: wire::EntryMutation = serde_json::from_value(original).unwrap();
    let wire::EntryChange::Append { entry_id, .. } = &request.change else {
        panic!()
    };
    input::receipt(
        &request,
        &wire::EntryReceipt {
            entry_id: entry_id.clone(),
            revision: 8,
        },
    )
    .unwrap();
    for (id, revision) in [
        (entry_id.clone(), 7),
        (Uuid::new_v4().to_string(), 8),
        (entry_id.clone(), 0),
    ] {
        assert!(
            input::receipt(
                &request,
                &wire::EntryReceipt {
                    entry_id: id,
                    revision
                }
            )
            .is_err()
        );
    }
}

#[test]
fn create_revise_and_lifecycle_validators_cover_content_and_encoding_edges() {
    let base = json!({"workspace_id":Uuid::new_v4().to_string(),"operation_id":Uuid::new_v4().to_string(),"agent":null,"change":{"action":"create","book_id":Uuid::new_v4().to_string(),"name":"entry","content":{"text":"", "description":"", "labels":[]}}});
    let valid: wire::EntryMutation = serde_json::from_value(base.clone()).unwrap();
    input::validate(&valid).unwrap();
    for (path, value) in [
        ("/change/name", json!(" \n ")),
        ("/change/name", json!("é".repeat(129))),
        ("/change/content/text", json!("x".repeat(256 * 1024 + 1))),
        ("/change/content/description", json!("é".repeat(2049))),
        ("/change/content/labels", json!(["duplicate", "duplicate"])),
        ("/change/content/labels", json!(["bad\nlabel"])),
        ("/change/content/labels", json!(["z".repeat(129)])),
        (
            "/change/content/labels",
            json!((0..33).map(|n| n.to_string()).collect::<Vec<_>>()),
        ),
    ] {
        let mut changed = base.clone();
        *changed.pointer_mut(path).unwrap() = value;
        assert!(input::validate(&serde_json::from_value(changed).unwrap()).is_err());
    }
    for action in ["revise", "archive", "restore"] {
        let mut value = base.clone();
        value["change"] =
            json!({"action":action,"entry_id":Uuid::new_v4().to_string(),"expected_revision":1});
        if action == "revise" {
            value["change"]["content"] = base["change"]["content"].clone();
        }
        input::validate(&serde_json::from_value(value.clone()).unwrap()).unwrap();
        value["change"]["expected_revision"] = json!(u64::MAX);
        assert!(input::validate(&serde_json::from_value(value).unwrap()).is_err());
    }
}

#[test]
fn maximum_escaped_payload_and_terminal_receipt_consistency_are_preserved() {
    let (dir, mut p) = profile();
    let mut request = request();
    if let wire::EntryChange::Append { text, .. } = &mut request.change {
        *text = "\0".repeat(256 * 1024);
    }
    let hash = input::validate(&request).unwrap();
    let target = match &request.change {
        wire::EntryChange::Append { entry_id, .. } => entry_id.clone(),
        _ => panic!(),
    };
    p.prepare_content(request, hash).unwrap();
    let binding = p.content_binding().unwrap();
    let result = |revision| wire::EntryOperationResolution {
        workspace_id: binding.workspace_id.clone(),
        operation_id: binding.operation_id.clone(),
        credential_id: binding.credential_id.clone(),
        request_hash: binding.request_hash.clone(),
        outcome: wire::EntryOperationOutcome::Committed {
            entry_id: target.clone(),
            revision,
        },
    };
    assert!(p.accept_resolution(binding.clone(), Ok(result(7))).is_err());
    assert!(p.pending_entry_operation().is_some());
    p.accept_resolution(binding.clone(), Ok(result(8))).unwrap();
    assert!(p.pending_entry_operation().is_none());
    assert!(p.accept_resolution(binding.clone(), Ok(result(9))).is_err());
    p.accept_resolution(binding.clone(), Ok(result(8))).unwrap();
    drop(p);
    let p = WorkspaceProfile::open(&dir).unwrap();
    assert_eq!(p.info().unwrap().last_content.unwrap().state, "committed");
    drop(p);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn expected_binding_checks_pending_and_terminal_without_changing_either_record() {
    let (dir, mut p) = profile();
    let request = request();
    let hash = input::validate(&request).unwrap();
    p.prepare_content(request, hash).unwrap();
    let target = p.content_binding().unwrap().query();
    let raw = p.content_state.pending_record.clone();
    p.require_expected_operation(&target).unwrap();
    let original = serde_json::to_value(&target).unwrap();
    for field in [
        "workspace_id",
        "operation_id",
        "credential_id",
        "request_hash",
    ] {
        let mut changed = original.clone();
        changed[field] = json!(if field == "request_hash" {
            "f".repeat(64)
        } else {
            Uuid::new_v4().to_string()
        });
        let expected = serde_json::from_value(changed).unwrap();
        assert!(p.require_expected_operation(&expected).is_err());
        assert_eq!(p.content_state.pending_record, raw);
        assert!(p.content_state.last_record.is_none());
    }
    p.finish_content(Terminal::Cancelled).unwrap();
    let last = p.content_state.last_record.clone();
    p.require_expected_operation(&target).unwrap();
    assert_eq!(p.content_state.last_record, last);
    assert!(p.content_state.pending.is_none());
    drop(p);
    fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn via_profile_rejects_other_instances_pins_and_client_identities_before_network_use() {
    let (dir, mut original) = profile();
    let request = request();
    let hash = input::validate(&request).unwrap();
    original.prepare_content(request, hash).unwrap();
    let raw = original.content_state.pending_record.clone();
    let (other_dir, mut other) = profile();
    assert!(
        original
            .resolve_entry_operation_via(&mut other)
            .await
            .is_err()
    );
    other.trust.instance_id = original.trust.instance_id.clone();
    assert!(
        original
            .resolve_entry_operation_via(&mut other)
            .await
            .is_err()
    );
    other.trust.ca_pem = original.trust.ca_pem.clone();
    assert!(
        original
            .resolve_entry_operation_via(&mut other)
            .await
            .is_err()
    );
    assert_eq!(original.content_state.pending_record, raw);
    assert!(original.pending_entry_operation().is_some());
    assert!(other.content_state.pending.is_none());
    drop(original);
    drop(other);
    fs::remove_dir_all(dir).unwrap();
    fs::remove_dir_all(other_dir).unwrap();
}
