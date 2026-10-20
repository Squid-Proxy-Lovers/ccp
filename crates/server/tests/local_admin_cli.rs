// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};

    use rusqlite::Connection;
    use serde_json::Value;
    use server::directory_mode::{DirectoryLease, DirectoryMode};
    use server::foundation::{AccessStore, Action, Actor, ClientRegistration, DataRole};
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
        serde_json::from_slice(&output.stdout).unwrap()
    }

    fn now() -> i64 {
        i64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        )
        .unwrap()
    }

    struct Fixture {
        directory: PathBuf,
        setup: Value,
    }

    impl Fixture {
        fn new() -> Self {
            let directory =
                std::env::temp_dir().join(format!("ccp-local-admin-cli-{}", Uuid::new_v4()));
            let setup = json(command(
                &directory,
                &[
                    "workspace-init",
                    "--operator",
                    "deployer",
                    "--workspace",
                    "first",
                ],
            ));
            Self { directory, setup }
        }
        fn workspace(&self) -> Uuid {
            Uuid::parse_str(self.setup["workspace_id"].as_str().unwrap()).unwrap()
        }
        fn principal(&self) -> &str {
            self.setup["principal_id"].as_str().unwrap()
        }
        fn global(&self) -> &str {
            self.setup["instance_admin_key"]["secret"].as_str().unwrap()
        }
        fn owner(&self) -> &str {
            self.setup["workspace_owner_key"]["secret"]
                .as_str()
                .unwrap()
        }
        fn connection(&self) -> Connection {
            Connection::open(self.directory.join("ccp.sqlite3")).unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.directory).unwrap();
        }
    }

    #[test]
    fn setup_commands_return_independent_scoped_keys_and_secret_free_recovery_metadata() {
        let fixture = Fixture::new();
        assert_ne!(fixture.global(), fixture.owner());
        let mut connection = fixture.connection();
        let mut store = AccessStore::new(&mut connection).unwrap();
        let global = store
            .authenticate_admin_key(fixture.global(), now())
            .unwrap();
        let owner = store
            .authenticate_admin_key(fixture.owner(), now())
            .unwrap();
        assert!(
            server::foundation::authorize(
                &connection,
                global,
                Action::CreateWorkspace,
                None,
                now()
            )
            .is_ok()
        );
        assert!(
            server::foundation::authorize(
                &connection,
                global,
                Action::WriteContext,
                Some(fixture.workspace()),
                now()
            )
            .is_err()
        );
        assert!(
            server::foundation::authorize(
                &connection,
                owner,
                Action::WriteContext,
                Some(fixture.workspace()),
                now()
            )
            .is_ok()
        );
        let metadata = json(command(&fixture.directory, &["admin-info"]));
        assert_eq!(metadata["instance_id"], fixture.setup["instance_id"]);
        assert_eq!(
            metadata["global_admins"][0]["id"].as_str().unwrap(),
            fixture.principal()
        );
        assert_eq!(
            metadata["workspaces"][0]["owner_id"].as_str().unwrap(),
            fixture.principal()
        );
        assert_eq!(metadata["mode"], "native");
        assert!(!metadata.to_string().contains("ccp_admin_"));
        assert!(!metadata.to_string().contains("token_hash"));
        assert!(!fixture.directory.join("ccp_ca_key.pem").exists());
        let again = command(
            &fixture.directory,
            &[
                "workspace-init",
                "--operator",
                "other",
                "--workspace",
                "other",
            ],
        );
        assert!(!again.status.success());
        assert!(again.stdout.is_empty());
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM ccp_admin_keys", [], |row| row
                    .get::<_, u32>(0))
                .unwrap(),
            2
        );
    }

    #[test]
    fn reset_commands_replace_only_the_selected_scope_and_preserve_identity() {
        let fixture = Fixture::new();
        let owner = json(command(
            &fixture.directory,
            &[
                "reset-owner-key",
                "--workspace",
                &fixture.workspace().to_string(),
            ],
        ));
        assert_eq!(owner["workspace_id"], fixture.setup["workspace_id"]);
        assert_eq!(owner["principal_id"], fixture.setup["principal_id"]);
        assert_eq!(owner["revoked_keys"], 1);
        assert_eq!(owner["expires_at"], Value::Null);
        let mut connection = fixture.connection();
        let mut store = AccessStore::new(&mut connection).unwrap();
        assert!(
            store
                .authenticate_admin_key(fixture.owner(), now())
                .is_err()
        );
        assert!(
            store
                .authenticate_admin_key(fixture.global(), now())
                .is_ok()
        );
        assert!(
            store
                .authenticate_admin_key(owner["secret"].as_str().unwrap(), now())
                .is_ok()
        );
        let global = json(command(
            &fixture.directory,
            &["reset-instance-key", "--principal", fixture.principal()],
        ));
        assert_eq!(global["workspace_id"], Value::Null);
        assert_eq!(global["principal_id"], fixture.setup["principal_id"]);
        assert_eq!(global["revoked_keys"], 1);
        assert!(
            store
                .authenticate_admin_key(fixture.global(), now())
                .is_err()
        );
        assert!(
            store
                .authenticate_admin_key(owner["secret"].as_str().unwrap(), now())
                .is_ok()
        );
        assert!(
            store
                .authenticate_admin_key(global["secret"].as_str().unwrap(), now())
                .is_ok()
        );
        let metadata = json(command(&fixture.directory, &["admin-info"]));
        assert_eq!(metadata["instance_id"], fixture.setup["instance_id"]);
        assert_eq!(
            metadata["workspaces"][0]["owner_id"],
            fixture.setup["principal_id"]
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM ccp_admin_keys WHERE revoked_at IS NULL",
                    [],
                    |row| row.get::<_, u32>(0)
                )
                .unwrap(),
            2
        );
    }

    #[test]
    fn local_commands_require_exclusive_access_and_preserve_workspace_mode_and_client_state() {
        let fixture = Fixture::new();
        let native = DirectoryLease::native(&fixture.directory).unwrap();
        let blocked = command(
            &fixture.directory,
            &["reset-instance-key", "--principal", fixture.principal()],
        );
        assert!(!blocked.status.success());
        assert!(blocked.stdout.is_empty());
        assert!(String::from_utf8_lossy(&blocked.stderr).contains("in use"));
        drop(native);
        let client = archived_client(&fixture);
        let workspace_runtime = DirectoryLease::activate_workspace(&fixture.directory).unwrap();
        let blocked = command(
            &fixture.directory,
            &[
                "reset-owner-key",
                "--workspace",
                &fixture.workspace().to_string(),
            ],
        );
        assert!(!blocked.status.success());
        assert!(blocked.stdout.is_empty());
        assert!(String::from_utf8_lossy(&blocked.stderr).contains("in use"));
        drop(workspace_runtime);
        let reset = json(command(
            &fixture.directory,
            &[
                "reset-owner-key",
                "--workspace",
                &fixture.workspace().to_string(),
            ],
        ));
        assert_eq!(
            json(command(&fixture.directory, &["admin-info"]))["mode"],
            "workspace"
        );
        assert_eq!(
            DirectoryLease::maintenance(&fixture.directory)
                .unwrap()
                .mode(),
            DirectoryMode::Workspace
        );
        assert!(DirectoryLease::native(&fixture.directory).is_err());
        let mut connection = fixture.connection();
        let new = AccessStore::new(&mut connection)
            .unwrap()
            .authenticate_admin_key(reset["secret"].as_str().unwrap(), now())
            .unwrap();
        assert!(
            server::foundation::authorize(
                &connection,
                new,
                Action::ReadContext,
                Some(fixture.workspace()),
                now()
            )
            .is_ok()
        );
        assert!(
            server::foundation::authorize(
                &connection,
                new,
                Action::WriteContext,
                Some(fixture.workspace()),
                now()
            )
            .is_err()
        );
        assert!(
            server::foundation::authorize(
                &connection,
                Actor::Credential(client.credential_id),
                Action::ReadContext,
                Some(fixture.workspace()),
                now()
            )
            .is_ok()
        );
        assert!(
        connection
            .query_row(
                "SELECT closed_at IS NOT NULL FROM ccp_initial_enrollment WHERE workspace_id = ?1",
                [fixture.workspace().to_string()],
                |row| row.get::<_, bool>(0)
            )
            .unwrap()
    );
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM ccp_grants", [], |row| row
                    .get::<_, u32>(0))
                .unwrap(),
            1
        );
    }

    #[test]
    fn reset_cli_validates_ttl_and_role_before_any_key_changes() {
        let fixture = Fixture::new();
        for ttl in ["0", "18446744073709551615"] {
            let failed = command(
                &fixture.directory,
                &[
                    "reset-instance-key",
                    "--principal",
                    fixture.principal(),
                    "--ttl-seconds",
                    ttl,
                ],
            );
            assert!(!failed.status.success());
            assert!(failed.stdout.is_empty());
        }
        let invalid = command(
            &fixture.directory,
            &["reset-owner-key", "--workspace", "not-a-uuid"],
        );
        assert!(!invalid.status.success());
        let mut connection = fixture.connection();
        let mut store = AccessStore::new(&mut connection).unwrap();
        assert!(
            store
                .authenticate_admin_key(fixture.global(), now())
                .is_ok()
        );
        let unprivileged = store
            .create_principal(
                Actor::AdminKey(
                    Uuid::parse_str(fixture.setup["instance_admin_key"]["id"].as_str().unwrap())
                        .unwrap(),
                ),
                "unprivileged",
                now(),
            )
            .unwrap();
        let failed = command(
            &fixture.directory,
            &[
                "reset-instance-key",
                "--principal",
                &unprivileged.to_string(),
            ],
        );
        assert!(!failed.status.success());
        assert!(failed.stdout.is_empty());
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM ccp_admin_keys", [], |row| row
                    .get::<_, u32>(0))
                .unwrap(),
            2
        );
        let before = now();
        let reset = json(command(
            &fixture.directory,
            &[
                "reset-instance-key",
                "--principal",
                fixture.principal(),
                "--ttl-seconds",
                "120",
            ],
        ));
        let expires = reset["expires_at"].as_i64().unwrap();
        assert!(expires >= before + 120 && expires <= now() + 120);
        let mut store = AccessStore::new(&mut connection).unwrap();
        assert!(
            store
                .authenticate_admin_key(reset["secret"].as_str().unwrap(), expires)
                .is_err()
        );
        assert!(store.authenticate_admin_key(fixture.owner(), now()).is_ok());
    }

    #[test]
    fn metadata_and_reset_of_missing_database_do_not_create_an_instance() {
        let directory =
            std::env::temp_dir().join(format!("ccp-missing-admin-cli-{}", Uuid::new_v4()));
        let metadata = command(&directory, &["admin-info"]);
        assert!(!metadata.status.success());
        let reset = command(
            &directory,
            &[
                "reset-instance-key",
                "--principal",
                &Uuid::new_v4().to_string(),
            ],
        );
        assert!(!reset.status.success());
        assert!(!directory.exists());
    }

    fn archived_client(fixture: &Fixture) -> server::foundation::Enrollment {
        let mut connection = fixture.connection();
        let mut store = AccessStore::new(&mut connection).unwrap();
        let owner = store
            .authenticate_admin_key(fixture.owner(), now())
            .unwrap();
        let client = store
            .register_client(
                owner,
                &ClientRegistration {
                    workspace_id: fixture.workspace(),
                    role: DataRole::ReadWrite,
                    expires_at: now() + 3600,
                },
                now(),
            )
            .unwrap();
        store
            .close_initial_enrollment(owner, fixture.workspace(), now())
            .unwrap();
        store
            .set_archived(owner, fixture.workspace(), true, now())
            .unwrap();
        drop(connection);
        client
    }
    #[test]
    fn lost_setup_stdout_is_recovered_by_metadata_and_scoped_reset() {
        let directory = std::env::temp_dir().join(format!("ccp-lost-setup-cli-{}", Uuid::new_v4()));
        let mut child = Command::new(env!("CARGO_BIN_EXE_server"))
            .args([
                "workspace-init",
                "--operator",
                "deployer",
                "--workspace",
                "first",
                "--data-dir",
            ])
            .arg(&directory)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        drop(child.stdout.take().unwrap());
        assert!(!child.wait().unwrap().success());
        let metadata = json(command(&directory, &["admin-info"]));
        let principal = metadata["global_admins"][0]["id"].as_str().unwrap();
        let workspace = metadata["workspaces"][0]["id"].as_str().unwrap();
        let global = json(command(
            &directory,
            &["reset-instance-key", "--principal", principal],
        ));
        let owner = json(command(
            &directory,
            &["reset-owner-key", "--workspace", workspace],
        ));
        let mut connection = Connection::open(directory.join("ccp.sqlite3")).unwrap();
        let mut store = AccessStore::new(&mut connection).unwrap();
        assert!(
            store
                .authenticate_admin_key(global["secret"].as_str().unwrap(), now())
                .is_ok()
        );
        assert!(
            store
                .authenticate_admin_key(owner["secret"].as_str().unwrap(), now())
                .is_ok()
        );
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM ccp_admin_principals", [], |row| row
                    .get::<_, u32>(
                    0
                ))
                .unwrap(),
            1
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM ccp_admin_keys WHERE revoked_at IS NULL",
                    [],
                    |row| row.get::<_, u32>(0)
                )
                .unwrap(),
            2
        );
        drop(connection);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn setup_on_native_data_preserves_sessions_without_claiming_legacy_display_owners() {
        let directory =
            std::env::temp_dir().join(format!("ccp-native-setup-cli-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let connection = Connection::open(directory.join("ccp.sqlite3")).unwrap();
        connection
            .execute_batch(include_str!("../src/init-db.sql"))
            .unwrap();
        connection
            .execute("INSERT INTO schema_version(version) VALUES (2)", [])
            .unwrap();
        connection.execute("INSERT INTO sessions(id, name, description, owner) VALUES (7, 'legacy', 'keep existing context', 'deployer')", []).unwrap();
        drop(connection);
        let setup = json(command(
            &directory,
            &[
                "workspace-init",
                "--operator",
                "deployer",
                "--workspace",
                "first",
            ],
        ));
        let info = json(command(&directory, &["admin-info"]));
        let legacy = info["workspaces"]
            .as_array()
            .unwrap()
            .iter()
            .find(|workspace| workspace["name"] == "legacy")
            .unwrap();
        assert_eq!(legacy["owner_id"], Value::Null);
        assert_eq!(info["workspaces"].as_array().unwrap().len(), 2);
        let mut connection = Connection::open(directory.join("ccp.sqlite3")).unwrap();
        let original: (String, String, String) = connection
            .query_row(
                "SELECT name, description, owner FROM sessions WHERE id = 7",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            original,
            (
                "legacy".into(),
                "keep existing context".into(),
                "deployer".into()
            )
        );
        let mut store = AccessStore::new(&mut connection).unwrap();
        let owner = store
            .authenticate_admin_key(
                setup["workspace_owner_key"]["secret"].as_str().unwrap(),
                now(),
            )
            .unwrap();
        let global = store
            .authenticate_admin_key(
                setup["instance_admin_key"]["secret"].as_str().unwrap(),
                now(),
            )
            .unwrap();
        let legacy_id = Uuid::parse_str(legacy["id"].as_str().unwrap()).unwrap();
        assert!(
            server::foundation::authorize(
                &connection,
                owner,
                Action::ReadContext,
                Some(legacy_id),
                now()
            )
            .is_err()
        );
        assert!(
            server::foundation::authorize(
                &connection,
                global,
                Action::ReadContext,
                Some(legacy_id),
                now()
            )
            .is_ok()
        );
        drop(connection);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
