// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Deterministic synchronization points for concurrency tests. Every hook
//! compiles away outside `#[cfg(test)]`; production builds never pay for or
//! depend on this module.

#[cfg(test)]
pub(crate) use armed::{Gate, arm};

/// Called by production code right before a persist step a test may want to
/// park on. A no-op unless a test has armed `label` with [`arm`].
#[cfg(test)]
pub(crate) async fn checkpoint(label: &'static str) {
    if let Some((reached, release, _blocking_release)) = armed::take(label) {
        reached.notify_one();
        release.notified().await;
    }
}

#[cfg(test)]
mod armed {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, OnceLock, mpsc};

    use tokio::sync::Notify;

    struct Checkpoint {
        reached: Arc<Notify>,
        release: Arc<Notify>,
        blocking_release: mpsc::Receiver<()>,
    }

    static REGISTRY: OnceLock<Mutex<HashMap<&'static str, Checkpoint>>> = OnceLock::new();

    fn registry() -> &'static Mutex<HashMap<&'static str, Checkpoint>> {
        REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
    }

    /// A test-controlled gate for one `label`. Dropping it clears the gate.
    pub(crate) struct Gate {
        label: &'static str,
        reached: Arc<Notify>,
        release: Arc<Notify>,
        blocking_release: mpsc::Sender<()>,
    }

    impl Gate {
        /// Waits until production code reaches `checkpoint(label)`.
        pub(crate) async fn wait_until_reached(&self) {
            self.reached.notified().await;
        }

        /// Lets the paused production code continue past the checkpoint.
        pub(crate) fn release(&self) {
            self.release.notify_one();
            self.blocking_release.send(()).unwrap_or(());
        }
    }

    impl Drop for Gate {
        fn drop(&mut self) {
            self.release.notify_one();
            self.blocking_release.send(()).unwrap_or(());
            registry()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(self.label);
        }
    }

    /// Arms a checkpoint so the next `checkpoint(label)` call parks there
    /// until [`Gate::release`] is called.
    pub(crate) fn arm(label: &'static str) -> Gate {
        let reached = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let (blocking_release, blocking_receiver) = mpsc::channel();
        registry()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(
                label,
                Checkpoint {
                    reached: Arc::clone(&reached),
                    release: Arc::clone(&release),
                    blocking_release: blocking_receiver,
                },
            );
        Gate {
            label,
            reached,
            release,
            blocking_release,
        }
    }

    pub(super) fn take(
        label: &'static str,
    ) -> Option<(Arc<Notify>, Arc<Notify>, mpsc::Receiver<()>)> {
        registry()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(label)
            .map(|checkpoint| {
                (
                    Arc::clone(&checkpoint.reached),
                    Arc::clone(&checkpoint.release),
                    checkpoint.blocking_release,
                )
            })
    }
}
