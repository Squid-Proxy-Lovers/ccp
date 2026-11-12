// SPDX-License-Identifier: AGPL-3.0-or-later
//! Original source observations, not verified creators or reconstructed revisions.
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeHistory {
    pub workspace_id: String,
    pub entry_id: String,
    pub source: Option<NativeSource>,
    pub items: Vec<NativeRecord>,
    pub next_cursor: Option<u64>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeSource {
    pub database_sha256: String,
    pub schema: u32,
    /// Conversion/adoption time, never the original creation time.
    pub converted_at: i64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeRecord {
    pub ordinal: u64,
    pub source_table: String,
    pub source_key: String,
    pub row: NativeRow,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeRow {
    pub columns: Vec<String>,
    pub values: Vec<NativeValue>,
}

#[derive(Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum NativeValue {
    Null,
    Integer(String),
    Real(String),
    Text(String),
    TextBytes(Vec<u8>),
    Blob(Vec<u8>),
}
