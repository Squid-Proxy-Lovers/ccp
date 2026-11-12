// SPDX-License-Identifier: AGPL-3.0-or-later
use super::ConversionOptions;
use crate::{
    foundation::NativeRow,
    legacy_inspection::{query, snapshot::Snapshot},
};
use anyhow::{Context, bail};
use rusqlite::{Connection, OpenFlags, ToSql, types::ValueRef};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Table {
    Sessions,
    Shelves,
    Books,
    Entries,
    Archives,
    Appends,
    ArchivedAppends,
    Certificates,
    Tokens,
    Transfers,
}
impl Table {
    pub const ALL: [Self; 10] = [
        Self::Sessions,
        Self::Shelves,
        Self::Books,
        Self::Entries,
        Self::Archives,
        Self::Appends,
        Self::ArchivedAppends,
        Self::Certificates,
        Self::Tokens,
        Self::Transfers,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Sessions => "sessions",
            Self::Shelves => "shelves",
            Self::Books => "books",
            Self::Entries => "message_packs",
            Self::Archives => "deleted_message_packs",
            Self::Appends => "message_history",
            Self::ArchivedAppends => "deleted_message_history",
            Self::Certificates => "issued_client_certs",
            Self::Tokens => "auth_tokens",
            Self::Transfers => "transfer_log",
        }
    }
    fn key(self) -> &'static str {
        if self == Self::Archives {
            "entry_key"
        } else {
            "id"
        }
    }
    fn filter(self) -> &'static str {
        match self {
            Self::Sessions => "id",
            Self::Appends => "message_pack_id",
            Self::ArchivedAppends => "entry_key",
            _ => "session_id",
        }
    }
}
pub(super) enum Key {
    Integer(i64),
    Text(String),
}
impl Key {
    pub fn sql(&self) -> &dyn ToSql {
        match self {
            Self::Integer(v) => v,
            Self::Text(v) => v,
        }
    }
    pub fn label(&self) -> String {
        match self {
            Self::Integer(v) => v.to_string(),
            Self::Text(v) => v.clone(),
        }
    }
}
pub(super) struct Record {
    pub table: Table,
    pub key: Key,
    pub row: NativeRow,
    pub json: String,
}

pub(super) struct Source {
    // Field order closes SQLite before the owned private snapshot is removed.
    db: Connection,
    snapshot: Snapshot,
    columns: BTreeMap<Table, Vec<String>>,
    pub expected: BTreeMap<Table, u64>,
}
pub(super) struct Ledger {
    pub seen: BTreeMap<Table, u64>,
    bytes: u64,
}
impl Ledger {
    pub fn new() -> Self {
        Self {
            seen: Table::ALL.into_iter().map(|t| (t, 0)).collect(),
            bytes: 0,
        }
    }
    fn record(&mut self, record: &Record) -> anyhow::Result<()> {
        self.bytes = self
            .bytes
            .checked_add(u64::try_from(record.json.len())?)
            .context("raw size overflow")?;
        if self.bytes > 2 * 1024 * 1024 * 1024 {
            bail!("raw conversion budget exceeded");
        }
        let count = self
            .seen
            .get_mut(&record.table)
            .context("unknown source table")?;
        *count = count.checked_add(1).context("row count overflow")?;
        Ok(())
    }
}
impl Source {
    pub fn open(options: &ConversionOptions) -> anyhow::Result<Self> {
        let snapshot = Snapshot::capture(&options.data_dir)?;
        if snapshot.fingerprint.sha256 != options.expected_source_sha256 {
            bail!("source fingerprint mismatch");
        }
        let db =
            Connection::open_with_flags(snapshot.database(), OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        db.execute_batch("PRAGMA query_only=ON; PRAGMA trusted_schema=OFF;")?;
        ordinary(&db)?;
        let (version, mode, native, _workspace) = query::inventory(&db)?;
        if version != 2 || mode != "native" {
            bail!("unsupported conversion source");
        }
        let values = [
            native.sessions,
            native.shelves,
            native.books,
            native.entries,
            native.archives,
            native.append_records,
            native.archived_append_records,
            native.certificates,
            native.tokens,
            native.transfer_records,
        ];
        if native.sessions > 256
            || values
                .iter()
                .try_fold(0_u64, |sum, v| sum.checked_add(*v))
                .context("row count overflow")?
                > 100_000
        {
            bail!("source conversion row budget exceeded");
        }
        validate_keys(&db)?;
        validate_relationships(&db)?;
        let expected = Table::ALL.into_iter().zip(values).collect();
        let mut columns = BTreeMap::new();
        for table in Table::ALL {
            let names = db
                .prepare(&format!("PRAGMA table_info({})", table.name()))?
                .query_map([], |r| r.get::<_, String>(1))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            if names.is_empty() || names.len() > 256 {
                bail!("unsupported source columns");
            }
            columns.insert(table, names);
        }
        Ok(Self {
            db,
            snapshot,
            columns,
            expected,
        })
    }
    pub fn fingerprint(&self) -> &str {
        &self.snapshot.fingerprint.sha256
    }
    pub fn verify(&self) -> anyhow::Result<()> {
        self.snapshot.verify()?;
        Ok(())
    }
    pub fn visit(
        &self,
        table: Table,
        filter: Option<&dyn ToSql>,
        ledger: &mut Ledger,
        mut callback: impl FnMut(Record, &mut Ledger) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        let columns = self.columns.get(&table).context("missing source columns")?;
        let sizes = columns
            .iter()
            .map(|name| format!("coalesce(length(CAST({} AS BLOB)),0)", quoted(name)))
            .collect::<Vec<_>>()
            .join("+");
        let predicate = if filter.is_some() {
            format!(" WHERE {} COLLATE BINARY=?1", table.filter())
        } else {
            String::new()
        };
        let mut statement = self.db.prepare(&format!(
            "SELECT {},{sizes} FROM {}{predicate} ORDER BY {} COLLATE BINARY",
            table.key(),
            table.name(),
            table.key()
        ))?;
        let parameters = filter.into_iter().collect::<Vec<_>>();
        let mut rows = statement.query(rusqlite::params_from_iter(parameters))?;
        while let Some(row) = rows.next()? {
            let key = match (table, row.get_ref(0)?) {
                (Table::Archives, ValueRef::Text(v)) if !v.is_empty() && v.len() <= 8192 => {
                    Key::Text(std::str::from_utf8(v)?.to_owned())
                }
                (table, ValueRef::Integer(v)) if table != Table::Archives => Key::Integer(v),
                _ => bail!("unsupported source key"),
            };
            let size: u64 = row.get(1)?;
            if size > 2 * 1024 * 1024 {
                bail!("source row exceeds conversion bound");
            }
            let native = self.db.query_row(
                &format!(
                    "SELECT * FROM {} WHERE {} COLLATE BINARY=?1",
                    table.name(),
                    table.key()
                ),
                [key.sql()],
                NativeRow::capture,
            )?;
            let json = serde_json::to_string(&native)?;
            if !native.is_valid() || json.len() > 2 * 1024 * 1024 {
                bail!("unsupported source row");
            }
            let record = Record {
                table,
                key,
                row: native,
                json,
            };
            ledger.record(&record)?;
            callback(record, ledger)?;
        }
        Ok(())
    }
}
fn quoted(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}
fn ordinary(db: &Connection) -> anyhow::Result<()> {
    let rows = db
        .prepare("PRAGMA main.table_list")?
        .query_map([], |r| Ok((r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (name, kind) in &rows {
        let lower = name.to_ascii_lowercase();
        if lower.starts_with("sqlite_") {
            continue;
        }
        let native = lower == "schema_version" || Table::ALL.iter().any(|t| t.name() == lower);
        let derived = (lower == "message_packs_fts" && kind == "virtual")
            || ([
                "message_packs_fts_data",
                "message_packs_fts_idx",
                "message_packs_fts_docsize",
                "message_packs_fts_config",
            ]
            .contains(&lower.as_str())
                && kind == "shadow");
        if !(native && kind == "table" || derived) {
            bail!("unknown or nonordinary source table");
        }
    }
    for name in std::iter::once("schema_version").chain(Table::ALL.iter().map(|t| t.name())) {
        if !rows
            .iter()
            .any(|(n, k)| n.eq_ignore_ascii_case(name) && k == "table")
        {
            bail!("ordinary source table missing");
        }
    }
    Ok(())
}

// Payload lookups use these keys without a session predicate. Validate globally
// before building any target, even when source DDL omitted its PK constraints.
pub(super) fn validate_keys(db: &Connection) -> anyhow::Result<()> {
    for table in Table::ALL {
        let key = table.key();
        let valid = key_type_predicate(key, table == Table::Archives);
        let invalid: u64 = db.query_row(
            &format!("SELECT count(*) FROM {} WHERE NOT ({valid})", table.name()),
            [],
            |r| r.get(0),
        )?;
        if invalid != 0 {
            bail!("unsupported source keys");
        }
        // Types and text lengths are bounded before DISTINCT materializes keys.
        let (total, distinct, present): (u64, u64, u64) = db.query_row(
            &format!(
                "SELECT count(*),count(DISTINCT {key} COLLATE BINARY),count({key}) FROM {}",
                table.name()
            ),
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        if total != distinct || total != present {
            bail!("ambiguous source keys");
        }
    }
    Ok(())
}

fn key_type_predicate(column: &str, text: bool) -> String {
    if text {
        format!("typeof({column})='text' AND length(CAST({column} AS BLOB)) BETWEEN 1 AND 8192")
    } else {
        format!("typeof({column})='integer'")
    }
}

// Source DDL may omit or alter FK/collation constraints. Every child must have
// one exact parent independently of aggregate accounting or declared collation.
pub(super) fn validate_relationships(db: &Connection) -> anyhow::Result<()> {
    for table in Table::ALL.into_iter().filter(|t| *t != Table::Sessions) {
        let parent = match table {
            Table::Appends => Table::Entries,
            Table::ArchivedAppends => Table::Archives,
            _ => Table::Sessions,
        };
        let column = table.filter();
        let valid = key_type_predicate(column, table == Table::ArchivedAppends);
        let invalid: u64 = db.query_row(
            &format!("SELECT count(*) FROM {} WHERE NOT ({valid})", table.name()),
            [],
            |r| r.get(0),
        )?;
        if invalid != 0 {
            bail!("unsupported source parent keys");
        }
        let orphan: u64 = db.query_row(
            &format!("SELECT count(*) FROM {} AS child LEFT JOIN {} AS parent ON parent.{} COLLATE BINARY=child.{column} COLLATE BINARY WHERE parent.{} IS NULL",table.name(),parent.name(),parent.key(),parent.key()),
            [],
            |r| r.get(0),
        )?;
        if orphan != 0 {
            bail!("source parent missing");
        }
    }
    Ok(())
}
