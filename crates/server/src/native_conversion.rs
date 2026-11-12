// SPDX-License-Identifier: AGPL-3.0-or-later
//! Explicit local conversion into a new instance. No source writes or trust reuse.
use crate::foundation::IssuedAdminKey;
use serde::Serialize;
use std::{collections::BTreeMap, path::PathBuf};
use uuid::Uuid;

mod destination;
mod projection;
mod source;
mod target;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConversionError {
    InvalidInput,
    SourceRefused,
    SourceChanged,
    DataRefused,
    DestinationUnavailable,
    PublicationUnconfirmed,
}
impl std::fmt::Display for ConversionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidInput => "native conversion: explicit offline confirmation, valid operator, source fingerprint and absolute separate destination required",
            Self::SourceRefused => "native conversion: requires a complete native schema-2 source with matching fingerprint, ordinary tables and no pending recovery files; existing workspace data is not supported",
            Self::SourceChanged => "native conversion: source changed during conversion; no database published",
            Self::DataRefused => "native conversion: source values, layout or counts cannot be preserved within supported conversion bounds; no database published",
            Self::DestinationUnavailable => "native conversion: cannot publish to a new destination without replacing existing material",
            Self::PublicationUnconfirmed => "native conversion: complete database may already be published; inspect destination and recover local keys if needed; do not retry or delete it automatically",
        })
    }
}
impl std::error::Error for ConversionError {}
type Result<T> = std::result::Result<T, ConversionError>;

pub struct ConversionOptions {
    pub data_dir: PathBuf,
    pub destination: PathBuf,
    pub expected_source_sha256: String,
    /// Explicit selection of the new global administrator and workspace owner.
    pub operator: String,
    pub confirm_offline: bool,
}

#[derive(Serialize)]
pub struct ConversionReceipt {
    pub instance_id: String,
    pub source_sha256: String,
    pub source_schema: u32,
    pub source_rows: BTreeMap<String, u64>,
    pub converted_at: i64,
    pub workspace_count: u64,
    pub entry_count: u64,
    pub conversion_performed: bool,
    pub source_database_unchanged_at_verification: bool,
}

/// One-time key delivery, intentionally not Debug, Clone or Serialize.
pub struct ConvertedWorkspace {
    pub id: Uuid,
    pub name: String,
    pub owner_key: IssuedAdminKey,
}
pub struct ConversionResult {
    pub receipt: ConversionReceipt,
    pub instance_key: IssuedAdminKey,
    pub workspaces: Vec<ConvertedWorkspace>,
}

pub fn convert(options: &ConversionOptions) -> Result<ConversionResult> {
    if !options.confirm_offline
        || !projection::valid_name(&options.operator)
        || options.expected_source_sha256.len() != 64
        || !options
            .expected_source_sha256
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(ConversionError::InvalidInput);
    }
    let source = source::Source::open(options).map_err(|_error| ConversionError::SourceRefused)?;
    let stage = destination::Stage::create(&options.data_dir, &options.destination)
        .map_err(|_error| ConversionError::DestinationUnavailable)?;
    let result = target::build(&source, &stage, &options.operator)
        .map_err(|_error| ConversionError::DataRefused)?;
    finish(&source, &stage, result)
}

fn finish(
    source: &source::Source,
    stage: &destination::Stage,
    mut result: ConversionResult,
) -> Result<ConversionResult> {
    source
        .verify()
        .map_err(|_error| ConversionError::SourceChanged)?;
    stage.publish()?;
    result.receipt.source_database_unchanged_at_verification = true;
    Ok(result)
}

#[cfg(test)]
mod tests;
