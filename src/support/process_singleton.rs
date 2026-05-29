// EasyNet CLI — Process-wide singleton helper
// ===========================================
//
// File: src/support/process_singleton.rs
//
// Why this module exists
// ----------------------
// The daemon hosts several "set at boot, read on the hot path" handles
// — the live `McpClientService` (for `[exec] kind = "mcp"`), Axon
// runtime bridge state, the OpenAI-compat caller identity, etc. Each of
// these was historically a free-floating `OnceLock<Arc<T>>` or
// `RwLock<Option<Arc<T>>>` static at the top of its owning module.
//
// The two shapes are NOT stylistic — they correspond to two different
// test contracts:
//
//   * Production-only writers (`OnceLock`): one writer ever, "set
//     once at boot." Stronger guarantee — the read on the hot path
//     can never see a torn value, and a misbehaving test cannot swap
//     the handle out from under an in-flight call.
//
//   * Test-rebindable writers (`RwLock<Option<T>>`): the test binary
//     shares the static across many in-process test cases that each
//     want to install their own fixture. A `OnceLock` here would
//     silently pin the handle for every later test, which is the
//     exact stale-state bug the OpenAI-compat path paid for and now
//     needs `RwLock<Option<_>>` to avoid.
//
// Before this module existed, each call site picked one of the two
// shapes and wrote a doc-comment explaining *which* shape it picked
// and *why*. Reading the call sites required reading those two
// doc-comments and cross-referencing. [`ProcessSingleton<T>`]
// promotes the choice into a type-level enum (`Mode`) so the
// invariant the doc-comment described is now expressed in code.
//
// Public surface
// --------------
// One type, two constructors:
//
//   * `ProcessSingleton::<T>::once()` — production write-once. A
//     second `set` is a no-op (returns the canonical existing
//     handle); the dispatch hot path's `get()` is therefore racier
//     than `RwLock` only in the sense that a reader that hits before
//     boot finishes sees `None`, which is the exact contract callers
//     already handle (returning a typed "not yet initialised" error).
//
//   * `ProcessSingleton::<T>::last_writer_wins()` — test-rebindable.
//     `set` overwrites. Used in the OpenAI-compat path where the test
//     binary rebinds the registry between cases.
//
// Both expose the same API surface (`set`, `get`, `is_set`) so a
// future migration between modes is a single-line change at the
// declaration site, not a call-site rewrite.
//
// Why not crate-foreign deps (once_cell, arc-swap):
// `std::sync::OnceLock` and `std::sync::RwLock` are enough — the
// daemon's other process-wide handles already use them, and pulling
// a third "atomic Arc" crate just to express two flavours of "set
// once" is overweight.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::{Arc, OnceLock, RwLock};

/// Concurrency contract of a [`ProcessSingleton`]. The choice is
/// fixed at construction; it is intentionally NOT a runtime knob —
/// the lifecycle the singleton implements is a static property of
/// the call site, not a feature flag.
#[derive(Debug)]
enum SingletonStorage<T> {
    /// Production write-once. First `set` wins; later `set` calls
    /// silently return the existing handle. Read with no lock.
    Once(OnceLock<Arc<T>>),
    /// Test-rebindable. `set` overwrites; readers see the most
    /// recent write. RwLock-backed so concurrent readers are not
    /// serialised against each other.
    LastWriterWins(RwLock<Option<Arc<T>>>),
}

/// Process-wide singleton holding an `Arc<T>` set at daemon boot and
/// read on the dispatch hot path. See module header for the choice
/// between the two modes.
///
/// `T: Send + Sync + 'static` is the same bound `Arc<T>` already
/// enforces when shared across threads, which is the only way this
/// type gets used (declared as a `static`).
#[derive(Debug)]
pub struct ProcessSingleton<T: Send + Sync + 'static> {
    storage: SingletonStorage<T>,
}

impl<T: Send + Sync + 'static> ProcessSingleton<T> {
    /// Build a production write-once singleton. Use this when the
    /// only writer is the daemon's boot path and no test ever
    /// rebinds the handle mid-process.
    ///
    /// `const fn` so the singleton can be declared as a `static`.
    pub const fn once() -> Self {
        Self {
            storage: SingletonStorage::Once(OnceLock::new()),
        }
    }

    /// Build a test-rebindable singleton. Use this when the in-
    /// process test binary needs to install its own fixture per test
    /// case AND a stale read from an earlier-test handle would be
    /// observable. Production still calls `set` exactly once at
    /// boot; the lock cost is paid only on rare write paths.
    ///
    /// `const fn` so the singleton can be declared as a `static`.
    pub const fn last_writer_wins() -> Self {
        Self {
            storage: SingletonStorage::LastWriterWins(RwLock::new(None)),
        }
    }

    /// Install `value`. Returns the `Arc<T>` future readers will
    /// observe — either `value` (success) or the pre-existing
    /// handle (Once mode, second writer). LastWriterWins always
    /// returns `value`.
    ///
    /// **Observability**: a rejected second-writer in `Once` mode
    /// emits a `kind = second_writer_rejected level = warn`
    /// op-event so the integration-test scenario this guards
    /// against (boot path called twice in a single test binary,
    /// or a misconfigured component initialising the same
    /// singleton from two paths) leaves a grep-able trail rather
    /// than silently returning the wrong handle. Production sets
    /// each singleton exactly once at boot, so a real production
    /// occurrence of this event is itself a bug to investigate.
    ///
    /// **Diagnostic recipe** (kept out of the emitted log line so
    /// SRE pipelines that split on whitespace see stable field
    /// boundaries; the runbook lives here, the line stays terse):
    ///
    ///   * In production, the event indicates two boot paths are
    ///     racing on the same singleton — fix the boot wiring so
    ///     only one writer exists.
    ///   * In an integration test binary that shares the static
    ///     across cases, switch the declaration to
    ///     [`ProcessSingleton::last_writer_wins`] so each test can
    ///     install its own fixture.
    pub fn set(&self, value: Arc<T>) -> Arc<T> {
        match &self.storage {
            SingletonStorage::Once(cell) => match cell.set(value.clone()) {
                Ok(()) => value,
                Err(_rejected) => {
                    let type_name = std::any::type_name::<T>();
                    crate::op_event!(
                        component = process_singleton,
                        kind = second_writer_rejected,
                        level = "warn",
                        type_name = type_name,
                    );
                    cell.get()
                        .expect("OnceLock::set returned Err only when populated")
                        .clone()
                }
            },
            SingletonStorage::LastWriterWins(lock) => {
                let mut guard = lock.write().expect("ProcessSingleton RwLock poisoned");
                *guard = Some(value.clone());
                value
            }
        }
    }

    /// Read the currently-installed handle, if any.
    pub fn get(&self) -> Option<Arc<T>> {
        match &self.storage {
            SingletonStorage::Once(cell) => cell.get().cloned(),
            SingletonStorage::LastWriterWins(lock) => lock
                .read()
                .expect("ProcessSingleton RwLock poisoned")
                .clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Once-mode: first writer wins; second `set` returns the
    /// existing handle and DOES NOT mutate the slot. This is the
    /// guarantee the MCP executor's hot path relies on (no
    /// surprise swap mid-call).
    #[test]
    fn once_mode_first_writer_wins() {
        let s: ProcessSingleton<String> = ProcessSingleton::once();
        let first = Arc::new("first".to_string());
        let second = Arc::new("second".to_string());

        let installed = s.set(first.clone());
        assert!(Arc::ptr_eq(&installed, &first));

        let after_second = s.set(second.clone());
        assert!(
            Arc::ptr_eq(&after_second, &first),
            "second set must return the pre-existing handle, not the rejected one"
        );
        let observed = s.get().expect("set");
        assert!(Arc::ptr_eq(&observed, &first));
    }

    /// LastWriterWins-mode: later `set` overwrites. This is the
    /// guarantee the OpenAI-compat path needs because the in-process
    /// test binary installs its own fixture per case.
    #[test]
    fn last_writer_wins_mode_overwrites() {
        let s: ProcessSingleton<String> = ProcessSingleton::last_writer_wins();
        let first = Arc::new("first".to_string());
        let second = Arc::new("second".to_string());

        s.set(first);
        let after_second = s.set(second.clone());
        assert!(Arc::ptr_eq(&after_second, &second));
        let observed = s.get().expect("set");
        assert!(Arc::ptr_eq(&observed, &second));
    }

    /// Before `set` is called, both modes report `None`. Pinned so
    /// a future caller cannot mistake "unset" for "set to default"
    /// and dispatch against a phantom value.
    #[test]
    fn unset_returns_none_in_both_modes() {
        let once: ProcessSingleton<String> = ProcessSingleton::once();
        assert!(once.get().is_none());

        let lww: ProcessSingleton<String> = ProcessSingleton::last_writer_wins();
        assert!(lww.get().is_none());
    }

    /// `const fn` constructors must allow declaration as a `static`.
    /// Compilation alone is the assertion.
    #[test]
    fn const_constructors_allow_static_declaration() {
        static GLOBAL_ONCE: ProcessSingleton<u32> = ProcessSingleton::once();
        static GLOBAL_LWW: ProcessSingleton<u32> = ProcessSingleton::last_writer_wins();
        // Touch them so the linker keeps them in the test binary.
        assert!(GLOBAL_ONCE.get().is_none());
        assert!(GLOBAL_LWW.get().is_none());
    }
}
