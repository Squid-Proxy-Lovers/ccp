// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use std::collections::BTreeMap;

fn snapshot(f: &Fixture) -> BTreeMap<String, Vec<u8>> {
    fs::read_dir(&f.profile)
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

async fn preflight_cli(f: &Fixture) -> Output {
    f.cli(&[
        "workspace",
        "update-preflight",
        "--profile",
        f.profile.to_str().unwrap(),
    ])
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn preflight_library_and_cli_preserve_real_identity_selection_and_all_profile_bytes() {
    let f = Fixture::new().await;
    WorkspaceProfile::trust(&f.profile, f.config())
        .await
        .unwrap();
    let before_enrollment = snapshot(&f);
    let result = value(preflight_cli(&f).await);
    assert_eq!(result["server"]["instance_id"], f.trust.instance_id);
    assert_eq!(
        result["server"]["package_version"],
        env!("CARGO_PKG_VERSION")
    );
    assert_eq!(result["server"]["native_wire"], 1);
    assert_eq!(result["server"]["workspace_api"], 1);
    assert_eq!(result["profile_format"], 5);
    assert_eq!(result["migration_required"], false);
    assert_eq!(result["pending"], false);
    assert_eq!(snapshot(&f), before_enrollment);
    let token = f.invitation();
    let mut profile = WorkspaceProfile::open(&f.profile).unwrap();
    let info = profile.enroll(&token).await.unwrap();
    let caller = f.root.join("saved-caller");
    let selection = profile.initialize_selection(&caller).unwrap();
    let selection_bytes = fs::read(caller.join("caller-selection.sqlite3")).unwrap();
    drop(profile);
    let before = snapshot(&f);
    let credentials = f.count("ccp_credentials");
    let library = client::workspace::update_preflight(&f.profile)
        .await
        .unwrap();
    assert!(!library.migration_required && !library.pending);
    assert_eq!(library.server.instance_id, f.trust.instance_id);
    assert_eq!(snapshot(&f), before);
    assert_eq!(value(preflight_cli(&f).await)["pending"], false);
    assert_eq!(snapshot(&f), before);
    assert_eq!(f.count("ccp_credentials"), credentials);
    assert_eq!(
        fs::read(caller.join("caller-selection.sqlite3")).unwrap(),
        selection_bytes
    );
    let profile = WorkspaceProfile::open(&f.profile).unwrap();
    assert_eq!(profile.info().unwrap().client_id, info.client_id);
    assert_eq!(profile.info().unwrap().credential_id, info.credential_id);
    assert_eq!(
        profile.selection(&caller).unwrap().revision,
        selection.revision
    );
    drop(profile);
    assert!(!f.root.join("native-home").exists());
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn preflight_pending_and_old_profile_are_facts_without_mutation_or_automatic_resolution() {
    let f = Fixture::new().await;
    WorkspaceProfile::trust(&f.profile, f.config())
        .await
        .unwrap();
    let db = Connection::open(f.profile.join("workspace-profile.sqlite3")).unwrap();
    db.execute(
        "UPDATE status_state SET pending='PRIVATE_PENDING_SENTINEL'",
        [],
    )
    .unwrap();
    drop(db);
    let before = snapshot(&f);
    let result = value(preflight_cli(&f).await);
    assert_eq!(result["pending"], true);
    assert!(!result.to_string().contains("PRIVATE_PENDING_SENTINEL"));
    assert_eq!(snapshot(&f), before);
    let db = Connection::open(f.profile.join("workspace-profile.sqlite3")).unwrap();
    db.execute_batch("DROP TRIGGER status_pending_excludes_other; DROP TRIGGER content_pending_excludes_status; DROP TRIGGER lifecycle_pending_excludes_status; DROP TABLE status_state; PRAGMA user_version=4").unwrap();
    drop(db);
    let before = snapshot(&f);
    let result = value(preflight_cli(&f).await);
    assert_eq!(result["migration_required"], true);
    assert_eq!(result["profile_format"], 4);
    assert_eq!(result["pending"], false);
    assert_eq!(snapshot(&f), before);
    assert_eq!(before.len(), 1);
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn preflight_pins_instance_name_and_root_and_refuses_offline_without_state_changes() {
    let f = Fixture::with_names(vec!["localhost".into()]).await;
    let trust = WorkspaceTrust::new(
        &f.trust.instance_id,
        &f.https.replace("127.0.0.1", "localhost"),
        &f.mtls.replace("127.0.0.1", "localhost"),
        &f.trust.ca_certificate_pem,
    )
    .unwrap();
    WorkspaceProfile::trust(&f.profile, trust).await.unwrap();
    let db = Connection::open(f.profile.join("workspace-profile.sqlite3")).unwrap();
    let raw: String = db
        .query_row("SELECT trust FROM profile", [], |r| r.get(0))
        .unwrap();
    drop(db);
    let key = rcgen::KeyPair::generate().unwrap();
    let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let foreign_ca = params.self_signed(&key).unwrap().pem();
    let offline = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let offline_origin = format!("https://{}/", offline.local_addr().unwrap());
    drop(offline);
    for (field, replacement) in [
        ("instance_id", Uuid::new_v4().to_string()),
        ("https_origin", format!("{}/", f.https)),
        ("ca_pem", foreign_ca),
        ("https_origin", offline_origin),
    ] {
        let mut trust: Value = serde_json::from_str(&raw).unwrap();
        trust[field] = json!(replacement);
        let db = Connection::open(f.profile.join("workspace-profile.sqlite3")).unwrap();
        db.execute("UPDATE profile SET trust=?1", [trust.to_string()])
            .unwrap();
        drop(db);
        let before = snapshot(&f);
        let result = preflight_cli(&f).await;
        assert!(!result.status.success(), "{field}");
        assert!(result.stdout.is_empty());
        assert_eq!(snapshot(&f), before);
    }
    f.finish().await;
}
