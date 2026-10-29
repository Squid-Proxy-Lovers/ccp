// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;
fn root() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("ccp-reservation-{}", Uuid::new_v4()))
}
fn tree(directory: &Path) -> Vec<(String, Vec<u8>)> {
    let mut entries: Vec<_> = fs::read_dir(directory)
        .unwrap()
        .map(|e| {
            let e = e.unwrap();
            (
                e.file_name().to_string_lossy().to_string(),
                fs::read(e.path()).unwrap(),
            )
        })
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    entries
}
#[test]
fn empty_reserves_one_instance_without_any_owner_and_restarts_preserve_it() {
    let directory = root();
    let state = prepare(&directory).unwrap();
    assert!(matches!(state, State::Pending(_)));
    let before = tree(&directory);
    assert_eq!(prepare(&directory).unwrap(), state);
    assert_eq!(tree(&directory), before);
    let db = Connection::open(directory.join("ccp.sqlite3")).unwrap();
    for table in [
        "ccp_admin_principals",
        "ccp_server_admins",
        "ccp_workspaces",
        "ccp_admin_keys",
    ] {
        assert_eq!(
            db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                .get::<_, u64>(0))
                .unwrap(),
            0
        );
    }
    assert_eq!(
        db.query_row("SELECT mode FROM ccp_runtime_mode", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "workspace"
    );
    drop(db);
    assert!(crate::directory_mode::DirectoryLease::native(&directory).is_err());
    fs::remove_dir_all(directory).unwrap();
}
#[test]
fn stray_native_corrupt_and_unreserved_inputs_are_refused_without_mutation() {
    for kind in ["stray", "native", "corrupt", "unreserved"] {
        let directory = root();
        fs::create_dir(&directory).unwrap();
        match kind {
            "stray" => fs::write(directory.join("notes.txt"), b"preserve me").unwrap(),
            "corrupt" => fs::write(directory.join("ccp.sqlite3"), b"not sqlite").unwrap(),
            _ => {
                crate::init::init_sqlite(&directory.join("ccp.sqlite3")).unwrap();
                if kind == "unreserved" {
                    Connection::open(directory.join("ccp.sqlite3"))
                        .unwrap()
                        .execute("UPDATE ccp_runtime_mode SET mode='workspace'", [])
                        .unwrap();
                }
            }
        }
        let before = tree(&directory);
        assert!(prepare(&directory).is_err());
        assert_eq!(before, tree(&directory));
        fs::remove_dir_all(directory).unwrap();
    }
}
#[test]
fn existing_initialized_workspace_is_observed_without_new_identity_or_keys() {
    let directory = root();
    let setup = crate::local_admin::bootstrap(&directory, "owner", "workspace").unwrap();
    let lease = crate::directory_mode::DirectoryLease::activate_workspace(&directory).unwrap();
    drop(lease);
    let before = tree(&directory);
    assert_eq!(
        prepare(&directory).unwrap(),
        State::Initialized(setup.instance_id)
    );
    assert_eq!(before, tree(&directory));
    fs::remove_dir_all(directory).unwrap();
}
#[test]
fn unpublished_staging_is_not_accepted_as_a_fresh_or_live_database() {
    let directory = root();
    fs::create_dir(&directory).unwrap();
    fs::write(directory.join(".ccp-setup-orphan.sqlite3"), b"incomplete").unwrap();
    let before = tree(&directory);
    assert!(prepare(&directory).is_err());
    assert_eq!(before, tree(&directory));
    fs::remove_dir_all(directory).unwrap();
}
#[cfg(unix)]
#[test]
fn symlinked_directory_or_database_is_never_adopted() {
    use std::os::unix::fs::symlink;
    let target = root();
    prepare(&target).unwrap();
    let before = tree(&target);
    let link = root();
    symlink(&target, &link).unwrap();
    assert!(prepare(&link).is_err());
    fs::remove_file(link).unwrap();
    let directory = root();
    fs::create_dir(&directory).unwrap();
    symlink(target.join("ccp.sqlite3"), directory.join("ccp.sqlite3")).unwrap();
    assert!(prepare(&directory).is_err());
    assert_eq!(before, tree(&target));
    fs::remove_dir_all(directory).unwrap();
    fs::remove_dir_all(target).unwrap();
}

#[test]
fn reserved_tls_is_stable_without_an_owner_and_legacy_init_stays_owner_first() {
    let directory = root();
    let state = prepare(&directory).unwrap();
    assert!(
        crate::instance_tls::initialize(&directory, &["localhost".into()])
            .unwrap_err()
            .to_string()
            .contains("administration")
    );
    let first = provision(&directory, &["localhost".into(), "127.0.0.1".into()]).unwrap();
    assert_eq!(
        State::Pending(Uuid::parse_str(&first.instance_id).unwrap()),
        state
    );
    let next = provision(&directory, &["127.0.0.1".into(), "localhost".into()]).unwrap();
    assert_eq!(first, next);
    let lease = crate::directory_mode::DirectoryLease::activate_workspace(&directory).unwrap();
    let identity = crate::instance_tls::InstanceTls::load(&lease).unwrap();
    assert_eq!(identity.trust(), &first);
    let db = Connection::open(directory.join("ccp.sqlite3")).unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM ccp_server_admins", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM ccp_admin_keys", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        0
    );
    drop(db);
    drop(identity);
    drop(lease);
    let before = tree(&directory);
    assert!(provision(&directory, &["different.example".into()]).is_err());
    // Lease revision can advance, but no identity material or database pin is replaced.
    let after = tree(&directory);
    for (name, bytes) in before
        .into_iter()
        .filter(|(name, _)| name != "ccp-directory-mode.sqlite3")
    {
        assert_eq!(after.iter().find(|(n, _)| *n == name).unwrap().1, bytes);
    }
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn simultaneous_reservations_never_publish_different_active_instances() {
    use std::sync::{Arc, Barrier};
    let directory = root();
    fs::create_dir(&directory).unwrap();
    let barrier = Arc::new(Barrier::new(8));
    let tasks: Vec<_> = (0..8)
        .map(|_| {
            let directory = directory.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                prepare(&directory)
            })
        })
        .collect();
    let expected = tasks
        .into_iter()
        .filter_map(|task| task.join().unwrap().ok())
        .collect::<Vec<_>>();
    assert!(!expected.is_empty());
    assert!(expected.iter().all(|state| *state == expected[0]));
    assert_eq!(prepare(&directory).unwrap(), expected[0]);
    fs::remove_dir_all(directory).unwrap();
}
#[test]
fn failed_publication_never_overwrites_a_newly_arrived_foreign_database() {
    let directory = root();
    fs::create_dir(&directory).unwrap();
    let staging = directory.join("owned-staging.sqlite3");
    File::create(&staging).unwrap();
    fs::write(
        directory.join("ccp.sqlite3"),
        b"foreign data arriving during setup",
    )
    .unwrap();
    assert!(publish(&directory, &staging).is_err());
    assert_eq!(
        fs::read(directory.join("ccp.sqlite3")).unwrap(),
        b"foreign data arriving during setup"
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn explicit_local_workspace_init_can_be_finished_but_native_journals_stay_untouched() {
    let directory = root();
    let setup = crate::local_admin::bootstrap(&directory, "owner", "first").unwrap();
    let before = tree(&directory);
    assert_eq!(
        prepare(&directory).unwrap(),
        State::Initialized(setup.instance_id)
    );
    assert_eq!(before, tree(&directory));
    fs::write(
        directory.join("runtime-journal.jsonl"),
        b"unrecovered native data",
    )
    .unwrap();
    let before = tree(&directory);
    assert!(prepare(&directory).is_err());
    assert_eq!(before, tree(&directory));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn promotion_requires_unique_maintenance_lease_and_keeps_native_callers_out() {
    let directory = root();
    crate::local_admin::bootstrap(&directory, "owner", "first").unwrap();
    let lease = crate::directory_mode::DirectoryLease::maintenance(&directory).unwrap();
    let extra = lease.clone();
    assert!(lease.promote_workspace().is_err());
    assert_eq!(extra.mode(), crate::directory_mode::DirectoryMode::Native);
    drop(extra);
    let lease = crate::directory_mode::DirectoryLease::maintenance(&directory).unwrap();
    assert!(crate::directory_mode::DirectoryLease::native(&directory).is_err());
    let lease = lease.promote_workspace().unwrap();
    assert!(lease.allows_workspace());
    assert_eq!(
        lease.mode(),
        crate::directory_mode::DirectoryMode::Workspace
    );
    assert!(crate::directory_mode::DirectoryLease::native(&directory).is_err());
    assert!(crate::instance_tls::initialize_setup_leased(&lease, &[]).is_err());
    drop(lease);
    assert!(crate::directory_mode::DirectoryLease::native(&directory).is_err());
    fs::remove_dir_all(directory).unwrap();
}
