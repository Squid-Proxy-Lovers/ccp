// SPDX-License-Identifier: AGPL-3.0-or-later

//! Source observations, never ownership, grants or reconstructed creation events.
use rusqlite::types::{ToSql, ToSqlOutput, ValueRef};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

mod encoding;
#[cfg(test)]
mod tests;

/// JSON numbers cannot carry every native SQLite integer or floating-point bit
/// pattern through consumers. Integers use decimal strings; reals use hex bits.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum NativeSqlValue {
    Null,
    Integer(#[serde(with = "encoding::integer")] i64),
    Real(#[serde(with = "encoding::real")] u64),
    Text(String),
    TextBytes(Vec<u8>),
    Blob(Vec<u8>),
}

impl From<ValueRef<'_>> for NativeSqlValue {
    fn from(value: ValueRef<'_>) -> Self {
        match value {
            ValueRef::Null => Self::Null,
            ValueRef::Integer(v) => Self::Integer(v),
            ValueRef::Real(v) => Self::Real(v.to_bits()),
            ValueRef::Text(v) => match std::str::from_utf8(v) {
                Ok(v) => Self::Text(v.to_owned()),
                Err(_) => Self::TextBytes(v.to_vec()),
            },
            ValueRef::Blob(v) => Self::Blob(v.to_vec()),
        }
    }
}

impl ToSql for NativeSqlValue {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::Borrowed(match self {
            Self::Null => ValueRef::Null,
            Self::Integer(v) => ValueRef::Integer(*v),
            Self::Real(v) => ValueRef::Real(f64::from_bits(*v)),
            Self::Text(v) => ValueRef::Text(v.as_bytes()),
            Self::TextBytes(v) => ValueRef::Text(v),
            Self::Blob(v) => ValueRef::Blob(v),
        }))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeRow {
    pub columns: Vec<String>,
    pub values: Vec<NativeSqlValue>,
}

impl NativeRow {
    /// Capture a physical source row, without cache projections or type coercion.
    pub fn capture(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        let columns = row
            .as_ref()
            .column_names()
            .into_iter()
            .map(str::to_owned)
            .collect();
        let values = (0..row.as_ref().column_count())
            .map(|i| row.get_ref(i).map(NativeSqlValue::from))
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(Self { columns, values })
    }

    /// Structural support check; this does not interpret source fields as authority.
    pub fn is_valid(&self) -> bool {
        !self.columns.is_empty()
            && self.columns.len() <= 256
            && self.columns.len() == self.values.len()
            && self
                .columns
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                == self.columns.len()
    }
}

/// The recording administrator and time identify the conversion operation only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeImportInfo {
    pub source_sha256: String,
    pub source_schema: u32,
    pub legacy_session_id: i64,
    pub recorded_by: Uuid,
    pub recorded_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeEntryRecord {
    pub ordinal: u64,
    pub source_table: String,
    pub source_key: String,
    pub row: NativeRow,
}

pub struct NativeHistoryRequest {
    pub entry_id: Uuid,
    pub after: Option<u64>,
    pub limit: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeEntryHistory {
    pub source: NativeImportInfo,
    pub items: Vec<NativeEntryRecord>,
    pub next_cursor: Option<u64>,
}
