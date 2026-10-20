// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

#[cfg(test)]
mod tests {
    use serde_json::Value;
    use server::directory_mode::DirectoryLease;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};
    use uuid::Uuid;

    fn command(directory: &Path, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_server"))
            .arg(args[0])
            .arg("--data-dir")
            .arg(directory)
            .args(&args[1..])
            .output()
            .unwrap()
    }
    fn json(output: Output) -> Value {
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stderr.is_empty());
        serde_json::from_slice(&output.stdout).unwrap()
    }
    struct Fixture {
        directory: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let directory = std::env::temp_dir().join(format!("ccp-tls-cli-{}", Uuid::new_v4()));
            json(command(
                &directory,
                &[
                    "workspace-init",
                    "--operator",
                    "operator",
                    "--workspace",
                    "first",
                ],
            ));
            Self { directory }
        }
        fn init(&self) -> Value {
            json(command(
                &self.directory,
                &[
                    "workspace-tls-init",
                    "--name",
                    "localhost",
                    "--name",
                    "127.0.0.1",
                ],
            ))
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.directory).unwrap();
        }
    }

    #[test]
    fn real_cli_returns_only_public_trust_and_preserves_it_across_restarts_and_key_resets() {
        let f = Fixture::new();
        let first = f.init();
        assert_eq!(first, json(command(&f.directory, &["workspace-tls-info"])));
        assert_eq!(first, f.init());
        assert_eq!(
            first["names"],
            serde_json::json!(["127.0.0.1", "localhost"])
        );
        assert_eq!(first["ca_sha256"].as_str().unwrap().len(), 64);
        assert!(
            first["ca_certificate_pem"]
                .as_str()
                .unwrap()
                .starts_with("-----BEGIN CERTIFICATE-----")
        );
        assert!(!first.to_string().contains("PRIVATE KEY"));
        let metadata = json(command(&f.directory, &["admin-info"]));
        assert_eq!(first["instance_id"], metadata["instance_id"]);
        let owner_workspace = metadata["workspaces"][0]["id"].as_str().unwrap();
        json(command(
            &f.directory,
            &["reset-owner-key", "--workspace", owner_workspace],
        ));
        assert_eq!(first, json(command(&f.directory, &["workspace-tls-info"])));
        assert!(!f.directory.join("ccp_ca_cert.pem").exists());
        let lease = DirectoryLease::activate_workspace(&f.directory).unwrap();
        drop(lease);
        assert_eq!(first, json(command(&f.directory, &["workspace-tls-info"])));
        assert!(DirectoryLease::native(&f.directory).is_err());
    }

    #[test]
    fn real_cli_refuses_missing_directory_bad_names_busy_runtime_and_changed_names() {
        let f = Fixture::new();
        assert!(
            !command(&f.directory, &["workspace-tls-info"])
                .status
                .success()
        );
        assert!(
            !command(&f.directory, &["workspace-tls-init"])
                .status
                .success()
        );
        for bad in ["https://localhost", "*.example.com", "bad name"] {
            assert!(
                !command(&f.directory, &["workspace-tls-init", "--name", bad])
                    .status
                    .success()
            );
        }
        let native = DirectoryLease::native(&f.directory).unwrap();
        assert!(
            !command(&f.directory, &["workspace-tls-init", "--name", "localhost"])
                .status
                .success()
        );
        drop(native);
        let trust = f.init();
        assert!(
            !command(
                &f.directory,
                &["workspace-tls-init", "--name", "other.example"]
            )
            .status
            .success()
        );
        assert_eq!(trust, json(command(&f.directory, &["workspace-tls-info"])));
        let workspace = DirectoryLease::activate_workspace(&f.directory).unwrap();
        assert!(
            !command(&f.directory, &["workspace-tls-info"])
                .status
                .success()
        );
        drop(workspace);
        let missing = f.directory.join("missing");
        assert!(
            !command(&missing, &["workspace-tls-init", "--name", "localhost"])
                .status
                .success()
        );
        assert!(!command(&missing, &["workspace-tls-info"]).status.success());
        assert!(!missing.exists());
    }

    #[test]
    fn real_cli_never_replaces_lost_pinned_or_corrupt_material() {
        let f = Fixture::new();
        let trust = f.init();
        let path = f.directory.join("ccp-instance-tls.json");
        let bytes = std::fs::read(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert!(
            !command(
                &f.directory,
                &[
                    "workspace-tls-init",
                    "--name",
                    "localhost",
                    "--name",
                    "127.0.0.1"
                ]
            )
            .status
            .success()
        );
        assert!(!path.exists());
        std::fs::write(&path, b"{\"PRIVATE_SENTINEL\":\"never echo input\"}").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        let result = command(&f.directory, &["workspace-tls-info"]);
        assert!(!result.status.success());
        assert!(result.stdout.is_empty());
        assert!(!String::from_utf8_lossy(&result.stderr).contains("PRIVATE_SENTINEL"));
        std::fs::write(&path, bytes).unwrap();
        assert_eq!(trust, json(command(&f.directory, &["workspace-tls-info"])));
    }

    fn concurrent_setup(directory: &Path, worker: u64) -> Output {
        // The lease deliberately refuses promptly. Simultaneous SQLite header
        // readers can make every initial exclusive attempt report Busy; retry
        // only that documented transient result with bounded, staggered waits.
        for _ in 0..20 {
            let output = command(directory, &["workspace-tls-init", "--name", "localhost"]);
            if output.status.success() {
                return output;
            }
            let error = String::from_utf8_lossy(&output.stderr);
            assert!(
                error.contains(
                    "data directory is in use by another server or maintenance operation"
                ),
                "unexpected setup failure: {error}"
            );
            std::thread::sleep(std::time::Duration::from_millis(10 * (worker + 1)));
        }
        panic!("concurrent setup never acquired the lease after bounded Busy retries")
    }

    #[test]
    fn parallel_real_setup_processes_publish_one_identity_and_one_audit() {
        let f = Fixture::new();
        let workers: Vec<_> = (0..8)
            .map(|worker| {
                let dir = f.directory.clone();
                std::thread::spawn(move || concurrent_setup(&dir, worker))
            })
            .collect();
        let outputs: Vec<_> = workers.into_iter().map(|w| w.join().unwrap()).collect();
        let errors: Vec<_> = outputs
            .iter()
            .map(|out| String::from_utf8_lossy(&out.stderr).into_owned())
            .collect();
        let successes: Vec<_> = outputs
            .into_iter()
            .filter(|o| o.status.success())
            .map(json)
            .collect();
        assert!(!successes.is_empty(), "setup errors: {errors:?}");
        assert_eq!(successes.len(), 8, "all workers must eventually succeed");
        let current = json(command(&f.directory, &["workspace-tls-info"]));
        assert!(successes.iter().all(|value| value == &current));
        let connection = rusqlite::Connection::open(f.directory.join("ccp.sqlite3")).unwrap();
        let count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM ccp_access_audit WHERE action = 'provision_instance_tls'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
        let pin_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM ccp_instance_tls", [], |r| r.get(0))
            .unwrap();
        assert_eq!(pin_count, 1);
    }
}
