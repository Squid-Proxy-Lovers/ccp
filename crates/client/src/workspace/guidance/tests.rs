// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use serde_json::{Value, json};

const WS: &str = "11111111-1111-4111-8111-111111111111";
const ACTOR: &str = "22222222-2222-4222-8222-222222222222";
const PRINCIPAL: &str = "33333333-3333-4333-8333-333333333333";
fn bundle() -> Value {
    let publisher =
        json!({"actor_kind":"key","actor_id":ACTOR,"principal_id":PRINCIPAL,"recorded_at":100});
    json!({"precedence":"global_first","global":{"scope":{"kind":"global"},"revision":1,"text":"global","publisher":publisher},"workspace":{"scope":{"kind":"workspace","workspace_id":WS},"revision":2,"text":"local","publisher":publisher}})
}
fn invalid(value: Value) {
    if let Ok(value) = serde_json::from_value::<Bundle>(value) {
        let error = validate(&value, Some(WS)).unwrap_err().to_string();
        assert!(!error.contains("PRIVATE_GUIDANCE"));
    }
}
#[test]
fn guidance_known_sources_revisions_and_publisher_metadata_are_checked_without_prose_echo() {
    let value = bundle();
    let valid: Bundle = serde_json::from_value(value.clone()).unwrap();
    validate(&valid, Some(WS)).unwrap();
    assert!(validate(&valid, None).is_err());
    assert!(validate(&valid, Some(PRINCIPAL)).is_err());
    for (field, bad) in [
        ("precedence", json!("workspace_first")),
        ("workspace", Value::Null),
    ] {
        let mut x = value.clone();
        x[field] = bad;
        invalid(x);
    }
    for (field, bad) in [
        (
            "scope",
            json!({"kind":"workspace","workspace_id":PRINCIPAL}),
        ),
        ("scope", json!({"kind":"global"})),
        ("scope", json!({"kind":"workspace","workspace_id":"bad"})),
        (
            "scope",
            json!({"kind":"workspace","workspace_id":Uuid::nil().to_string()}),
        ),
        (
            "scope",
            json!({"kind":"workspace","workspace_id":WS,"extra":"PRIVATE_GUIDANCE"}),
        ),
        ("revision", json!(u64::MAX)),
        ("revision", json!(-1)),
        ("text", json!("PRIVATE_GUIDANCE".repeat(1000))),
        ("publisher", Value::Null),
    ] {
        let mut x = value.clone();
        x["workspace"][field] = bad;
        invalid(x);
    }
    for (field, bad) in [
        ("actor_kind", json!("credential")),
        ("actor_id", json!("PRIVATE_GUIDANCE")),
        ("actor_id", json!(Uuid::nil().to_string())),
        ("principal_id", json!(Uuid::nil().to_string())),
        ("principal_id", json!("PRIVATE_GUIDANCE")),
        ("recorded_at", json!(-1)),
        ("actor_kind", json!("admin")),
    ] {
        let mut x = value.clone();
        x["workspace"]["publisher"][field] = bad;
        invalid(x);
    }
    let mut admin = value.clone();
    admin["workspace"]["publisher"]["actor_kind"] = json!("admin");
    admin["workspace"]["publisher"]["actor_id"] = json!(PRINCIPAL);
    validate(&serde_json::from_value::<Bundle>(admin).unwrap(), Some(WS)).unwrap();
    for bad in [
        "bad",
        "11111111111141118111111111111111",
        "00000000-0000-0000-0000-000000000000",
    ] {
        assert!(identity(bad).is_err());
    }
}

#[test]
fn guidance_empty_state_utf8_controls_and_additive_metadata_have_a_deliberate_projection() {
    let mut value = bundle();
    value["global"]["revision"] = json!(0);
    value["global"]["text"] = json!("");
    value["global"]["publisher"] = Value::Null;
    validate(
        &serde_json::from_value::<Bundle>(value.clone()).unwrap(),
        Some(WS),
    )
    .unwrap();
    let mut bad = value.clone();
    bad["global"]["text"] = json!("PRIVATE_GUIDANCE");
    invalid(bad);
    let mut bad = value.clone();
    bad["global"]["publisher"] = bundle()["workspace"]["publisher"].clone();
    invalid(bad);
    let mut bad = value.clone();
    bad["global"]["scope"] = json!({"kind":"workspace","workspace_id":WS});
    invalid(bad);
    for text in [
        "\0".repeat(TEXT_BYTES),
        "é".repeat(TEXT_BYTES / 2),
        String::new(),
    ] {
        let mut x = value.clone();
        x["workspace"]["text"] = json!(text);
        x["extra"] = json!("UNKNOWN_PRIVATE_FIELD");
        x["workspace"]["extra"] = json!("UNKNOWN_PRIVATE_FIELD");
        x["workspace"]["publisher"]["extra"] = json!("UNKNOWN_PRIVATE_FIELD");
        let typed: Bundle = serde_json::from_value(x).unwrap();
        validate(&typed, Some(WS)).unwrap();
        assert_eq!(typed.workspace.as_ref().unwrap().text, text);
        let out = serde_json::to_string(&typed).unwrap();
        assert!(!out.contains("UNKNOWN_PRIVATE_FIELD"));
    }
    for text in ["x".repeat(TEXT_BYTES + 1), "é".repeat(TEXT_BYTES / 2 + 1)] {
        let mut x = value.clone();
        x["workspace"]["text"] = json!(text);
        invalid(x);
    }
    value["workspace"] = Value::Null;
    validate(&serde_json::from_value::<Bundle>(value).unwrap(), None).unwrap();
}
