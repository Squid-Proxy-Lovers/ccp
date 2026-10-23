// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;
use serde_json::{Value, json};

fn fixture() -> (Value, HistoryPage) {
    let workspace = Uuid::new_v4().to_string();
    let entry = Uuid::new_v4().to_string();
    (
        json!({"workspace_id":workspace,"entry_id":entry,"source":{"database_sha256":"a".repeat(64),"schema":2,"converted_at":1},"items":[{"ordinal":1,"source_table":"message_packs","source_key":"-42","row":{"columns":["id","text","minimum","negative_zero","invalid_text","blob","null"],"values":[{"type":"integer","value":"-42"},{"type":"text","value":"PRIVATE_SOURCE\n\u{0000}"},{"type":"integer","value":"-9223372036854775808"},{"type":"real","value":"8000000000000000"},{"type":"text_bytes","value":[0,255]},{"type":"blob","value":[0,255]},{"type":"null"}]}}],"next_cursor":1}),
        HistoryPage {
            workspace_id: workspace,
            entry_id: entry,
            after: None,
            limit: Some(32),
        },
    )
}

#[test]
fn raw_values_roundtrip_without_rounding_coercion_or_full_count_cursor_assumptions() {
    let (value, request) = fixture();
    let parsed: NativeHistory = serde_json::from_value(value.clone()).unwrap();
    check(&parsed, &request).unwrap();
    assert_eq!(serde_json::to_value(parsed).unwrap(), value);
    let mut last = value;
    last["next_cursor"] = Value::Null;
    check(&serde_json::from_value(last.clone()).unwrap(), &request).unwrap();
    last["items"] = json!([]);
    check(&serde_json::from_value(last.clone()).unwrap(), &request).unwrap();
    last["source"] = Value::Null;
    check(&serde_json::from_value(last).unwrap(), &request).unwrap();
    for n in [i64::MIN, -1, 0, 1, i64::MAX] {
        assert!(integer(&n.to_string()));
    }
    for n in ["+1", "01", "-0", "9223372036854775808", ""] {
        assert!(!integer(n));
    }
}

#[test]
fn foreign_targets_malformed_rows_and_inconsistent_cursors_fail_without_payload_echo() {
    let (value, request) = fixture();
    for (pointer, bad) in [
        ("/workspace_id", json!(Uuid::new_v4().to_string())),
        ("/entry_id", json!(Uuid::new_v4().to_string())),
        ("/source/database_sha256", json!("A".repeat(64))),
        ("/source/converted_at", json!(-1)),
        ("/source", Value::Null),
        ("/items/0/ordinal", json!(0)),
        ("/items/0/ordinal", json!(u64::MAX)),
        ("/items/0/source_table", json!("auth_tokens")),
        ("/items/0/source_key", json!("0042")),
        ("/items/0/row/columns", json!(["id", "id"])),
        ("/items/0/row/columns", json!([])),
        ("/items/0/row/columns", json!(["id"])),
        ("/items/0/row/values/0/value", json!("01")),
        ("/items/0/row/values/3/value", json!("ABC0000000000000")),
        ("/next_cursor", json!(2)),
    ] {
        let mut bad_value = value.clone();
        *bad_value.pointer_mut(pointer).unwrap() = bad;
        let parsed: NativeHistory = serde_json::from_value(bad_value).unwrap();
        let error = check(&parsed, &request).unwrap_err().to_string();
        assert!(!error.contains("PRIVATE_SOURCE"));
    }
    let mut duplicate = value.clone();
    duplicate["items"] = json!([value["items"][0], value["items"][0]]);
    assert!(check(&serde_json::from_value(duplicate).unwrap(), &request).is_err());
    let mut after = request;
    after.after = Some(1);
    assert!(check(&serde_json::from_value(value).unwrap(), &after).is_err());
}

#[test]
fn typed_shapes_page_and_row_budgets_and_archive_keys_are_enforced() {
    let (mut value, mut request) = fixture();
    let original = value.clone();
    value["items"][0]["row"]["values"][0]["value"] = json!(42);
    assert!(serde_json::from_value::<NativeHistory>(value).is_err());
    let mut value = original.clone();
    value["source"]["recorded_by"] = json!("unexpected");
    assert!(serde_json::from_value::<NativeHistory>(value).is_err());
    let mut value = original.clone();
    value["items"][0]["row"]["values"][1]["value"] = json!("x".repeat(2 * 1024 * 1024));
    assert!(check(&serde_json::from_value(value).unwrap(), &request).is_err());
    let mut value = original.clone();
    value["items"][0]["source_table"] = json!("deleted_message_packs");
    for key in ["", &"x".repeat(8193)] {
        value["items"][0]["source_key"] = json!(key);
        assert!(check(&serde_json::from_value(value.clone()).unwrap(), &request).is_err());
    }
    value["items"][0]["source_key"] = json!("archive\u{0000}key");
    check(&serde_json::from_value(value.clone()).unwrap(), &request).unwrap();
    request.limit = Some(0);
    assert!(check(&serde_json::from_value(value).unwrap(), &request).is_err());
}
