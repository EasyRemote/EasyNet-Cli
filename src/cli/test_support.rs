// EasyNet CLI — Test Support
// ==========================
//
// File: src/cli/test_support.rs
// Description: Shared helpers for tests that need to override `$HOME`.
//
// Several CLI modules persist state under `~/.easynet/...` via
// `shared::config::state_dir()`, which derives from `$HOME`. Tests have
// to point HOME at a unique tempdir before they touch the disk, but
// `cargo test` runs tests in parallel across the whole binary — and
// `std::env::set_var` is process-wide. If two test modules each defined
// their own private `HOME_LOCK`, they could be holding their own locks
// simultaneously and stomp on HOME mid-test. Hence: ONE global lock,
// shared across every test module via this helper.
//
// Usage:
//   #[cfg(test)]
//   use crate::cli::test_support::HomeGuard;
//
//   #[test]
//   fn my_disk_test() {
//       let _g = HomeGuard::new();
//       // …everything that touches state_dir() is now isolated…
//   }
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

/// Process-wide lock. Every test that mutates `$HOME` must hold this.
static HOME_LOCK: Mutex<()> = Mutex::new(());

pub struct HomeGuard {
    _lock: MutexGuard<'static, ()>,
    prev: Option<String>,
    path: PathBuf,
}

impl HomeGuard {
    pub fn new() -> Self {
        let lock = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("HOME").ok();
        let path = std::env::temp_dir()
            .join(format!("easynet-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).expect("create tempdir");
        // SAFETY: process-wide env mutation is serialised by HOME_LOCK.
        std::env::set_var("HOME", &path);
        Self {
            _lock: lock,
            prev,
            path,
        }
    }

}

impl Drop for HomeGuard {
    fn drop(&mut self) {
        match &self.prev {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
