// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use serde_json::{Value, json};

fn snapshot_value() -> Value {
    json!({"entry":{"id":Uuid::new_v4().to_string(),"workspace_id":Uuid::new_v4().to_string(),"book_id":Uuid::new_v4().to_string(),"owner_client_id":Uuid::new_v4().to_string(),"name":"entry","revision":2,"archived":false,"created_at":1,"updated_at":2},"revision":{"revision":1,"action":"create","actor":{"kind":"credential","id":Uuid::new_v4().to_string()},"client_id":Uuid::new_v4().to_string(),"admin_principal_id":null,"agent":"agent","recorded_at":1},"content":{"text":"PRIVATE_BODY","description":"exact\nmetadata","labels":["one"]},"appended_text":null})
}

fn target(value: &Value) -> EntryTarget {
    EntryTarget {
        workspace_id: value["entry"]["workspace_id"].as_str().unwrap().into(),
        entry_id: value["entry"]["id"].as_str().unwrap().into(),
        revision: Some(1),
    }
}

#[test]
fn snapshot_target_archive_revision_metadata_and_field_bounds_fail_without_content_echo() {
    let original = snapshot_value();
    let request = target(&original);
    let parsed: EntrySnapshot = serde_json::from_value(original.clone()).unwrap();
    validate::snapshot(&parsed, &request).unwrap();
    for (pointer, value) in [
        ("/entry/workspace_id", json!(Uuid::new_v4().to_string())),
        ("/entry/id", json!(Uuid::new_v4().to_string())),
        ("/entry/book_id", json!("invalid")),
        ("/entry/archived", json!(true)),
        ("/entry/revision", json!(0)),
        ("/entry/created_at", json!(-1)),
        ("/entry/owner_client_id", json!("invalid")),
        ("/revision/revision", json!(2)),
        ("/revision/action", json!("purge")),
        ("/revision/actor/id", json!("invalid")),
        ("/revision/client_id", Value::Null),
        (
            "/revision/admin_principal_id",
            json!(Uuid::new_v4().to_string()),
        ),
        ("/revision/agent", json!("bad\nagent")),
        ("/content/text", json!("x".repeat(256 * 1024 + 1))),
        ("/content/description", json!("é".repeat(2049))),
        ("/content/labels", json!(["duplicate", "duplicate"])),
        ("/content/labels", json!(["bad\nlabel"])),
        ("/appended_text", json!("unexpected")),
    ] {
        let mut changed = original.clone();
        *changed.pointer_mut(pointer).unwrap() = value;
        let parsed: EntrySnapshot = serde_json::from_value(changed).unwrap();
        let error = validate::snapshot(&parsed, &request)
            .unwrap_err()
            .to_string();
        assert!(!error.contains("PRIVATE_BODY"));
    }
    let mut request = request;
    request.revision = None;
    assert!(validate::snapshot(&parsed, &request).is_err());
}

#[test]
fn snapshots_validate_retained_append_contributions_and_admin_attribution() {
    let mut value = snapshot_value();
    let request = target(&value);
    value["revision"]["action"] = json!("append");
    value["appended_text"] = json!("BODY");
    let parsed: EntrySnapshot = serde_json::from_value(value.clone()).unwrap();
    validate::snapshot(&parsed, &request).unwrap();
    for delta in [Value::Null, json!(""), json!("not a suffix")] {
        value["appended_text"] = delta;
        let parsed: EntrySnapshot = serde_json::from_value(value.clone()).unwrap();
        assert!(validate::snapshot(&parsed, &request).is_err());
    }
    value["appended_text"] = json!("BODY");
    value["revision"]["client_id"] = Value::Null;
    let principal = Uuid::new_v4().to_string();
    for actor in [
        json!({"kind":"admin","id":principal}),
        json!({"kind":"admin_key","id":Uuid::new_v4().to_string()}),
    ] {
        value["revision"]["actor"] = actor;
        value["revision"]["admin_principal_id"] = json!(principal);
        let parsed: EntrySnapshot = serde_json::from_value(value.clone()).unwrap();
        validate::snapshot(&parsed, &request).unwrap();
        value["revision"]["admin_principal_id"] = Value::Null;
        let parsed: EntrySnapshot = serde_json::from_value(value.clone()).unwrap();
        assert!(validate::snapshot(&parsed, &request).is_err());
    }
}

#[test]
fn metadata_pages_reject_foreign_parents_reordering_duplicates_and_inconsistent_cursors() {
    let workspace = Uuid::new_v4().to_string();
    let shelf = Uuid::new_v4().to_string();
    let book = Book {
        id: Uuid::new_v4().to_string(),
        workspace_id: workspace.clone(),
        shelf_id: shelf.clone(),
        name: "book".into(),
        description: String::new(),
        created_at: 0,
    };
    validate::book(&book, &workspace, Some(&shelf)).unwrap();
    assert!(validate::book(&book, &workspace, Some(&Uuid::new_v4().to_string())).is_err());
    assert!(validate::book(&book, &Uuid::new_v4().to_string(), None).is_err());
    let ids = [
        "00000000-0000-0000-0000-000000000001",
        "00000000-0000-0000-0000-000000000002",
    ];
    let mut page = Page {
        items: ids
            .iter()
            .map(|id| Shelf {
                id: (*id).into(),
                workspace_id: workspace.clone(),
                name: "shelf".into(),
                description: String::new(),
                created_at: 0,
            })
            .collect(),
        next_cursor: Some(ids[1].into()),
    };
    let check = |page: &Page<Shelf>, after: Option<&str>, limit| {
        validate::page(
            page,
            after,
            limit,
            |v| &v.id,
            |v| validate::shelf(v, &workspace),
        )
    };
    check(&page, None, Some(2)).unwrap();
    assert!(check(&page, None, Some(1)).is_err());
    assert!(check(&page, Some(ids[0]), Some(2)).is_err());
    page.next_cursor = Some(ids[0].into());
    assert!(check(&page, None, Some(2)).is_err());
    page.next_cursor = None;
    page.items.reverse();
    assert!(check(&page, None, Some(2)).is_err());
    page.items.reverse();
    page.items[1].id = ids[0].into();
    assert!(check(&page, None, Some(2)).is_err());
    assert!(validate::page_input(None, Some(0)).is_err());
    assert!(validate::page_input(None, Some(33)).is_err());
    assert!(validate::page_input(Some("invalid"), None).is_err());
    assert!(validate::revision_number(0).is_err());
    assert!(validate::revision_number(u64::MAX).is_err());
}

#[test]
fn history_metadata_is_bounded_ordered_and_has_no_invented_cursor() {
    let value = snapshot_value();
    let request = target(&value);
    let mut history: History =
        serde_json::from_value(json!({"items":[value["revision"]],"next_cursor":1})).unwrap();
    let request = HistoryPage {
        workspace_id: request.workspace_id,
        entry_id: request.entry_id,
        after: None,
        limit: Some(1),
    };
    validate::history(&history, &request).unwrap();
    history.next_cursor = Some(2);
    assert!(validate::history(&history, &request).is_err());
    history.next_cursor = None;
    history.items[0].revision = 0;
    assert!(validate::history(&history, &request).is_err());
    history.items[0].revision = 1;
    history.items[0].recorded_at = -1;
    assert!(validate::history(&history, &request).is_err());
}

#[test]
fn shelf_state_rejects_wrong_targets_and_revisions_and_drops_unknown_response_fields() {
    let request = NodeTarget {
        workspace_id: Uuid::new_v4().to_string(),
        node_id: Uuid::new_v4().to_string(),
    };
    let original = json!({"workspace_id":request.workspace_id,"shelf_id":request.node_id,"revision":2,"archived":true,"content":"PRIVATE_BODY"});
    let parsed: ShelfState = serde_json::from_value(original.clone()).unwrap();
    validate::shelf_state(&parsed, &request).unwrap();
    assert_eq!(
        serde_json::to_value(&parsed)
            .unwrap()
            .as_object()
            .unwrap()
            .len(),
        4
    );
    assert!(
        !serde_json::to_string(&parsed)
            .unwrap()
            .contains("PRIVATE_BODY")
    );
    for (field, value) in [
        ("workspace_id", json!(Uuid::new_v4().to_string())),
        ("shelf_id", json!(Uuid::new_v4().to_string())),
        ("shelf_id", json!("bad")),
        ("revision", json!(0)),
        ("revision", json!(u64::MAX)),
    ] {
        let mut changed = original.clone();
        changed[field] = value;
        let value: ShelfState = serde_json::from_value(changed).unwrap();
        assert!(validate::shelf_state(&value, &request).is_err());
    }
    for field in ["shelf_id", "revision", "archived"] {
        let mut missing = original.clone();
        missing.as_object_mut().unwrap().remove(field);
        assert!(serde_json::from_value::<ShelfState>(missing).is_err());
    }
    for (field, value) in [("revision", json!(true)), ("archived", json!("true"))] {
        let mut changed = original.clone();
        changed[field] = value;
        assert!(serde_json::from_value::<ShelfState>(changed).is_err());
    }
}

#[test]
fn archived_shelf_projection_bounds_and_page_invariants_preserve_safe_metadata_only() {
    let workspace = Uuid::new_v4().to_string();
    let original = json!({"workspace_id":workspace,"id":Uuid::new_v4().to_string(),"revision":2,"name":"é".repeat(128),"created_at":1,"description":"PRIVATE_BODY","content":"PRIVATE_BODY"});
    let value: ArchivedShelf = serde_json::from_value(original.clone()).unwrap();
    validate::archived_shelf(&value, &workspace).unwrap();
    let safe = serde_json::to_value(&value).unwrap();
    assert_eq!(safe.as_object().unwrap().len(), 5);
    assert!(!safe.to_string().contains("PRIVATE_BODY"));
    for (field, v) in [
        ("workspace_id", json!(Uuid::new_v4().to_string())),
        ("id", json!("invalid")),
        ("revision", json!(0)),
        ("revision", json!(u64::MAX)),
        ("created_at", json!(-1)),
        ("name", json!("")),
        ("name", json!(" trimmed ")),
        ("name", json!("bad\nname")),
        ("name", json!("é".repeat(129))),
    ] {
        let mut changed = original.clone();
        changed[field] = v;
        let value: ArchivedShelf = serde_json::from_value(changed).unwrap();
        assert!(validate::archived_shelf(&value, &workspace).is_err());
    }
    let mut page = Page {
        items: vec![value],
        next_cursor: Some(safe["id"].as_str().unwrap().into()),
    };
    let check = |v: &Page<ArchivedShelf>, after| {
        validate::page(
            v,
            after,
            Some(1),
            |x| &x.id,
            |x| validate::archived_shelf(x, &workspace),
        )
    };
    check(&page, None).unwrap();
    assert!(check(&page, Some(safe["id"].as_str().unwrap())).is_err());
    page.next_cursor = Some(Uuid::new_v4().to_string());
    assert!(check(&page, None).is_err());
}
