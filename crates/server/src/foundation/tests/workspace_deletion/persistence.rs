// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;

#[test]
fn failed_start_or_completion_rolls_back_atomically_and_reopen_resumes_confirmed_job_only() {
    let path =
        std::env::temp_dir().join(format!("ccp-workspace-delete-{}.sqlite3", Uuid::new_v4()));
    let mut f = fixture();
    archive(&mut f);
    let owner = Actor::Admin(f.owner);
    let request = intent(f.workspace, 2);
    f.connection.execute_batch("CREATE TRIGGER deletion_fault BEFORE INSERT ON ccp_access_audit WHEN NEW.action='begin_workspace_deletion' BEGIN SELECT RAISE(ABORT,'PRIVATE_BODY'); END;").unwrap();
    assert!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .begin_workspace_deletion(owner, &request, NOW)
            .is_err()
    );
    assert_eq!(rows(&f.connection, "ccp_workspace_deletions"), 0);
    assert_eq!(preview(&mut f, owner).lifecycle_revision, 2);
    f.connection
        .execute_batch("DROP TRIGGER deletion_fault")
        .unwrap();
    let result = begin(&mut f, owner, &request);
    f.connection.execute_batch("CREATE TRIGGER deletion_done_fault BEFORE INSERT ON ccp_access_audit WHEN NEW.action='complete_workspace_deletion' BEGIN SELECT RAISE(ABORT,'PRIVATE_BODY'); END;").unwrap();
    // This fixture has no keys or grants: bounded role/audit removal precedes completion.
    let mut reached = false;
    for _ in 0..20 {
        let attempt = AccessStore::new(&mut f.connection)
            .unwrap()
            .advance_workspace_deletion(f.workspace, 128, NOW);
        if attempt.is_err() {
            reached = true;
            break;
        }
        assert_eq!(attempt.unwrap().completed_at, None);
    }
    assert!(reached);
    assert_eq!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .workspace_deletion_status(owner, f.workspace, request.operation_id, NOW)
            .unwrap(),
        result
    );
    f.connection
        .execute("VACUUM INTO ?1", [path.to_str().unwrap()])
        .unwrap();
    f.connection = Connection::open(&path).unwrap();
    f.connection
        .execute_batch("DROP TRIGGER deletion_done_fault")
        .unwrap();
    let complete = finish(&mut f, 1);
    assert!(complete.completed_at.is_some());
    assert_eq!(rows(&f.connection, "ccp_workspace_deletions"), 1);
    assert_eq!(rows(&f.connection, "ccp_workspaces"), 2);
    assert_eq!(
        f.connection
            .query_row(
                "SELECT state FROM ccp_workspaces WHERE id=?1",
                [f.other_workspace.to_string()],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "active"
    );
    drop(f);
    fs::remove_file(path).unwrap();
}

#[test]
fn terminal_identity_and_receipt_cannot_be_replaced_reactivated_or_repopulated() {
    let mut f = fixture();
    archive(&mut f);
    let owner = Actor::Admin(f.owner);
    let ws = f.workspace;
    let request = intent(ws, 2);
    begin(&mut f, owner, &request);
    finish(&mut f, 128);
    for sql in [
        "UPDATE ccp_workspaces SET state='active' WHERE id=?1",
        "UPDATE ccp_workspaces SET name='restored' WHERE id=?1",
        "DELETE FROM ccp_workspaces WHERE id=?1",
        "INSERT OR REPLACE INTO ccp_workspaces(id,name) VALUES(?1,'restored')",
        "DELETE FROM ccp_workspace_deletions WHERE workspace_id=?1",
        "UPDATE ccp_workspace_deletions SET operation_id='changed' WHERE workspace_id=?1",
        "UPDATE ccp_workspace_deletions SET completed_at=NULL WHERE workspace_id=?1",
        "DELETE FROM ccp_workspace_lifecycle WHERE workspace_id=?1",
        "UPDATE ccp_workspace_lifecycle SET revision=1 WHERE workspace_id=?1",
        "INSERT INTO ccp_content_shelves(id,workspace_id,name,description,created_at) VALUES('late',?1,'late','PRIVATE_BODY',1000)",
    ] {
        assert!(
            f.connection.execute(sql, [ws.to_string()]).is_err(),
            "{sql}"
        );
    }
    assert_eq!(rows(&f.connection, "ccp_content_shelves"), 0);
    assert_eq!(rows(&f.connection, "ccp_workspace_deletions"), 1);
}

#[test]
fn concurrent_reactivation_and_confirmed_delete_linearize_without_a_stale_lifecycle_delete() {
    use std::sync::{Arc, Barrier};
    let path = std::env::temp_dir().join(format!("ccp-delete-race-{}.sqlite3", Uuid::new_v4()));
    let mut f = fixture();
    archive(&mut f);
    let ws = f.workspace;
    let global = f.global;
    f.connection
        .execute("VACUUM INTO ?1", [path.to_str().unwrap()])
        .unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let b = barrier.clone();
    let p = path.clone();
    let deletion = std::thread::spawn(move || {
        let mut db = Connection::open(p).unwrap();
        db.busy_timeout(std::time::Duration::from_secs(5)).unwrap();
        b.wait();
        AccessStore::new(&mut db)
            .unwrap()
            .begin_workspace_deletion(Actor::Admin(global), &intent(ws, 2), NOW)
            .is_ok()
    });
    let b = barrier.clone();
    let p = path.clone();
    let reactivation = std::thread::spawn(move || {
        let mut db = Connection::open(p).unwrap();
        db.busy_timeout(std::time::Duration::from_secs(5)).unwrap();
        b.wait();
        AccessStore::new(&mut db)
            .unwrap()
            .set_archived(Actor::Admin(global), ws, false, NOW)
            .is_ok()
    });
    let deleted = deletion.join().unwrap();
    let reactivated = reactivation.join().unwrap();
    assert_ne!(deleted, reactivated);
    let db = Connection::open(&path).unwrap();
    let state: String = db
        .query_row(
            "SELECT state FROM ccp_workspaces WHERE id=?1",
            [ws.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, if deleted { "purged" } else { "active" });
    assert_eq!(rows(&db, "ccp_workspace_deletions"), i64::from(deleted));
    drop(db);
    fs::remove_file(path).unwrap();
}
