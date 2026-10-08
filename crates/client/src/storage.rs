// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};

use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, bail};

use crate::enrollment_structs::{
    EnrollmentMaterial, EnrollmentMetadata, SessionSummary, StoredEnrollment,
};
use protocol::SessionMetadata;

const CLIENT_HOME_ENV: &str = "CCP_CLIENT_HOME";
const DEFAULT_CLIENT_HOME_DIR: &str = ".ccp-client";
const ENROLLMENTS_DIR_NAME: &str = "enrollments";
const DEFAULT_CERT_WARNING_WINDOW_SECONDS: u64 = 0;

pub(crate) fn save_enrollment(material: &EnrollmentMaterial) -> anyhow::Result<StoredEnrollment> {
    let base_dir = enrollments_dir()?;
    save_enrollment_to_dir(material, &base_dir)
}

pub(crate) fn save_subscription(
    endpoint: &str,
    session: &SessionMetadata,
) -> anyhow::Result<StoredEnrollment> {
    let endpoint = crate::transport_helpers::normalized_endpoint(endpoint)?;
    let material = EnrollmentMaterial {
        metadata: EnrollmentMetadata {
            session_name: session.session_name.clone(),
            session_id: session.session_id,
            session_description: session.description.clone(),
            owner: session.owner.clone(),
            labels: session.labels.clone(),
            visibility: session.visibility.clone(),
            purpose: session.purpose.clone(),
            access: "read_write".to_string(),
            client_cn: "plaintext-client".to_string(),
            mtls_endpoint: endpoint,
            client_cert_expires_at: u64::MAX,
            enrolled_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        },
        ca_pem: String::new(),
        client_cert_pem: String::new(),
        client_key_pem: String::new(),
    };
    save_enrollment(&material)
}

pub(crate) fn load_enrollments() -> anyhow::Result<Vec<StoredEnrollment>> {
    let base_dir = enrollments_dir()?;
    load_enrollments_from_dir(&base_dir)
}

pub(crate) fn select_enrollment(
    session_selector: &str,
    require_write: bool,
) -> anyhow::Result<StoredEnrollment> {
    let enrollments = load_enrollments()?;
    let selector = configured_selector(session_selector)?;
    select_enrollment_from_enrollments(&enrollments, &selector, require_write)
}

pub(crate) fn delete_session_enrollments(session_selector: &str) -> anyhow::Result<usize> {
    let base_dir = enrollments_dir()?;
    let selector = configured_selector(session_selector)?;
    delete_session_enrollments_from_dir(&base_dir, &selector)
}

pub(crate) fn summarize_sessions(enrollments: &[StoredEnrollment]) -> Vec<SessionSummary> {
    let mut sessions = BTreeMap::new();

    let mut ordered = enrollments.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|enrollment| std::cmp::Reverse(selection_order(enrollment)));
    for enrollment in ordered {
        let key = (
            enrollment.metadata.session_id,
            endpoint_identity(&enrollment.metadata.mtls_endpoint),
        );
        let summary = sessions.entry(key).or_insert_with(|| SessionSummary {
            session_name: enrollment.metadata.session_name.clone(),
            session_id: enrollment.metadata.session_id,
            endpoint: enrollment.metadata.mtls_endpoint.clone(),
            available_access: Vec::new(),
            enrollment_count: 0,
            session_description: enrollment.metadata.session_description.clone(),
            owner: enrollment.metadata.owner.clone(),
            labels: enrollment.metadata.labels.clone(),
            visibility: enrollment.metadata.visibility.clone(),
            purpose: enrollment.metadata.purpose.clone(),
            latest_client_cert_expires_at: enrollment.metadata.client_cert_expires_at,
            cert_warning: cert_warning_for_expiry(enrollment.metadata.client_cert_expires_at),
        });

        if !summary
            .available_access
            .iter()
            .any(|access| access == &enrollment.metadata.access)
        {
            summary
                .available_access
                .push(enrollment.metadata.access.clone());
            summary.available_access.sort();
        }

        summary.enrollment_count += 1;
        if enrollment.metadata.client_cert_expires_at >= summary.latest_client_cert_expires_at {
            summary.latest_client_cert_expires_at = enrollment.metadata.client_cert_expires_at;
            summary.cert_warning =
                cert_warning_for_expiry(enrollment.metadata.client_cert_expires_at);
        }
    }

    sessions.into_values().collect()
}

fn save_enrollment_to_dir(
    material: &EnrollmentMaterial,
    base_dir: &Path,
) -> anyhow::Result<StoredEnrollment> {
    fs::create_dir_all(base_dir)
        .with_context(|| format!("failed to create {}", base_dir.display()))?;

    // Names and IDs are not global: include server identity and preserve distinct
    // names that sanitize to the same filesystem component.
    let identity = serde_json::to_vec(&(
        endpoint_identity(&material.metadata.mtls_endpoint),
        material.metadata.session_id,
        &material.metadata.access,
        &material.metadata.client_cn,
    ))?;
    let digest = Sha256::digest(identity);
    let directory = base_dir.join(format!("subscription--{digest:x}"));
    fs::create_dir_all(&directory)
        .with_context(|| format!("failed to create {}", directory.display()))?;

    // Readers must observe either the old metadata or the complete new record.
    static NEXT_WRITE: AtomicU64 = AtomicU64::new(0);
    let temporary = directory.join(format!(
        ".metadata-{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        NEXT_WRITE.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| -> anyhow::Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(&serde_json::to_vec_pretty(&material.metadata)?)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, directory.join("metadata.json"))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.with_context(|| format!("failed to save metadata in {}", directory.display()))?;
    Ok(StoredEnrollment {
        metadata: material.metadata.clone(),
        directory,
    })
}

fn load_enrollments_from_dir(base_dir: &Path) -> anyhow::Result<Vec<StoredEnrollment>> {
    if !base_dir.exists() {
        return Ok(Vec::new());
    }

    let mut enrollments = Vec::new();
    for entry in
        fs::read_dir(base_dir).with_context(|| format!("failed to read {}", base_dir.display()))?
    {
        let entry = entry.context("failed to read enrollment directory entry")?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let metadata_path = path.join("metadata.json");
        if !metadata_path.exists() {
            continue;
        }

        let metadata = serde_json::from_slice::<EnrollmentMetadata>(
            &fs::read(&metadata_path)
                .with_context(|| format!("failed to read {}", metadata_path.display()))?,
        )
        .with_context(|| format!("failed to parse {}", metadata_path.display()))?;

        enrollments.push(StoredEnrollment {
            metadata,
            directory: path,
        });
    }

    Ok(enrollments)
}

fn delete_session_enrollments_from_dir(
    base_dir: &Path,
    session_selector: &str,
) -> anyhow::Result<usize> {
    let enrollments = load_enrollments_from_dir(base_dir)?;
    let matching = matching_enrollments(&enrollments, session_selector)?;
    let matching_directories = matching
        .into_iter()
        .map(|enrollment| enrollment.directory.clone())
        .collect::<Vec<_>>();

    if matching_directories.is_empty() {
        bail!("no saved enrollment found for session '{session_selector}'");
    }

    for directory in &matching_directories {
        fs::remove_dir_all(directory)
            .with_context(|| format!("failed to remove {}", directory.display()))?;
    }

    Ok(matching_directories.len())
}

fn select_enrollment_from_enrollments(
    enrollments: &[StoredEnrollment],
    session_selector: &str,
    require_write: bool,
) -> anyhow::Result<StoredEnrollment> {
    let mut candidates = matching_enrollments(enrollments, session_selector)?
        .into_iter()
        .filter(|enrollment| {
            if require_write {
                enrollment.metadata.access == "read_write" || enrollment.metadata.access == "admin"
            } else {
                enrollment.metadata.access == "read"
                    || enrollment.metadata.access == "read_write"
                    || enrollment.metadata.access == "admin"
            }
        })
        .cloned()
        .collect::<Vec<_>>();

    if candidates.is_empty() {
        if require_write {
            bail!("no saved read_write enrollment found for session '{session_selector}'");
        }
        bail!("no saved enrollment found for session '{session_selector}'");
    }

    candidates.sort_by_key(selection_order);
    Ok(candidates.pop().expect("candidate list is not empty"))
}

fn endpoint_identity(endpoint: &str) -> String {
    crate::transport_helpers::normalized_endpoint(endpoint)
        .unwrap_or_else(|_| endpoint.trim_end_matches('/').to_string())
}

fn selection_order(enrollment: &StoredEnrollment) -> (u64, bool, PathBuf) {
    let modern = enrollment
        .directory
        .file_name()
        .is_some_and(|name| name.to_string_lossy().starts_with("subscription--"));
    (
        enrollment.metadata.enrolled_at,
        modern,
        enrollment.directory.clone(),
    )
}

fn configured_selector(selector: &str) -> anyhow::Result<String> {
    if selector
        .rsplit_once('@')
        .is_some_and(|(_, endpoint)| endpoint.starts_with("http://"))
    {
        return Ok(selector.to_string());
    }
    match std::env::var("CCP_SERVER_URL") {
        Ok(endpoint) => Ok(format!(
            "{selector}@{}",
            crate::transport_helpers::normalized_endpoint(&endpoint)?
        )),
        Err(_) => Ok(selector.to_string()),
    }
}

fn matching_enrollments<'a>(
    enrollments: &'a [StoredEnrollment],
    selector: &str,
) -> anyhow::Result<Vec<&'a StoredEnrollment>> {
    let (session, endpoint) = match selector.rsplit_once('@') {
        Some((session, endpoint)) if endpoint.starts_with("http://") => (
            session,
            Some(crate::transport_helpers::normalized_endpoint(endpoint)?),
        ),
        _ => (selector, None),
    };
    let matches = enrollments
        .iter()
        .filter(|enrollment| {
            enrollment_matches_selector(enrollment, session)
                && endpoint.as_ref().is_none_or(|endpoint| {
                    endpoint_identity(&enrollment.metadata.mtls_endpoint) == *endpoint
                })
        })
        .collect::<Vec<_>>();
    let identities = matches
        .iter()
        .map(|enrollment| {
            (
                enrollment.metadata.session_id,
                endpoint_identity(&enrollment.metadata.mtls_endpoint),
            )
        })
        .collect::<BTreeSet<_>>();
    if identities.len() > 1 {
        bail!(
            "session selector '{selector}' is ambiguous; use <session-id>@http://<server> or set CCP_SERVER_URL"
        );
    }
    Ok(matches)
}

fn enrollment_matches_selector(enrollment: &StoredEnrollment, session_selector: &str) -> bool {
    enrollment.metadata.session_name == session_selector
        || enrollment.metadata.session_id.to_string() == session_selector
}

fn enrollments_dir() -> anyhow::Result<PathBuf> {
    Ok(client_home_dir()?.join(ENROLLMENTS_DIR_NAME))
}

fn client_home_dir() -> anyhow::Result<PathBuf> {
    if let Some(path) = std::env::var_os(CLIENT_HOME_ENV) {
        return Ok(PathBuf::from(path));
    }

    let Some(home) = home_dir() else {
        bail!("unable to determine client home directory; set {CLIENT_HOME_ENV}");
    };

    Ok(home.join(DEFAULT_CLIENT_HOME_DIR))
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn cert_warning_for_expiry(expires_at: u64) -> Option<String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    if now >= expires_at {
        return Some(format!(
            "client certificate expired at unix={expires_at}; request a new enrollment token and re-enroll"
        ));
    }

    let warning_window = std::env::var("CCP_CERT_WARNING_WINDOW_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(DEFAULT_CERT_WARNING_WINDOW_SECONDS);
    if warning_window == 0 {
        return None;
    }
    let remaining = expires_at.saturating_sub(now);
    if remaining <= warning_window {
        return Some(format!(
            "client certificate expires soon at unix={expires_at}; request a new enrollment token before it expires"
        ));
    }

    None
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn summarize_sessions_groups_multiple_certificates_per_session() {
        let read = StoredEnrollment {
            metadata: EnrollmentMetadata {
                session_name: "session-a".to_string(),
                session_id: 1,
                session_description: "desc".to_string(),
                owner: "owner".to_string(),
                labels: vec!["alpha".to_string()],
                visibility: "private".to_string(),
                purpose: "testing".to_string(),
                access: "read".to_string(),
                client_cn: "client-1".to_string(),
                mtls_endpoint: "https://localhost:1338".to_string(),
                enrolled_at: 10,
                client_cert_expires_at: 4_102_444_800,
            },
            directory: PathBuf::from("ignored"),
        };
        let read_write = StoredEnrollment {
            metadata: EnrollmentMetadata {
                session_name: "session-a".to_string(),
                session_id: 1,
                session_description: "desc".to_string(),
                owner: "owner".to_string(),
                labels: vec!["alpha".to_string()],
                visibility: "private".to_string(),
                purpose: "testing".to_string(),
                access: "read_write".to_string(),
                client_cn: "client-2".to_string(),
                mtls_endpoint: "https://localhost:1338".to_string(),
                enrolled_at: 20,
                client_cert_expires_at: 4_102_444_800,
            },
            directory: PathBuf::from("ignored"),
        };

        let sessions = summarize_sessions(&[read, read_write]);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_name, "session-a");
        assert_eq!(sessions[0].session_id, 1);
        assert_eq!(sessions[0].available_access, vec!["read", "read_write"]);
        assert_eq!(sessions[0].enrollment_count, 2);
        assert_eq!(sessions[0].owner, "owner");
    }

    #[test]
    fn save_and_load_enrollments_from_directory() {
        let base_dir = std::env::temp_dir().join(format!(
            "ccp-client-tests-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos()
        ));
        let material = EnrollmentMaterial {
            metadata: EnrollmentMetadata {
                session_name: "session-a".to_string(),
                session_id: 7,
                session_description: "desc".to_string(),
                owner: "owner".to_string(),
                labels: vec!["alpha".to_string()],
                visibility: "private".to_string(),
                purpose: "testing".to_string(),
                access: "read_write".to_string(),
                client_cn: "client-123".to_string(),
                mtls_endpoint: "https://localhost:1338".to_string(),
                enrolled_at: 99,
                client_cert_expires_at: 4_102_444_800,
            },
            ca_pem: "ca".to_string(),
            client_cert_pem: "cert".to_string(),
            client_key_pem: "key".to_string(),
        };

        let stored = save_enrollment_to_dir(&material, &base_dir).expect("save should work");
        let loaded = load_enrollments_from_dir(&base_dir).expect("load should work");

        assert_eq!(stored.metadata, material.metadata);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].metadata, material.metadata);
        assert_eq!(loaded[0].directory, stored.directory);

        fs::remove_dir_all(base_dir).expect("temp directory should be removable");
    }

    #[test]
    fn select_enrollment_prefers_latest_compatible_entry() {
        let latest = StoredEnrollment {
            metadata: EnrollmentMetadata {
                session_name: "session-a".to_string(),
                session_id: 1,
                session_description: "desc".to_string(),
                owner: "owner".to_string(),
                labels: vec!["alpha".to_string()],
                visibility: "private".to_string(),
                purpose: "testing".to_string(),
                access: "read_write".to_string(),
                client_cn: "client-2".to_string(),
                mtls_endpoint: "https://localhost:1338".to_string(),
                enrolled_at: 20,
                client_cert_expires_at: 4_102_444_800,
            },
            directory: PathBuf::from("ignored"),
        };
        let older = StoredEnrollment {
            metadata: EnrollmentMetadata {
                session_name: "session-a".to_string(),
                session_id: 1,
                session_description: "desc".to_string(),
                owner: "owner".to_string(),
                labels: vec!["alpha".to_string()],
                visibility: "private".to_string(),
                purpose: "testing".to_string(),
                access: "read".to_string(),
                client_cn: "client-1".to_string(),
                mtls_endpoint: "https://localhost:1338".to_string(),
                enrolled_at: 10,
                client_cert_expires_at: 4_102_444_800,
            },
            directory: PathBuf::from("ignored"),
        };

        let selected =
            select_enrollment_from_enrollments(&[older, latest.clone()], "session-a", false)
                .expect("selection should work");

        assert_eq!(selected.metadata.client_cn, latest.metadata.client_cn);
    }

    #[test]
    fn enrollment_selector_matches_name_and_session_id() {
        let enrollment = StoredEnrollment {
            metadata: EnrollmentMetadata {
                session_name: "session-a".to_string(),
                session_id: 42,
                session_description: "desc".to_string(),
                owner: "owner".to_string(),
                labels: vec!["alpha".to_string()],
                visibility: "private".to_string(),
                purpose: "testing".to_string(),
                access: "read".to_string(),
                client_cn: "client-1".to_string(),
                mtls_endpoint: "https://localhost:1338".to_string(),
                enrolled_at: 1,
                client_cert_expires_at: 4_102_444_800,
            },
            directory: PathBuf::from("ignored"),
        };

        assert!(enrollment_matches_selector(&enrollment, "session-a"));
        assert!(enrollment_matches_selector(&enrollment, "42"));
        assert!(!enrollment_matches_selector(&enrollment, "session-b"));
    }

    #[test]
    fn delete_session_enrollments_removes_all_matching_directories() {
        let base_dir = std::env::temp_dir().join(format!(
            "ccp-client-tests-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos()
        ));
        let session_a_read = EnrollmentMaterial {
            metadata: EnrollmentMetadata {
                session_name: "session-a".to_string(),
                session_id: 7,
                session_description: "desc".to_string(),
                owner: "owner".to_string(),
                labels: vec!["alpha".to_string()],
                visibility: "private".to_string(),
                purpose: "testing".to_string(),
                access: "read".to_string(),
                client_cn: "client-1".to_string(),
                mtls_endpoint: "https://localhost:1338".to_string(),
                enrolled_at: 99,
                client_cert_expires_at: 4_102_444_800,
            },
            ca_pem: "ca".to_string(),
            client_cert_pem: "cert".to_string(),
            client_key_pem: "key".to_string(),
        };
        let session_a_write = EnrollmentMaterial {
            metadata: EnrollmentMetadata {
                access: "read_write".to_string(),
                client_cn: "client-2".to_string(),
                ..session_a_read.metadata.clone()
            },
            ca_pem: "ca".to_string(),
            client_cert_pem: "cert".to_string(),
            client_key_pem: "key".to_string(),
        };
        let session_b = EnrollmentMaterial {
            metadata: EnrollmentMetadata {
                session_name: "session-b".to_string(),
                session_id: 8,
                client_cn: "client-3".to_string(),
                ..session_a_read.metadata.clone()
            },
            ca_pem: "ca".to_string(),
            client_cert_pem: "cert".to_string(),
            client_key_pem: "key".to_string(),
        };

        save_enrollment_to_dir(&session_a_read, &base_dir).expect("session-a read should save");
        save_enrollment_to_dir(&session_a_write, &base_dir).expect("session-a write should save");
        save_enrollment_to_dir(&session_b, &base_dir).expect("session-b should save");

        let removed = delete_session_enrollments_from_dir(&base_dir, "session-a")
            .expect("delete should work");
        let remaining = load_enrollments_from_dir(&base_dir).expect("load should work");

        assert_eq!(removed, 2);
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].metadata.session_name, "session-b");

        fs::remove_dir_all(base_dir).expect("temp directory should be removable");
    }

    #[test]
    fn admin_enrollment_is_selected_for_write_operations() {
        let admin = StoredEnrollment {
            metadata: EnrollmentMetadata {
                session_name: "session-a".to_string(),
                session_id: 1,
                session_description: "desc".to_string(),
                owner: "owner".to_string(),
                labels: vec![],
                visibility: "private".to_string(),
                purpose: "testing".to_string(),
                access: "admin".to_string(),
                client_cn: "admin-client".to_string(),
                mtls_endpoint: "https://localhost:1338".to_string(),
                enrolled_at: 10,
                client_cert_expires_at: 4_102_444_800,
            },
            directory: PathBuf::from("ignored"),
        };

        let selected = select_enrollment_from_enrollments(&[admin], "session-a", true)
            .expect("admin enrollment should be selectable for write");
        assert_eq!(selected.metadata.access, "admin");
    }

    fn fixture(name: &str, id: i64, endpoint: &str) -> EnrollmentMaterial {
        EnrollmentMaterial {
            metadata: serde_json::from_value(serde_json::json!({
                "session_name": name, "session_id": id, "access": "read_write",
                "client_cn": "plaintext-client", "mtls_endpoint": endpoint,
                "client_cert_expires_at": u64::MAX, "enrolled_at": 1
            }))
            .unwrap(),
            ca_pem: String::new(),
            client_cert_pem: String::new(),
            client_key_pem: String::new(),
        }
    }

    #[test]
    fn subscriptions_preserve_server_and_session_identity_and_update_atomically() {
        let base_dir = std::env::temp_dir().join(format!(
            "ccp-client-identity-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let first = fixture("alpha/beta", 1, "http://localhost:1338");
        let second = fixture("alpha/beta", 1, "http://localhost:1339");
        let third = fixture("alpha-beta", 2, "http://localhost:1338");
        let saved = save_enrollment_to_dir(&first, &base_dir).unwrap();
        save_enrollment_to_dir(&second, &base_dir).unwrap();
        save_enrollment_to_dir(&third, &base_dir).unwrap();
        assert_eq!(load_enrollments_from_dir(&base_dir).unwrap().len(), 3);
        let updated = EnrollmentMaterial {
            metadata: EnrollmentMetadata {
                session_name: "renamed".into(),
                session_description: "updated".into(),
                ..first.metadata.clone()
            },
            ..first
        };
        let replacement = save_enrollment_to_dir(&updated, &base_dir).unwrap();
        assert_eq!(replacement.directory, saved.directory);
        let loaded = load_enrollments_from_dir(&base_dir).unwrap();
        assert_eq!(loaded.len(), 3);
        assert!(
            loaded
                .iter()
                .any(|record| record.metadata == updated.metadata)
        );
        assert_eq!(fs::read_dir(&saved.directory).unwrap().count(), 1);
        fs::remove_dir_all(base_dir).unwrap();
    }

    #[test]
    fn legacy_records_remain_readable_and_endpoint_selectors_avoid_ambiguity() {
        let base_dir = std::env::temp_dir().join(format!(
            "ccp-client-legacy-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let first = fixture("topic", 42, "http://localhost:1338");
        let legacy = base_dir.join("topic--read_write--plaintext-client");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(
            legacy.join("metadata.json"),
            serde_json::to_vec(&first.metadata).unwrap(),
        )
        .unwrap();
        save_enrollment_to_dir(&fixture("topic", 42, "http://localhost:1339"), &base_dir).unwrap();
        let loaded = load_enrollments_from_dir(&base_dir).unwrap();
        assert!(
            select_enrollment_from_enrollments(&loaded, "topic", false)
                .unwrap_err()
                .to_string()
                .contains("ambiguous")
        );
        let selected =
            select_enrollment_from_enrollments(&loaded, "42@http://localhost:1338/", true).unwrap();
        assert_eq!(selected.directory, legacy);
        assert!(delete_session_enrollments_from_dir(&base_dir, "topic").is_err());
        assert_eq!(load_enrollments_from_dir(&base_dir).unwrap().len(), 2);
        assert_eq!(
            delete_session_enrollments_from_dir(&base_dir, "42@http://localhost:1338").unwrap(),
            1
        );
        assert_eq!(
            load_enrollments_from_dir(&base_dir).unwrap()[0]
                .metadata
                .mtls_endpoint,
            "http://localhost:1339"
        );
        fs::remove_dir_all(base_dir).unwrap();
    }

    #[test]
    fn summaries_use_latest_metadata_for_server_session_identity() {
        let older = StoredEnrollment {
            metadata: fixture("old-name", 42, "http://LOCALHOST:80/").metadata,
            directory: "legacy".into(),
        };
        let latest = StoredEnrollment {
            metadata: EnrollmentMetadata {
                session_name: "new-name".into(),
                session_description: "new description".into(),
                enrolled_at: 2,
                ..fixture("unused", 42, "http://localhost").metadata
            },
            directory: "subscription--new".into(),
        };
        let summaries = summarize_sessions(&[older, latest]);
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].session_name, "new-name");
        assert_eq!(summaries[0].session_description, "new description");
        assert_eq!(summaries[0].enrollment_count, 2);
    }
}
