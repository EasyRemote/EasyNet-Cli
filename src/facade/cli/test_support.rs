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
// This module is gated at the declaration site (`src/cli/mod.rs` with
// `#[cfg(test)] pub mod test_support;`), so it never appears in release
// builds. Do NOT add an inner `#![cfg(test)]` here — clippy flags it as a
// duplicated attribute, and the outer gate is the single source of truth.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

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
///   3. Saves the current `HOME` and `AXON_INVOCATION_LOG_DIR` env vars.
///   4. Sets `HOME` to the temp dir and `AXON_INVOCATION_LOG_DIR` to a
///      sibling dir under it (so PR-7 Timeline events are isolated per test).
///   5. On drop, restores the env vars and removes the temp dir.
///
/// Use it at the top of any test that calls `config::state_dir()`, any
/// function that persists to `~/.easynet/`, or any dispatch path that
/// emits to the PR-7 PersistentLog (which reads
/// `AXON_INVOCATION_LOG_DIR`).
pub struct HomeGuard {
    _lock: MutexGuard<'static, ()>,
    temp_dir: PathBuf,
    prev_home: Option<String>,
    prev_axon_log_dir: Option<String>,
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

        // Redirect the PR-7 Timeline log dir too. Without this, every
        // dispatch test would write uuid-named files into the real
        // `$TMPDIR/axon-invocations/`, leaving cruft across runs and
        // making cross-test event counts unpredictable if an earlier
        // test allocated a uuid that collides (statistically zero, but
        // the contract is "test isolation," not "statistically likely").
        let axon_log_dir = temp_dir.join("axon-invocations");
        let _ = std::fs::create_dir_all(&axon_log_dir);
        let prev_axon_log_dir = std::env::var("AXON_INVOCATION_LOG_DIR").ok();
        std::env::set_var("AXON_INVOCATION_LOG_DIR", &axon_log_dir);

        Self {
            _lock: lock,
            temp_dir,
            prev_home,
            prev_axon_log_dir,
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
        match self.prev_axon_log_dir.take() {
            Some(d) => std::env::set_var("AXON_INVOCATION_LOG_DIR", d),
            None => std::env::remove_var("AXON_INVOCATION_LOG_DIR"),
        }
        let _ = std::fs::remove_dir_all(&self.temp_dir);
    }
}
