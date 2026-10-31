// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use serde_json::{Value, json};

const ID: &str = "11111111-1111-4111-8111-111111111111";

fn reference(change: &EntryChange, agent: Option<&str>) -> Vec<u8> {
    let (action, payload) = match change {
        EntryChange::Create {
            book_id,
            name,
            content,
        } => ("create", json!([book_id, name, content])),
        EntryChange::Append {
            entry_id,
            expected_revision,
            text,
        } => ("append", json!([entry_id, expected_revision, text])),
        EntryChange::Revise {
            entry_id,
            expected_revision,
            content,
        } => ("revise", json!([entry_id, expected_revision, content])),
        EntryChange::Archive {
            entry_id,
            expected_revision,
        } => ("archive", json!([entry_id, expected_revision])),
        EntryChange::Restore {
            entry_id,
            expected_revision,
        } => ("restore", json!([entry_id, expected_revision])),
    };
    serde_json::to_vec(&sorted(json!(["entry-mutation-1", action, agent, payload]))).unwrap()
}

// The old implementation used Value objects with lexically sorted keys. Sort
// explicitly in this oracle so the test also works under preserve_order.
fn sorted(value: Value) -> Value {
    match value {
        Value::Object(values) => Value::Object(
            values
                .into_iter()
                .collect::<std::collections::BTreeMap<_, _>>()
                .into_iter()
                .map(|(k, v)| (k, sorted(v)))
                .collect(),
        ),
        Value::Array(values) => Value::Array(values.into_iter().map(sorted).collect()),
        value => value,
    }
}

fn content(text: String) -> EntryContent {
    EntryContent {
        text,
        description: "description\n\0".into(),
        labels: vec!["z".into(), "a".into()],
    }
}
fn changes(text: &str, revision: u64) -> Vec<EntryChange> {
    vec![
        EntryChange::Create {
            book_id: ID.into(),
            name: text.into(),
            content: content(text.into()),
        },
        EntryChange::Append {
            entry_id: ID.into(),
            expected_revision: revision,
            text: text.into(),
        },
        EntryChange::Revise {
            entry_id: ID.into(),
            expected_revision: revision,
            content: content(text.into()),
        },
        EntryChange::Archive {
            entry_id: ID.into(),
            expected_revision: revision,
        },
        EntryChange::Restore {
            entry_id: ID.into(),
            expected_revision: revision,
        },
    ]
}

#[test]
fn fixed_golden_bytes_pin_content_order_label_order_escaping_and_null_agent() {
    let request = EntryChange::Create {
        book_id: ID.into(),
        name: "n".into(),
        content: EntryContent {
            text: "x\n\0é".into(),
            description: "d".into(),
            labels: vec!["z".into(), "a".into()],
        },
    };
    let expected = format!(
        r#"["entry-mutation-1","create",null,["{ID}","n",{{"description":"d","labels":["z","a"],"text":"x\n\u0000é"}}]]"#
    );
    assert_eq!(
        fingerprint_bytes(&request, None).unwrap(),
        expected.as_bytes()
    );
    let request = EntryChange::Append {
        entry_id: ID.into(),
        expected_revision: 7,
        text: "\\\"".into(),
    };
    let expected = format!(r#"["entry-mutation-1","append","agent",["{ID}",7,"\\\""]]"#);
    assert_eq!(
        fingerprint_bytes(&request, Some("agent")).unwrap(),
        expected.as_bytes()
    );
}

fn generated(seed: &mut u64) -> String {
    let mut text = String::new();
    for index in 0..24 {
        *seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let scalar = u32::try_from(*seed % 0x110000).unwrap();
        let character = match index % 8 {
            0 => '\0',
            1 => '\n',
            2 => '"',
            3 => '\\',
            _ => char::from_u32(scalar).unwrap_or('\u{fffd}'),
        };
        text.push(character);
    }
    text
}

#[test]
fn generated_utf8_inputs_match_the_legacy_value_oracle_for_every_action_and_agent_state() {
    // Deterministic generated oracle checks, no new PBT dependency. Every
    // failing seed is reproducible; constraints are built into the generator.
    for initial in 0..1024 {
        let mut seed = initial;
        let text = generated(&mut seed);
        for change in changes(&text, seed) {
            for agent in [None, Some(text.as_str())] {
                assert_eq!(
                    fingerprint_bytes(&change, agent).unwrap(),
                    reference(&change, agent),
                    "seed {initial}"
                );
            }
        }
    }
    for text in [
        String::new(),
        "\0".repeat(256 * 1024),
        " ".repeat(1024),
        "\u{2028}\u{2029}\r\t\u{7f}".into(),
    ] {
        for change in changes(&text, u64::MAX) {
            assert_eq!(
                fingerprint_bytes(&change, None).unwrap(),
                reference(&change, None)
            );
        }
    }
}
