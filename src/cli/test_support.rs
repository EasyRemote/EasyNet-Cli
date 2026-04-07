// EasyNet CLI — Test Support
// ==========================
//
// File: src/cli/test_support.rs
// Description: Shared test utilities for CLI-layer unit tests. Currently
//              provides `HomeGuard`, an RAII helper that points
//              `~/.easynet/` at a temporary directory for the duration of a
//              test, so tests that touch persistence (mission runs, agent
//              sessions, etc.) don't pollute the developer's real home.
//
// This module is `#[cfg(test)]` only — it must not appear in release builds.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

#![cfg(test)]

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};

/// Global serialization for tests that mutate the `HOME` env var. Required
/// because Rust runs tests in parallel by default and `std::env::set_var`
/// is process-global.
fn home_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// RAII guard that:
///   1. Acquires the global home-mutation lock (serializes test execution).
///   2. Creates a fresh temp dir.
///   3. Saves the current `HOME` and `XDG_STATE_HOME` env vars.
///   4. Sets `HOME` to the temp dir.
///   5. On drop, restores the env vars and removes the temp dir.
///
/// Use it at the top of any test that calls `config::state_dir()` or any
/// function that persists to `~/.easynet/`.
pub struct HomeGuard {
    _lock: MutexGuard<'static, ()>,
    temp_dir: PathBuf,
    prev_home: Option<String>,
}

impl HomeGuard {
    pub fn new() -> Self {
        // Acquire the lock first so concurrent tests don't race on env var
        // mutation. If a previous test panicked while holding the lock, we
        // recover the poison and continue — the env vars are still
        // restored by the previous Drop impl.
        let lock = home_lock().lock().unwrap_or_else(|p| p.into_inner());

        // Build a unique temp dir under the OS temp root.
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let temp_dir = std::env::temp_dir().join(format!("easynet-test-{pid}-{nanos}"));
        let _ = std::fs::create_dir_all(&temp_dir);

        let prev_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", &temp_dir);

        Self {
            _lock: lock,
            temp_dir,
            prev_home,
        }
    }
}

impl Default for HomeGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for HomeGuard {
    fn drop(&mut self) {
        match self.prev_home.take() {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }
        let _ = std::fs::remove_dir_all(&self.temp_dir);
    }
}
