// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;

#[test]
fn destination_is_new_and_source_changes_prevent_publication() {
    let f = Fixture::new();
    fs::create_dir(&f.destination).unwrap();
    error(
        convert(&f.options()),
        ConversionError::DestinationUnavailable,
    );
    assert_eq!(fs::read_dir(&f.destination).unwrap().count(), 0);
    fs::remove_dir(&f.destination).unwrap();
    let source = source::Source::open(&f.options()).unwrap();
    let stage = destination::Stage::create(&f.source, &f.destination).unwrap();
    let result = target::build(&source, &stage, "operator").unwrap();
    f.db()
        .execute("UPDATE sessions SET description='new source'", [])
        .unwrap();
    error(
        finish(&source, &stage, result),
        ConversionError::SourceChanged,
    );
    assert!(!f.destination.exists());
}

#[test]
fn post_link_sync_failure_keeps_complete_database_and_recovers_only_through_local_reset() {
    let f = Fixture::new();
    let before = f.files();
    let source = source::Source::open(&f.options()).unwrap();
    let stage = destination::Stage::create(&f.source, &f.destination).unwrap();
    let result = target::build(&source, &stage, "operator").unwrap();
    source.verify().unwrap();
    let digest = hash(&stage.database());
    let instance = result.receipt.instance_id.clone();
    let principal = result.instance_key.principal_id;
    error(
        stage.publish_with_sync(|_| Err(std::io::Error::other("owned sync fault"))),
        ConversionError::PublicationUnconfirmed,
    );
    drop(result);
    drop(stage);
    assert_eq!(hash(&f.destination.join("ccp.sqlite3")), digest);
    assert_eq!(f.files(), before);
    error(
        convert(&f.options()),
        ConversionError::DestinationUnavailable,
    );
    let info = crate::local_admin::info(&f.destination).unwrap();
    assert_eq!(info.instance_id, instance);
    assert_eq!(info.workspaces.len(), 1);
    let reset = crate::local_admin::reset_key(
        &f.destination,
        crate::foundation::LocalKeyTarget::InstancePrincipal(principal),
        None,
    )
    .unwrap();
    let mut db = Connection::open(f.destination.join("ccp.sqlite3")).unwrap();
    assert!(
        AccessStore::new(&mut db)
            .unwrap()
            .authenticate_admin_key(reset.key.secret(), result_time())
            .is_ok()
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM ccp_content_entries", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        2
    );
}
fn result_time() -> i64 {
    crate::instance_tls::now().unwrap()
}

#[test]
fn publication_race_and_overlapping_or_symlink_targets_never_replace_another_artifact() {
    let f = Fixture::new();
    let stage = destination::Stage::create(&f.source, &f.destination).unwrap();
    fs::write(stage.database(), b"candidate").unwrap();
    fs::create_dir(&f.destination).unwrap();
    fs::write(f.destination.join("ccp.sqlite3"), b"another writer").unwrap();
    error(stage.publish(), ConversionError::DestinationUnavailable);
    assert_eq!(
        fs::read(f.destination.join("ccp.sqlite3")).unwrap(),
        b"another writer"
    );
    let mut options = f.options();
    options.destination = f.source.join("nested");
    error(convert(&options), ConversionError::DestinationUnavailable);
    assert!(!f.source.join("nested").exists());
    #[cfg(unix)]
    {
        let alias = f.root.join("alias");
        std::os::unix::fs::symlink(&f.destination, &alias).unwrap();
        options.destination = alias;
        error(convert(&options), ConversionError::DestinationUnavailable);
        assert_eq!(
            fs::read(f.destination.join("ccp.sqlite3")).unwrap(),
            b"another writer"
        );
    }
}

#[cfg(unix)]
#[test]
fn published_database_and_directory_are_private_and_no_stage_is_left_after_success() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    convert(&f.options()).unwrap();
    assert_eq!(
        fs::metadata(&f.destination).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(f.destination.join("ccp.sqlite3"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(fs::read_dir(&f.root).unwrap().count(), 2);
    assert_eq!(fs::read_dir(&f.destination).unwrap().count(), 1);
}
