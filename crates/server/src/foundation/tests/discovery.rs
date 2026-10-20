// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

fn key(f: &mut Fixture, principal: Uuid, scope: AdminKeyScope) -> IssuedAdminKey {
    AccessStore::new(&mut f.connection)
        .unwrap()
        .issue_admin_key(
            Actor::Admin(principal),
            &AdminKeyRequest {
                principal_id: principal,
                scope,
                name: "page key".into(),
                expires_at: None,
            },
            NOW,
        )
        .unwrap()
}

#[test]
fn workspace_pages_cover_unicode_and_legacy_name_collisions_once_in_stable_order() {
    let mut f = fixture();
    let global = f.global;
    let key = key(&mut f, global, AdminKeyScope::Instance);
    let actor = Actor::AdminKey(key.id);
    for name in ["quote\"", "α", "zebra"] {
        AccessStore::new(&mut f.connection)
            .unwrap()
            .create_workspace(actor, name, f.owner, NOW)
            .unwrap();
    }
    f.connection
        .execute("INSERT INTO sessions(id,name) VALUES (77,'alpha')", [])
        .unwrap();
    map_legacy_session(&f.connection, 77).unwrap();
    let mut after = None;
    let mut found = Vec::new();
    loop {
        let page = AccessStore::new(&mut f.connection)
            .unwrap()
            .visible_workspaces_page(actor, after.as_deref(), 2, NOW)
            .unwrap();
        assert!(page.workspaces.len() <= 2);
        found.extend(
            page.workspaces
                .into_iter()
                .map(|w| (w.name, w.id.to_string())),
        );
        match page.next_cursor {
            Some(cursor) => after = Some(cursor),
            None => break,
        }
    }
    assert_eq!(found.len(), 6);
    let mut sorted = found.clone();
    sorted.sort();
    assert_eq!(found, sorted);
    let mut ids: Vec<_> = found.into_iter().map(|row| row.1).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 6);
}

#[test]
fn workspace_pages_never_borrow_global_roles_or_unbound_client_permissions() {
    let mut f = fixture();
    let owner = f.owner;
    let scope = f.workspace;
    let scoped = key(&mut f, owner, AdminKeyScope::Workspace(scope));
    f.connection
        .execute(
            "INSERT INTO ccp_server_admins(admin_id) VALUES (?1)",
            [owner.to_string()],
        )
        .unwrap();
    let page = AccessStore::new(&mut f.connection)
        .unwrap()
        .visible_workspaces_page(Actor::AdminKey(scoped.id), None, 500, NOW)
        .unwrap();
    assert_eq!(page.workspaces.len(), 1);
    assert_eq!(page.workspaces[0].id, scope);
    assert!(page.next_cursor.is_none());
    let enrolled = enroll(&mut f, DataRole::Read);
    let actor = Actor::Credential(enrolled.credential_id);
    let page = AccessStore::new(&mut f.connection)
        .unwrap()
        .visible_workspaces_page(actor, None, 1, NOW)
        .unwrap();
    assert_eq!(page.workspaces.len(), 1);
    assert!(page.next_cursor.is_none());
    f.connection
        .execute(
            "UPDATE ccp_grants SET revoked_at=1 WHERE id=?1",
            [enrolled.grant_id.to_string()],
        )
        .unwrap();
    assert!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .visible_workspaces_page(actor, None, 1, NOW)
            .unwrap()
            .workspaces
            .is_empty()
    );
    f.connection
        .execute(
            "UPDATE ccp_clients SET disabled=1 WHERE id=?1",
            [enrolled.client_id.to_string()],
        )
        .unwrap();
    assert!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .visible_workspaces_page(actor, None, 1, NOW)
            .is_err()
    );
}

#[test]
fn workspace_page_bounds_and_cursors_are_non_authoritative_and_current_roles_are_rechecked() {
    let mut f = fixture();
    let global = f.global;
    let operator = key(&mut f, global, AdminKeyScope::Instance);
    let actor = Actor::AdminKey(operator.id);
    for limit in [0, 501] {
        assert!(
            AccessStore::new(&mut f.connection)
                .unwrap()
                .visible_workspaces_page(actor, None, limit, NOW)
                .is_err()
        );
    }
    for value in [
        "not json",
        "[]",
        "[\"alpha\",\"not UUID\"]",
        &"x".repeat(1025),
    ] {
        assert!(
            AccessStore::new(&mut f.connection)
                .unwrap()
                .visible_workspaces_page(actor, Some(value), 1, NOW)
                .is_err()
        );
    }
    let enrolled = enroll(&mut f, DataRole::Read);
    let cursor = serde_json::to_string(&("beta", f.other_workspace.to_string())).unwrap();
    let page = AccessStore::new(&mut f.connection)
        .unwrap()
        .visible_workspaces_page(
            Actor::Credential(enrolled.credential_id),
            Some(&cursor),
            10,
            NOW,
        )
        .unwrap();
    assert!(page.workspaces.is_empty());
    f.connection
        .execute(
            "DELETE FROM ccp_server_admins WHERE admin_id=?1",
            [global.to_string()],
        )
        .unwrap();
    assert!(
        AccessStore::new(&mut f.connection)
            .unwrap()
            .visible_workspaces_page(actor, None, 1, NOW)
            .is_err()
    );
}
