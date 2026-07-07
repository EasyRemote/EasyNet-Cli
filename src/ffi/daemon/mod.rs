// EasyNet CLI — Daemon lifecycle C ABI
// =====================================
//
// File: src/ffi/daemon.rs
// Description: C ABI entry points for starting, stopping, inspecting,
//              and discovering the local `easynet-daemon`.
//
// Boundary
// --------
// This module exposes EasyNet-Cli daemon lifecycle control to
// language bindings. It owns only product daemon process lifecycle
// and endpoint discovery. It does not submit ability calls, define
// Axon Invocation semantics, or replace `ffi::invocation`.
//
// What this module is NOT
// -----------------------
// - It is not the `easynet_init` client-session registry. A daemon
//   lifecycle handle names a process/status object, while
//   `EasynetHandle` names an IPC client session.
// - It is not an Axon runtime lifecycle API. Starting
//   `axon-runtime` belongs to Axon SDK reference-runtime surfaces,
//   not to `libeasynet_cli`.
// - It is not a JSON-control ability bridge. Product calls must use
//   the complete Invocation ABI in `ffi::invocation`.

use std::os::raw::c_char;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use crate::daemon::{DaemonHandle, DaemonStartConfig};
use crate::ffi::client::handle::{alloc, ClientSession, EasynetHandle};
use crate::ffi::errors::{
    clear_last_error, set_last_error_code, EASYNET_OK, ERR_DAEMON_DOWN, ERR_GENERIC,
    ERR_INVALID_ARG, ERR_INVALID_HANDLE, ERR_INVALID_UTF8, ERR_NULL_POINTER,
};
use crate::ffi::strings::{alloc_output_cstring, read_cstr, StringError};

/// Opaque handle for a daemon process lifecycle session.
///
/// A value of 0 means "no daemon lifecycle handle allocated".
/// The handle is process-local and must not be persisted. It is
/// intentionally separate from `EasynetHandle`, which names an IPC
/// client session returned by `easynet_init`.
pub type EasynetDaemonHandle = u64;

/// Start or attach to `easynet-daemon`.
///
/// `config_json` must be a UTF-8 JSON object with this shape:
///
/// ```text
/// {
///   "mode": "device" | "hub",
///   "device_id": "dev-a",            // required for device
///   "daemon_bin": "/path/to/bin",    // optional
///   "log_path": "/path/to/log",      // optional
///   "detached": true,                // optional
///   "env": {"KEY": "VALUE"}          // optional string map
/// }
/// ```
///
/// On success, `*out_daemon_handle` receives a daemon lifecycle
/// handle that can be passed to `easynet_daemon_status`,
/// `easynet_daemon_invocation_endpoint`, and `easynet_daemon_stop`.
///
/// # Safety
/// - `config_json` must point to a valid UTF-8 C string.
/// - `out_daemon_handle` must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_daemon_start(
    config_json: *const c_char,
    out_daemon_handle: *mut EasynetDaemonHandle,
) -> i32 {
    if out_daemon_handle.is_null() {
        set_last_error_code(
            ERR_NULL_POINTER,
            "easynet_daemon_start: out_daemon_handle pointer is null",
        );
        return ERR_NULL_POINTER;
    }
    unsafe { *out_daemon_handle = 0 };

    let raw = match read_cstr(config_json) {
        Ok(value) => value,
        Err(StringError::Null) => {
            set_last_error_code(
                ERR_NULL_POINTER,
                "easynet_daemon_start: config_json pointer is null",
            );
            return ERR_NULL_POINTER;
        }
        Err(StringError::NotUtf8) => {
            set_last_error_code(
                ERR_INVALID_UTF8,
                "easynet_daemon_start: config_json is not valid UTF-8",
            );
            return ERR_INVALID_UTF8;
        }
    };

    let config = match DaemonStartConfigJson::parse(raw).and_then(DaemonStartConfigJson::build) {
        Ok(config) => config,
        Err(err) => {
            set_last_error_code(ERR_INVALID_ARG, format!("easynet_daemon_start: {err}"));
            return ERR_INVALID_ARG;
        }
    };

    let handle = match crate::daemon::start_daemon(&config) {
        Ok(handle) => handle,
        Err(err) => {
            set_last_error_code(ERR_DAEMON_DOWN, format!("easynet_daemon_start: {err}"));
            return ERR_DAEMON_DOWN;
        }
    };

    let id = insert_daemon_handle(handle);
    unsafe { *out_daemon_handle = id };
    clear_last_error();
    EASYNET_OK
}

/// Attach to an already-running daemon without spawning it.
///
/// `options_json` is reserved for future endpoint override fields and
/// may be NULL today. Attach fails closed when control is up but the
/// Invocation endpoint is down.
///
/// # Safety
/// - `options_json` may be null; if non-null it must be valid UTF-8.
/// - `out_daemon_handle` must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_daemon_attach(
    options_json: *const c_char,
    out_daemon_handle: *mut EasynetDaemonHandle,
) -> i32 {
    if out_daemon_handle.is_null() {
        set_last_error_code(
            ERR_NULL_POINTER,
            "easynet_daemon_attach: out_daemon_handle pointer is null",
        );
        return ERR_NULL_POINTER;
    }
    unsafe { *out_daemon_handle = 0 };
    if !options_json.is_null() {
        match read_cstr(options_json) {
            Ok(raw) => {
                if let Err(err) = validate_attach_options(raw) {
                    set_last_error_code(ERR_INVALID_ARG, format!("easynet_daemon_attach: {err}"));
                    return ERR_INVALID_ARG;
                }
            }
            Err(StringError::NotUtf8) => {
                set_last_error_code(
                    ERR_INVALID_UTF8,
                    "easynet_daemon_attach: options_json is not valid UTF-8",
                );
                return ERR_INVALID_UTF8;
            }
            Err(StringError::Null) => {}
        }
    }
    let handle = match DaemonHandle::attach_current() {
        Ok(handle) => handle,
        Err(err) => {
            set_last_error_code(ERR_DAEMON_DOWN, format!("easynet_daemon_attach: {err}"));
            return ERR_DAEMON_DOWN;
        }
    };
    let id = insert_daemon_handle(handle);
    unsafe { *out_daemon_handle = id };
    clear_last_error();
    EASYNET_OK
}

/// Discover current daemon endpoints and readiness without allocating
/// a lifecycle handle.
///
/// The returned string is caller-owned and must be freed with
/// `easynet_string_free`.
///
/// # Safety
/// - `options_json` may be null; if non-null it must be valid UTF-8.
/// - `out_discovery_json` must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_daemon_discover(
    options_json: *const c_char,
    out_discovery_json: *mut *mut c_char,
) -> i32 {
    if out_discovery_json.is_null() {
        set_last_error_code(
            ERR_NULL_POINTER,
            "easynet_daemon_discover: out_discovery_json pointer is null",
        );
        return ERR_NULL_POINTER;
    }
    unsafe { *out_discovery_json = std::ptr::null_mut() };
    if !options_json.is_null() {
        match read_cstr(options_json) {
            Ok(raw) => {
                if let Err(err) = validate_attach_options(raw) {
                    set_last_error_code(ERR_INVALID_ARG, format!("easynet_daemon_discover: {err}"));
                    return ERR_INVALID_ARG;
                }
            }
            Err(StringError::NotUtf8) => {
                set_last_error_code(
                    ERR_INVALID_UTF8,
                    "easynet_daemon_discover: options_json is not valid UTF-8",
                );
                return ERR_INVALID_UTF8;
            }
            Err(StringError::Null) => {}
        }
    }
    let status = crate::daemon::DaemonStatus::current();
    let ptr = alloc_output_cstring(daemon_status_json(&status).to_string());
    if ptr.is_null() {
        set_last_error_code(
            ERR_GENERIC,
            "easynet_daemon_discover: out-of-memory allocating discovery string",
        );
        return ERR_GENERIC;
    }
    unsafe { *out_discovery_json = ptr };
    clear_last_error();
    EASYNET_OK
}

/// Stop a daemon lifecycle handle.
///
/// The handle is removed only after the stop operation succeeds.
/// Unknown handles return `ERR_INVALID_HANDLE`.
#[no_mangle]
pub extern "C" fn easynet_daemon_stop(handle: EasynetDaemonHandle) -> i32 {
    let Some(daemon) = get_daemon_handle(handle) else {
        set_last_error_code(
            ERR_INVALID_HANDLE,
            format!("easynet_daemon_stop: daemon handle {handle} is not registered"),
        );
        return ERR_INVALID_HANDLE;
    };
    let stop_result = daemon
        .inner
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .stop();
    if let Err(err) = stop_result {
        set_last_error_code(ERR_DAEMON_DOWN, format!("easynet_daemon_stop: {err}"));
        return ERR_DAEMON_DOWN;
    }
    let _ = remove_daemon_handle(handle);
    clear_last_error();
    EASYNET_OK
}

/// Detach a daemon lifecycle handle without stopping the daemon.
#[no_mangle]
pub extern "C" fn easynet_daemon_detach(handle: EasynetDaemonHandle) -> i32 {
    let Some(daemon) = remove_daemon_handle(handle) else {
        set_last_error_code(
            ERR_INVALID_HANDLE,
            format!("easynet_daemon_detach: daemon handle {handle} is not registered"),
        );
        return ERR_INVALID_HANDLE;
    };
    daemon
        .inner
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .detach();
    clear_last_error();
    EASYNET_OK
}

/// Return daemon liveness and endpoint status as JSON.
///
/// The returned string is caller-owned and must be freed with
/// `easynet_string_free`.
///
/// # Safety
/// `out_status_json` must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_daemon_status(
    handle: EasynetDaemonHandle,
    out_status_json: *mut *mut c_char,
) -> i32 {
    if out_status_json.is_null() {
        set_last_error_code(
            ERR_NULL_POINTER,
            "easynet_daemon_status: out_status_json pointer is null",
        );
        return ERR_NULL_POINTER;
    }
    unsafe { *out_status_json = std::ptr::null_mut() };

    let Some(daemon) = get_daemon_handle(handle) else {
        set_last_error_code(
            ERR_INVALID_HANDLE,
            format!("easynet_daemon_status: daemon handle {handle} is not registered"),
        );
        return ERR_INVALID_HANDLE;
    };
    let status = daemon
        .inner
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .status();
    let json = daemon_status_json(&status).to_string();
    let ptr = alloc_output_cstring(json);
    if ptr.is_null() {
        set_last_error_code(
            ERR_GENERIC,
            "easynet_daemon_status: out-of-memory allocating status string",
        );
        return ERR_GENERIC;
    }
    unsafe { *out_status_json = ptr };
    clear_last_error();
    EASYNET_OK
}

/// Return the daemon Axon Invocation endpoint path.
///
/// The returned string is caller-owned and must be freed with
/// `easynet_string_free`.
///
/// # Safety
/// `out_endpoint` must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_daemon_invocation_endpoint(
    handle: EasynetDaemonHandle,
    out_endpoint: *mut *mut c_char,
) -> i32 {
    if out_endpoint.is_null() {
        set_last_error_code(
            ERR_NULL_POINTER,
            "easynet_daemon_invocation_endpoint: out_endpoint pointer is null",
        );
        return ERR_NULL_POINTER;
    }
    unsafe { *out_endpoint = std::ptr::null_mut() };

    let Some(daemon) = get_daemon_handle(handle) else {
        set_last_error_code(
            ERR_INVALID_HANDLE,
            format!("easynet_daemon_invocation_endpoint: daemon handle {handle} is not registered"),
        );
        return ERR_INVALID_HANDLE;
    };
    let endpoint = daemon
        .inner
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .invocation_endpoint()
        .display()
        .to_string();
    let ptr = alloc_output_cstring(endpoint);
    if ptr.is_null() {
        set_last_error_code(
            ERR_GENERIC,
            "easynet_daemon_invocation_endpoint: out-of-memory allocating endpoint",
        );
        return ERR_GENERIC;
    }
    unsafe { *out_endpoint = ptr };
    clear_last_error();
    EASYNET_OK
}

/// Return all daemon endpoints as JSON.
///
/// # Safety
/// `out_endpoints_json` must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_daemon_endpoints(
    handle: EasynetDaemonHandle,
    out_endpoints_json: *mut *mut c_char,
) -> i32 {
    if out_endpoints_json.is_null() {
        set_last_error_code(
            ERR_NULL_POINTER,
            "easynet_daemon_endpoints: out_endpoints_json pointer is null",
        );
        return ERR_NULL_POINTER;
    }
    unsafe { *out_endpoints_json = std::ptr::null_mut() };

    let Some(daemon) = get_daemon_handle(handle) else {
        set_last_error_code(
            ERR_INVALID_HANDLE,
            format!("easynet_daemon_endpoints: daemon handle {handle} is not registered"),
        );
        return ERR_INVALID_HANDLE;
    };
    let daemon = daemon
        .inner
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let json = daemon_endpoints_json(daemon.endpoints()).to_string();
    let ptr = alloc_output_cstring(json);
    if ptr.is_null() {
        set_last_error_code(
            ERR_GENERIC,
            "easynet_daemon_endpoints: out-of-memory allocating endpoints string",
        );
        return ERR_GENERIC;
    }
    unsafe { *out_endpoints_json = ptr };
    clear_last_error();
    EASYNET_OK
}

/// Open an Invocation-capable client handle from a daemon lifecycle
/// handle.
///
/// This is the binding-friendly bridge between the process lifecycle
/// ABI and the Invocation ABI: callers may start or attach to a daemon,
/// then call this function and pass the returned `EasynetHandle` to
/// `easynet_invocation_*`. The returned handle is released with
/// `easynet_shutdown`.
///
/// # Safety
/// `out_handle` must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_daemon_open_client(
    daemon_handle: EasynetDaemonHandle,
    out_handle: *mut EasynetHandle,
) -> i32 {
    if out_handle.is_null() {
        set_last_error_code(
            ERR_NULL_POINTER,
            "easynet_daemon_open_client: out_handle pointer is null",
        );
        return ERR_NULL_POINTER;
    }
    unsafe { *out_handle = 0 };

    let Some(daemon) = get_daemon_handle(daemon_handle) else {
        set_last_error_code(
            ERR_INVALID_HANDLE,
            format!("easynet_daemon_open_client: daemon handle {daemon_handle} is not registered"),
        );
        return ERR_INVALID_HANDLE;
    };

    let daemon = daemon
        .inner
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let status = daemon.status();
    if !status.control_accepting() || !status.invocation_accepting() {
        set_last_error_code(
            ERR_DAEMON_DOWN,
            format!(
                "easynet_daemon_open_client: daemon is not ready; control_accepting={}, invocation_accepting={}",
                status.control_accepting(),
                status.invocation_accepting()
            ),
        );
        return ERR_DAEMON_DOWN;
    }

    let control_path = daemon.control_endpoint().display().to_string();
    let invocation_endpoint = daemon.invocation_endpoint().display().to_string();
    let (handle, _) = alloc(ClientSession::with_control_path_only(
        control_path,
        Some(invocation_endpoint),
    ));
    unsafe { *out_handle = handle };
    clear_last_error();
    EASYNET_OK
}

/// List local desktop companion statuses as JSON.
///
/// # Safety
/// `out_json` must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_companion_list(
    handle: EasynetDaemonHandle,
    out_json: *mut *mut c_char,
) -> i32 {
    companion_json_output(handle, out_json, "easynet_companion_list", || {
        let state = crate::daemon::plugins::default_state()?;
        let manager = crate::daemon::plugins::DesktopCompanionManager::current();
        let companions = state
            .index()
            .packages()
            .iter()
            .filter(|package| {
                package.manifest().kind() == crate::daemon::plugins::PluginKind::DesktopCompanion
            })
            .filter_map(|package| manager.status_json(package).ok())
            .collect::<Vec<_>>();
        Ok(serde_json::json!({
            "kind": "desktop_companion_list",
            "companions": companions,
        }))
    })
}

/// Return one local desktop companion status as JSON.
///
/// # Safety
/// `package_id`, when non-null, and `version_or_null`, when non-null, must be
/// valid UTF-8 C strings. `out_json` must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_companion_status(
    handle: EasynetDaemonHandle,
    package_id: *const c_char,
    version_or_null: *const c_char,
    out_json: *mut *mut c_char,
) -> i32 {
    companion_package_action(
        handle,
        package_id,
        version_or_null,
        out_json,
        "easynet_companion_status",
        |manager, package| manager.status_json(&package),
    )
}

/// Enable one local desktop companion.
#[no_mangle]
pub unsafe extern "C" fn easynet_companion_enable(
    handle: EasynetDaemonHandle,
    package_id: *const c_char,
    version_or_null: *const c_char,
    out_json: *mut *mut c_char,
) -> i32 {
    companion_package_action(
        handle,
        package_id,
        version_or_null,
        out_json,
        "easynet_companion_enable",
        |manager, package| manager.enable(&package),
    )
}

/// Disable one local desktop companion.
#[no_mangle]
pub unsafe extern "C" fn easynet_companion_disable(
    handle: EasynetDaemonHandle,
    package_id: *const c_char,
    version_or_null: *const c_char,
    out_json: *mut *mut c_char,
) -> i32 {
    companion_package_action(
        handle,
        package_id,
        version_or_null,
        out_json,
        "easynet_companion_disable",
        |manager, package| manager.disable(&package),
    )
}

/// Start one local desktop companion.
#[no_mangle]
pub unsafe extern "C" fn easynet_companion_start(
    handle: EasynetDaemonHandle,
    package_id: *const c_char,
    version_or_null: *const c_char,
    out_json: *mut *mut c_char,
) -> i32 {
    companion_package_action(
        handle,
        package_id,
        version_or_null,
        out_json,
        "easynet_companion_start",
        |manager, package| manager.start(&package),
    )
}

/// Stop one local desktop companion.
#[no_mangle]
pub unsafe extern "C" fn easynet_companion_stop(
    handle: EasynetDaemonHandle,
    package_id: *const c_char,
    version_or_null: *const c_char,
    out_json: *mut *mut c_char,
) -> i32 {
    companion_package_action(
        handle,
        package_id,
        version_or_null,
        out_json,
        "easynet_companion_stop",
        |manager, package| manager.stop(&package),
    )
}

fn companion_package_action<F>(
    handle: EasynetDaemonHandle,
    package_id: *const c_char,
    version_or_null: *const c_char,
    out_json: *mut *mut c_char,
    label: &'static str,
    action: F,
) -> i32
where
    F: FnOnce(
        crate::daemon::plugins::DesktopCompanionManager,
        crate::daemon::plugins::package::SharedPluginPackage,
    ) -> crate::daemon::plugins::Result<serde_json::Value>,
{
    companion_json_output(handle, out_json, label, || {
        let package_id = read_required_cstr(package_id, label, "package_id")?;
        let version = read_optional_cstr(version_or_null, label, "version_or_null")?;
        let package = resolve_companion_package(&package_id, version.as_deref(), label)?;
        action(crate::daemon::plugins::DesktopCompanionManager::current(), package)
    })
}

fn companion_json_output<F>(
    handle: EasynetDaemonHandle,
    out_json: *mut *mut c_char,
    label: &'static str,
    build: F,
) -> i32
where
    F: FnOnce() -> crate::daemon::plugins::Result<serde_json::Value>,
{
    if out_json.is_null() {
        set_last_error_code(
            ERR_NULL_POINTER,
            format!("{label}: out_json pointer is null"),
        );
        return ERR_NULL_POINTER;
    }
    unsafe { *out_json = std::ptr::null_mut() };
    if get_daemon_handle(handle).is_none() {
        set_last_error_code(
            ERR_INVALID_HANDLE,
            format!("{label}: daemon handle {handle} is not registered"),
        );
        return ERR_INVALID_HANDLE;
    }
    let value = match build() {
        Ok(value) => value,
        Err(err) => {
            set_last_error_code(ERR_GENERIC, format!("{label}: {err}"));
            return ERR_GENERIC;
        }
    };
    let ptr = alloc_output_cstring(value.to_string());
    if ptr.is_null() {
        set_last_error_code(
            ERR_GENERIC,
            format!("{label}: out-of-memory allocating JSON string"),
        );
        return ERR_GENERIC;
    }
    unsafe { *out_json = ptr };
    clear_last_error();
    EASYNET_OK
}

fn read_required_cstr(
    ptr: *const c_char,
    label: &'static str,
    field: &'static str,
) -> crate::daemon::plugins::Result<String> {
    let raw = read_cstr(ptr).map_err(|err| {
        crate::daemon::plugins::PluginHostError::InvalidCompanionManifest {
            id: label.to_string(),
            reason: match err {
                StringError::Null => format!("{field} pointer is null"),
                StringError::NotUtf8 => format!("{field} is not valid UTF-8"),
            },
        }
    })?;
    let value = raw.trim();
    if value.is_empty() {
        return Err(crate::daemon::plugins::PluginHostError::InvalidCompanionManifest {
            id: label.to_string(),
            reason: format!("{field} must not be empty"),
        });
    }
    Ok(value.to_string())
}

fn read_optional_cstr(
    ptr: *const c_char,
    label: &'static str,
    field: &'static str,
) -> crate::daemon::plugins::Result<Option<String>> {
    if ptr.is_null() {
        return Ok(None);
    }
    read_required_cstr(ptr, label, field).map(Some)
}

fn resolve_companion_package(
    package_id: &str,
    package_version: Option<&str>,
    label: &'static str,
) -> crate::daemon::plugins::Result<crate::daemon::plugins::package::SharedPluginPackage> {
    let state = crate::daemon::plugins::default_state()?;
    let matches = state
        .index()
        .packages()
        .iter()
        .filter(|package| {
            package.id().as_str() == package_id
                && package_version
                    .map(|version| package.version().as_str() == version)
                    .unwrap_or(true)
        })
        .cloned()
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [package]
            if package.manifest().kind() == crate::daemon::plugins::PluginKind::DesktopCompanion =>
        {
            Ok(package.clone())
        }
        [package] => Err(
            crate::daemon::plugins::PluginHostError::InvalidCompanionManifest {
                id: package.id().as_str().to_string(),
                reason: format!("{label}: package is not a desktop_companion"),
            },
        ),
        [] => Err(
            crate::daemon::plugins::PluginHostError::InvalidCompanionManifest {
                id: package_id.to_string(),
                reason: format!("{label}: package not found"),
            },
        ),
        _ => Err(
            crate::daemon::plugins::PluginHostError::InvalidCompanionManifest {
                id: package_id.to_string(),
                reason: format!("{label}: multiple versions found; pass version_or_null"),
            },
        ),
    }
}

#[derive(Debug)]
struct ActiveDaemonHandle {
    inner: Mutex<DaemonHandle>,
}

#[derive(Debug)]
struct DaemonHandleRegistry {
    next: AtomicU64,
    entries: Mutex<std::collections::HashMap<EasynetDaemonHandle, Arc<ActiveDaemonHandle>>>,
}

fn daemon_handle_registry() -> &'static DaemonHandleRegistry {
    static REGISTRY: OnceLock<DaemonHandleRegistry> = OnceLock::new();
    REGISTRY.get_or_init(|| DaemonHandleRegistry {
        next: AtomicU64::new(1),
        entries: Mutex::new(std::collections::HashMap::new()),
    })
}

fn lock_daemon_entries(
    registry: &DaemonHandleRegistry,
) -> MutexGuard<'_, std::collections::HashMap<EasynetDaemonHandle, Arc<ActiveDaemonHandle>>> {
    registry
        .entries
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn insert_daemon_handle(handle: DaemonHandle) -> EasynetDaemonHandle {
    let registry = daemon_handle_registry();
    let id = registry.next.fetch_add(1, Ordering::Relaxed);
    lock_daemon_entries(registry).insert(
        id,
        Arc::new(ActiveDaemonHandle {
            inner: Mutex::new(handle),
        }),
    );
    id
}

fn get_daemon_handle(handle: EasynetDaemonHandle) -> Option<Arc<ActiveDaemonHandle>> {
    if handle == 0 {
        return None;
    }
    lock_daemon_entries(daemon_handle_registry())
        .get(&handle)
        .cloned()
}

fn remove_daemon_handle(handle: EasynetDaemonHandle) -> Option<Arc<ActiveDaemonHandle>> {
    if handle == 0 {
        return None;
    }
    lock_daemon_entries(daemon_handle_registry()).remove(&handle)
}

#[derive(Debug)]
struct DaemonStartConfigJson {
    mode: DaemonStartMode,
    device_id: Option<String>,
    realm: Option<String>,
    daemon_bin: Option<String>,
    log_path: Option<String>,
    detached: Option<bool>,
    env: std::collections::BTreeMap<String, String>,
}

impl DaemonStartConfigJson {
    fn parse(raw: &str) -> Result<Self, DaemonStartConfigError> {
        let value: serde_json::Value = serde_json::from_str(raw)?;
        let obj = value
            .as_object()
            .ok_or(DaemonStartConfigError::ExpectedObject)?;
        Ok(Self {
            mode: DaemonStartMode::parse(required_string(obj, "mode")?)?,
            device_id: optional_string(obj, "device_id")?,
            realm: optional_string(obj, "realm")?,
            daemon_bin: optional_string(obj, "daemon_bin")?,
            log_path: optional_string(obj, "log_path")?,
            detached: optional_bool(obj, "detached")?,
            env: parse_env(obj)?,
        })
    }

    fn build(self) -> Result<DaemonStartConfig, DaemonStartConfigError> {
        let mut config = match self.mode {
            DaemonStartMode::Device => {
                let device_id = self
                    .device_id
                    .ok_or(DaemonStartConfigError::MissingField("device_id"))?;
                DaemonStartConfig::device(device_id)?
            }
            DaemonStartMode::Hub => DaemonStartConfig::hub(),
        };
        if let Some(realm) = self.realm {
            config = config.with_realm(realm);
        }
        if let Some(path) = self.daemon_bin {
            config = config.with_daemon_bin(path)?;
        }
        if let Some(path) = self.log_path {
            config = config.with_log_path(path);
        }
        if let Some(detached) = self.detached {
            config = config.detached(detached);
        }
        for (key, value) in self.env {
            config = config.with_env(key, value);
        }
        Ok(config)
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum DaemonStartMode {
    Device,
    Hub,
}

impl DaemonStartMode {
    fn parse(raw: String) -> Result<Self, DaemonStartConfigError> {
        match raw.as_str() {
            "device" => Ok(Self::Device),
            "hub" => Ok(Self::Hub),
            other => Err(DaemonStartConfigError::UnsupportedMode(other.to_string())),
        }
    }
}

#[derive(Debug, thiserror::Error)]
enum DaemonStartConfigError {
    #[error("config_json must be a JSON object")]
    ExpectedObject,
    #[error("missing field `{0}`")]
    MissingField(&'static str),
    #[error("field `{0}` must be a non-empty string")]
    InvalidString(&'static str),
    #[error("unsupported daemon mode `{0}`")]
    UnsupportedMode(String),
    #[error("field `{0}` must be a boolean")]
    InvalidBool(&'static str),
    #[error("field `env` must be a JSON object")]
    InvalidEnv,
    #[error("env key must not be empty")]
    EmptyEnvKey,
    #[error("env value for `{0}` must be a string")]
    InvalidEnvValue(String),
    #[error("invalid daemon config: {0}")]
    Daemon(#[from] crate::daemon::DaemonError),
    #[error("decode config_json failed: {0}")]
    Json(#[from] serde_json::Error),
}

fn required_string(
    obj: &serde_json::Map<String, serde_json::Value>,
    field: &'static str,
) -> Result<String, DaemonStartConfigError> {
    let value = obj
        .get(field)
        .ok_or(DaemonStartConfigError::MissingField(field))?
        .as_str()
        .ok_or(DaemonStartConfigError::InvalidString(field))?
        .trim()
        .to_string();
    if value.is_empty() {
        return Err(DaemonStartConfigError::InvalidString(field));
    }
    Ok(value)
}

fn optional_string(
    obj: &serde_json::Map<String, serde_json::Value>,
    field: &'static str,
) -> Result<Option<String>, DaemonStartConfigError> {
    match obj.get(field) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(_) => Ok(Some(required_string(obj, field)?)),
    }
}

fn optional_bool(
    obj: &serde_json::Map<String, serde_json::Value>,
    field: &'static str,
) -> Result<Option<bool>, DaemonStartConfigError> {
    match obj.get(field) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => value
            .as_bool()
            .ok_or(DaemonStartConfigError::InvalidBool(field))
            .map(Some),
    }
}

fn parse_env(
    obj: &serde_json::Map<String, serde_json::Value>,
) -> Result<std::collections::BTreeMap<String, String>, DaemonStartConfigError> {
    let Some(value) = obj.get("env") else {
        return Ok(std::collections::BTreeMap::new());
    };
    if value.is_null() {
        return Ok(std::collections::BTreeMap::new());
    }
    let env = value
        .as_object()
        .ok_or(DaemonStartConfigError::InvalidEnv)?;
    let mut out = std::collections::BTreeMap::new();
    for (key, value) in env {
        let key = key.trim();
        if key.is_empty() {
            return Err(DaemonStartConfigError::EmptyEnvKey);
        }
        let value = value
            .as_str()
            .ok_or_else(|| DaemonStartConfigError::InvalidEnvValue(key.to_string()))?;
        out.insert(key.to_string(), value.to_string());
    }
    Ok(out)
}

fn daemon_status_json(status: &crate::daemon::DaemonStatus) -> serde_json::Value {
    serde_json::json!({
        "pid": status.pid(),
        "pid_alive": status.pid_alive(),
        "control_accepting": status.control_accepting(),
        "invocation_accepting": status.invocation_accepting(),
        "control_endpoint": status.endpoints().control().display().to_string(),
        "invocation_endpoint": status.endpoints().invocation().display().to_string(),
    })
}

fn daemon_endpoints_json(endpoints: &crate::daemon::DaemonEndpoints) -> serde_json::Value {
    serde_json::json!({
        "control_endpoint": endpoints.control().display().to_string(),
        "invocation_endpoint": endpoints.invocation().display().to_string(),
        "public_endpoint": null,
    })
}

fn validate_attach_options(raw: &str) -> Result<(), DaemonStartConfigError> {
    let value: serde_json::Value = serde_json::from_str(raw)?;
    if value.is_null() {
        return Ok(());
    }
    value
        .as_object()
        .ok_or(DaemonStartConfigError::ExpectedObject)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::{CStr, CString};

    #[test]
    fn parse_start_config_requires_explicit_mode() {
        let err = DaemonStartConfigJson::parse("{}").unwrap_err();
        assert!(
            err.to_string().contains("mode"),
            "missing mode must be explicit: {err}"
        );
    }

    #[test]
    fn parse_start_config_builds_device_config() {
        let config = DaemonStartConfigJson::parse(
            r#"{
                "mode": "device",
                "device_id": "dev-a",
                "daemon_bin": "/tmp/easynet-daemon",
                "log_path": "/tmp/easynet.log",
                "detached": false,
                "env": {"EASYNET_TEST": "1"}
            }"#,
        )
        .unwrap()
        .build()
        .unwrap();

        assert_eq!(config.node_id(), "dev-a");
    }

    #[test]
    fn parse_start_config_rejects_device_without_device_id() {
        let err = DaemonStartConfigJson::parse(r#"{"mode":"device"}"#)
            .unwrap()
            .build()
            .unwrap_err();
        assert!(
            err.to_string().contains("device_id"),
            "missing device_id must be reported: {err}"
        );
    }

    #[test]
    fn daemon_start_rejects_null_out_handle_before_io() {
        let raw = CString::new(r#"{"mode":"hub"}"#).unwrap();
        let code = unsafe { easynet_daemon_start(raw.as_ptr(), std::ptr::null_mut()) };
        assert_eq!(code, ERR_NULL_POINTER);
    }

    #[test]
    fn daemon_start_rejects_malformed_json_after_zeroing_handle() {
        let raw = CString::new("{not-json").unwrap();
        let mut handle: EasynetDaemonHandle = 42;
        let code = unsafe { easynet_daemon_start(raw.as_ptr(), &mut handle) };
        assert_eq!(code, ERR_INVALID_ARG);
        assert_eq!(handle, 0);
    }

    #[test]
    fn daemon_attach_rejects_malformed_options_after_zeroing_handle() {
        let raw = CString::new("{not-json").unwrap();
        let mut handle: EasynetDaemonHandle = 42;
        let code = unsafe { easynet_daemon_attach(raw.as_ptr(), &mut handle) };
        assert_eq!(code, ERR_INVALID_ARG);
        assert_eq!(handle, 0);
    }

    #[test]
    fn daemon_discover_rejects_null_output() {
        let code = unsafe { easynet_daemon_discover(std::ptr::null(), std::ptr::null_mut()) };
        assert_eq!(code, ERR_NULL_POINTER);
    }

    #[test]
    fn daemon_status_rejects_invalid_handle_after_zeroing_output() {
        let mut out: *mut c_char = std::ptr::dangling_mut();
        let code = unsafe { easynet_daemon_status(9_999_999, &mut out) };
        assert_eq!(code, ERR_INVALID_HANDLE);
        assert!(out.is_null());

        let error = read_last_error_json();
        assert_eq!(error["code"], "INVALID_HANDLE");
        assert_eq!(error["details"]["abi_code"], ERR_INVALID_HANDLE);
        assert_eq!(error["details"]["legacy_untyped"], false);
    }

    #[test]
    fn daemon_invocation_endpoint_rejects_invalid_handle_after_zeroing_output() {
        let mut out: *mut c_char = std::ptr::dangling_mut();
        let code = unsafe { easynet_daemon_invocation_endpoint(9_999_999, &mut out) };
        assert_eq!(code, ERR_INVALID_HANDLE);
        assert!(out.is_null());
    }

    #[test]
    fn daemon_endpoints_rejects_invalid_handle_after_zeroing_output() {
        let mut out: *mut c_char = std::ptr::dangling_mut();
        let code = unsafe { easynet_daemon_endpoints(9_999_999, &mut out) };
        assert_eq!(code, ERR_INVALID_HANDLE);
        assert!(out.is_null());
    }

    #[test]
    fn daemon_detach_rejects_invalid_handle() {
        let code = easynet_daemon_detach(9_999_999);
        assert_eq!(code, ERR_INVALID_HANDLE);
    }

    #[test]
    fn daemon_open_client_rejects_null_out_handle_before_registry_lookup() {
        let code = unsafe { easynet_daemon_open_client(9_999_999, std::ptr::null_mut()) };
        assert_eq!(code, ERR_NULL_POINTER);
    }

    #[test]
    fn daemon_open_client_rejects_invalid_handle_after_zeroing_output() {
        let mut out: EasynetHandle = 42;
        let code = unsafe { easynet_daemon_open_client(9_999_999, &mut out) };
        assert_eq!(code, ERR_INVALID_HANDLE);
        assert_eq!(out, 0);
    }

    #[test]
    fn daemon_stop_rejects_invalid_handle() {
        let code = easynet_daemon_stop(9_999_999);
        assert_eq!(code, ERR_INVALID_HANDLE);
    }

    fn read_last_error_json() -> serde_json::Value {
        let mut out: *mut c_char = std::ptr::null_mut();
        let code = unsafe { crate::ffi::errors::easynet_last_error_json(&mut out) };
        assert_eq!(code, EASYNET_OK);
        assert!(!out.is_null());
        let value = unsafe { serde_json::from_str(CStr::from_ptr(out).to_str().unwrap()).unwrap() };
        unsafe { crate::ffi::strings::easynet_string_free(out) };
        value
    }
}
