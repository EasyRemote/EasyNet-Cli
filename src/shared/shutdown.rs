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

impl ShutdownSignal {
    pub fn new() -> Self {
        Self {
            inner: Arc::new((Mutex::new(false), Condvar::new())),
        }
    }

    /// Signal shutdown — wakes all waiting threads.
    pub fn trigger(&self) {
        let (lock, cvar) = &*self.inner;
        let mut fired = lock.lock().unwrap();
        *fired = true;
        cvar.notify_all();
    }

    /// Returns true if shutdown has been signaled.
    pub fn is_triggered(&self) -> bool {
        let (lock, _) = &*self.inner;
        *lock.lock().unwrap()
    }

    /// Block until shutdown is signaled or `duration` elapses.
    /// Returns `true` if the timeout elapsed (shutdown was NOT signaled).
    pub fn wait_timeout(&self, duration: Duration) -> bool {
        let (lock, cvar) = &*self.inner;
        let fired = lock.lock().unwrap();
        if *fired {
            return false;
        }
        let (fired, timeout_result) = cvar.wait_timeout(fired, duration).unwrap();
        // Returns true if we timed out (i.e., shutdown was NOT signaled)
        timeout_result.timed_out() && !*fired
    }

    /// Block until shutdown is signaled (no timeout).
    pub fn wait(&self) {
        let (lock, cvar) = &*self.inner;
        let fired = lock.lock().unwrap();
        if *fired {
            return;
        }
        let _guard = cvar
            .wait_while(fired, |fired| !*fired)
            .unwrap();
    }
}
