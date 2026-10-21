// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
const ORIGIN: &str = "https://ccp.test:8443";

#[test]
fn browser_tokens_are_distinct_origin_bound_and_csrf_is_per_session() {
    let issued_at = crate::instance_tls::now().unwrap();
    let table = Sessions::new().unwrap();
    let key = Uuid::new_v4();
    let one = table.issue(key, None, ORIGIN, None, issued_at).unwrap();
    let two = table.issue(key, None, ORIGIN, None, issued_at).unwrap();
    assert_eq!(one.token.len(), 64);
    assert_eq!(one.session.csrf.len(), 64);
    assert_ne!(one.token, two.token);
    assert_ne!(one.session.csrf, two.session.csrf);
    assert_eq!(
        table
            .resolve(&one.token, ORIGIN, Some(&one.session.csrf))
            .unwrap()
            .key_id,
        key
    );
    assert_eq!(
        table.resolve(&one.token, ORIGIN, None).unwrap().expires_at,
        issued_at + i64::try_from(MAX_AGE).unwrap()
    );
    assert!(matches!(
        table.resolve(&one.token, "https://ccp.test:8444", None),
        Err(Error::Unauthorized)
    ));
    assert!(matches!(
        table.resolve(&one.token, ORIGIN, Some(&two.session.csrf)),
        Err(Error::Forbidden)
    ));
    assert!(
        Sessions::new()
            .unwrap()
            .resolve(&one.token, ORIGIN, None)
            .is_err()
    );
    for invalid in [
        "",
        "PRIVATE_KEY",
        &"0".repeat(63),
        &"Z".repeat(64),
        &"é".repeat(32),
    ] {
        assert!(matches!(
            table.resolve(invalid, ORIGIN, None),
            Err(Error::Unauthorized)
        ));
        assert!(matches!(
            table.resolve(&one.token, ORIGIN, Some(invalid)),
            Err(Error::Forbidden)
        ));
    }
}

#[test]
fn expiration_is_absolute_and_replacement_or_logout_only_removes_the_expected_sid() {
    let issued_at = crate::instance_tls::now().unwrap();
    let table = Sessions::new().unwrap();
    let key = Uuid::new_v4();
    let first = table.issue(key, None, ORIGIN, None, issued_at).unwrap();
    let independent = table.issue(key, None, ORIGIN, None, issued_at).unwrap();
    let next = table
        .issue(
            Uuid::new_v4(),
            None,
            ORIGIN,
            Some(&first.token),
            issued_at + 10,
        )
        .unwrap();
    assert!(table.resolve(&first.token, ORIGIN, None).is_err());
    assert!(table.resolve(&independent.token, ORIGIN, None).is_ok());
    let before = table
        .records
        .lock()
        .unwrap()
        .get(&hash(&next.token))
        .unwrap()
        .deadline;
    table.resolve(&next.token, ORIGIN, None).unwrap();
    assert_eq!(
        table
            .records
            .lock()
            .unwrap()
            .get(&hash(&next.token))
            .unwrap()
            .deadline,
        before
    );
    table
        .records
        .lock()
        .unwrap()
        .get_mut(&hash(&next.token))
        .unwrap()
        .deadline = Instant::now();
    assert!(table.resolve(&next.token, ORIGIN, None).is_err());
    table.remove(&independent.token).unwrap();
    assert!(table.resolve(&independent.token, ORIGIN, None).is_err());
    assert!(table.records.lock().unwrap().is_empty());
}

#[test]
fn per_key_and_workspace_budgets_leave_capacity_for_instance_administration() {
    let issued_at = crate::instance_tls::now().unwrap();
    let table = Sessions::new().unwrap();
    let workspace = Uuid::new_v4();
    let key = Uuid::new_v4();
    let one = table
        .issue(key, Some(workspace), ORIGIN, None, issued_at)
        .unwrap();
    for _ in 1..PER_KEY {
        table
            .issue(key, Some(workspace), ORIGIN, None, issued_at)
            .unwrap();
    }
    assert!(
        table
            .issue(key, Some(workspace), ORIGIN, None, issued_at)
            .is_err()
    );
    let replacement = table
        .issue(key, Some(workspace), ORIGIN, Some(&one.token), issued_at)
        .unwrap();
    assert!(table.resolve(&one.token, ORIGIN, None).is_err());
    assert!(table.resolve(&replacement.token, ORIGIN, None).is_ok());
    for _ in 1..PER_WORKSPACE / PER_KEY {
        let key = Uuid::new_v4();
        for _ in 0..PER_KEY {
            table
                .issue(key, Some(workspace), ORIGIN, None, issued_at)
                .unwrap();
        }
    }
    assert!(
        table
            .issue(Uuid::new_v4(), Some(workspace), ORIGIN, None, issued_at)
            .is_err()
    );
    while table.records.lock().unwrap().len() < TOTAL - INSTANCE_RESERVE {
        let workspace = Uuid::new_v4();
        let key = Uuid::new_v4();
        for _ in 0..PER_KEY {
            table
                .issue(key, Some(workspace), ORIGIN, None, issued_at)
                .unwrap();
        }
    }
    assert!(
        table
            .issue(
                Uuid::new_v4(),
                Some(Uuid::new_v4()),
                ORIGIN,
                None,
                issued_at
            )
            .is_err()
    );
    for _ in 0..INSTANCE_RESERVE / PER_KEY {
        let key = Uuid::new_v4();
        for _ in 0..PER_KEY {
            table.issue(key, None, ORIGIN, None, issued_at).unwrap();
        }
    }
    assert_eq!(table.records.lock().unwrap().len(), TOTAL);
    assert!(
        table
            .issue(Uuid::new_v4(), None, ORIGIN, None, issued_at)
            .is_err()
    );
}

#[test]
fn failed_issue_preserves_old_session_and_expired_entries_are_pruned_for_capacity() {
    let issued_at = crate::instance_tls::now().unwrap();
    let table = Sessions::new().unwrap();
    let key = Uuid::new_v4();
    let first = table.issue(key, None, ORIGIN, None, issued_at).unwrap();
    for now in [-1, i64::MAX] {
        assert!(
            table
                .issue(key, None, ORIGIN, Some(&first.token), now)
                .is_err()
        );
        assert!(table.resolve(&first.token, ORIGIN, None).is_ok());
    }
    for _ in 1..PER_KEY {
        table.issue(key, None, ORIGIN, None, issued_at).unwrap();
    }
    assert!(
        table
            .issue(
                key,
                None,
                "https://foreign.test",
                Some(&first.token),
                issued_at
            )
            .is_err()
    );
    for record in table.records.lock().unwrap().values_mut() {
        record.deadline = Instant::now();
    }
    assert!(table.issue(key, None, ORIGIN, None, issued_at).is_ok());
    assert_eq!(table.records.lock().unwrap().len(), 1);
    for _ in 1..PER_KEY {
        table.issue(key, None, ORIGIN, None, issued_at).unwrap();
    }
    for record in table.records.lock().unwrap().values_mut() {
        record.expires_at = issued_at;
    }
    assert!(table.issue(key, None, ORIGIN, None, issued_at).is_ok());
    assert_eq!(table.records.lock().unwrap().len(), 1);
}

#[test]
fn poisoned_session_state_fails_closed_without_sensitive_error_details() {
    let issued_at = crate::instance_tls::now().unwrap();
    let table = std::sync::Arc::new(Sessions::new().unwrap());
    let one = table
        .issue(Uuid::new_v4(), None, ORIGIN, None, issued_at)
        .unwrap();
    let poison = table.clone();
    assert!(
        std::thread::spawn(move || {
            let _lock = poison.records.lock().unwrap();
            panic!("intentional test poison");
        })
        .join()
        .is_err()
    );
    assert!(matches!(
        table.issue(Uuid::new_v4(), None, ORIGIN, None, issued_at),
        Err(Error::Unavailable)
    ));
    assert!(matches!(
        table.resolve(&one.token, ORIGIN, None),
        Err(Error::Unavailable)
    ));
    assert!(matches!(table.remove(&one.token), Err(Error::Unavailable)));
}

#[test]
fn utc_deadline_is_enforced_even_while_the_monotonic_deadline_is_future() {
    let table = Sessions::new().unwrap();
    let now = crate::instance_tls::now().unwrap();
    let issued = table
        .issue(Uuid::new_v4(), None, ORIGIN, None, now)
        .unwrap();
    {
        let mut rows = table.records.lock().unwrap();
        let record = rows.get_mut(&hash(&issued.token)).unwrap();
        assert!(record.deadline > Instant::now());
        record.expires_at = now;
    }
    assert!(matches!(
        table.resolve(&issued.token, ORIGIN, None),
        Err(Error::Unauthorized)
    ));
}
