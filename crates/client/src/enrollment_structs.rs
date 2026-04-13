// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct EnrollmentMetadata {
    pub session_name: String,
    pub session_id: i64,
    #[serde(default)]
    pub session_description: String,
    #[serde(default)]
    pub owner: String,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default = "default_visibility")]
    pub visibility: String,
    #[serde(default)]
    pub purpose: String,
    pub access: String,
    pub client_cn: String,
    pub mtls_endpoint: String,
    pub client_cert_expires_at: u64,
    pub enrolled_at: u64,
}

#[derive(Debug)]
pub(crate) struct EnrollmentMaterial {
    pub(crate) metadata: EnrollmentMetadata,
    pub(crate) ca_pem: String,
    pub(crate) client_cert_pem: String,
    pub(crate) client_key_pem: String,
}

#[derive(Debug, Clone)]
pub struct StoredEnrollment {
    pub metadata: EnrollmentMetadata,
    pub directory: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSummary {
    pub session_name: String,
    pub session_id: i64,
    pub endpoint: String,
    pub available_access: Vec<String>,
    pub enrollment_count: usize,
    pub session_description: String,
    pub owner: String,
    pub labels: Vec<String>,
    pub visibility: String,
    pub purpose: String,
    pub latest_client_cert_expires_at: u64,
    pub cert_warning: Option<String>,
}

fn default_visibility() -> String {
    "private".to_string()
}
