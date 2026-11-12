// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

fn enrolled(trust: &WorkspaceTrust, material: &Credential) -> (PathBuf, WorkspaceProfile) {
    let dir = directory();
    store::create(&dir, trust).unwrap();
    rusqlite::Connection::open(dir.join("workspace-profile.sqlite3"))
        .unwrap()
        .execute(
            "UPDATE profile SET credential=?1",
            [serde_json::to_string(material).unwrap()],
        )
        .unwrap();
    let profile = WorkspaceProfile::open(&dir).unwrap();
    (dir, profile)
}

fn clear(revision: u64) -> SelectionUpdate {
    SelectionUpdate {
        workspace_ids: Vec::new(),
        default_workspace_id: None,
        expected_revision: revision,
    }
}

#[tokio::test]
async fn caller_selections_initialize_empty_and_optimistic_updates_never_overwrite_a_new_revision()
{
    let (ca, key) = authority();
    let trust = trust(&ca);
    let (root, mut p) = enrolled(&trust, &material(&trust, &ca, &key, |_| {}));
    let a = root.join("caller-a");
    let b = root.join("caller-b");
    let initial = p.initialize_selection(&a).unwrap();
    assert!(initial.workspace_ids.is_empty());
    assert!(initial.default_workspace_id.is_none());
    assert_eq!(initial.revision, 0);
    p.initialize_selection(&b).unwrap();
    assert!(p.initialize_selection(&a).is_err());
    assert_eq!(p.set_selection(&a, clear(0)).await.unwrap().revision, 1);
    assert!(p.set_selection(&a, clear(0)).await.is_err());
    assert_eq!(p.selection(&a).unwrap().revision, 1);
    assert_eq!(p.selection(&b).unwrap().revision, 0);
    drop(p);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn malformed_duplicate_oversized_or_unselected_default_never_changes_preferences() {
    let (ca, key) = authority();
    let trust = trust(&ca);
    let (root, mut p) = enrolled(&trust, &material(&trust, &ca, &key, |_| {}));
    let state = root.join("caller");
    p.initialize_selection(&state).unwrap();
    let id = Uuid::new_v4().to_string();
    for (ids, default) in [
        (vec!["invalid".to_owned()], None),
        (vec![id.clone(), id.clone()], None),
        ((0..101).map(|_| Uuid::new_v4().to_string()).collect(), None),
        (Vec::new(), Some(id)),
    ] {
        assert!(
            p.set_selection(
                &state,
                SelectionUpdate {
                    workspace_ids: ids,
                    default_workspace_id: default,
                    expected_revision: 0
                }
            )
            .await
            .is_err()
        );
        assert_eq!(p.selection(&state).unwrap().revision, 0);
    }
    drop(p);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn caller_selection_is_bound_to_instance_and_client_without_claiming_authority() {
    let (ca, key) = authority();
    let trust = trust(&ca);
    let (root, p) = enrolled(&trust, &material(&trust, &ca, &key, |_| {}));
    let state = root.join("caller");
    p.initialize_selection(&state).unwrap();
    let (other, q) = enrolled(&trust, &material(&trust, &ca, &key, |_| {}));
    assert!(q.selection(&state).is_err());
    let db = rusqlite::Connection::open(state.join("caller-selection.sqlite3")).unwrap();
    let mut value = serde_json::to_value(p.selection(&state).unwrap()).unwrap();
    value["instance_id"] = serde_json::json!(Uuid::new_v4().to_string());
    db.execute("UPDATE caller_selection SET state=?1", [value.to_string()])
        .unwrap();
    drop(db);
    assert!(p.selection(&state).is_err());
    drop(p);
    drop(q);
    fs::remove_dir_all(root).unwrap();
    fs::remove_dir_all(other).unwrap();
}

#[tokio::test]
async fn selection_write_failure_keeps_prior_revision_and_foreign_or_future_files_are_refused() {
    let (ca, key) = authority();
    let trust = trust(&ca);
    let (root, mut p) = enrolled(&trust, &material(&trust, &ca, &key, |_| {}));
    let state = root.join("caller");
    assert!(p.selection(&state).is_err());
    assert!(!state.exists());
    p.initialize_selection(&state).unwrap();
    let db = rusqlite::Connection::open(state.join("caller-selection.sqlite3")).unwrap();
    db.execute_batch("CREATE TRIGGER fail_choice BEFORE UPDATE ON caller_selection BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    drop(db);
    assert!(p.set_selection(&state, clear(0)).await.is_err());
    assert_eq!(p.selection(&state).unwrap().revision, 0);
    let db = rusqlite::Connection::open(state.join("caller-selection.sqlite3")).unwrap();
    db.pragma_update(None, "user_version", 2).unwrap();
    drop(db);
    assert!(p.selection(&state).is_err());
    assert!(p.initialize_selection(&state).is_err());
    let db = rusqlite::Connection::open(state.join("caller-selection.sqlite3")).unwrap();
    db.pragma_update(None, "user_version", 1).unwrap();
    db.pragma_update(None, "application_id", 42).unwrap();
    drop(db);
    assert!(p.selection(&state).is_err());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(state.join("caller-selection.sqlite3"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    assert_eq!(p.info().unwrap().format, store::PROFILE_FORMAT);
    drop(p);
    fs::remove_dir_all(root).unwrap();
}
