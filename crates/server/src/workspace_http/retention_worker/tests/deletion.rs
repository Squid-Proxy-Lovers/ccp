// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;
use crate::foundation::{Actor, WorkspaceDeletionRequest};

fn confirm(db: &mut Connection, workspace: Uuid) {
    let global: String = db
        .query_row("SELECT admin_id FROM ccp_server_admins LIMIT 1", [], |r| {
            r.get(0)
        })
        .unwrap();
    let actor = Actor::Admin(global.parse().unwrap());
    let mut s = AccessStore::new(db).unwrap();
    let now = crate::instance_tls::now().unwrap();
    s.set_archived(actor, workspace, true, now).unwrap();
    let p = s.workspace_deletion_preview(actor, workspace, now).unwrap();
    s.begin_workspace_deletion(
        actor,
        &WorkspaceDeletionRequest {
            workspace_id: workspace,
            operation_id: Uuid::new_v4(),
            expected_revision: p.lifecycle_revision,
            confirm_workspace_id: workspace,
        },
        now,
    )
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deletion_cursor_is_independent_and_a_fault_cannot_starve_later_confirmed_jobs() {
    let (directory, first) = setup();
    let server = server(&directory).await;
    let mut db = Connection::open(directory.join("ccp.sqlite3")).unwrap();
    let (global,owner):(String,String)=db.query_row("SELECT s.admin_id,w.owner_admin_id FROM ccp_server_admins s,ccp_workspaces w WHERE w.id=?1",[first.to_string()],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    let mut ids = vec![first];
    for index in 0..WORKSPACES {
        let id = AccessStore::new(&mut db)
            .unwrap()
            .create_workspace(
                Actor::Admin(global.parse().unwrap()),
                &format!("delete{index}"),
                owner.parse().unwrap(),
                crate::instance_tls::now().unwrap(),
            )
            .unwrap();
        ids.push(id);
    }
    for id in &ids {
        confirm(&mut db, *id);
    }
    ids.sort();
    let blocked = ids[0];
    db.execute_batch(&format!("CREATE TRIGGER deletion_worker_fault BEFORE UPDATE ON ccp_workspace_deletions WHEN NEW.workspace_id='{blocked}' BEGIN SELECT RAISE(ABORT,'PRIVATE_BODY'); END;")).unwrap();
    let mut cursor = None;
    let mut faults = 0;
    for _ in 0..15 {
        let c = cycle(&server.state, None, cursor).unwrap();
        cursor = c.deletion_after;
        faults += c.failures;
        assert!(c.after.is_none());
    }
    assert!(faults > 0);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM ccp_workspace_deletions WHERE completed_at IS NOT NULL",
            [],
            |r| r.get::<_, usize>(0)
        )
        .unwrap(),
        WORKSPACES
    );
    db.execute_batch("DROP TRIGGER deletion_worker_fault")
        .unwrap();
    for _ in 0..4 {
        cycle(&server.state, None, None).unwrap();
    }
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM ccp_workspace_deletions WHERE completed_at IS NULL",
            [],
            |r| r.get::<_, usize>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM ccp_content_entries", [], |r| r
            .get::<_, usize>(0))
            .unwrap(),
        0
    );
    drop(db);
    drop(server);
    std::fs::remove_dir_all(directory).unwrap();
}
