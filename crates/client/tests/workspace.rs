// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

#[cfg(test)]
#[path = "workspace"]
mod tests {
    mod content;
    mod recovery;
    mod selection;
    use anyhow::Context;
    use client::workspace::{WorkspaceProfile, WorkspaceTrust};
    use rusqlite::Connection;
    use serde_json::{Value, json};
    use server::{
        foundation::{AccessStore, BootstrappedInstance, DataRole, InitialInvitationRequest},
        instance_tls::TrustInfo,
        workspace_http::{Config, WorkspaceServer},
    };
    use std::{fs, path::PathBuf, process::Output};
    use tokio::sync::oneshot;
    use uuid::Uuid;

    struct Fixture {
        root: PathBuf,
        profile: PathBuf,
        setup: BootstrappedInstance,
        trust: TrustInfo,
        https: String,
        mtls: String,
        stop: oneshot::Sender<()>,
        task: tokio::task::JoinHandle<anyhow::Result<server::workspace_http::Report>>,
    }
    impl Fixture {
        async fn new() -> Self {
            Self::with_names(vec!["127.0.0.1".into(), "localhost".into()]).await
        }
        async fn with_names(names: Vec<String>) -> Self {
            let root = std::env::temp_dir().join(format!("ccp-client-http-{}", Uuid::new_v4()));
            let data = root.join("server");
            let setup = server::local_admin::bootstrap(&data, "owner", "first").unwrap();
            let trust = server::instance_tls::initialize(&data, &names).unwrap();
            fs::write(root.join("ca.pem"), &trust.ca_certificate_pem).unwrap();
            let runtime = WorkspaceServer::bind(Config {
                data_dir: data,
                https_listen: "127.0.0.1:0".parse().unwrap(),
                mtls_listen: "127.0.0.1:0".parse().unwrap(),
                activate_workspace: true,
            })
            .await
            .unwrap();
            let startup = runtime.startup().unwrap();
            let (stop, signal) = oneshot::channel();
            let task =
                tokio::spawn(runtime.run_until(async { signal.await.context("test shutdown") }));
            Self {
                profile: root.join("profile"),
                https: format!("https://{}", startup.https_address),
                mtls: format!("https://{}", startup.mtls_address),
                root,
                setup,
                trust,
                stop,
                task,
            }
        }
        fn config(&self) -> WorkspaceTrust {
            WorkspaceTrust::new(
                &self.trust.instance_id,
                &self.https,
                &self.mtls,
                &self.trust.ca_certificate_pem,
            )
            .unwrap()
        }
        fn db(&self) -> Connection {
            Connection::open(self.root.join("server/ccp.sqlite3")).unwrap()
        }
        fn count(&self, table: &str) -> i64 {
            self.db()
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                .unwrap()
        }
        fn invitation(&self) -> String {
            let now = i64::try_from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
            )
            .unwrap();
            let mut db = self.db();
            let mut store = AccessStore::new(&mut db).unwrap();
            let actor = store
                .authenticate_admin_key(self.setup.owner_key.secret(), now)
                .unwrap();
            store
                .issue_initial_invitation(
                    actor,
                    InitialInvitationRequest {
                        workspace_id: self.setup.workspace_id,
                        role: DataRole::ReadWrite,
                        max_clients: Some(1),
                        expires_at: Some(now + 3600),
                    },
                    now,
                )
                .unwrap()
                .secret()
                .to_owned()
        }
        fn token_file(&self, token: &str) -> PathBuf {
            use std::io::Write;
            let path = self.root.join("invitation.txt");
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            options
                .open(&path)
                .unwrap()
                .write_all(token.as_bytes())
                .unwrap();
            path
        }
        async fn cli(&self, args: &[&str]) -> Output {
            tokio::process::Command::new(env!("CARGO_BIN_EXE_client"))
                .args(args)
                .env("CCP_CLIENT_HOME", self.root.join("native-home"))
                .output()
                .await
                .unwrap()
        }
        async fn trust_cli(&self) -> Output {
            self.cli(&[
                "workspace",
                "trust",
                "--profile",
                self.profile.to_str().unwrap(),
                "--instance-id",
                &self.trust.instance_id,
                "--https-origin",
                &self.https,
                "--mtls-origin",
                &self.mtls,
                "--ca-file",
                self.root.join("ca.pem").to_str().unwrap(),
            ])
            .await
        }
        async fn enroll_cli(&self) -> Output {
            self.cli(&[
                "workspace",
                "enroll",
                "--profile",
                self.profile.to_str().unwrap(),
                "--invitation-file",
                self.root.join("invitation.txt").to_str().unwrap(),
            ])
            .await
        }
        async fn finish(self) {
            self.stop.send(()).unwrap();
            self.task.await.unwrap().unwrap();
            fs::remove_dir_all(self.root).unwrap();
        }
    }
    fn value(output: Output) -> Value {
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn library_trust_enrollment_and_mtls_discovery_use_one_persistent_registration() {
        let f = Fixture::new().await;
        let token = f.invitation();
        let info = WorkspaceProfile::trust(&f.profile, f.config())
            .await
            .unwrap();
        assert_eq!(info.instance_id, f.trust.instance_id);
        assert!(info.client_id.is_none());
        let mut profile = WorkspaceProfile::open(&f.profile).unwrap();
        let enrolled = profile.enroll(&token).await.unwrap();
        assert!(enrolled.pending_request_id.is_none());
        assert!(!enrolled.expired);
        let page = profile.workspaces(None, 100).await.unwrap();
        assert_eq!(page.workspaces.len(), 1);
        assert_eq!(page.workspaces[0].id, f.setup.workspace_id.to_string());
        {
            use server::foundation::{Admission, InitialOwner, OwnerProvisioning};
            let now = i64::try_from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
            )
            .unwrap();
            let mut db = f.db();
            let mut store = AccessStore::new(&mut db).unwrap();
            let global = store
                .authenticate_admin_key(f.setup.instance_key.secret(), now)
                .unwrap();
            let second = store
                .create_workspace_with_owner_key(
                    global,
                    "second quoted \"区\"",
                    &OwnerProvisioning {
                        owner: InitialOwner::Existing(f.setup.owner_key.principal_id),
                        key_expires_at: None,
                    },
                    now,
                )
                .unwrap();
            let owner = store
                .authenticate_admin_key(second.owner_key.secret(), now)
                .unwrap();
            store
                .admit_existing_client(
                    owner,
                    Admission {
                        workspace_id: second.workspace_id,
                        presented_credential: Uuid::parse_str(
                            enrolled.credential_id.as_deref().unwrap(),
                        )
                        .unwrap(),
                        role: DataRole::Read,
                    },
                    now,
                )
                .unwrap();
            let first = profile.workspaces(None, 1).await.unwrap();
            assert_eq!(first.workspaces.len(), 1);
            assert!(first.next_cursor.is_some());
            let next = profile
                .workspaces(first.next_cursor.as_deref(), 1)
                .await
                .unwrap();
            assert_eq!(next.workspaces[0].id, second.workspace_id.to_string());
            assert!(next.next_cursor.is_none());
            assert_eq!(profile.info().unwrap().client_id, enrolled.client_id);
        }
        assert!(profile.enroll(&token).await.is_err());
        assert_eq!(f.count("ccp_clients"), 1);
        assert_eq!(f.count("ccp_credentials"), 1);
        let info = serde_json::to_string(&enrolled).unwrap();
        assert!(!info.contains(&token));
        assert!(!info.contains("PRIVATE KEY"));
        drop(profile);
        let mut reopened = WorkspaceProfile::open(&f.profile).unwrap();
        assert_eq!(reopened.info().unwrap().client_id, enrolled.client_id);
        assert_eq!(
            reopened.workspaces(None, 1).await.unwrap().workspaces.len(),
            1
        );
        assert!(reopened.workspaces(None, 0).await.is_err());
        assert!(reopened.workspaces(None, 501).await.is_err());
        drop(reopened);
        f.finish().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn actual_cli_trust_enroll_info_and_workspaces_keep_native_commands_available() {
        let f = Fixture::new().await;
        let token = f.invitation();
        f.token_file(&token);
        let trusted = value(f.trust_cli().await);
        assert_eq!(trusted["format"], 3);
        assert_eq!(trusted["instance_id"], f.trust.instance_id);
        let enrolled = value(f.enroll_cli().await);
        assert!(enrolled["client_id"].as_str().is_some());
        let info = value(
            f.cli(&[
                "workspace",
                "info",
                "--profile",
                f.profile.to_str().unwrap(),
            ])
            .await,
        );
        assert_eq!(info["client_id"], enrolled["client_id"]);
        let listed = value(
            f.cli(&[
                "workspace",
                "workspaces",
                "--profile",
                f.profile.to_str().unwrap(),
            ])
            .await,
        );
        assert_eq!(
            listed["workspaces"][0]["id"],
            f.setup.workspace_id.to_string()
        );
        assert!(!listed.to_string().contains(&token));
        assert!(!enrolled.to_string().contains("PRIVATE KEY"));
        let help = f.cli(&["--help"]).await;
        assert!(help.status.success());
        let help = String::from_utf8(help.stdout).unwrap();
        assert!(help.contains("workspace"));
        assert!(help.contains("get-entry-at"));
        assert!(f.cli(&["sessions"]).await.status.success());
        assert_eq!(f.count("ccp_clients"), 1);
        f.finish().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn committed_server_response_and_failed_local_activation_retry_across_cli_restart_exactly_once()
     {
        let f = Fixture::new().await;
        let token = f.invitation();
        f.token_file(&token);
        value(f.trust_cli().await);
        let db = Connection::open(f.profile.join("workspace-profile.sqlite3")).unwrap();
        db.execute_batch("CREATE TRIGGER fail_activate BEFORE UPDATE OF credential ON profile BEGIN SELECT RAISE(ABORT,'injected activation failure'); END;").unwrap();
        drop(db);
        let failed = f.enroll_cli().await;
        assert!(!failed.status.success());
        assert!(!String::from_utf8_lossy(&failed.stderr).contains(&token));
        assert!(!String::from_utf8_lossy(&failed.stderr).contains("PRIVATE KEY"));
        assert_eq!(f.count("ccp_clients"), 1);
        assert_eq!(f.count("ccp_credentials"), 1);
        let pending = value(
            f.cli(&[
                "workspace",
                "info",
                "--profile",
                f.profile.to_str().unwrap(),
            ])
            .await,
        );
        assert!(pending["pending_request_id"].as_str().is_some());
        assert_eq!(pending["client_id"], Value::Null);
        let credential: String = f
            .db()
            .query_row("SELECT id FROM ccp_credentials", [], |r| r.get(0))
            .unwrap();
        let db = Connection::open(f.profile.join("workspace-profile.sqlite3")).unwrap();
        db.execute_batch("DROP TRIGGER fail_activate").unwrap();
        drop(db);
        let enrolled = value(f.enroll_cli().await);
        assert_eq!(enrolled["credential_id"], credential);
        assert_eq!(enrolled["pending_request_id"], Value::Null);
        assert_eq!(f.count("ccp_clients"), 1);
        assert_eq!(f.count("ccp_credentials"), 1);
        let uses: i64 = f
            .db()
            .query_row("SELECT uses FROM ccp_invitations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(uses, 1);
        f.finish().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn wrong_instance_is_checked_again_before_any_invitation_or_csr_is_sent() {
        let f = Fixture::new().await;
        let token = f.invitation();
        WorkspaceProfile::trust(&f.profile, f.config())
            .await
            .unwrap();
        let db = Connection::open(f.profile.join("workspace-profile.sqlite3")).unwrap();
        let record: String = db
            .query_row("SELECT trust FROM profile", [], |r| r.get(0))
            .unwrap();
        let mut trust: Value = serde_json::from_str(&record).unwrap();
        trust["instance_id"] = json!(Uuid::new_v4().to_string());
        db.execute("UPDATE profile SET trust=?1", [trust.to_string()])
            .unwrap();
        drop(db);
        let mut profile = WorkspaceProfile::open(&f.profile).unwrap();
        let error = profile.enroll(&token).await.err().unwrap().to_string();
        assert!(error.contains("instance"));
        assert!(!error.contains(&token));
        assert_eq!(f.count("ccp_clients"), 0);
        assert_eq!(f.count("ccp_credentials"), 0);
        assert!(profile.info().unwrap().pending_request_id.is_some());
        drop(profile);
        f.finish().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn trust_refuses_wrong_ca_name_and_instance_without_publishing_a_profile() {
        let f = Fixture::with_names(vec!["127.0.0.1".into()]).await;
        let foreign = Fixture::new().await;
        let wrong = WorkspaceTrust::new(
            &f.trust.instance_id,
            &f.https,
            &f.mtls,
            &foreign.trust.ca_certificate_pem,
        )
        .unwrap();
        assert!(WorkspaceProfile::trust(&f.profile, wrong).await.is_err());
        let wrong = WorkspaceTrust::new(
            &Uuid::new_v4().to_string(),
            &f.https,
            &f.mtls,
            &f.trust.ca_certificate_pem,
        )
        .unwrap();
        assert!(WorkspaceProfile::trust(&f.profile, wrong).await.is_err());
        let wrong = WorkspaceTrust::new(
            &f.trust.instance_id,
            &f.https.replace("127.0.0.1", "localhost"),
            &f.mtls,
            &f.trust.ca_certificate_pem,
        )
        .unwrap();
        assert!(WorkspaceProfile::trust(&f.profile, wrong).await.is_err());
        assert!(!f.profile.exists());
        assert_eq!(f.count("ccp_clients"), 0);
        WorkspaceProfile::trust(&f.profile, f.config())
            .await
            .unwrap();
        let before = WorkspaceProfile::open(&f.profile)
            .unwrap()
            .info()
            .unwrap()
            .ca_sha256;
        assert!(
            WorkspaceProfile::trust(&f.profile, f.config())
                .await
                .is_err()
        );
        assert_eq!(
            WorkspaceProfile::open(&f.profile)
                .unwrap()
                .info()
                .unwrap()
                .ca_sha256,
            before
        );
        foreign.finish().await;
        f.finish().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn current_server_disable_and_revocation_deny_discovery_without_implicit_reenrollment() {
        let f = Fixture::new().await;
        let token = f.invitation();
        WorkspaceProfile::trust(&f.profile, f.config())
            .await
            .unwrap();
        let mut profile = WorkspaceProfile::open(&f.profile).unwrap();
        let info = profile.enroll(&token).await.unwrap();
        f.db()
            .execute(
                "UPDATE ccp_workspaces SET state='archived' WHERE id=?1",
                [f.setup.workspace_id.to_string()],
            )
            .unwrap();
        assert!(profile.workspaces(None, 10).await.unwrap().workspaces[0].archived);
        f.db()
            .execute("UPDATE ccp_clients SET disabled=1", [])
            .unwrap();
        assert!(profile.workspaces(None, 10).await.is_err());
        f.db()
            .execute_batch(
                "UPDATE ccp_clients SET disabled=0; UPDATE ccp_credentials SET revoked_at=1;",
            )
            .unwrap();
        assert!(profile.workspaces(None, 10).await.is_err());
        assert_eq!(profile.info().unwrap().client_id, info.client_id);
        assert_eq!(f.count("ccp_clients"), 1);
        assert_eq!(f.count("ccp_credentials"), 1);
        drop(profile);
        f.finish().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn another_cli_cannot_read_or_mutate_a_profile_while_its_exclusive_lease_is_held() {
        let f = Fixture::new().await;
        WorkspaceProfile::trust(&f.profile, f.config())
            .await
            .unwrap();
        let guard = WorkspaceProfile::open(&f.profile).unwrap();
        let refused = f
            .cli(&[
                "workspace",
                "info",
                "--profile",
                f.profile.to_str().unwrap(),
            ])
            .await;
        assert!(!refused.status.success());
        assert_eq!(f.count("ccp_clients"), 0);
        drop(guard);
        value(
            f.cli(&[
                "workspace",
                "info",
                "--profile",
                f.profile.to_str().unwrap(),
            ])
            .await,
        );
        f.finish().await;
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cli_requires_a_private_invitation_file_and_writes_private_profile_material() {
        use std::os::unix::fs::PermissionsExt;
        let f = Fixture::new().await;
        let token = f.invitation();
        let file = f.token_file(&token);
        value(f.trust_cli().await);
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
        let refused = f.enroll_cli().await;
        assert!(!refused.status.success());
        assert_eq!(f.count("ccp_clients"), 0);
        assert_eq!(
            fs::metadata(&f.profile).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(f.profile.join("workspace-profile.sqlite3"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
        value(f.enroll_cli().await);
        assert_eq!(f.count("ccp_clients"), 1);
        f.finish().await;
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn fresh_targeted_readmission_profile_keeps_the_existing_client_identity() {
        use server::foundation::ReadmissionInvitationRequest;
        let f = Fixture::new().await;
        let token = f.invitation();
        WorkspaceProfile::trust(&f.profile, f.config())
            .await
            .unwrap();
        let mut old = WorkspaceProfile::open(&f.profile).unwrap();
        let original = old.enroll(&token).await.unwrap();
        let now = i64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        )
        .unwrap();
        let invitation = {
            let mut db = f.db();
            let grant: String = db
                .query_row("SELECT id FROM ccp_grants", [], |r| r.get(0))
                .unwrap();
            let mut store = AccessStore::new(&mut db).unwrap();
            let actor = store
                .authenticate_admin_key(f.setup.owner_key.secret(), now)
                .unwrap();
            store
                .revoke_grant(actor, Uuid::parse_str(&grant).unwrap(), now)
                .unwrap();
            store
                .issue_readmission_invitation(
                    actor,
                    ReadmissionInvitationRequest {
                        workspace_id: f.setup.workspace_id,
                        client_id: Uuid::parse_str(original.client_id.as_deref().unwrap()).unwrap(),
                        role: DataRole::Read,
                        expires_at: now + 3600,
                    },
                    now,
                )
                .unwrap()
                .secret()
                .to_owned()
        };
        let target = f.root.join("readmitted");
        WorkspaceProfile::trust(&target, f.config()).await.unwrap();
        let mut new = WorkspaceProfile::open(&target).unwrap();
        let restored = new.enroll(&invitation).await.unwrap();
        assert_eq!(restored.client_id, original.client_id);
        assert_ne!(restored.credential_id, original.credential_id);
        assert_eq!(f.count("ccp_clients"), 1);
        assert_eq!(f.count("ccp_grants"), 2);
        assert!(
            old.workspaces(None, 10)
                .await
                .unwrap()
                .workspaces
                .is_empty()
        );
        assert_eq!(new.workspaces(None, 10).await.unwrap().workspaces.len(), 1);
        drop(new);
        drop(old);
        f.finish().await;
    }
    fn clock() -> i64 {
        i64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        )
        .unwrap()
    }
    fn second_invitation(f: &Fixture) -> (String, Uuid) {
        use server::foundation::{InitialOwner, OwnerProvisioning};
        let mut db = f.db();
        let mut store = AccessStore::new(&mut db).unwrap();
        let actor = store
            .authenticate_admin_key(f.setup.instance_key.secret(), clock())
            .unwrap();
        let w = store
            .create_workspace_with_owner_key(
                actor,
                "second",
                &OwnerProvisioning {
                    owner: InitialOwner::Existing(f.setup.owner_key.principal_id),
                    key_expires_at: None,
                },
                clock(),
            )
            .unwrap();
        let owner = store
            .authenticate_admin_key(w.owner_key.secret(), clock())
            .unwrap();
        let token = store
            .issue_initial_invitation(
                owner,
                InitialInvitationRequest {
                    workspace_id: w.workspace_id,
                    role: DataRole::Read,
                    max_clients: Some(1),
                    expires_at: Some(clock() + 3600),
                },
                clock(),
            )
            .unwrap();
        (token.secret().to_owned(), w.workspace_id)
    }
    fn private_invitation(f: &Fixture, name: &str, token: &str) -> PathBuf {
        use std::io::Write;
        let path = f.root.join(name);
        let mut options = fs::OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options
            .open(&path)
            .unwrap()
            .write_all(token.as_bytes())
            .unwrap();
        path
    }
    async fn enroll_profile(f: &Fixture) -> client::workspace::ProfileInfo {
        WorkspaceProfile::trust(&f.profile, f.config())
            .await
            .unwrap();
        let token = f.invitation();
        WorkspaceProfile::open(&f.profile)
            .unwrap()
            .enroll(&token)
            .await
            .unwrap()
    }
    fn fail_activation(f: &Fixture) {
        Connection::open(f.profile.join("workspace-profile.sqlite3")).unwrap().execute_batch("CREATE TRIGGER fail_lifecycle BEFORE UPDATE OF credential ON profile BEGIN SELECT RAISE(ABORT,'injected lifecycle activation'); END;").unwrap();
    }
    fn allow_activation(f: &Fixture) {
        Connection::open(f.profile.join("workspace-profile.sqlite3"))
            .unwrap()
            .execute_batch("DROP TRIGGER fail_lifecycle")
            .unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn client_join_reuses_client_certificate_and_known_invitation_does_not_start_another_operation()
     {
        let f = Fixture::new().await;
        let original = enroll_profile(&f).await;
        let (token, second) = second_invitation(&f);
        let mut p = WorkspaceProfile::open(&f.profile).unwrap();
        let joined = p.join(&token).await.unwrap();
        assert_eq!(joined.client_id, original.client_id);
        assert_eq!(joined.credential_id, original.credential_id);
        assert!(joined.pending_operation.is_none());
        let page = p.workspaces(None, 100).await.unwrap();
        assert_eq!(page.workspaces.len(), 2);
        assert!(page.workspaces.iter().any(|w| w.id == second.to_string()));
        assert!(p.join(&token).await.is_err());
        assert!(p.info().unwrap().pending_operation.is_none());
        assert_eq!(f.count("ccp_clients"), 1);
        assert_eq!(f.count("ccp_credentials"), 1);
        assert_eq!(f.count("ccp_grants"), 2);
        drop(p);
        f.finish().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn actual_cli_join_and_renew_preserve_registration_and_rotate_only_the_credential() {
        let f = Fixture::new().await;
        let original = enroll_profile(&f).await;
        let (token, _) = second_invitation(&f);
        let file = private_invitation(&f, "join.txt", &token);
        let joined = value(
            f.cli(&[
                "workspace",
                "join",
                "--profile",
                f.profile.to_str().unwrap(),
                "--invitation-file",
                file.to_str().unwrap(),
            ])
            .await,
        );
        assert_eq!(
            joined["client_id"],
            original.client_id.as_ref().unwrap().as_str()
        );
        assert_eq!(
            joined["credential_id"],
            original.credential_id.as_ref().unwrap().as_str()
        );
        let renewed = value(
            f.cli(&[
                "workspace",
                "renew",
                "--profile",
                f.profile.to_str().unwrap(),
            ])
            .await,
        );
        assert_eq!(renewed["client_id"], joined["client_id"]);
        assert_ne!(renewed["credential_id"], joined["credential_id"]);
        let page = value(
            f.cli(&[
                "workspace",
                "workspaces",
                "--profile",
                f.profile.to_str().unwrap(),
            ])
            .await,
        );
        assert_eq!(page["workspaces"].as_array().unwrap().len(), 2);
        assert_eq!(f.count("ccp_clients"), 1);
        assert_eq!(f.count("ccp_credentials"), 2);
        assert_eq!(f.count("ccp_grants"), 2);
        f.finish().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn joined_grant_commit_with_local_failure_retries_same_join_after_cli_restart() {
        let f = Fixture::new().await;
        let original = enroll_profile(&f).await;
        let (token, _) = second_invitation(&f);
        let file = private_invitation(&f, "join-retry.txt", &token);
        fail_activation(&f);
        let args = [
            "workspace",
            "join",
            "--profile",
            f.profile.to_str().unwrap(),
            "--invitation-file",
            file.to_str().unwrap(),
        ];
        assert!(!f.cli(&args).await.status.success());
        assert_eq!(f.count("ccp_grants"), 2);
        assert_eq!(f.count("ccp_credentials"), 1);
        let info = value(
            f.cli(&[
                "workspace",
                "info",
                "--profile",
                f.profile.to_str().unwrap(),
            ])
            .await,
        );
        assert_eq!(info["pending_operation"], "admission");
        assert_eq!(
            info["client_id"],
            original.client_id.as_ref().unwrap().as_str()
        );
        let nonce = info["pending_request_id"].clone();
        allow_activation(&f);
        let done = value(f.cli(&args).await);
        assert_eq!(
            done["credential_id"],
            original.credential_id.as_ref().unwrap().as_str()
        );
        assert_eq!(done["pending_request_id"], Value::Null);
        assert_eq!(f.count("ccp_clients"), 1);
        assert_eq!(f.count("ccp_grants"), 2);
        assert_eq!(f.count("ccp_credentials"), 1);
        let db = f.db();
        let recorded: String = db
            .query_row(
                "SELECT request_id FROM ccp_invitation_receipts WHERE request_id=?1",
                [nonce.as_str().unwrap()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(recorded, nonce.as_str().unwrap());
        drop(db);
        f.finish().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn superseded_renewal_resumes_exact_receipt_before_discovery_after_process_restart() {
        let f = Fixture::new().await;
        let original = enroll_profile(&f).await;
        let old = f.root.join("old-profile");
        let mut dirs = fs::DirBuilder::new();
        dirs.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            dirs.mode(0o700);
        }
        dirs.create(&old).unwrap();
        fs::copy(
            f.profile.join("workspace-profile.sqlite3"),
            old.join("workspace-profile.sqlite3"),
        )
        .unwrap();
        fail_activation(&f);
        assert!(
            !f.cli(&[
                "workspace",
                "renew",
                "--profile",
                f.profile.to_str().unwrap()
            ])
            .await
            .status
            .success()
        );
        assert_eq!(f.count("ccp_credentials"), 2);
        let info = value(
            f.cli(&[
                "workspace",
                "info",
                "--profile",
                f.profile.to_str().unwrap(),
            ])
            .await,
        );
        assert_eq!(info["pending_operation"], "renewal");
        assert_eq!(
            info["credential_id"],
            original.credential_id.as_ref().unwrap().as_str()
        );
        let mut stale = WorkspaceProfile::open(&old).unwrap();
        assert!(stale.workspaces(None, 10).await.is_err());
        drop(stale);
        allow_activation(&f);
        let page = value(
            f.cli(&[
                "workspace",
                "workspaces",
                "--profile",
                f.profile.to_str().unwrap(),
            ])
            .await,
        );
        assert_eq!(page["workspaces"].as_array().unwrap().len(), 1);
        let current = value(
            f.cli(&[
                "workspace",
                "info",
                "--profile",
                f.profile.to_str().unwrap(),
            ])
            .await,
        );
        assert_eq!(current["pending_operation"], Value::Null);
        assert_eq!(current["client_id"], info["client_id"]);
        assert_ne!(current["credential_id"], info["credential_id"]);
        assert_eq!(f.count("ccp_credentials"), 2);
        assert_eq!(f.count("ccp_clients"), 1);
        f.finish().await;
    }

    fn short_credential(f: &Fixture, expired: bool) {
        use rcgen::{CertificateParams, KeyPair};
        use server::foundation::{IssuingAuthority, RedemptionRequest};
        let token = f.invitation();
        let bundle: Value =
            serde_json::from_slice(&fs::read(f.root.join("server/ccp-instance-tls.json")).unwrap())
                .unwrap();
        let ca_key = KeyPair::from_pem(bundle["ca_key"].as_str().unwrap()).unwrap();
        let mut ca = CertificateParams::from_ca_cert_pem(&f.trust.ca_certificate_pem).unwrap();
        ca.not_after = ca.not_before
            + std::time::Duration::from_secs(if expired { 30 } else { 3 * 24 * 60 * 60 });
        let issued_at = if expired {
            ca.not_before.unix_timestamp() + 10
        } else {
            clock()
        };
        let short_ca = ca.self_signed(&ca_key).unwrap();
        let issuer =
            IssuingAuthority::from_pem(&short_ca.pem(), bundle["ca_key"].as_str().unwrap())
                .unwrap();
        let key = KeyPair::generate().unwrap();
        let csr = CertificateParams::new(Vec::<String>::new())
            .unwrap()
            .serialize_request(&key)
            .unwrap()
            .pem()
            .unwrap();
        let mut db = f.db();
        let receipt = AccessStore::new(&mut db)
            .unwrap()
            .redeem_invitation(
                &token,
                &RedemptionRequest {
                    request_id: Uuid::new_v4(),
                    csr_pem: csr,
                    existing_credential: None,
                },
                &issuer,
                issued_at,
            )
            .unwrap();
        let record = json!({"receipt":{"instance_id":f.trust.instance_id,"client_id":receipt.client_id.to_string(),"credential_id":receipt.credential_id.to_string(),"certificate_pem":receipt.certificate_pem,"expires_at":receipt.expires_at},"key_pem":key.serialize_pem()});
        Connection::open(f.profile.join("workspace-profile.sqlite3"))
            .unwrap()
            .execute("UPDATE profile SET credential=?1", [record.to_string()])
            .unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn readable_short_lived_credential_renews_automatically_once_but_grantless_discovery_does_not()
     {
        for readable in [true, false] {
            let f = Fixture::new().await;
            WorkspaceProfile::trust(&f.profile, f.config())
                .await
                .unwrap();
            short_credential(&f, false);
            if !readable {
                f.db()
                    .execute("UPDATE ccp_grants SET revoked_at=?1", [clock()])
                    .unwrap();
            }
            let mut p = WorkspaceProfile::open(&f.profile).unwrap();
            let old = p.info().unwrap();
            assert!(old.renewal_due);
            let page = p.workspaces(None, 10).await.unwrap();
            assert_eq!(page.workspaces.len(), usize::from(readable));
            let next = p.info().unwrap();
            assert_eq!(next.client_id, old.client_id);
            assert_eq!(next.credential_id == old.credential_id, !readable);
            assert!(next.pending_operation.is_none());
            assert_eq!(f.count("ccp_credentials"), if readable { 2 } else { 1 });
            p.workspaces(None, 10).await.unwrap();
            assert_eq!(f.count("ccp_credentials"), if readable { 2 } else { 1 });
            drop(p);
            f.finish().await;
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn active_pending_join_blocks_renewal_and_different_invitation_without_losing_working_identity()
     {
        let f = Fixture::new().await;
        let original = enroll_profile(&f).await;
        let (token, _) = second_invitation(&f);
        fail_activation(&f);
        let mut p = WorkspaceProfile::open(&f.profile).unwrap();
        assert!(p.join(&token).await.is_err());
        let pending = p.info().unwrap().pending_request_id;
        assert!(p.renew().await.is_err());
        assert!(
            p.join(&format!("ccp_join_{}", "f".repeat(64)))
                .await
                .is_err()
        );
        assert_eq!(p.info().unwrap().pending_request_id, pending);
        assert_eq!(p.info().unwrap().credential_id, original.credential_id);
        assert_eq!(p.workspaces(None, 10).await.unwrap().workspaces.len(), 2);
        drop(p);
        allow_activation(&f);
        let mut p = WorkspaceProfile::open(&f.profile).unwrap();
        p.join(&token).await.unwrap();
        p.renew().await.unwrap();
        assert_eq!(f.count("ccp_credentials"), 2);
        drop(p);
        f.finish().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn disabled_client_cannot_renew_resume_or_create_another_registration() {
        let f = Fixture::new().await;
        let original = enroll_profile(&f).await;
        f.db()
            .execute("UPDATE ccp_clients SET disabled=1", [])
            .unwrap();
        let mut p = WorkspaceProfile::open(&f.profile).unwrap();
        assert!(p.renew().await.is_err());
        assert!(p.workspaces(None, 10).await.is_err());
        assert_eq!(p.info().unwrap().client_id, original.client_id);
        assert_eq!(f.count("ccp_clients"), 1);
        assert_eq!(f.count("ccp_credentials"), 1);
        drop(p);
        f.finish().await;
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn expired_certificate_never_starts_join_or_renewal_and_its_identity_remains_inspectable()
    {
        let f = Fixture::new().await;
        WorkspaceProfile::trust(&f.profile, f.config())
            .await
            .unwrap();
        short_credential(&f, true);
        let (token, _) = second_invitation(&f);
        let mut p = WorkspaceProfile::open(&f.profile).unwrap();
        let original = p.info().unwrap();
        assert!(original.expired);
        assert!(p.renew().await.is_err());
        assert!(p.join(&token).await.is_err());
        assert!(p.workspaces(None, 10).await.is_err());
        assert!(p.info().unwrap().pending_operation.is_none());
        assert_eq!(p.info().unwrap().client_id, original.client_id);
        assert_eq!(f.count("ccp_clients"), 1);
        assert_eq!(f.count("ccp_credentials"), 1);
        assert_eq!(f.count("ccp_grants"), 1);
        drop(p);
        f.finish().await;
    }
}
