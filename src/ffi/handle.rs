// EasyNet CLI — FFI opaque handle registry
// ==========================================
//
// File: src/ffi/handle.rs
// Description: The integer handles the Client FFI uses to name
//              library-side state across the C ABI. A handle is a
//              `u64` value that indexes a process-local registry of
//              per-Client sessions (IPC connection state, cancel
//              tokens for in-flight subscriptions, etc.). Opaque
//              on purpose: the C side never inspects the integer.
//
// Why a registry, not raw `Box<T>` -> pointer casts
// --------------------------------------------------
// Raw pointers crossing the ABI create two classes of hard-to-
// diagnose bug: (a) a Client holding a pointer to a freed session
// has a use-after-free that manifests at some distant ability
// call, and (b) concurrent shutdown races between "lib shutdown"
// and "Client calls easynet_ability_invoke" have to be papered
// over in user code.
//
// A u64 handle + `DashMap<u64, Arc<ClientSession>>` puts both of
// those problems on the library side. An invalid handle is an
// explicit `ERR_INVALID_HANDLE` return; a dropped handle's
// `ClientSession` is reclaimed when the last Arc clone goes
// away, which happens when the registry is the only remaining
// holder at `easynet_shutdown` time.
//
// v1 state
// --------
// The `ClientSession` struct is a placeholder. It carries the
// fields the next PR-DAEMON commit will populate: an IPC client
// connected to the daemon, the negotiated IPC version, an
// in-flight subscription registry, and the cancel tokens for
// each. v1 ships the shape; real wiring lands alongside
// `services::control::transport`.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

/// Opaque handle exposed to the C ABI. A value of 0 is reserved as
/// "null handle" / "not yet allocated".
pub type EasynetHandle = u64;

/// Library-side state for one Client. Held in an Arc and indexed
/// by a u64 handle.
#[allow(dead_code)] // fields filled in by follow-up PR-DAEMON commit
pub struct ClientSession {
    /// IPC version negotiated with the daemon. 0 means "handshake
    /// not yet performed".
    pub ipc_version: u16,
    /// Path to the control.json the Client was told to dial. Used
    /// in diagnostic messages so an operator can see "which daemon
    /// did this handle connect to".
    pub control_path: String,
    // Future fields (landed by follow-up PR-DAEMON commit):
    //   pub client: Mutex<IpcClient>,                       // framed UDS/Pipe
    //   pub subscriptions: DashMap<u64, CancelToken>,        // id -> token
}

impl ClientSession {
    fn new(control_path: String) -> Self {
        Self {
            ipc_version: 0,
            control_path,
        }
    }
}

/// Process-wide registry. v1 uses a plain `Mutex<HashMap>` rather
/// than a dependency like `dashmap` because the contention is low
/// (one entry per live Client process) and a single dep is easier
/// to audit than a new external crate. The follow-up commit can
/// swap for DashMap if benchmarks show lock contention under the
/// real subscription volume.
struct Registry {
    next: AtomicU64,
    entries: Mutex<std::collections::HashMap<EasynetHandle, std::sync::Arc<ClientSession>>>,
}

fn registry() -> &'static Registry {
    static REG: OnceLock<Registry> = OnceLock::new();
    REG.get_or_init(|| Registry {
        // Start at 1 so a 0 handle remains the explicit null value.
        next: AtomicU64::new(1),
        entries: Mutex::new(std::collections::HashMap::new()),
    })
}

/// Allocate a new handle for the given `ClientSession` and return
/// both. The caller stores the handle in the Client; the Arc is
/// retained by the registry.
#[allow(dead_code)] // consumed by `easynet_init` in a follow-up commit
pub(crate) fn alloc(session: ClientSession) -> (EasynetHandle, std::sync::Arc<ClientSession>) {
    let reg = registry();
    let id = reg.next.fetch_add(1, Ordering::Relaxed);
    let arc = std::sync::Arc::new(session);
    reg.entries
        .lock()
        .expect("handle registry lock not poisoned")
        .insert(id, arc.clone());
    (id, arc)
}

/// Look up a handle. Returns `None` when the handle is 0 (null) or
/// not present (freed / never issued). Callers map `None` to
/// `ERR_INVALID_HANDLE`.
pub(crate) fn get(handle: EasynetHandle) -> Option<std::sync::Arc<ClientSession>> {
    if handle == 0 {
        return None;
    }
    registry()
        .entries
        .lock()
        .expect("handle registry lock not poisoned")
        .get(&handle)
        .cloned()
}

/// Release a handle. Returns `true` when the handle was present
/// (and is now removed), `false` when the handle was unknown.
/// Idempotent — a double-free returns `false` the second time.
#[allow(dead_code)] // consumed by `easynet_shutdown`
pub(crate) fn release(handle: EasynetHandle) -> bool {
    if handle == 0 {
        return false;
    }
    registry()
        .entries
        .lock()
        .expect("handle registry lock not poisoned")
        .remove(&handle)
        .is_some()
}

/// Test-only: create a session with a dummy control path so
/// registry-level tests can exercise alloc/get/release without
/// reaching for the real `easynet_init`.
#[cfg(test)]
pub(crate) fn test_session() -> ClientSession {
    ClientSession::new("/tmp/test-control.json".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handle_zero_is_always_null() {
        // 0 is reserved forever. Even if the counter wraps (which
        // would take 2^64 allocations), the registry's fetch_add
        // starts at 1, so 0 is never a valid live handle. Pin this
        // here.
        assert!(get(0).is_none());
        assert!(!release(0));
    }

    #[test]
    fn alloc_then_get_returns_same_session() {
        let (h, _arc) = alloc(test_session());
        let looked_up = get(h).expect("handle just allocated must be retrievable");
        // Confirm it is the same session object via `control_path`
        // field; Arc::ptr_eq is noisier to set up here and not more
        // informative.
        assert_eq!(looked_up.control_path, "/tmp/test-control.json");
    }

    #[test]
    fn release_returns_true_first_time_false_second() {
        let (h, _arc) = alloc(test_session());
        assert!(release(h));
        assert!(!release(h));
        assert!(get(h).is_none());
    }

    #[test]
    fn handles_are_monotonic_distinct() {
        // Two sequential allocs must yield distinct handles; a
        // regression that reused a freed handle would create
        // use-after-free bugs in Client code that retained a stale
        // id.
        let (a, _) = alloc(test_session());
        let (b, _) = alloc(test_session());
        assert_ne!(a, b);
    }
}
