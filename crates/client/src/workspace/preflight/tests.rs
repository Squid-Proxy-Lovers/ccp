// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use crate::workspace::tests::{authority, directory, trust};
use rusqlite::Connection;
use serde_json::json;
use std::{collections::BTreeMap, fs};

fn snapshot(path: &Path) -> BTreeMap<String, Vec<u8>> {
    fs::read_dir(path)
        .unwrap()
        .map(|item| {
            let item = item.unwrap();
            (
                item.file_name().to_str().unwrap().to_owned(),
                fs::read(item.path()).unwrap(),
            )
        })
        .collect()
}

fn remove_status(db: &Connection) {
    db.execute_batch("DROP TRIGGER status_pending_excludes_other; DROP TRIGGER content_pending_excludes_status; DROP TRIGGER lifecycle_pending_excludes_status; DROP TABLE status_state;").unwrap();
}

#[test]
fn preflight_inspects_old_profile_schemas_without_migration_backup_or_byte_changes() {
    let (ca, _) = authority();
    let trust = trust(&ca);
    for version in 1..=5 {
        let path = directory();
        store::create(&path, &trust).unwrap();
        let db = Connection::open(path.join("workspace-profile.sqlite3")).unwrap();
        if version < 5 {
            remove_status(&db);
        }
        if version < 4 {
            db.execute_batch("DROP TABLE content_state").unwrap();
        }
        db.pragma_update(None, "user_version", version).unwrap();
        drop(db);
        let before = snapshot(&path);
        let (observed, format, pending) = store::inspect_update(&path).unwrap();
        assert_eq!(observed.instance_id, trust.instance_id);
        assert_eq!(format, version);
        assert!(!pending);
        assert_eq!(snapshot(&path), before);
        assert_eq!(before.len(), 1);
        fs::remove_dir_all(path).unwrap();
    }
}

#[test]
fn preflight_reports_each_pending_slot_without_decoding_or_exposing_its_body() {
    let (ca, _) = authority();
    for query in [
        "UPDATE profile SET pending='PRIVATE_PENDING_SENTINEL'",
        "UPDATE content_state SET pending='PRIVATE_PENDING_SENTINEL'",
        "UPDATE status_state SET pending='PRIVATE_PENDING_SENTINEL'",
    ] {
        let path = directory();
        store::create(&path, &trust(&ca)).unwrap();
        let db = Connection::open(path.join("workspace-profile.sqlite3")).unwrap();
        db.execute_batch(query).unwrap();
        drop(db);
        let before = snapshot(&path);
        assert!(store::inspect_update(&path).unwrap().2);
        assert_eq!(snapshot(&path), before);
        fs::remove_dir_all(path).unwrap();
    }
}

#[test]
fn preflight_refuses_missing_busy_unsupported_and_corrupt_state_without_repair() {
    let (ca, _) = authority();
    let missing = directory();
    assert!(store::inspect_update(&missing).is_err());
    assert!(!missing.exists());
    assert!(store::inspect_update(Path::new("relative-profile")).is_err());
    for query in [
        "PRAGMA user_version=99",
        "PRAGMA application_id=0",
        "UPDATE profile SET trust='PRIVATE_TRUST_SENTINEL'",
        "DROP TABLE status_state",
        "PRAGMA journal_mode=WAL",
    ] {
        let path = directory();
        store::create(&path, &trust(&ca)).unwrap();
        Connection::open(path.join("workspace-profile.sqlite3"))
            .unwrap()
            .execute_batch(query)
            .unwrap();
        let before = snapshot(&path);
        let error = store::inspect_update(&path).err().unwrap().to_string();
        assert!(!error.contains("PRIVATE_TRUST_SENTINEL"));
        assert_eq!(snapshot(&path), before);
        fs::remove_dir_all(path).unwrap();
    }
    let path = directory();
    store::create(&path, &trust(&ca)).unwrap();
    let db = Connection::open(path.join("workspace-profile.sqlite3")).unwrap();
    db.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let before = snapshot(&path);
    assert!(store::inspect_update(&path).is_err());
    assert_eq!(snapshot(&path), before);
    db.execute_batch("ROLLBACK").unwrap();
    drop(db);
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn preflight_leaves_existing_journal_sidecars_untouched_and_refuses_before_sqlite_open() {
    let (ca, _) = authority();
    for suffix in ["-wal", "-shm", "-journal"] {
        let path = directory();
        store::create(&path, &trust(&ca)).unwrap();
        fs::write(
            path.join(format!("workspace-profile.sqlite3{suffix}")),
            b"KEEP_SIDECAR",
        )
        .unwrap();
        let before = snapshot(&path);
        assert!(store::inspect_update(&path).is_err());
        assert_eq!(snapshot(&path), before);
        fs::remove_dir_all(path).unwrap();
    }
}

#[tokio::test]
async fn preflight_metadata_decoder_refuses_unknown_endpoint_invalid_envelope_and_schema() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    // Isolate only the existing bounded JSON decoder; production uses pinned HTTPS.
    for (status, body) in [
        ("404 Not Found", "PRIVATE_OLD_SERVER"),
        (
            "200 OK",
            r#"{"api_version":99,"data":{"instance_id":"a","package_version":"0.1.0","native_wire":1,"workspace_api":1}}"#,
        ),
        ("200 OK", r#"{"api_version":1,"data":{"instance_id":"a"}}"#),
        (
            "200 OK",
            r#"{"api_version":1,"data":{"instance_id":"a","package_version":"0.1.0","native_wire":true,"workspace_api":1}}"#,
        ),
        (
            "200 OK",
            r#"{"api_version":1,"data":{"instance_id":"a","package_version":"0.1.0","native_wire":1,"workspace_api":1,"unknown":true}}"#,
        ),
        ("200 OK", "PRIVATE_MALFORMED_BODY"),
    ] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            assert!(stream.read(&mut request).await.unwrap() > 0);
            let reply = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(reply.as_bytes()).await.unwrap();
        });
        let http = reqwest::Client::builder().no_proxy().build().unwrap();
        let result: anyhow::Result<api::SoftwareInfo> =
            network::json(http.get(format!("http://{address}/"))).await;
        let error = result.err().unwrap().to_string();
        assert!(!error.contains("PRIVATE_"));
        task.await.unwrap();
    }
}

#[test]
fn preflight_requires_strict_metadata_and_matching_instance_wire_and_api() {
    let instance = uuid::Uuid::new_v4().to_string();
    let good =
        json!({"instance_id":instance,"package_version":"0.1.0","native_wire":1,"workspace_api":1});
    let parsed: api::SoftwareInfo = serde_json::from_value(good.clone()).unwrap();
    assert!(validate(&parsed, &instance).is_ok());
    for (field, value) in [
        ("instance_id", json!(uuid::Uuid::new_v4().to_string())),
        ("native_wire", json!(99)),
        ("workspace_api", json!(99)),
        ("package_version", json!("")),
        ("package_version", json!("x".repeat(65))),
        ("package_version", json!("bad\nversion")),
        ("package_version", json!(" 0.1.0")),
        ("package_version", json!("非ASCII")),
    ] {
        let mut value_row = good.clone();
        value_row[field] = value;
        assert!(validate(&serde_json::from_value(value_row).unwrap(), &instance).is_err());
    }
    for value in [
        json!({"instance_id":instance,"native_wire":1,"workspace_api":1}),
        json!({"instance_id":instance,"package_version":"0.1.0","native_wire":true,"workspace_api":1}),
        json!({"instance_id":instance,"package_version":"0.1.0","native_wire":1,"workspace_api":1,"unknown":true}),
    ] {
        assert!(serde_json::from_value::<api::SoftwareInfo>(value).is_err());
    }
}

#[cfg(unix)]
#[test]
fn preflight_refuses_symlinks_and_loose_private_metadata_without_changing_them() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let (ca, _) = authority();
    let path = directory();
    store::create(&path, &trust(&ca)).unwrap();
    let alias = directory();
    symlink(&path, &alias).unwrap();
    assert!(store::inspect_update(&alias).is_err());
    fs::remove_file(alias).unwrap();
    let database = path.join("workspace-profile.sqlite3");
    let before = snapshot(&path);
    fs::set_permissions(&database, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(store::inspect_update(&path).is_err());
    assert_eq!(snapshot(&path), before);
    fs::set_permissions(&database, fs::Permissions::from_mode(0o600)).unwrap();
    let saved = path.join("saved.sqlite3");
    fs::rename(&database, &saved).unwrap();
    symlink(&saved, &database).unwrap();
    assert!(store::inspect_update(&path).is_err());
    assert!(database.is_symlink());
    assert_eq!(
        fs::read(saved).unwrap(),
        before["workspace-profile.sqlite3"]
    );
    fs::remove_dir_all(path).unwrap();
}
