// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use uuid::Uuid;

fn directory() -> PathBuf {
    let path = std::env::temp_dir().join(format!("ccp-directory-mode-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn conflict(result: anyhow::Result<DirectoryLease>, expected: DirectoryConflict) {
    let error = result.err().expect("operation should be refused");
    assert_eq!(
        error.downcast_ref::<DirectoryConflict>(),
        Some(&expected),
        "{error:#}"
    );
}

struct Probe {
    input: ChildStdin,
    child: Child,
}

impl Probe {
    fn start(directory: &Path, kind: &str) -> Self {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "directory_mode::tests::lease_child",
                "--nocapture",
            ])
            .env("CCP_TEST_MODE_DIRECTORY", directory)
            .env("CCP_TEST_MODE_KIND", kind)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (send, receive) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut ready = false;
            for line in BufReader::new(stdout).lines() {
                if line.unwrap() == "CCP_MODE_READY" && !ready {
                    send.send(()).unwrap();
                    ready = true;
                }
            }
        });
        let input = child.stdin.take().unwrap();
        receive
            .recv_timeout(Duration::from_secs(10))
            .expect("child should acquire its lease");
        Self { input, child }
    }

    fn stop(mut self) {
        self.input.write_all(b"stop\n").unwrap();
        self.input.flush().unwrap();
        assert!(self.child.wait().unwrap().success());
    }

    fn crash(mut self) {
        self.child.kill().unwrap();
        assert!(!self.child.wait().unwrap().success());
    }
}

// A normal test with no environment input is a no-op. Parent cases run this
// exact test in a fresh process, so kernel locks and crash cleanup are real.
#[test]
fn lease_child() {
    if std::env::var_os("CCP_TEST_NATIVE_ACTION").is_some() {
        native_entry_child();
        return;
    }
    if std::env::var_os("CCP_TEST_DISCOVERY_HOME").is_some() {
        discovery_child();
        return;
    }
    let Some(directory) = std::env::var_os("CCP_TEST_MODE_DIRECTORY") else {
        return;
    };
    let kind = std::env::var("CCP_TEST_MODE_KIND").unwrap();
    let path = PathBuf::from(directory);
    let lease = match kind.as_str() {
        "native" => DirectoryLease::native(&path),
        "workspace" => DirectoryLease::activate_workspace(&path),
        "maintenance" => DirectoryLease::maintenance(&path),
        _ => panic!("unknown child mode"),
    }
    .unwrap();
    println!("\nCCP_MODE_READY");
    std::io::stdout().flush().unwrap();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).unwrap();
    drop(lease);
}

#[test]
fn native_leases_share_and_all_block_exclusive_operations() {
    let dir = directory();
    let first = DirectoryLease::native(&dir).unwrap();
    let second = DirectoryLease::native(&dir).unwrap();
    let clone = first.clone();
    assert_eq!(first.mode(), DirectoryMode::Native);
    conflict(DirectoryLease::maintenance(&dir), DirectoryConflict::Busy);
    conflict(
        DirectoryLease::activate_workspace(&dir),
        DirectoryConflict::Busy,
    );
    drop(first);
    drop(second);
    conflict(DirectoryLease::maintenance(&dir), DirectoryConflict::Busy);
    drop(clone);
    let maintenance = DirectoryLease::maintenance(&dir).unwrap();
    assert_eq!(maintenance.mode(), DirectoryMode::Native);
    conflict(DirectoryLease::native(&dir), DirectoryConflict::Busy);
    drop(maintenance);
    drop(DirectoryLease::native(&dir).unwrap());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn workspace_activation_is_exclusive_and_persists_after_release() {
    let dir = directory();
    let workspace = DirectoryLease::activate_workspace(&dir).unwrap();
    assert_eq!(workspace.mode(), DirectoryMode::Workspace);
    conflict(DirectoryLease::native(&dir), DirectoryConflict::Busy);
    conflict(
        DirectoryLease::activate_workspace(&dir),
        DirectoryConflict::Busy,
    );
    conflict(DirectoryLease::maintenance(&dir), DirectoryConflict::Busy);
    drop(workspace);
    conflict(
        DirectoryLease::native(&dir),
        DirectoryConflict::WorkspaceActivated,
    );
    let maintenance = DirectoryLease::maintenance(&dir).unwrap();
    assert_eq!(maintenance.mode(), DirectoryMode::Workspace);
    drop(maintenance);
    conflict(
        DirectoryLease::native(&dir),
        DirectoryConflict::WorkspaceActivated,
    );
    drop(DirectoryLease::activate_workspace(&dir).unwrap());
    let database = Connection::open(dir.join("ccp.sqlite3")).unwrap();
    let version: u32 = database
        .query_row("SELECT MAX(version) FROM schema_version", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(version, crate::init::SCHEMA_VERSION);
    drop(database);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn separate_native_processes_share_until_the_last_process_exits() {
    let dir = directory();
    let first = Probe::start(&dir, "native");
    let second = Probe::start(&dir, "native");
    conflict(DirectoryLease::maintenance(&dir), DirectoryConflict::Busy);
    first.stop();
    conflict(
        DirectoryLease::activate_workspace(&dir),
        DirectoryConflict::Busy,
    );
    second.stop();
    drop(DirectoryLease::activate_workspace(&dir).unwrap());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn crashed_native_process_releases_its_kernel_lease() {
    let dir = directory();
    let native = Probe::start(&dir, "native");
    conflict(DirectoryLease::maintenance(&dir), DirectoryConflict::Busy);
    native.crash();
    drop(DirectoryLease::maintenance(&dir).unwrap());
    drop(DirectoryLease::native(&dir).unwrap());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn crashed_workspace_process_releases_lock_but_never_clears_activation() {
    let dir = directory();
    let workspace = Probe::start(&dir, "workspace");
    conflict(DirectoryLease::native(&dir), DirectoryConflict::Busy);
    workspace.crash();
    conflict(
        DirectoryLease::native(&dir),
        DirectoryConflict::WorkspaceActivated,
    );
    drop(DirectoryLease::activate_workspace(&dir).unwrap());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn child_maintenance_blocks_native_without_activating_workspace() {
    let dir = directory();
    let maintenance = Probe::start(&dir, "maintenance");
    conflict(DirectoryLease::native(&dir), DirectoryConflict::Busy);
    maintenance.stop();
    drop(DirectoryLease::native(&dir).unwrap());
    assert!(!dir.join("ccp.sqlite3").exists());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn unknown_or_corrupted_mode_metadata_is_never_reinitialized() {
    for scenario in ["future", "foreign", "corrupt", "wal"] {
        let dir = directory();
        if scenario == "corrupt" {
            std::fs::write(dir.join(LEDGER_NAME), b"invalid SQLite mode metadata").unwrap();
        } else {
            drop(DirectoryLease::native(&dir).unwrap());
            let connection = Connection::open(dir.join(LEDGER_NAME)).unwrap();
            match scenario {
                "future" => connection
                    .pragma_update(None, "user_version", FORMAT_VERSION + 1)
                    .unwrap(),
                "foreign" => connection
                    .pragma_update(None, "application_id", 42)
                    .unwrap(),
                "wal" => connection
                    .pragma_update(None, "journal_mode", "WAL")
                    .unwrap(),
                _ => panic!("unknown scenario"),
            }
        }
        let prior = std::fs::read(dir.join(LEDGER_NAME)).unwrap();
        assert!(DirectoryLease::native(&dir).is_err(), "{scenario}");
        assert!(
            DirectoryLease::activate_workspace(&dir).is_err(),
            "{scenario}"
        );
        assert_eq!(
            std::fs::read(dir.join(LEDGER_NAME)).unwrap(),
            prior,
            "{scenario}"
        );
        assert!(!dir.join("ccp.sqlite3").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn unrecognized_foreign_database_is_not_claimed_as_an_empty_ledger() {
    let dir = directory();
    let connection = Connection::open(dir.join(LEDGER_NAME)).unwrap();
    connection
        .execute_batch("CREATE TABLE other_data(secret TEXT);")
        .unwrap();
    drop(connection);
    assert!(DirectoryLease::native(&dir).is_err());
    let connection = Connection::open(dir.join(LEDGER_NAME)).unwrap();
    assert!(
        !connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name = 'directory_mode')",
                [],
                |row| row.get::<_, bool>(0)
            )
            .unwrap()
    );
    drop(connection);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn dirty_legacy_journal_blocks_activation_and_preserves_native_recovery() {
    let dir = directory();
    std::fs::write(
        dir.join("runtime-journal.jsonl"),
        b"unrecovered legacy data\n",
    )
    .unwrap();
    let error = DirectoryLease::activate_workspace(&dir).err().unwrap();
    assert!(error.to_string().contains("lease data directory"));
    assert!(format!("{error:#}").contains("recover the legacy journal"));
    assert_eq!(
        std::fs::read(dir.join("runtime-journal.jsonl")).unwrap(),
        b"unrecovered legacy data\n"
    );
    assert!(!dir.join("ccp.sqlite3").exists());
    drop(DirectoryLease::native(&dir).unwrap());
    std::fs::write(dir.join("runtime-journal.jsonl"), []).unwrap();
    drop(DirectoryLease::activate_workspace(&dir).unwrap());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn journal_handle_and_writer_keep_the_native_lease_after_bootstrap_drops() {
    let dir = directory();
    let lease = DirectoryLease::native(&dir).unwrap();
    let journal = crate::journal::JournalHandle::start_guarded(
        dir.join("runtime-journal.jsonl"),
        lease.clone(),
    )
    .unwrap();
    drop(lease);
    conflict(DirectoryLease::maintenance(&dir), DirectoryConflict::Busy);
    journal.shutdown().unwrap();
    conflict(DirectoryLease::maintenance(&dir), DirectoryConflict::Busy);
    journal.truncate_blocking().unwrap();
    drop(journal);
    let limit = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        match DirectoryLease::maintenance(&dir) {
            Ok(lease) => {
                drop(lease);
                break;
            }
            Err(error)
                if error.downcast_ref::<DirectoryConflict>() == Some(&DirectoryConflict::Busy)
                    && std::time::Instant::now() < limit =>
            {
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("writer lease should release: {error:#}"),
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn guarded_journal_rejects_workspace_or_wrong_directory_guards() {
    let first = directory();
    let second = directory();
    let workspace = DirectoryLease::activate_workspace(&first).unwrap();
    assert!(
        crate::journal::JournalHandle::start_guarded(
            first.join("runtime-journal.jsonl"),
            workspace.clone()
        )
        .is_err()
    );
    let native = DirectoryLease::native(&second).unwrap();
    assert!(
        crate::journal::JournalHandle::start_guarded(
            first.join("runtime-journal.jsonl"),
            native.clone()
        )
        .is_err()
    );
    assert!(!first.join("runtime-journal.jsonl").exists());
    drop(native);
    drop(workspace);
    std::fs::remove_dir_all(first).unwrap();
    std::fs::remove_dir_all(second).unwrap();
}

#[cfg(unix)]
#[test]
fn canonical_directory_aliases_share_the_same_kernel_lease() {
    let dir = directory();
    let alias = dir.with_extension("alias");
    std::os::unix::fs::symlink(&dir, &alias).unwrap();
    let native = DirectoryLease::native(&alias).unwrap();
    assert_eq!(native.directory(), std::fs::canonicalize(&dir).unwrap());
    conflict(
        DirectoryLease::activate_workspace(&dir),
        DirectoryConflict::Busy,
    );
    drop(native);
    let workspace = DirectoryLease::activate_workspace(&dir).unwrap();
    conflict(DirectoryLease::native(&alias), DirectoryConflict::Busy);
    drop(workspace);
    conflict(
        DirectoryLease::native(&alias),
        DirectoryConflict::WorkspaceActivated,
    );
    std::fs::remove_file(alias).unwrap();
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn missing_control_ledger_never_weakens_the_data_activation_marker() {
    let dir = directory();
    drop(DirectoryLease::activate_workspace(&dir).unwrap());
    std::fs::remove_file(dir.join(LEDGER_NAME)).unwrap();
    conflict(
        DirectoryLease::native(&dir),
        DirectoryConflict::WorkspaceActivated,
    );
    let maintenance = DirectoryLease::maintenance(&dir).unwrap();
    assert_eq!(maintenance.mode(), DirectoryMode::Workspace);
    drop(maintenance);
    conflict(
        DirectoryLease::native(&dir),
        DirectoryConflict::WorkspaceActivated,
    );
    drop(DirectoryLease::activate_workspace(&dir).unwrap());
    let connection = Connection::open(dir.join("ccp.sqlite3")).unwrap();
    let count: u32 = connection
        .query_row(
            "SELECT COUNT(*) FROM ccp_access_audit WHERE action = 'activate_workspace_mode'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
    drop(connection);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn activation_audit_failure_keeps_native_mode_and_data_unchanged() {
    let dir = directory();
    crate::init::init_sqlite(&dir.join("ccp.sqlite3")).unwrap();
    let connection = Connection::open(dir.join("ccp.sqlite3")).unwrap();
    connection.execute_batch("CREATE TRIGGER fail_activation BEFORE INSERT ON ccp_access_audit WHEN NEW.action = 'activate_workspace_mode' BEGIN SELECT RAISE(ABORT, 'audit down'); END;").unwrap();
    drop(connection);
    assert!(DirectoryLease::activate_workspace(&dir).is_err());
    drop(DirectoryLease::native(&dir).unwrap());
    let connection = Connection::open(dir.join("ccp.sqlite3")).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT mode FROM ccp_runtime_mode", [], |row| row
                .get::<_, String>(0))
            .unwrap(),
        "native"
    );
    drop(connection);
    std::fs::remove_dir_all(dir).unwrap();
}

fn native_entry_child() {
    let Ok(action) = std::env::var("CCP_TEST_NATIVE_ACTION") else {
        return;
    };
    let result = match action.as_str() {
        "runtime" => tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(crate::run_server("guarded")),
        "startup" => tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(crate::init::initialize_cpp_server("guarded"))
            .map(drop),
        "health" => crate::init::check_server_health("guarded").map(drop),
        "token" => crate::init::issue_enrollment_token("guarded", "read", Some(3600)).map(drop),
        _ => panic!("unknown native action"),
    };
    let error = result.expect_err("native entry must refuse protected mode");
    let expected = if std::env::var("CCP_TEST_EXPECT_BUSY").unwrap() == "1" {
        DirectoryConflict::Busy
    } else {
        DirectoryConflict::WorkspaceActivated
    };
    assert_eq!(
        error.downcast_ref::<DirectoryConflict>(),
        Some(&expected),
        "{error:#}"
    );
}

#[test]
fn every_native_entry_refuses_before_sqlite_journal_or_key_operations() {
    for active in [false, true] {
        let dir = directory();
        let workspace = DirectoryLease::activate_workspace(&dir).unwrap();
        let held = if active {
            Some(workspace)
        } else {
            drop(workspace);
            None
        };
        std::fs::write(
            dir.join("ccp.sqlite3"),
            b"must not be opened as native SQLite",
        )
        .unwrap();
        std::fs::create_dir(dir.join("runtime-journal.jsonl")).unwrap();
        std::fs::write(
            dir.join("runtime-journal.jsonl/sentinel"),
            b"must not be replayed",
        )
        .unwrap();
        for action in ["runtime", "startup", "health", "token"] {
            let output = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "directory_mode::tests::lease_child",
                    "--nocapture",
                ])
                .env("CCP_TEST_NATIVE_ACTION", action)
                .env("CCP_TEST_EXPECT_BUSY", if active { "1" } else { "0" })
                .env(crate::init::SERVER_DATA_DIR_ENV, &dir)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{action}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        assert_eq!(
            std::fs::read(dir.join("ccp.sqlite3")).unwrap(),
            b"must not be opened as native SQLite"
        );
        assert_eq!(
            std::fs::read(dir.join("runtime-journal.jsonl/sentinel")).unwrap(),
            b"must not be replayed"
        );
        assert!(!dir.join("ccp_ca_key.pem").exists());
        assert!(!dir.join("auth_urls.txt").exists());
        drop(held);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

fn discovery_child() {
    let home = PathBuf::from(std::env::var_os("CCP_TEST_DISCOVERY_HOME").unwrap());
    let resolved = crate::init::config_session_dir("guarded").unwrap();
    assert_eq!(resolved, home.join(crate::init::session_slug("guarded")));
}

#[test]
fn legacy_discovery_skips_protected_sqlite_without_reading_its_sessions() {
    let home = directory();
    let dir = home.join("protected");
    let workspace = DirectoryLease::activate_workspace(&dir).unwrap();
    let connection = Connection::open(dir.join("ccp.sqlite3")).unwrap();
    connection
        .execute("INSERT INTO sessions(name) VALUES ('guarded')", [])
        .unwrap();
    drop(connection);
    let prior = std::fs::read(dir.join("ccp.sqlite3")).unwrap();
    assert_discovery(&home);
    drop(workspace);
    assert_discovery(&home);
    assert_eq!(std::fs::read(dir.join("ccp.sqlite3")).unwrap(), prior);
    std::fs::remove_dir_all(home).unwrap();
}

fn race(kind: LeaseKind) {
    let dir = directory();
    drop(DirectoryLease::native(&dir).unwrap());
    let mut acquired = 0;
    for round in 0..20 {
        acquired = race_round(&dir, kind, round > 0);
        if kind == LeaseKind::Native {
            break;
        }
        // Zero is valid for the prompt-refusal API when all initial exclusive
        // attempts meet transient SQLite header readers. Every round must still
        // exclude overlapping writers; only an all-Busy round may retry.
        assert!(acquired <= 1, "exclusive leases overlapped");
        if acquired == 1 {
            break;
        }
    }
    assert_eq!(acquired, if kind == LeaseKind::Native { 8 } else { 1 });
    std::fs::remove_dir_all(dir).unwrap();
}

fn race_round(dir: &Path, kind: LeaseKind, stagger: bool) -> usize {
    let barrier = Arc::new(std::sync::Barrier::new(8));
    let finished = Arc::new(std::sync::Barrier::new(8));
    let mut workers = Vec::new();
    for index in 0..8 {
        let path = dir.to_path_buf();
        let start = barrier.clone();
        let finished = finished.clone();
        workers.push(std::thread::spawn(move || {
            start.wait();
            if stagger {
                std::thread::sleep(Duration::from_millis(index * 2));
            }
            let result = match kind {
                LeaseKind::Native => DirectoryLease::native(&path),
                LeaseKind::Workspace => DirectoryLease::activate_workspace(&path),
                LeaseKind::Maintenance => DirectoryLease::maintenance(&path),
            };
            finished.wait();
            match result {
                Ok(lease) => {
                    drop(lease);
                    true
                }
                Err(error) => {
                    assert_eq!(
                        error.downcast_ref::<DirectoryConflict>(),
                        Some(&DirectoryConflict::Busy),
                        "{error:#}"
                    );
                    false
                }
            }
        }));
    }
    workers
        .into_iter()
        .map(|worker| usize::from(worker.join().unwrap()))
        .sum::<usize>()
}

#[test]
fn concurrent_shared_native_leases_all_succeed() {
    race(LeaseKind::Native);
}

#[test]
fn concurrent_exclusive_maintenance_leases_have_one_winner() {
    race(LeaseKind::Maintenance);
}

fn assert_discovery(home: &Path) {
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "directory_mode::tests::lease_child",
            "--nocapture",
        ])
        .env("CCP_TEST_DISCOVERY_HOME", home)
        .env(crate::init::SERVER_HOME_ENV, home)
        .env_remove(crate::init::SERVER_DATA_DIR_ENV)
        .current_dir(home)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn concurrent_workspace_activations_have_one_winner() {
    race(LeaseKind::Workspace);
}

#[test]
fn control_commit_failure_never_weakens_committed_data_activation() {
    let dir = directory();
    drop(DirectoryLease::native(&dir).unwrap());
    let control = Connection::open(dir.join(LEDGER_NAME)).unwrap();
    control.execute_batch("CREATE TRIGGER reject_activation BEFORE UPDATE ON directory_mode WHEN NEW.mode = 'workspace' BEGIN SELECT RAISE(ABORT, 'control write down'); END;").unwrap();
    drop(control);
    assert!(DirectoryLease::activate_workspace(&dir).is_err());
    conflict(
        DirectoryLease::native(&dir),
        DirectoryConflict::WorkspaceActivated,
    );
    let control = Connection::open(dir.join(LEDGER_NAME)).unwrap();
    assert_eq!(
        control
            .query_row("SELECT mode FROM directory_mode", [], |row| row
                .get::<_, String>(0))
            .unwrap(),
        "native"
    );
    control
        .execute_batch("DROP TRIGGER reject_activation;")
        .unwrap();
    drop(control);
    assert_eq!(
        DirectoryLease::maintenance(&dir).unwrap().mode(),
        DirectoryMode::Workspace
    );
    conflict(
        DirectoryLease::native(&dir),
        DirectoryConflict::WorkspaceActivated,
    );
    drop(DirectoryLease::activate_workspace(&dir).unwrap());
    let data = Connection::open(dir.join("ccp.sqlite3")).unwrap();
    assert_eq!(
        data.query_row(
            "SELECT COUNT(*) FROM ccp_access_audit WHERE action = 'activate_workspace_mode'",
            [],
            |row| row.get::<_, u32>(0)
        )
        .unwrap(),
        1
    );
    drop(data);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn schema11_activation_preserves_existing_admin_keys_workspaces_and_grants() {
    use crate::foundation::{AccessStore, Action, Actor, ClientRegistration, DataRole, authorize};
    let dir = directory();
    let path = dir.join("ccp.sqlite3");
    crate::init::init_sqlite(&path).unwrap();
    let mut connection = Connection::open(&path).unwrap();
    let mut store = AccessStore::new(&mut connection).unwrap();
    let key = store.bootstrap_admin_key("operator", 1000).unwrap();
    let workspace = store
        .create_workspace(Actor::AdminKey(key.id), "existing", key.principal_id, 1000)
        .unwrap();
    let registered = store
        .register_client(
            Actor::Admin(key.principal_id),
            &ClientRegistration {
                workspace_id: workspace,
                role: DataRole::ReadWrite,
                expires_at: 2000,
            },
            1000,
        )
        .unwrap();
    connection
        .execute_batch("DROP TABLE ccp_runtime_mode; UPDATE schema_version SET version = 11;")
        .unwrap();
    drop(connection);
    let mode = DirectoryLease::activate_workspace(&dir).unwrap();
    let mut connection = Connection::open(&path).unwrap();
    assert_eq!(
        AccessStore::new(&mut connection)
            .unwrap()
            .authenticate_admin_key(key.secret(), 1000)
            .unwrap(),
        Actor::AdminKey(key.id)
    );
    assert!(
        authorize(
            &connection,
            Actor::Credential(registered.credential_id),
            Action::WriteContext,
            Some(workspace),
            1000
        )
        .is_ok()
    );
    let client: String = connection
        .query_row(
            "SELECT client_id FROM ccp_grants WHERE id = ?1",
            [registered.grant_id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(client, registered.client_id.to_string());
    assert_eq!(
        connection
            .query_row("SELECT MAX(version) FROM schema_version", [], |row| row
                .get::<_, u32>(0))
            .unwrap(),
        crate::init::SCHEMA_VERSION
    );
    drop(connection);
    drop(mode);
    conflict(
        DirectoryLease::native(&dir),
        DirectoryConflict::WorkspaceActivated,
    );
    std::fs::remove_dir_all(dir).unwrap();
}
