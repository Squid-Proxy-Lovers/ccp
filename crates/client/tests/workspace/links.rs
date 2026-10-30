// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;
mod probe;
use client::workspace::links::enroll_link;
use protocol::enrollment_link::{EnrollmentLink, LinkTarget};
use sha2::{Digest, Sha256};

fn link(f: &Fixture, secret: &str, workspace: Uuid, name: &str) -> EnrollmentLink {
    let (_, pem) = x509_parser::pem::parse_x509_pem(f.trust.ca_certificate_pem.as_bytes()).unwrap();
    EnrollmentLink::new(
        LinkTarget {
            https_origin: f.https.clone(),
            mtls_origin: f.mtls.clone(),
            instance_id: Uuid::parse_str(&f.trust.instance_id).unwrap(),
            ca_sha256: Sha256::digest(pem.contents).into(),
            workspace_id: workspace,
            workspace_name: name.into(),
        },
        secret,
    )
    .unwrap()
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn link_happy_path_creates_private_profile_and_exact_reuse_has_no_second_effect() {
    let f = Fixture::new().await;
    let secret = f.invitation();
    let uri = link(&f, &secret, f.setup.workspace_id, "first");
    let profiles = f.root.join("profiles");
    let first = enroll_link(&uri, None, &profiles).await.unwrap();
    assert_eq!(first.workspace.id, f.setup.workspace_id.to_string());
    assert_eq!(first.profile, profiles.join("first"));
    assert!(!first.reused_registration);
    assert!(!first.name_changed);
    let repeated = enroll_link(&uri, None, &profiles).await.unwrap();
    assert!(repeated.reused_registration);
    assert_eq!(repeated.client_id, first.client_id);
    assert_eq!(repeated.profile, first.profile);
    assert_eq!(f.count("ccp_clients"), 1);
    assert_eq!(f.count("ccp_grants"), 1);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&first.profile).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(first.profile.join("workspace-profile.sqlite3"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    f.finish().await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn link_join_reuses_registration_without_changing_any_caller_selection() {
    let f = Fixture::new().await;
    let secret = f.invitation();
    let original = link(&f, &secret, f.setup.workspace_id, "first");
    let profiles = f.root.join("profiles");
    let first = enroll_link(&original, None, &profiles).await.unwrap();
    let caller = f.root.join("caller");
    let mut profile = WorkspaceProfile::open(&first.profile).unwrap();
    profile.initialize_selection(&caller).unwrap();
    profile
        .set_selection(
            &caller,
            client::workspace::SelectionUpdate {
                workspace_ids: vec![first.workspace.id.clone()],
                default_workspace_id: Some(first.workspace.id.clone()),
                expected_revision: 0,
            },
        )
        .await
        .unwrap();
    let before = fs::read(caller.join("caller-selection.sqlite3")).unwrap();
    drop(profile);
    let (secret, workspace) = second_invitation(&f);
    let joined = enroll_link(&link(&f, &secret, workspace, "second"), None, &profiles)
        .await
        .unwrap();
    assert_eq!(joined.client_id, first.client_id);
    assert_eq!(joined.profile, first.profile);
    assert!(joined.reused_registration);
    let mut profile = WorkspaceProfile::open(&joined.profile).unwrap();
    let selected = profile.selection(&caller).unwrap();
    assert_eq!(selected.workspace_ids, vec![first.workspace.id.clone()]);
    assert_eq!(selected.default_workspace_id, Some(first.workspace.id));
    assert_eq!(selected.revision, 1);
    assert_eq!(
        fs::read(caller.join("caller-selection.sqlite3")).unwrap(),
        before
    );
    assert_eq!(
        profile
            .workspaces(None, 100)
            .await
            .unwrap()
            .workspaces
            .len(),
        2
    );
    assert_eq!(f.count("ccp_clients"), 1);
    assert_eq!(f.count("ccp_credentials"), 1);
    drop(profile);
    f.finish().await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pin_uuid_name_mismatch_refuse_before_identity_creation_or_profile_publication() {
    let f = Fixture::new().await;
    let secret = f.invitation();
    let valid = link(&f, &secret, f.setup.workspace_id, "first");
    for kind in 0..2 {
        let mut target = valid.target().clone();
        if kind == 0 {
            target.ca_sha256[0] ^= 1;
        }
        if kind == 1 {
            target.instance_id = Uuid::new_v4();
        }
        let wrong = EnrollmentLink::new(target, &secret).unwrap();
        let profiles = f.root.join(format!("rejected-{kind}"));
        let error = enroll_link(&wrong, None, &profiles).await.err().unwrap();
        let expected = if kind == 0 {
            "CA pin mismatch"
        } else {
            "server instance does not match"
        };
        assert!(format!("{error:#}").contains(expected), "{error:#}");
        assert!(!profiles.exists());
        assert_eq!(f.count("ccp_clients"), 0);
        assert_eq!(f.count("ccp_invitation_receipts"), 0);
    }
    f.finish().await;
    let f = Fixture::with_names(vec!["localhost".into()]).await;
    let secret = f.invitation();
    let wrong = link(&f, &secret, f.setup.workspace_id, "first");
    let profiles = f.root.join("wrong-name");
    assert!(enroll_link(&wrong, None, &profiles).await.is_err());
    assert!(!profiles.exists());
    assert_eq!(f.count("ccp_clients"), 0);
    f.finish().await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lost_local_activation_preserves_original_nonce_and_retries_same_certificate() {
    let f = Fixture::new().await;
    let secret = f.invitation();
    let uri = link(&f, &secret, f.setup.workspace_id, "first");
    WorkspaceProfile::trust(&f.profile, f.config())
        .await
        .unwrap();
    fail_activation(&f);
    assert!(
        enroll_link(&uri, Some(&f.profile), &f.root.join("unused"))
            .await
            .is_err()
    );
    assert_eq!(f.count("ccp_clients"), 1);
    let pending: String = Connection::open(f.profile.join("workspace-profile.sqlite3"))
        .unwrap()
        .query_row("SELECT pending FROM profile", [], |row| row.get(0))
        .unwrap();
    let pending: Value = serde_json::from_str(&pending).unwrap();
    let request = pending["request_id"].as_str().unwrap();
    let certificate: String = f
        .db()
        .query_row(
            "SELECT certificate_pem FROM ccp_credential_certificates",
            [],
            |row| row.get(0),
        )
        .unwrap();
    allow_activation(&f);
    let result = enroll_link(&uri, Some(&f.profile), &f.root.join("unused"))
        .await
        .unwrap();
    assert_eq!(result.workspace.id, f.setup.workspace_id.to_string());
    let nonce: String = f
        .db()
        .query_row(
            "SELECT request_id FROM ccp_invitation_receipts",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(nonce, request);
    let p = WorkspaceProfile::open(&f.profile).unwrap();
    assert!(p.info().unwrap().pending_operation.is_none());
    drop(p);
    let raw: String = Connection::open(f.profile.join("workspace-profile.sqlite3"))
        .unwrap()
        .query_row("SELECT credential FROM profile", [], |row| row.get(0))
        .unwrap();
    let saved: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(
        saved["receipt"]["certificate_pem"].as_str().unwrap(),
        certificate
    );
    assert_eq!(f.count("ccp_clients"), 1);
    assert_eq!(f.count("ccp_grants"), 1);
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn positional_uri_cli_and_bounded_stdin_keep_bearers_out_of_output() {
    let f = Fixture::new().await;
    let secret = f.invitation();
    let uri = link(&f, &secret, f.setup.workspace_id, "first").to_uri();
    let output = f.cli(&["enroll", &uri]).await;
    assert!(output.status.success());
    assert!(!String::from_utf8_lossy(&output.stdout).contains(&secret));
    assert!(!String::from_utf8_lossy(&output.stderr).contains(&secret));
    assert!(String::from_utf8_lossy(&output.stdout).contains("Workspace:"));
    let (other, workspace) = second_invitation(&f);
    let second = link(&f, &other, workspace, "second").to_uri();
    use tokio::io::AsyncWriteExt;
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_client"))
        .args(["enroll", "-"])
        .env("CCP_CLIENT_HOME", f.root.join("native-home"))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(format!("{second}\n").as_bytes())
        .await
        .unwrap();
    let output = child.wait_with_output().await.unwrap();
    assert!(output.status.success());
    assert!(!String::from_utf8_lossy(&output.stdout).contains(&other));
    assert!(!String::from_utf8_lossy(&output.stderr).contains(&other));
    assert_eq!(f.count("ccp_clients"), 1);
    assert_eq!(f.count("ccp_grants"), 2);
    let error = f.cli(&["enroll", &uri, "--unknown-option"]).await;
    assert!(!error.status.success());
    assert!(!String::from_utf8_lossy(&error.stderr).contains(&secret));
    assert!(!String::from_utf8_lossy(&error.stderr).contains(&uri));
    f.finish().await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wrong_target_hint_preserves_registration_and_corrected_hint_can_resume() {
    let f = Fixture::new().await;
    let secret = f.invitation();
    let root = f.root.join("profiles");
    let wrong = link(&f, &secret, Uuid::new_v4(), "../hostile name");
    assert!(enroll_link(&wrong, None, &root).await.is_err());
    assert_eq!(f.count("ccp_clients"), 1);
    assert_eq!(f.count("ccp_grants"), 1);
    let corrected = link(&f, &secret, f.setup.workspace_id, "stale display name");
    let result = enroll_link(&corrected, None, &root).await.unwrap();
    assert!(result.name_changed);
    assert_eq!(result.workspace.name, "first");
    assert_eq!(f.count("ccp_clients"), 1);
    assert_eq!(f.count("ccp_grants"), 1);
    assert!(result.profile.starts_with(&root));
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn default_discovery_never_forks_a_disabled_client_or_guesses_between_identities() {
    let f = Fixture::new().await;
    let secret = f.invitation();
    let profiles = f.root.join("profiles");
    let first = enroll_link(
        &link(&f, &secret, f.setup.workspace_id, "first"),
        None,
        &profiles,
    )
    .await
    .unwrap();
    f.db()
        .execute(
            "UPDATE ccp_clients SET disabled=1 WHERE id=?1",
            [&first.client_id],
        )
        .unwrap();
    let (secret, second) = second_invitation(&f);
    let error = enroll_link(&link(&f, &secret, second, "second"), None, &profiles)
        .await
        .err()
        .unwrap();
    assert!(format!("{error:#}").contains("HTTP 403"), "{error:#}");
    assert_eq!(f.count("ccp_clients"), 1);
    assert_eq!(f.count("ccp_credentials"), 1);
    assert!(!profiles.join("second").exists());
    f.finish().await;
    let f = Fixture::new().await;
    let profiles = f.root.join("profiles");
    let secret = f.invitation();
    enroll_link(
        &link(&f, &secret, f.setup.workspace_id, "first"),
        None,
        &profiles,
    )
    .await
    .unwrap();
    let (secret, second) = second_invitation(&f);
    enroll_link(
        &link(&f, &secret, second, "second"),
        Some(&profiles.join("another-client")),
        &profiles,
    )
    .await
    .unwrap();
    f.db()
        .execute(
            "UPDATE ccp_workspaces SET name='renamed-second' WHERE id=?1",
            [second.to_string()],
        )
        .unwrap();
    let (secret, third) = second_invitation(&f);
    let error = enroll_link(&link(&f, &secret, third, "third"), None, &profiles)
        .await
        .err()
        .unwrap();
    assert!(
        format!("{error:#}").contains("multiple profiles"),
        "{error:#}"
    );
    assert_eq!(f.count("ccp_clients"), 2);
    assert_eq!(f.count("ccp_grants"), 2);
    assert!(!profiles.join("third").exists());
    f.finish().await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn explicit_different_instance_profile_and_colliding_paths_are_never_replaced() {
    let f = Fixture::new().await;
    let other = Fixture::new().await;
    WorkspaceProfile::trust(&f.profile, f.config())
        .await
        .unwrap();
    let original = fs::read(f.profile.join("workspace-profile.sqlite3")).unwrap();
    let secret = other.invitation();
    assert!(
        enroll_link(
            &link(&other, &secret, other.setup.workspace_id, "other"),
            Some(&f.profile),
            &f.root
        )
        .await
        .is_err()
    );
    assert_eq!(
        fs::read(f.profile.join("workspace-profile.sqlite3")).unwrap(),
        original
    );
    assert_eq!(f.count("ccp_clients"), 0);
    assert_eq!(other.count("ccp_clients"), 0);
    let collision = f.root.join("collision");
    fs::write(&collision, b"retain this file").unwrap();
    let secret = f.invitation();
    assert!(
        enroll_link(
            &link(&f, &secret, f.setup.workspace_id, "first"),
            Some(&collision),
            &f.root
        )
        .await
        .is_err()
    );
    assert_eq!(fs::read(collision).unwrap(), b"retain this file");
    assert_eq!(f.count("ccp_clients"), 0);
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let symbolic = f.root.join("symbolic-profile");
        symlink(&f.profile, &symbolic).unwrap();
        assert!(
            enroll_link(
                &link(&f, &secret, f.setup.workspace_id, "first"),
                Some(&symbolic),
                &f.root
            )
            .await
            .is_err()
        );
        assert_eq!(
            fs::read(f.profile.join("workspace-profile.sqlite3")).unwrap(),
            original
        );
    }
    f.finish().await;
    other.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refused_first_link_retains_pending_and_new_link_names_safe_recovery_options() {
    let f = Fixture::new().await;
    let root = f.root.join("profiles");
    let original = f.invitation();
    f.db()
        .execute("UPDATE ccp_invitations SET revoked_at=1", [])
        .unwrap();
    let error = enroll_link(
        &link(&f, &original, f.setup.workspace_id, "first"),
        None,
        &root,
    )
    .await
    .err()
    .unwrap();
    assert!(format!("{error:#}").contains("HTTP 403"), "{error:#}");
    assert!(format!("{error:#}").contains("enrollment state is saved"));
    let directory = root.join("first");
    let before = fs::read(directory.join("workspace-profile.sqlite3")).unwrap();
    let (secret, workspace) = second_invitation(&f);
    let error = enroll_link(&link(&f, &secret, workspace, "second"), None, &root)
        .await
        .err()
        .unwrap();
    let message = format!("{error:#}");
    assert!(message.contains(directory.to_str().unwrap()), "{message}");
    assert!(message.contains("Retry the original link"));
    assert!(message.contains("administrator confirms it was never accepted"));
    assert!(message.contains("--profile keeps both profiles"));
    assert!(!message.contains(&secret));
    assert!(!root.join("second").exists());
    assert_eq!(
        fs::read(directory.join("workspace-profile.sqlite3")).unwrap(),
        before
    );
    assert_eq!(f.count("ccp_clients"), 0);
    assert_eq!(f.count("ccp_invitation_receipts"), 0);
    f.finish().await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn same_workspace_names_on_different_instances_have_distinct_default_profiles() {
    let f = Fixture::new().await;
    let other = Fixture::new().await;
    let root = f.root.join("profiles");
    let first = enroll_link(
        &link(&f, &f.invitation(), f.setup.workspace_id, "first"),
        None,
        &root,
    )
    .await
    .unwrap();
    let second = enroll_link(
        &link(
            &other,
            &other.invitation(),
            other.setup.workspace_id,
            "first",
        ),
        None,
        &root,
    )
    .await
    .unwrap();
    assert_ne!(first.profile, second.profile);
    assert!(
        second
            .profile
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("first-")
    );
    assert_ne!(first.client_id, second.client_id);
    assert_eq!(f.count("ccp_clients"), 1);
    assert_eq!(other.count("ccp_clients"), 1);
    f.finish().await;
    other.finish().await;
}
