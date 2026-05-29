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
    /// Snapshot of `EASYNET_PAGES_USER` at guard construction.
    /// HomeGuard pins this var to `"self"` for the test duration so
    /// every HOME-touching test sees the same `<user>.api_key.*` /
    /// `<user>.pages.*` / `<user>.files.*` registration set —
    /// regardless of what an earlier test on the same thread (or
    /// a parallel test that didn't take the home_lock) set it to.
    /// Restored on drop. The home_lock above guarantees one
    /// HomeGuard at a time, so the swap window is exclusive.
    prev_pages_user: Option<String>,
}

impl HomeGuard {
    pub fn new() -> Self {
        // Acquire the lock first so concurrent tests don't race on env var
        // mutation. If a previous test panicked while holding the lock, we
        // recover the poison and continue — the env vars are still
        // restored by the previous Drop impl.
        let lock = home_lock().lock().unwrap_or_else(|p| p.into_inner());

        // Build a unique temp dir under the OS temp root. We use a
        // process-global atomic counter rather than a timestamp
        // because the home_lock above serialises HomeGuard
        // construction — two tests can drop+acquire within the same
        // nanosecond when SystemTime resolution is coarser than the
        // lock-release-to-acquire latency. Counter eliminates the
        // collision class entirely; was the root cause of the
        // intermittent agent-registry test flake.
        static TEMPDIR_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let pid = std::process::id();
        let seq = TEMPDIR_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let temp_dir = std::env::temp_dir().join(format!("easynet-test-{pid}-{seq}"));
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

        // Clear EASYNET_PAGES_USER so every HomeGuard test sees
        // an unpaired daemon (no user-rooted ability family
        // registered). Tests that exercise the user-rooted family
        // build the registry inline with an explicit username,
        // bypassing the env var path entirely.
        //
        // Why clear (not pin to a value): pinning would still
        // register `<that-value>.api_key.*` etc., which (a) leaks
        // into every published-ability test's expected catalogue
        // and (b) reintroduces the `<self>` placeholder M5 banned.
        // An empty / absent var is the production "unpaired"
        // shape and the registry agrees by skipping registration.
        let prev_pages_user = std::env::var("EASYNET_PAGES_USER").ok();
        std::env::remove_var("EASYNET_PAGES_USER");

        Self {
            _lock: lock,
            temp_dir,
            prev_home,
            prev_axon_log_dir,
            prev_pages_user,
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
        match self.prev_pages_user.take() {
            Some(u) => std::env::set_var("EASYNET_PAGES_USER", u),
            None => std::env::remove_var("EASYNET_PAGES_USER"),
        }
        let _ = std::fs::remove_dir_all(&self.temp_dir);
    }
}
