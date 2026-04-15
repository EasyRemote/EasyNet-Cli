// EasyNet CLI — Shutdown Signal
// ==============================
//
// File: src/shared/shutdown.rs
// Description: Condvar-based shutdown signal for clean blocking without busy-polling.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// A signal that can be awaited by one thread and triggered by another.
/// Uses `Condvar` for efficient blocking instead of busy-polling.
#[derive(Clone)]
pub struct ShutdownSignal {
    inner: Arc<(Mutex<bool>, Condvar)>,
}

impl Default for ShutdownSignal {
    fn default() -> Self {
        Self::new()
    }
}

impl ShutdownSignal {
    pub fn new() -> Self {
        Self {
            inner: Arc::new((Mutex::new(false), Condvar::new())),
        }
    }

    /// Signal shutdown — wakes all waiting threads.
    pub fn trigger(&self) {
        let (lock, cvar) = &*self.inner;
        let mut fired = lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *fired = true;
        cvar.notify_all();
    }

    /// Returns true if shutdown has been signaled.
    pub fn is_triggered(&self) -> bool {
        let (lock, _) = &*self.inner;
        *lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Sleep for `duration` unless shutdown is signaled first.
    /// Returns `true` if the caller should continue (timeout elapsed, no shutdown).
    /// Returns `false` if shutdown was signaled (caller should stop).
    pub fn sleep_unless_triggered(&self, duration: Duration) -> bool {
        let (lock, cvar) = &*self.inner;
        let fired = lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *fired {
            return false;
        }
        let result = cvar.wait_timeout(fired, duration);
        let (fired, timeout_result) = result.unwrap_or_else(std::sync::PoisonError::into_inner);
        // Continue if we timed out normally (no shutdown signal).
        timeout_result.timed_out() && !*fired
    }

    /// Block until shutdown is signaled (no timeout).
    pub fn wait(&self) {
        let (lock, cvar) = &*self.inner;
        let fired = lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *fired {
            return;
        }
        let _guard = cvar
            .wait_while(fired, |fired| !*fired)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
    }
}
