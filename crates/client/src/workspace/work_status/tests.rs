// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use crate::workspace::{store, tests as fixture};
use serde_json::{Value, json};
use std::{fs, path::PathBuf};
mod migration;
mod validation;

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
fn request() -> wire::Mutation {
    wire::Mutation {
        workspace_id: Uuid::new_v4().to_string(),
        status_id: Uuid::new_v4().to_string(),
        operation_id: Uuid::new_v4().to_string(),
        expected_revision: 7,
        change: wire::Change::Set {
            agent: Some("\u{feff}".into()),
            message: "PRIVATE_WORK é\u{feff}".into(),
        },
    }
}
fn copied(value: &wire::Mutation) -> wire::Mutation {
    serde_json::from_value(serde_json::to_value(value).unwrap()).unwrap()
}
fn prepare(p: &mut WorkspaceProfile, value: wire::Mutation) {
    let hash = input::mutation(&value).unwrap();
    p.prepare_status(value, hash).unwrap();
}
fn target(p: &WorkspaceProfile) -> WorkStatusTarget {
    p.status_state
        .pending
        .as_ref()
        .unwrap()
        .binding
        .target
        .clone()
}
fn receipt(p: &WorkspaceProfile) -> wire::Receipt {
    let b = &p.status_state.pending.as_ref().unwrap().binding;
    wire::Receipt {
        workspace_id: b.target.workspace_id.clone(),
        status_id: b.target.status_id.clone(),
        operation_id: b.target.operation_id.clone(),
        client_id: b.client_id.clone(),
        revision: b.expected_revision + 1,
        cleared: b.cleared,
        updated_at: 1000,
    }
}

#[test]
fn status_pending_preserves_exact_unicode_and_blocks_lifecycle_entry_and_credential_sql_changes() {
    let (dir, mut p) = profile();
    let value = request();
    let json = serde_json::to_string(&value).unwrap();
    prepare(&mut p, value);
    let raw = p.status_state.pending_record.clone();
    let original = target(&p);
    let credential = p.credential_record.clone();
    let info = serde_json::to_string(&p.info().unwrap()).unwrap();
    assert!(!info.contains("PRIVATE_WORK"));
    assert!(!info.contains("PRIVATE KEY"));
    assert!(p.require_no_content_pending().is_err());
    assert!(p.prepare_operation(Operation::Renewal, None).is_err());
    assert!(
        p.prepare_operation(Operation::Admission, Some(&fixture::invite()))
            .is_err()
    );
    assert!(
        p.prepare_recovery(&Uuid::new_v4().to_string(), "invalid")
            .is_err()
    );
    assert!(
        p.connection
            .execute("UPDATE profile SET credential=credential", [])
            .is_err()
    );
    assert!(
        p.connection
            .execute("UPDATE content_state SET pending='other'", [])
            .is_err()
    );
    drop(p);
    let mut p = WorkspaceProfile::open(&dir).unwrap();
    assert_eq!(target(&p), original);
    assert_eq!(p.status_state.pending_record, raw);
    assert_eq!(p.credential_record, credential);
    assert_eq!(
        serde_json::to_string(&p.status_state.pending.as_ref().unwrap().request).unwrap(),
        json
    );
    assert!(p.forget_status_operation(&original, false).is_err());
    p.forget_status_operation(&original, true).unwrap();
    assert!(p.status_state.pending.is_none());
    assert_eq!(
        p.info().unwrap().last_status.unwrap().state,
        "locally_forgotten"
    );
    assert!(
        !p.status_state
            .last_record
            .as_ref()
            .unwrap()
            .contains("PRIVATE_WORK")
    );
    assert!(p.require_no_content_pending().is_ok());
    drop(p);
    let p = WorkspaceProfile::open(&dir).unwrap();
    assert!(p.status_state.pending.is_none());
    assert_eq!(
        p.info().unwrap().last_status.unwrap().state,
        "locally_forgotten"
    );
    drop(p);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn status_expected_target_and_receipt_checks_never_replace_or_discard_the_original() {
    let (dir, mut p) = profile();
    prepare(&mut p, request());
    let original = target(&p);
    let raw = p.status_state.pending_record.clone();
    for field in [
        "workspace_id",
        "status_id",
        "operation_id",
        "credential_id",
        "request_hash",
    ] {
        let mut value = serde_json::to_value(&original).unwrap();
        value[field] = json!(if field == "request_hash" {
            "a".repeat(64)
        } else {
            Uuid::new_v4().to_string()
        });
        let altered: WorkStatusTarget = serde_json::from_value(value).unwrap();
        assert!(p.forget_status_operation(&altered, true).is_err());
        assert_eq!(p.status_state.pending_record, raw);
    }
    let good = receipt(&p);
    for field in [
        "workspace_id",
        "status_id",
        "operation_id",
        "client_id",
        "revision",
        "cleared",
        "updated_at",
    ] {
        let mut value = serde_json::to_value(&good).unwrap();
        value[field] = match field {
            "revision" => json!(99),
            "cleared" => json!(true),
            "updated_at" => json!(-1),
            _ => json!(Uuid::new_v4().to_string()),
        };
        let bad: wire::Receipt = serde_json::from_value(value).unwrap();
        assert!(p.finish_status(Some(bad)).is_err());
        assert_eq!(p.status_state.pending_record, raw);
    }
    p.finish_status(Some(good.clone())).unwrap();
    assert!(p.status_state.pending.is_none());
    assert_eq!(
        p.info().unwrap().last_status.unwrap().receipt,
        Some(good.clone())
    );
    assert!(
        !p.status_state
            .last_record
            .as_ref()
            .unwrap()
            .contains("PRIVATE_WORK")
    );
    drop(p);
    let p = WorkspaceProfile::open(&dir).unwrap();
    assert_eq!(p.info().unwrap().last_status.unwrap().receipt, Some(good));
    drop(p);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn ignored_and_aborted_status_persistence_never_advance_memory_or_erase_original_work() {
    for fault in ["IGNORE", "ABORT,'PRIVATE_WORK'"] {
        let (dir, mut p) = profile();
        let value = request();
        let hash = input::mutation(&value).unwrap();
        p.connection.execute_batch(&format!("CREATE TRIGGER status_test_fault BEFORE UPDATE ON status_state BEGIN SELECT RAISE({fault}); END;")).unwrap();
        assert!(
            !p.prepare_status(copied(&value), hash.clone())
                .unwrap_err()
                .to_string()
                .contains("PRIVATE_WORK")
        );
        assert!(p.status_state.pending.is_none());
        p.connection
            .execute_batch("DROP TRIGGER status_test_fault")
            .unwrap();
        prepare(&mut p, value);
        let raw = p.status_state.pending_record.clone();
        let good = receipt(&p);
        p.connection.execute_batch(&format!("CREATE TRIGGER status_test_fault BEFORE UPDATE ON status_state BEGIN SELECT RAISE({fault}); END;")).unwrap();
        assert!(p.finish_status(Some(good.clone())).is_err());
        assert_eq!(p.status_state.pending_record, raw);
        assert!(p.status_state.last.is_none());
        p.connection
            .execute_batch("DROP TRIGGER status_test_fault")
            .unwrap();
        p.finish_status(Some(good)).unwrap();
        drop(p);
        fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn corrupted_status_binding_payload_and_simultaneous_intents_refuse_open_without_fallback() {
    for field in [
        "workspace_id",
        "status_id",
        "operation_id",
        "credential_id",
        "request_hash",
    ] {
        let (dir, mut p) = profile();
        prepare(&mut p, request());
        let mut value: Value =
            serde_json::from_str(p.status_state.pending_record.as_ref().unwrap()).unwrap();
        value["binding"]["target"][field] = json!(if field == "request_hash" {
            "0".repeat(64)
        } else {
            Uuid::new_v4().to_string()
        });
        p.connection
            .execute("UPDATE status_state SET pending=?1", [value.to_string()])
            .unwrap();
        drop(p);
        assert!(WorkspaceProfile::open(&dir).is_err());
        fs::remove_dir_all(dir).unwrap();
    }
    let (dir, mut p) = profile();
    prepare(&mut p, request());
    let mut value: Value =
        serde_json::from_str(p.status_state.pending_record.as_ref().unwrap()).unwrap();
    value["request"]["change"]["message"] = json!("altered private request");
    p.connection
        .execute("UPDATE status_state SET pending=?1", [value.to_string()])
        .unwrap();
    drop(p);
    assert!(WorkspaceProfile::open(&dir).is_err());
    fs::remove_dir_all(dir).unwrap();
}
