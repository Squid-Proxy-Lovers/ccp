// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

#[test]
fn exact_status_input_bounds_keep_server_unicode_semantics_and_guard_all_identifiers() {
    let original = request();
    assert!(input::mutation(&original).is_ok());
    let mut v = serde_json::to_value(&original).unwrap();
    for field in ["workspace_id", "status_id", "operation_id"] {
        let saved = v[field].clone();
        v[field] = json!(Uuid::nil().to_string());
        assert!(input::mutation(&serde_json::from_value(v.clone()).unwrap()).is_err());
        v[field] = saved;
    }
    for revision in [i64::MAX.unsigned_abs(), u64::MAX] {
        v["expected_revision"] = json!(revision);
        assert!(input::mutation(&serde_json::from_value(v.clone()).unwrap()).is_err());
    }
    v["expected_revision"] = json!(0);
    for text in ["".into(), " \n".into(), "\0".into(), "é".repeat(2049)] {
        v["change"]["message"] = json!(text);
        assert!(input::mutation(&serde_json::from_value(v.clone()).unwrap()).is_err());
    }
    for text in ["\u{feff}".into(), "é".repeat(2048)] {
        v["change"]["message"] = json!(text);
        assert!(input::mutation(&serde_json::from_value(v.clone()).unwrap()).is_ok());
    }
    for label in ["".into(), " \n".into(), "\0".into(), "é".repeat(65)] {
        v["change"]["agent"] = json!(label);
        assert!(input::mutation(&serde_json::from_value(v.clone()).unwrap()).is_err());
    }
    v["change"]["agent"] = json!("é".repeat(64));
    assert!(input::mutation(&serde_json::from_value(v).unwrap()).is_ok());
}

#[test]
fn status_reader_preserves_tombstones_and_rejects_foreign_cursor_age_and_size_without_unicode_trimming()
 {
    let ws = Uuid::new_v4().to_string();
    let id = Uuid::from_u128(1).to_string();
    let request = wire::List {
        workspace_id: ws.clone(),
        after: None,
        limit: Some(32),
    };
    let raw = json!({"workspace_id":ws,"status_id":id,"client_id":Uuid::new_v4().to_string(),"client_name":"registered","client_disabled":false,"revision":1,"agent":"\u{feff}","message":"\u{feff}","updated_at":1000,"stale":false});
    let page = |row: Value| {
        serde_json::from_value::<wire::Page>(
            json!({"items":[row],"next_cursor":id,"observed_at":1001,"stale_after_seconds":300}),
        )
        .unwrap()
    };
    assert!(reads::page(&page(raw.clone()), &request).is_ok());
    for (field, value) in [
        ("workspace_id", json!(Uuid::new_v4().to_string())),
        ("revision", json!(0)),
        ("message", json!("é".repeat(2049))),
        ("agent", json!("é".repeat(65))),
        ("stale", json!(true)),
        ("message", Value::Null),
    ] {
        let mut row = raw.clone();
        row[field] = value;
        assert!(reads::page(&page(row), &request).is_err());
    }
    let mut value = page(raw.clone());
    value.next_cursor = Some(Uuid::from_u128(2).to_string());
    assert!(reads::page(&value, &request).is_err());
    let mut value = page(raw.clone());
    value.observed_at = 1300;
    value.items[0].stale = true;
    assert!(reads::page(&value, &request).is_ok());
    let mut tombstone: wire::Status = serde_json::from_value(raw).unwrap();
    tombstone.message = None;
    tombstone.agent = None;
    tombstone.stale = true;
    assert!(reads::status(&tombstone, &ws, true).is_ok());
    assert!(reads::status(&tombstone, &ws, false).is_err());
}
