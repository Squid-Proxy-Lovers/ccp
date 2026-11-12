// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;
use rusqlite::{Connection, params};

#[test]
fn scalar_json_and_sql_roundtrips_preserve_types_bytes_and_numeric_boundaries() {
    let db = Connection::open_in_memory().unwrap();
    let bytes: Vec<u8> = (u8::MIN..=u8::MAX).collect();
    let mut values = vec![
        NativeSqlValue::Null,
        NativeSqlValue::Text("raw\0雪\n".into()),
        NativeSqlValue::TextBytes(bytes.clone()),
        NativeSqlValue::Blob(bytes),
    ];
    for integer in [
        i64::MIN,
        -9_007_199_254_740_993,
        -1,
        0,
        1,
        9_007_199_254_740_993,
        i64::MAX,
    ] {
        values.push(NativeSqlValue::Integer(integer));
    }
    for real in [
        -0.0_f64,
        0.0,
        f64::MIN,
        f64::MAX,
        f64::MIN_POSITIVE,
        f64::INFINITY,
        f64::NEG_INFINITY,
        0.1,
    ] {
        values.push(NativeSqlValue::Real(real.to_bits()));
    }
    for value in values {
        let encoded = serde_json::to_string(&value).unwrap();
        let decoded: NativeSqlValue = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, value);
        let (kind, raw) = db
            .query_row("SELECT typeof(?1),?1", params![decoded], |r| {
                Ok((r.get::<_, String>(0)?, NativeSqlValue::from(r.get_ref(1)?)))
            })
            .unwrap();
        assert_eq!(raw, value, "{kind}");
    }
    assert_eq!(
        serde_json::to_string(&NativeSqlValue::Integer(i64::MAX)).unwrap(),
        "{\"type\":\"integer\",\"value\":\"9223372036854775807\"}"
    );
}

#[test]
fn captured_rows_keep_original_columns_order_and_sql_storage_classes() {
    let db = Connection::open_in_memory().unwrap();
    let row = db.query_row("SELECT -9223372036854775808 AS id, ' raw , labels ' AS labels, CAST(x'00ff' AS TEXT) AS unusual, x'00ff' AS binary, NULL AS missing, 'arbitrary old time' AS created_at",[],NativeRow::capture).unwrap();
    assert_eq!(
        row.columns,
        ["id", "labels", "unusual", "binary", "missing", "created_at"]
    );
    assert_eq!(
        row.values,
        [
            NativeSqlValue::Integer(i64::MIN),
            NativeSqlValue::Text(" raw , labels ".into()),
            NativeSqlValue::TextBytes(vec![0, 255]),
            NativeSqlValue::Blob(vec![0, 255]),
            NativeSqlValue::Null,
            NativeSqlValue::Text("arbitrary old time".into())
        ]
    );
    assert!(row.is_valid());
    assert_eq!(
        serde_json::from_str::<NativeRow>(&serde_json::to_string(&row).unwrap()).unwrap(),
        row
    );
}

#[test]
fn numeric_encodings_refuse_coercion_overflow_and_noncanonical_spellings() {
    for (kind, encoded) in [
        ("integer", "1"),
        ("integer", "\"+1\""),
        ("integer", "\"01\""),
        ("integer", "\"-0\""),
        ("integer", "\"9223372036854775808\""),
        ("integer", "\"PRIVATE\""),
        ("real", "0"),
        ("real", "\"0\""),
        ("real", "\"FFFFFFFFFFFFFFFF\""),
        ("real", "\"10000000000000000\""),
    ] {
        assert!(
            serde_json::from_str::<NativeSqlValue>(&format!(
                "{{\"type\":\"{kind}\",\"value\":{encoded}}}"
            ))
            .is_err()
        );
    }
    for value in [
        0_u64,
        1,
        u64::MAX,
        0x7ff8_0000_0000_0001,
        0x8000_0000_0000_0000,
    ] {
        let raw = NativeSqlValue::Real(value);
        assert_eq!(
            serde_json::from_str::<NativeSqlValue>(&serde_json::to_string(&raw).unwrap()).unwrap(),
            raw
        );
    }
}
