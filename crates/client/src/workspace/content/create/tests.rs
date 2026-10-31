// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use serde_json::{Value, json};

#[test]
fn creation_input_matches_core_bounds_and_refuses_claims_before_network_use() {
    let base = json!({"workspace_id":Uuid::new_v4().to_string(),"name":"  notes  ","description":"exact\nmetadata\0é"});
    let valid: CreateShelf = serde_json::from_value(base.clone()).unwrap();
    input(&valid.workspace_id, &valid.name, &valid.description, &valid).unwrap();
    for (field, value) in [
        ("workspace_id", json!("invalid")),
        ("name", json!(" \n ")),
        ("name", json!("bad\nname")),
        ("name", json!("é".repeat(129))),
        ("description", json!("é".repeat(2049))),
        ("name", json!(format!("{}n", " ".repeat(MAX_REQUEST)))),
    ] {
        let mut changed = base.clone();
        changed[field] = value;
        let request: CreateShelf = serde_json::from_value(changed).unwrap();
        let error = input(
            &request.workspace_id,
            &request.name,
            &request.description,
            &request,
        )
        .unwrap_err()
        .to_string();
        assert!(!error.contains("metadata"));
    }
    for claim in ["credential_id", "client_id", "operation_id", "admin_key"] {
        let mut changed = base.clone();
        changed[claim] = json!("PRIVATE_VALUE");
        assert!(serde_json::from_value::<CreateShelf>(changed).is_err());
    }
    let mut book = base;
    book["shelf_id"] = json!(Uuid::new_v4().to_string());
    assert!(serde_json::from_value::<CreateBook>(book.clone()).is_ok());
    book["description"] = Value::Null;
    assert!(serde_json::from_value::<CreateBook>(book).is_err());
}

#[test]
fn creation_result_checks_exact_normalized_name_description_scope_and_parent() {
    result("notes", "exact\0é", "  notes  ", "exact\0é").unwrap();
    for (name, description) in [
        (" notes ", "exact\0é"),
        ("notes", "changed"),
        ("other", "exact\0é"),
    ] {
        assert!(result(name, description, "notes", "exact\0é").is_err());
    }
    let workspace = Uuid::new_v4().to_string();
    let parent = Uuid::new_v4().to_string();
    let base = json!({"id":Uuid::new_v4().to_string(),"workspace_id":workspace,"shelf_id":parent,"name":"notes","description":"exact","created_at":1});
    let valid: Book = serde_json::from_value(base.clone()).unwrap();
    validate::book(&valid, &workspace, Some(&parent)).unwrap();
    for (field, value) in [
        ("id", json!("invalid")),
        ("workspace_id", json!(Uuid::new_v4().to_string())),
        ("shelf_id", json!(Uuid::new_v4().to_string())),
        ("name", json!(" untrimmed ")),
        ("description", json!("x".repeat(4097))),
        ("created_at", json!(-1)),
    ] {
        let mut changed = base.clone();
        changed[field] = value;
        let value = serde_json::from_value(changed).unwrap();
        assert!(validate::book(&value, &workspace, Some(&parent)).is_err());
    }
}
