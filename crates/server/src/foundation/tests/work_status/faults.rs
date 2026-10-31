// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

#[test]
fn ignored_or_aborted_head_receipt_and_audit_roll_back_the_complete_status_change() {
    for (table, event) in [
        ("ccp_work_status", "INSERT"),
        ("ccp_work_status", "UPDATE"),
        ("ccp_work_status_operations", "INSERT"),
        ("ccp_access_audit", "INSERT"),
    ] {
        for fault in ["IGNORE", "ABORT,'private fault'"] {
            let mut f = fixture();
            let a = enroll(&mut f, DataRole::ReadWrite);
            let id = Uuid::new_v4();
            let existing = event == "UPDATE";
            if existing {
                apply(&mut f, a.credential_id, &post(id, 0, "old"), NOW).unwrap();
            }
            let heads = rows(&f.connection, "ccp_work_status");
            let receipts = rows(&f.connection, "ccp_work_status_operations");
            let audits = rows(&f.connection, "ccp_access_audit");
            f.connection.execute_batch(&format!("CREATE TRIGGER status_fault BEFORE {event} ON {table} BEGIN SELECT RAISE({fault}); END;")).unwrap();
            let request = post(id, u64::from(existing), "new");
            assert!(apply(&mut f, a.credential_id, &request, NOW + 1).is_err());
            assert_eq!(rows(&f.connection, "ccp_work_status"), heads);
            assert_eq!(rows(&f.connection, "ccp_work_status_operations"), receipts);
            assert_eq!(rows(&f.connection, "ccp_access_audit"), audits);
            if existing {
                assert_eq!(read(&mut f, id, NOW + 1).message.as_deref(), Some("old"));
                assert_eq!(read(&mut f, id, NOW + 1).revision, 1);
            }
            f.connection
                .execute_batch("DROP TRIGGER status_fault")
                .unwrap();
            assert_eq!(
                apply(&mut f, a.credential_id, &request, NOW + 1)
                    .unwrap()
                    .revision,
                if existing { 2 } else { 1 }
            );
        }
    }
}

#[test]
fn ownership_receipt_and_tombstone_constraints_prevent_untracked_removal() {
    let mut f = fixture();
    let a = enroll(&mut f, DataRole::ReadWrite);
    let b = enroll(&mut f, DataRole::ReadWrite);
    let id = Uuid::new_v4();
    apply(&mut f, a.credential_id, &post(id, 0, "retained"), NOW).unwrap();
    assert!(
        f.connection
            .execute(
                "UPDATE ccp_work_status SET client_id=?2 WHERE id=?1",
                params![id.to_string(), b.client_id.to_string()]
            )
            .is_err()
    );
    assert!(
        f.connection
            .execute("DELETE FROM ccp_work_status WHERE id=?1", [id.to_string()])
            .is_err()
    );
    assert!(
        f.connection
            .execute("DELETE FROM ccp_work_status_operations", [])
            .is_err()
    );
    assert!(
        f.connection
            .execute("UPDATE ccp_work_status_operations SET revision=99", [])
            .is_err()
    );
    assert!(f.connection.execute("INSERT INTO ccp_work_status_operations(workspace_id,operation_id,actor_kind,actor_id,request_hash,status_id,client_id,revision,cleared,updated_at) VALUES(?1,?2,'credential',?3,zeroblob(32),?4,?5,99,0,1000)",params![f.workspace.to_string(),Uuid::new_v4().to_string(),a.credential_id.to_string(),id.to_string(),a.client_id.to_string()]).is_err());
    assert_eq!(read(&mut f, id, NOW).revision, 1);
    assert_eq!(rows(&f.connection, "ccp_work_status_operations"), 1);
}
