// EasyNet CLI — Shutdown Signal
// ==============================
//
// File: src/shared/shutdown.rs
// Description: Condvar-based shutdown signal for clean blocking without busy-polling.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::{Arc, Condvar, Mutex};

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
