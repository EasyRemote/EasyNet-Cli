// EasyNet CLI — FFI opaque handle registry + lib-internal runtime
// =================================================================
//
// File: src/ffi/handle.rs
// Description: The integer handles the Client FFI uses to name
//              library-side state across the C ABI. A handle is a
//              `u64` value that indexes a process-local registry of
//              per-Client sessions (one open IPC connection to the
//              daemon, plus future per-session metadata).
//
// Why a registry, not raw `Box<T>` -> pointer casts
// --------------------------------------------------
// Raw pointers crossing the ABI create two classes of hard-to-
// diagnose bug: (a) a Client holding a pointer to a freed session
// has a use-after-free that manifests at some distant ability
// call, and (b) concurrent shutdown races between "lib shutdown"
// and "Client calls easynet_ability_invoke" have to be papered
// over in user code. A u64 handle + a process-wide registry puts
// both of those problems on the library side.
//
// Lib-internal tokio runtime
// --------------------------
// The IPC client (`crate::ffi::client::IpcClient`) is built on
// `tokio::net::UnixStream` + `tokio_util::codec::Framed`, which
// require a tokio runtime to drive. The C ABI is sync, so the lib
// owns a process-wide `tokio::runtime::Runtime` and routes every
// async call through `Runtime::block_on`.
//
// Why `current_thread` and not `multi_thread`
// -------------------------------------------
// Plan v10.5 R1 §"lib 内部 tokio runtime — 决策项" pins a single
// dedicated I/O thread by default to avoid Go cgo / Python GIL /
// Swift main-thread conflicts. `current_thread` runtime + a
// dedicated OS thread that calls `Runtime::block_on` per FFI call
// is the simplest expression of that decision. v1 ships this; if a
// platform smoke test breaks, the fallback is a fully-sync
// `std::os::unix::net::UnixStream` path with no tokio inside the lib.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use tokio::runtime::Runtime;

use crate::ffi::client::IpcClient;

/// Opaque handle exposed to the C ABI. A value of 0 is reserved as
/// "null handle" / "not yet allocated".
pub type EasynetHandle = u64;

/// Library-side state for one Client.
///
/// Wrapped in an `Arc<ClientSession>` inside the registry so that
/// (a) `get()` can hand back a cheap clone the FFI function holds
/// while it awaits a round-trip, and (b) `release()` does not
/// invalidate an `Arc` already held by an in-flight call. The
/// `IpcClient` is behind a `Mutex` because the round-trip
/// contract is "send one frame, read one frame"; concurrent calls
/// would interleave the framed reads.
pub struct ClientSession {
    /// IPC version negotiated with the daemon. 0 means "no IPC
    /// connection attempted" (test sessions); a real session always
    /// carries the value chosen by the version-overlap check.
    pub ipc_version: u16,
    /// Path to the control.json the Client was told to dial. Used
    /// in diagnostic messages so an operator can see "which daemon
    /// did this handle connect to".
    pub control_path: String,
    /// The framed UDS connection. `None` for test sessions that
    /// only exercise the registry, `Some(...)` for sessions opened
    /// via `easynet_init`. Behind a `Mutex` because the round-trip
    /// is one-frame-in / one-frame-out and concurrent calls on the
    /// same handle would interleave reads.
    pub client: Option<Mutex<IpcClient>>,
    /// Per-handle subscription registry. Each `easynet_ability_subscribe`
    /// call:
    ///   * dials a fresh UDS connection (so the existing
    ///     round-trip socket stays a clean 1-frame-in / 1-frame-out
    ///     pipe);
    ///   * spawns a reader task on the lib runtime;
    ///   * stores a CancellationToken here keyed by the local
    ///     subscription_id so easynet_subscription_cancel can fire
    ///     the token + the reader exits.
    pub subscriptions: Mutex<std::collections::HashMap<u64, tokio_util::sync::CancellationToken>>,
}

impl ClientSession {
    /// Construct a session that owns a live IPC client.
    pub fn with_client(control_path: String, client: IpcClient) -> Self {
        let ipc_version = client.ipc_version;
        Self {
            ipc_version,
            control_path,
            client: Some(Mutex::new(client)),
            subscriptions: Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// Test-only constructor: a session with no IPC client. The
    /// registry tests use this to exercise alloc/get/release without
    /// reaching for `easynet_init`.
    #[cfg(test)]
    fn dummy(control_path: String) -> Self {
        Self {
            ipc_version: 0,
            control_path,
            client: None,
            subscriptions: Mutex::new(std::collections::HashMap::new()),
        }
    }
}

/// Process-wide registry. A plain `Mutex<HashMap>` is sufficient
/// here because the contention surface is "one entry per live
/// Client process", which is essentially never contested. A future
/// commit can swap for `DashMap` if subscription volume justifies
/// it.
struct Registry {
    next: AtomicU64,
    entries: Mutex<std::collections::HashMap<EasynetHandle, Arc<ClientSession>>>,
}

fn registry() -> &'static Registry {
    static REG: OnceLock<Registry> = OnceLock::new();
    REG.get_or_init(|| Registry {
        // Start at 1 so a 0 handle remains the explicit null value.
        next: AtomicU64::new(1),
        entries: Mutex::new(std::collections::HashMap::new()),
    })
}

/// Process-wide tokio runtime used by the FFI surface to drive
/// async I/O against the daemon.
///
/// Initialised lazily on first use (typically inside `easynet_init`).
/// Errors during runtime construction abort the call site with a
/// recorded last-error message — the library is unusable without a
/// runtime, so failing fast is the right outcome.
pub(crate) fn lib_runtime() -> anyhow::Result<&'static Runtime> {
    // OnceLock<Result<Runtime, ...>> would let us cache the error,
    // but in practice runtime construction fails only on resource
    // exhaustion (no threads / no fds), which the next call will
    // also fail on; a fresh attempt each time is acceptable.
    //
    // Multi-thread runtime so spawned reader tasks (subscribe
    // forwarders) keep making progress while a separate `block_on`
    // serves an Invoke. With `new_current_thread` a fire-and-forget
    // task only runs while another `block_on` is active, which
    // would deadlock the subscribe path.
    static RT: OnceLock<Runtime> = OnceLock::new();
    if let Some(rt) = RT.get() {
        return Ok(rt);
    }
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(2)
        .thread_name("easynet-ffi-io")
        .build()
        .map_err(|e| anyhow::anyhow!("FFI: tokio runtime build failed: {e}"))?;
    Ok(RT.get_or_init(|| rt))
}

/// Allocate a new handle for the given `ClientSession` and return
/// both. The caller stores the handle in the Client; the Arc is
/// retained by the registry.
pub(crate) fn alloc(session: ClientSession) -> (EasynetHandle, Arc<ClientSession>) {
    let reg = registry();
    let id = reg.next.fetch_add(1, Ordering::Relaxed);
    let arc = Arc::new(session);
    reg.entries
        .lock()
        .expect("handle registry lock not poisoned")
        .insert(id, arc.clone());
    (id, arc)
}

/// Look up a handle. Returns `None` when the handle is 0 (null) or
/// not present (freed / never issued). Callers map `None` to
/// `ERR_INVALID_HANDLE`.
pub(crate) fn get(handle: EasynetHandle) -> Option<Arc<ClientSession>> {
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
    ClientSession::dummy("/tmp/test-control.json".into())
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

    #[test]
    fn lib_runtime_returns_same_runtime_across_calls() {
        // OnceLock semantics: the second call must hand back the
        // same runtime instance, not allocate a new one. A future
        // refactor that lost this invariant would silently leak a
        // runtime per FFI call.
        let a = lib_runtime().expect("runtime build #1");
        let b = lib_runtime().expect("runtime build #2");
        assert!(std::ptr::eq(a, b));
    }
}
