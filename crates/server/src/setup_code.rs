// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Process-local setup capability. This does not grant a server/workspace role.
//! The owner transaction must independently refuse any already-claimed instance.
use sha2::{Digest, Sha256};
use std::{sync::Mutex, time::SystemTime};
use subtle::ConstantTimeEq;
use tokio::time::{Duration, Instant};

const PREFIX: &str = "ccp_setup_";
const CODE_BYTES: usize = 32;
const CODE_LENGTH: usize = PREFIX.len() + CODE_BYTES * 2;
const LIFETIME: Duration = Duration::from_secs(60 * 60);
const WINDOW: Duration = Duration::from_secs(10);
const ATTEMPTS: u16 = 16;

/// Intentionally contains no Debug/Serialize implementation: only the startup
/// caller receives the one-time plaintext code and may construct the setup URL.
pub struct SetupCode {
    state: Mutex<State>,
}
struct State {
    verifier: Option<[u8; CODE_BYTES]>,
    deadline: Instant,
    wall_deadline: SystemTime,
    window: Instant,
    attempts: u16,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    Unauthorized,
    Expired,
    Throttled,
    Claimed,
    Unavailable,
}
#[derive(Debug)]
pub enum ClaimError<E> {
    Refused(Refusal),
    Operation(E),
}
/// The DB operation must distinguish a retryable rollback from an already-owned
/// instance, including a commit whose response was lost.
pub enum ClaimFailure<E> {
    Retryable(E),
    AlreadyClaimed,
}
impl SetupCode {
    /// Each new pending process gets fresh entropy; no bearer is persisted here.
    /// The caller must keep the printed startup link in host-controlled logs.
    pub fn issue() -> Result<(Self, String), getrandom::Error> {
        let mut entropy = [0; CODE_BYTES];
        getrandom::getrandom(&mut entropy)?;
        let suffix: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
        let code = format!("{PREFIX}{suffix}");
        let now = Instant::now();
        let verifier = Sha256::digest(code.as_bytes()).into();
        Ok((
            Self {
                state: Mutex::new(State {
                    verifier: Some(verifier),
                    deadline: now + LIFETIME,
                    wall_deadline: SystemTime::now() + LIFETIME,
                    window: now,
                    attempts: 0,
                }),
            },
            code,
        ))
    }

    /// A check never consumes the capability. Both checks and claims share the
    /// failed-attempt budget. A valid code always bypasses throttling. This check
    /// never waits behind an owner transaction and is safe on an async worker.
    pub fn verify(&self, code: &str) -> Result<(), Refusal> {
        let mut state = self
            .state
            .try_lock()
            .map_err(|_busy_or_poisoned| Refusal::Unavailable)?;
        state.check(code)
    }

    /// Serialize verification, the caller's atomic owner transaction and successful
    /// consumption. The synchronous operation must not invoke this guard recursively.
    /// Run this blocking entry point off async workers (e.g. spawn_blocking).
    /// Retryable errors preserve the capability; AlreadyClaimed consumes it. A
    /// committed transaction must return success despite subsequent delivery loss.
    /// An operation panic poisons the guard; it fails closed until process restart.
    /// Both monotonic and wall deadlines apply, so suspension cannot extend expiry.
    pub fn claim<T, E>(
        &self,
        code: &str,
        operation: impl FnOnce() -> Result<T, ClaimFailure<E>>,
    ) -> Result<T, ClaimError<E>> {
        let mut state = self
            .state
            .lock()
            .map_err(|_poisoned| ClaimError::Refused(Refusal::Unavailable))?;
        state.check(code).map_err(ClaimError::Refused)?;
        let value = match operation() {
            Ok(value) => value,
            Err(ClaimFailure::Retryable(error)) => return Err(ClaimError::Operation(error)),
            Err(ClaimFailure::AlreadyClaimed) => {
                state.verifier = None;
                return Err(ClaimError::Refused(Refusal::Claimed));
            }
        };
        state.verifier = None;
        Ok(value)
    }
}
impl State {
    fn check(&mut self, code: &str) -> Result<(), Refusal> {
        let Some(expected) = self.verifier else {
            return Err(Refusal::Claimed);
        };
        let now = Instant::now();
        if now >= self.deadline || SystemTime::now() >= self.wall_deadline {
            return Err(Refusal::Expired);
        }
        if code.len() == CODE_LENGTH && code.starts_with(PREFIX) {
            let actual: [u8; CODE_BYTES] = Sha256::digest(code.as_bytes()).into();
            if bool::from(actual.ct_eq(&expected)) {
                return Ok(());
            }
        }
        if now.duration_since(self.window) >= WINDOW {
            self.window = now;
            self.attempts = 0;
        }
        if self.attempts >= ATTEMPTS {
            return Err(Refusal::Throttled);
        }
        self.attempts += 1;
        Err(Refusal::Unauthorized)
    }
}

#[cfg(test)]
mod tests;
