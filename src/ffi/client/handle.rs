// EasyNet CLI — FFI opaque handle registry + lib-internal runtime
// =================================================================
//
// File: src/ffi/handle.rs
// Description: The integer handles the Client FFI uses to name
//              library-side state across the C ABI. A handle is a
//              `u64` value that indexes a process-local registry of
//              per-Client sessions (control discovery state, plus
//              future per-session metadata).
//
// Why a registry, not raw `Box<T>` -> pointer casts
// --------------------------------------------------
// Raw pointers crossing the ABI create two classes of hard-to-
// diagnose bug: (a) a Client holding a pointer to a freed session
// has a use-after-free that manifests at some distant FFI call, and
// (b) concurrent shutdown races between "lib shutdown" and "Client
// calls into the ABI" have to be papered over in user code. A u64
// handle + a process-wide registry puts both of those problems on
// the library side.
//
// Lib-internal tokio runtime
// --------------------------
// The IPC client (`crate::ffi::client::IpcClient`) is built on
// `tokio::net::UnixStream` + `tokio_util::codec::Framed`, which
// require a tokio runtime to drive. The C ABI is sync, so the lib
// owns a process-wide `tokio::runtime::Runtime` and routes every
// async call through `Runtime::block_on`.
//
// Runtime shape
// -------------
// The FFI layer uses a small multi-thread runtime. Long-running
// Invocation observers, stream readers, and short health/status calls
// can then make progress independently even when the embedding app
// calls a synchronous ABI function on one thread.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use tokio::runtime::Runtime;

use crate::ffi::client::IpcClient;

const MIN_FFI_WORKER_THREADS: usize = 4;

/// Opaque handle exposed to the C ABI. A value of 0 is reserved as
/// "null handle" / "not yet allocated".
pub type RuntimeHandle = u64;

/// Process-local identity of one live client session.
///
/// `RuntimeHandle` is the public C ABI token. `incarnation` is the
/// library-private session generation minted when a `ClientSession` is
/// constructed. FFI sub-resources bind to this pair so they cannot be
/// controlled by a later session that happens to present the same numeric
/// handle value after release/reallocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ClientSessionBinding {
    pub handle: RuntimeHandle,
    pub incarnation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClientSessionLifecyclePhase {
    Active,
    Closing,
    Released,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ClientSessionClosed;

pub(crate) struct ClientSessionResourceGuard<'a> {
    binding: ClientSessionBinding,
    _guard: MutexGuard<'a, ClientSessionLifecyclePhase>,
}

impl ClientSessionResourceGuard<'_> {
    pub(crate) fn binding(&self) -> ClientSessionBinding {
        self.binding
    }
}

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
    /// Explicit lifecycle state for resource registration. Submit/open paths
    /// must hold this lock while inserting child FFI resources; shutdown holds
    /// the same lock while moving the session to Closing and draining children.
    lifecycle: Mutex<ClientSessionLifecyclePhase>,
    /// Library-private session generation used to bind child FFI resources to
    /// the live client session that created them.
    incarnation: u64,
    /// IPC version negotiated with the daemon. 0 means "no IPC
    /// connection attempted" (test sessions); a real session always
    /// carries the value chosen by the version-overlap check.
    pub ipc_version: u16,
    /// Path to the control.json the Client was told to dial. Used
    /// in diagnostic messages so an operator can see "which daemon
    /// did this handle connect to".
    pub control_path: String,
    /// Optional direct daemon Invocation endpoint. Sessions created
    /// from `runtime_host_open_client` already know the daemon
    /// lifecycle handle's endpoint, so Invocation ABI calls should
    /// not re-derive `daemon.sock` from a control descriptor path.
    pub invocation_endpoint: Option<String>,
    /// The framed UDS connection. `None` for test sessions that
    /// only exercise the registry, `Some(...)` for sessions opened
    /// via `runtime_init`. Behind a `Mutex` because the round-trip
    /// is one-frame-in / one-frame-out and concurrent calls on the
    /// same handle would interleave reads.
    pub client: Option<Mutex<IpcClient>>,
}

impl ClientSession {
    /// Construct a session that owns a live IPC client.
    pub fn with_client(control_path: String, client: IpcClient) -> Self {
        let ipc_version = client.ipc_version;
        let invocation_endpoint = client
            .daemon_discovery
            .invocation_endpoint
            .as_ref()
            .map(|path| path.display().to_string());
        Self {
            lifecycle: Mutex::new(ClientSessionLifecyclePhase::Active),
            incarnation: next_session_incarnation(),
            ipc_version,
            control_path,
            invocation_endpoint,
            client: Some(Mutex::new(client)),
        }
    }

    /// Construct a session that names a daemon control path but does
    /// not hold a JSON-control IPC connection.
    ///
    /// This is used by daemon lifecycle C ABI helpers that only need
    /// an `RuntimeHandle` for daemon Invocation calls. The complete
    /// Invocation ABI prefers the explicit `invocation_endpoint` and
    /// does not use the JSON-control client.
    pub(crate) fn with_control_path_only(
        control_path: String,
        invocation_endpoint: Option<String>,
    ) -> Self {
        Self {
            lifecycle: Mutex::new(ClientSessionLifecyclePhase::Active),
            incarnation: next_session_incarnation(),
            ipc_version: 0,
            control_path,
            invocation_endpoint,
            client: None,
        }
    }

    /// Test-only constructor: a session with no IPC client. The
    /// registry tests use this to exercise alloc/get/release without
    /// reaching for `runtime_init`.
    #[cfg(test)]
    fn dummy(control_path: String) -> Self {
        Self {
            lifecycle: Mutex::new(ClientSessionLifecyclePhase::Active),
            incarnation: next_session_incarnation(),
            ipc_version: 0,
            control_path,
            invocation_endpoint: None,
            client: None,
        }
    }

    pub(crate) fn binding(&self, handle: RuntimeHandle) -> ClientSessionBinding {
        ClientSessionBinding {
            handle,
            incarnation: self.incarnation,
        }
    }

    pub(crate) fn resource_registration_guard(
        &self,
        handle: RuntimeHandle,
    ) -> Result<ClientSessionResourceGuard<'_>, ClientSessionClosed> {
        let guard = self
            .lifecycle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if *guard != ClientSessionLifecyclePhase::Active {
            return Err(ClientSessionClosed);
        }
        Ok(ClientSessionResourceGuard {
            binding: self.binding(handle),
            _guard: guard,
        })
    }

    pub(crate) fn begin_closing(
        &self,
        handle: RuntimeHandle,
    ) -> Result<ClientSessionBinding, ClientSessionClosed> {
        let mut guard = self
            .lifecycle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if *guard != ClientSessionLifecyclePhase::Active {
            return Err(ClientSessionClosed);
        }
        *guard = ClientSessionLifecyclePhase::Closing;
        Ok(self.binding(handle))
    }

    pub(crate) fn mark_released(&self) {
        let mut guard = self
            .lifecycle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *guard = ClientSessionLifecyclePhase::Released;
    }
}

fn next_session_incarnation() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// Process-wide registry. A plain `Mutex<HashMap>` is sufficient
/// here because the contention surface is "one entry per live
/// Client process", which is essentially never contested. A future
/// commit can swap for `DashMap` if subscription volume justifies
/// it.
struct Registry {
    next: AtomicU64,
    entries: Mutex<std::collections::HashMap<RuntimeHandle, Arc<ClientSession>>>,
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
/// Initialised lazily on first use (typically inside `runtime_init`).
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
        .worker_threads(ffi_worker_threads())
        .thread_name("easynet-ffi-io")
        .build()
        .map_err(|e| anyhow::anyhow!("FFI: tokio runtime build failed: {e}"))?;
    Ok(RT.get_or_init(|| rt))
}

fn ffi_worker_threads() -> usize {
    std::env::var("EASYNET_FFI_WORKER_THREADS")
        .ok()
        .and_then(|raw| raw.trim().parse::<usize>().ok())
        .filter(|threads| *threads > 0)
        .unwrap_or_else(host_default_ffi_worker_threads)
}

fn host_default_ffi_worker_threads() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get().saturating_mul(2).max(MIN_FFI_WORKER_THREADS))
        .unwrap_or(MIN_FFI_WORKER_THREADS)
}

/// Allocate a new handle for the given `ClientSession` and return
/// both. The caller stores the handle in the Client; the Arc is
/// retained by the registry.
pub(crate) fn alloc(session: ClientSession) -> (RuntimeHandle, Arc<ClientSession>) {
    let reg = registry();
    let id = reg.next.fetch_add(1, Ordering::Relaxed);
    let arc = Arc::new(session);
    reg.entries
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(id, arc.clone());
    (id, arc)
}

/// Look up a handle. Returns `None` when the handle is 0 (null) or
/// not present (freed / never issued). Callers map `None` to
/// `ERR_INVALID_HANDLE`.
pub(crate) fn get(handle: RuntimeHandle) -> Option<Arc<ClientSession>> {
    if handle == 0 {
        return None;
    }
    registry()
        .entries
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&handle)
        .cloned()
}

pub(crate) fn binding_for_handle(handle: RuntimeHandle) -> Option<ClientSessionBinding> {
    get(handle).map(|session| session.binding(handle))
}

/// Release a handle. Returns `true` when the handle was present
/// (and is now removed), `false` when the handle was unknown.
/// Idempotent — a double-free returns `false` the second time.
pub(crate) fn release(handle: RuntimeHandle) -> bool {
    if handle == 0 {
        return false;
    }
    registry()
        .entries
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(&handle)
        .is_some()
}

/// Test-only: create a session with a dummy control path so
/// registry-level tests can exercise alloc/get/release without
/// reaching for the real `runtime_init`.
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
        assert_eq!(looked_up.binding(h).handle, h);
    }

    #[test]
    fn session_binding_changes_per_allocated_session() {
        let (first_handle, first_session) = alloc(test_session());
        let (second_handle, second_session) = alloc(test_session());

        let first_binding = first_session.binding(first_handle);
        let second_binding = second_session.binding(second_handle);

        assert_ne!(first_binding, second_binding);
        assert_eq!(binding_for_handle(first_handle), Some(first_binding));
        assert_eq!(binding_for_handle(second_handle), Some(second_binding));

        assert!(release(first_handle));
        assert_eq!(binding_for_handle(first_handle), None);
        assert_eq!(binding_for_handle(second_handle), Some(second_binding));
        assert!(release(second_handle));
    }

    #[test]
    fn session_lifecycle_rejects_registration_after_closing() {
        let (handle, session) = alloc(test_session());
        let binding = session.begin_closing(handle).expect("begin closing");
        assert_eq!(binding.handle, handle);
        assert!(session.resource_registration_guard(handle).is_err());
        session.mark_released();
        assert!(session.begin_closing(handle).is_err());
        assert!(release(handle));
    }

    #[test]
    fn session_lifecycle_blocks_closing_until_registration_guard_drops() {
        let (handle, session) = alloc(test_session());
        let registration = session
            .resource_registration_guard(handle)
            .expect("resource registration");
        let session_for_shutdown = Arc::clone(&session);
        let (tx, rx) = std::sync::mpsc::channel();

        std::thread::spawn(move || {
            let result = session_for_shutdown.begin_closing(handle).is_ok();
            tx.send(result).expect("send shutdown result");
        });

        assert!(
            rx.recv_timeout(std::time::Duration::from_millis(50))
                .is_err(),
            "shutdown must wait while resource registration is in progress"
        );
        drop(registration);

        assert_eq!(
            rx.recv_timeout(std::time::Duration::from_secs(2))
                .expect("shutdown should proceed after registration guard drops"),
            true
        );
        session.mark_released();
        assert!(release(handle));
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
    fn host_default_ffi_worker_threads_respects_minimum() {
        assert!(
            host_default_ffi_worker_threads() >= MIN_FFI_WORKER_THREADS,
            "host-derived FFI runtime sizing must never shrink below the fixed progress floor"
        );
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
