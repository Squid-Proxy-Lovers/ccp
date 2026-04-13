// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, bail};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

use crate::enrollment_structs::{
    EnrollmentMaterial, EnrollmentMetadata, SessionSummary, StoredEnrollment,
};

const CLIENT_HOME_ENV: &str = "CCP_CLIENT_HOME";
const DEFAULT_CLIENT_HOME_DIR: &str = ".ccp-client";
const ENROLLMENTS_DIR_NAME: &str = "enrollments";
const DEFAULT_CERT_WARNING_WINDOW_SECONDS: u64 = 0;

pub(crate) fn save_enrollment(material: &EnrollmentMaterial) -> anyhow::Result<StoredEnrollment> {
    let base_dir = enrollments_dir()?;
    save_enrollment_to_dir(material, &base_dir)
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
    select_enrollment_from_enrollments(&enrollments, session_selector, require_write)
}

pub(crate) fn delete_session_enrollments(session_selector: &str) -> anyhow::Result<usize> {
    let base_dir = enrollments_dir()?;
    delete_session_enrollments_from_dir(&base_dir, session_selector)
}

pub(crate) fn summarize_sessions(enrollments: &[StoredEnrollment]) -> Vec<SessionSummary> {
    let mut sessions = BTreeMap::new();

    for enrollment in enrollments {
        let key = (
            enrollment.metadata.session_name.clone(),
            enrollment.metadata.session_id,
            enrollment.metadata.mtls_endpoint.clone(),
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

    let directory = base_dir.join(format!(
        "{}--{}--{}",
        sanitize(&material.metadata.session_name),
        sanitize(&material.metadata.access),
        sanitize(&material.metadata.client_cn)
    ));
    fs::create_dir_all(&directory)
        .with_context(|| format!("failed to create {}", directory.display()))?;

    fs::write(
        directory.join("metadata.json"),
        serde_json::to_vec_pretty(&material.metadata)?,
    )
    .with_context(|| {
        format!(
            "failed to write {}",
            directory.join("metadata.json").display()
        )
    })?;
    fs::write(directory.join("ca.pem"), material.ca_pem.as_bytes())
        .with_context(|| format!("failed to write {}", directory.join("ca.pem").display()))?;
    fs::write(
        directory.join("client.pem"),
        material.client_cert_pem.as_bytes(),
    )
    .with_context(|| format!("failed to write {}", directory.join("client.pem").display()))?;
    write_private_file(
        &directory.join("client.key"),
        material.client_key_pem.as_bytes(),
    )?;

    let mut identity_pem = material.client_cert_pem.clone();
    identity_pem.push_str(&material.client_key_pem);
    write_private_file(&directory.join("identity.pem"), identity_pem.as_bytes())?;

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
    let matching_directories = enrollments
        .into_iter()
        .filter(|enrollment| enrollment_matches_selector(enrollment, session_selector))
        .map(|enrollment| enrollment.directory)
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
    let mut candidates = enrollments
        .iter()
        .filter(|enrollment| enrollment_matches_selector(enrollment, session_selector))
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

    candidates.sort_by_key(|enrollment| enrollment.metadata.enrolled_at);
    Ok(candidates.pop().expect("candidate list is not empty"))
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

fn write_private_file(path: &Path, contents: &[u8]) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(path)
            .with_context(|| format!("failed to open {}", path.display()))?;
        file.write_all(contents)
            .with_context(|| format!("failed to write {}", path.display()))?;
        file.flush()?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        fs::write(path, contents).with_context(|| format!("failed to write {}", path.display()))
    }
}

fn sanitize(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect()
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
    fn sanitize_replaces_non_filesystem_chars() {
        assert_eq!(sanitize("alpha/beta gamma"), "alpha-beta-gamma");
    }

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

    #[test]
    fn private_key_files_have_restricted_permissions() {
        let base_dir = std::env::temp_dir().join(format!(
            "ccp-perms-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let material = EnrollmentMaterial {
            metadata: EnrollmentMetadata {
                session_name: "perms-test".to_string(),
                session_id: 99,
                session_description: "".to_string(),
                owner: "".to_string(),
                labels: vec![],
                visibility: "private".to_string(),
                purpose: "".to_string(),
                access: "read".to_string(),
                client_cn: "cn".to_string(),
                mtls_endpoint: "https://localhost:1338".to_string(),
                enrolled_at: 1,
                client_cert_expires_at: 4_102_444_800,
            },
            ca_pem: "ca".to_string(),
            client_cert_pem: "cert".to_string(),
            client_key_pem: "key".to_string(),
        };

        let stored = save_enrollment_to_dir(&material, &base_dir).expect("save should work");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let key_mode = fs::metadata(stored.directory.join("client.key"))
                .expect("client.key should exist")
                .permissions()
                .mode()
                & 0o777;
            let id_mode = fs::metadata(stored.directory.join("identity.pem"))
                .expect("identity.pem should exist")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(key_mode, 0o600, "client.key should be owner-only");
            assert_eq!(id_mode, 0o600, "identity.pem should be owner-only");
        }

        fs::remove_dir_all(base_dir).expect("cleanup");
    }
}
