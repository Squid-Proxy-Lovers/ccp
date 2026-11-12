// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;
use std::sync::{
    Arc, Barrier,
    atomic::{AtomicUsize, Ordering},
};

#[tokio::test(start_paused = true)]
async fn verify_does_not_claim_success_consumes_and_later_requests_never_execute() {
    let (gate, code) = SetupCode::issue().unwrap();
    assert_eq!(gate.verify(&code), Ok(()));
    let calls = AtomicUsize::new(0);
    assert_eq!(
        gate.claim(&code, || {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok::<_, ClaimFailure<()>>(42)
        })
        .unwrap(),
        42
    );
    assert_eq!(gate.verify(&code), Err(Refusal::Claimed));
    assert!(matches!(
        gate.claim(&code, || {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok::<_, ClaimFailure<()>>(())
        }),
        Err(ClaimError::Refused(Refusal::Claimed))
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
#[tokio::test(start_paused = true)]
async fn invalid_and_expired_codes_do_not_execute_the_operation() {
    let (gate, code) = SetupCode::issue().unwrap();
    let calls = AtomicUsize::new(0);
    for bad in [
        String::new(),
        "ccp_setup_".into(),
        format!("{code}x"),
        format!("{}{}", PREFIX, "z".repeat(64)),
    ] {
        assert!(matches!(
            gate.claim(&bad, || {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok::<_, ClaimFailure<()>>(())
            }),
            Err(ClaimError::Refused(Refusal::Unauthorized))
        ));
    }
    tokio::time::advance(LIFETIME).await;
    assert_eq!(gate.verify(&code), Err(Refusal::Expired));
    assert!(matches!(
        gate.claim(&code, || {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok::<_, ClaimFailure<()>>(())
        }),
        Err(ClaimError::Refused(Refusal::Expired))
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}
#[tokio::test(start_paused = true)]
async fn throttling_recovers_after_window_without_refreshing_lifetime() {
    let (gate, code) = SetupCode::issue().unwrap();
    for _ in 0..ATTEMPTS {
        assert_eq!(gate.verify("invalid"), Err(Refusal::Unauthorized));
    }
    assert_eq!(gate.verify(&code), Ok(()));
    assert_eq!(gate.verify("invalid"), Err(Refusal::Throttled));
    tokio::time::advance(WINDOW).await;
    assert_eq!(gate.verify(&code), Ok(()));
    tokio::time::advance(LIFETIME - WINDOW).await;
    assert_eq!(gate.verify(&code), Err(Refusal::Expired));
}
#[tokio::test]
async fn failed_transaction_preserves_code_and_restart_uses_fresh_entropy() {
    let (gate, code) = SetupCode::issue().unwrap();
    assert!(matches!(
        gate.claim(&code, || Err::<(), _>(ClaimFailure::Retryable("rollback"))),
        Err(ClaimError::Operation("rollback"))
    ));
    assert_eq!(gate.verify(&code), Ok(()));
    let (new, next) = SetupCode::issue().unwrap();
    assert_ne!(code, next);
    assert_eq!(new.verify(&code), Err(Refusal::Unauthorized));
    assert_eq!(new.verify(&next), Ok(()));
}
#[tokio::test]
async fn concurrent_claims_execute_exactly_one_successful_owner_operation() {
    let (gate, code) = SetupCode::issue().unwrap();
    let gate = Arc::new(gate);
    let code = Arc::new(code);
    let barrier = Arc::new(Barrier::new(8));
    let calls = Arc::new(AtomicUsize::new(0));
    let tasks: Vec<_> = (0..8)
        .map(|_| {
            let (gate, code, barrier, calls) =
                (gate.clone(), code.clone(), barrier.clone(), calls.clone());
            std::thread::spawn(move || {
                barrier.wait();
                gate.claim(&code, || {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok::<_, ClaimFailure<()>>(())
                })
            })
        })
        .collect();
    let mut successes = 0;
    for task in tasks {
        match task.join().unwrap() {
            Ok(()) => successes += 1,
            Err(ClaimError::Refused(Refusal::Claimed)) => {}
            other => panic!("unexpected losing claim: {other:?}"),
        }
    }
    assert_eq!(successes, 1);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn flooding_cannot_lock_out_the_valid_owner_and_bad_claim_never_executes() {
    let (gate, code) = SetupCode::issue().unwrap();
    for _ in 0..ATTEMPTS {
        assert_eq!(gate.verify("junk"), Err(Refusal::Unauthorized));
    }
    let calls = AtomicUsize::new(0);
    assert!(matches!(
        gate.claim("junk", || {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok::<_, ClaimFailure<()>>(())
        }),
        Err(ClaimError::Refused(Refusal::Throttled))
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(gate.verify(&code), Ok(()));
    assert!(
        gate.claim(&code, || {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok::<_, ClaimFailure<()>>(())
        })
        .is_ok()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
#[tokio::test(start_paused = true)]
async fn deadline_is_exclusive_and_wall_time_also_caps_the_code() {
    let (gate, code) = SetupCode::issue().unwrap();
    tokio::time::advance(LIFETIME - Duration::from_millis(1)).await;
    assert_eq!(gate.verify(&code), Ok(()));
    tokio::time::advance(Duration::from_millis(1)).await;
    assert_eq!(gate.verify(&code), Err(Refusal::Expired));
    let (gate, code) = SetupCode::issue().unwrap();
    gate.state.lock().unwrap().wall_deadline = SystemTime::UNIX_EPOCH;
    assert_eq!(gate.verify(&code), Err(Refusal::Expired));
}
#[test]
fn correct_format_wrong_hex_and_terminal_claim_fail_closed() {
    let (gate, code) = SetupCode::issue().unwrap();
    let last = code.chars().last().unwrap();
    let mut wrong = code.clone();
    wrong.pop();
    wrong.push(if last == '0' { '1' } else { '0' });
    assert_eq!(wrong.len(), code.len());
    assert_eq!(gate.verify(&wrong), Err(Refusal::Unauthorized));
    assert!(matches!(
        gate.claim(&code, || Err::<(), ClaimFailure<()>>(
            ClaimFailure::AlreadyClaimed
        )),
        Err(ClaimError::Refused(Refusal::Claimed))
    ));
    assert_eq!(gate.verify(&code), Err(Refusal::Claimed));
}
#[test]
fn verify_does_not_wait_for_a_claim_and_failed_first_claim_can_be_retried() {
    let (gate, code) = SetupCode::issue().unwrap();
    let gate = Arc::new(gate);
    let (entered, wait) = std::sync::mpsc::channel();
    let (release, released) = std::sync::mpsc::channel();
    let worker = {
        let gate = gate.clone();
        let code = code.clone();
        std::thread::spawn(move || {
            gate.claim(&code, || {
                entered.send(()).unwrap();
                released.recv().unwrap();
                Err::<(), _>(ClaimFailure::Retryable("rollback"))
            })
        })
    };
    wait.recv().unwrap();
    let (checked, result) = std::sync::mpsc::channel();
    let verifier = {
        let gate = gate.clone();
        let code = code.clone();
        std::thread::spawn(move || checked.send(gate.verify(&code)).unwrap())
    };
    let observed = result.recv_timeout(std::time::Duration::from_secs(1));
    // Always release/join before asserting, so restoring the blocking-lock bug
    // causes a bounded failure rather than hanging the test suite.
    release.send(()).unwrap();
    verifier.join().unwrap();
    assert_eq!(observed.unwrap(), Err(Refusal::Unavailable));
    assert!(matches!(
        worker.join().unwrap(),
        Err(ClaimError::Operation("rollback"))
    ));
    assert_eq!(gate.verify(&code), Ok(()));
    assert!(gate.claim(&code, || Ok::<_, ClaimFailure<()>>(())).is_ok());
    assert_eq!(gate.verify(&code), Err(Refusal::Claimed));
}
#[test]
fn panic_poisoning_requires_restart_and_never_executes_a_later_operation() {
    let (gate, code) = SetupCode::issue().unwrap();
    let result = std::panic::catch_unwind(|| {
        gate.claim(&code, || -> Result<(), ClaimFailure<()>> {
            panic!("fixture transaction panic")
        })
    });
    assert!(result.is_err());
    assert_eq!(gate.verify(&code), Err(Refusal::Unavailable));
    assert!(matches!(
        gate.claim(&code, || Ok::<_, ClaimFailure<()>>(())),
        Err(ClaimError::Refused(Refusal::Unavailable))
    ));
}
