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
// - It is not the `runtime_init` client-session registry. A daemon
//   lifecycle handle names a process/status object, while
//   `RuntimeHandle` names an IPC client session.
// - It is not an Axon runtime lifecycle API. Starting
//   `axon-runtime` belongs to Axon SDK reference-runtime surfaces,
//   not to `libeasynet_cli`.
// - It is not a JSON-control ability bridge. Product calls must use
//   the complete Invocation ABI in `ffi::invocation`.

use std::os::raw::c_char;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use crate::daemon::{DaemonEndpoints, DaemonHandle, DaemonStartConfig};
use crate::ffi::client::handle::{alloc, ClientSession, RuntimeHandle};
use crate::ffi::errors::{
    clear_last_error, set_last_error_code, ERR_DAEMON_DOWN, ERR_GENERIC, ERR_INVALID_ARG,
    ERR_INVALID_HANDLE, ERR_INVALID_UTF8, ERR_NULL_POINTER, RUNTIME_OK,
};
use crate::ffi::strings::{alloc_output_cstring, read_cstr, StringError};

/// Opaque handle for a daemon process lifecycle session.
///
/// A value of 0 means "no daemon lifecycle handle allocated".
/// The handle is process-local and must not be persisted. It is
/// intentionally separate from `RuntimeHandle`, which names an IPC
/// client session returned by `runtime_init`.
pub type RuntimeHostHandle = u64;

/// Start or attach to the local runtime host.
///
/// `config_json` must be a UTF-8 JSON object with this shape:
///
/// ```text
/// {
///   "mode": "edge" | "authority",
///   "runtime_instance_id": "dev-a",  // required for edge host mode
///   "runtime_bin": "/path/to/bin",   // optional
///   "log_path": "/path/to/log",      // optional
///   "detached": true,                // optional
///   "env": {"KEY": "VALUE"}          // optional string map
/// }
/// ```
///
/// On success, `*out_host_handle` receives a daemon lifecycle
/// handle that can be passed to `runtime_host_status`,
/// `runtime_host_invocation_endpoint`, and `runtime_host_stop`.
///
/// # Safety
/// - `config_json` must point to a valid UTF-8 C string.
/// - `out_host_handle` must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn runtime_host_start(
    config_json: *const c_char,
    out_host_handle: *mut RuntimeHostHandle,
) -> i32 {
    if out_host_handle.is_null() {
        set_last_error_code(
            ERR_NULL_POINTER,
            "runtime_host_start: out_host_handle pointer is null",
        );
        return ERR_NULL_POINTER;
    }
    unsafe { *out_host_handle = 0 };

    let raw = match read_cstr(config_json) {
        Ok(value) => value,
        Err(StringError::Null) => {
            set_last_error_code(
                ERR_NULL_POINTER,
                "runtime_host_start: config_json pointer is null",
            );
            return ERR_NULL_POINTER;
        }
        Err(StringError::NotUtf8) => {
            set_last_error_code(
                ERR_INVALID_UTF8,
                "runtime_host_start: config_json is not valid UTF-8",
            );
            return ERR_INVALID_UTF8;
        }
    };

    let config = match DaemonStartConfigJson::parse(raw).and_then(DaemonStartConfigJson::build) {
        Ok(config) => config,
        Err(err) => {
            set_last_error_code(ERR_INVALID_ARG, format!("runtime_host_start: {err}"));
            return ERR_INVALID_ARG;
        }
    };

    let handle = match crate::daemon::start_daemon(&config) {
        Ok(handle) => handle,
        Err(err) => {
            set_last_error_code(ERR_DAEMON_DOWN, format!("runtime_host_start: {err}"));
            return ERR_DAEMON_DOWN;
        }
    };

    let id = insert_host_handle(handle);
    unsafe { *out_host_handle = id };
    clear_last_error();
    RUNTIME_OK
}

/// Attach to an already-running daemon without spawning it.
///
/// `options_json` is reserved for future endpoint override fields and
/// may be NULL today. Attach fails closed when control is up but the
/// Invocation endpoint is down.
///
/// # Safety
/// - `options_json` may be null; if non-null it must be valid UTF-8.
/// - `out_host_handle` must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn runtime_host_attach(
    options_json: *const c_char,
    out_host_handle: *mut RuntimeHostHandle,
) -> i32 {
    if out_host_handle.is_null() {
        set_last_error_code(
            ERR_NULL_POINTER,
            "runtime_host_attach: out_host_handle pointer is null",
        );
        return ERR_NULL_POINTER;
    }
    unsafe { *out_host_handle = 0 };
    let options = match read_attach_options(options_json, "runtime_host_attach") {
        Ok(options) => options,
        Err(code) => return code,
    };
    let handle = match attach_daemon_from_options(&options) {
        Ok(handle) => handle,
        Err(err) => {
            set_last_error_code(ERR_DAEMON_DOWN, format!("runtime_host_attach: {err}"));
            return ERR_DAEMON_DOWN;
        }
    };
    let id = insert_host_handle(handle);
    unsafe { *out_host_handle = id };
    clear_last_error();
    RUNTIME_OK
}

/// Discover current daemon endpoints and readiness without allocating
/// a lifecycle handle.
///
/// The returned string is caller-owned and must be freed with
/// `runtime_string_free`.
///
/// # Safety
/// - `options_json` may be null; if non-null it must be valid UTF-8.
/// - `out_discovery_json` must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn runtime_host_discover(
    options_json: *const c_char,
    out_discovery_json: *mut *mut c_char,
) -> i32 {
    if out_discovery_json.is_null() {
        set_last_error_code(
            ERR_NULL_POINTER,
            "runtime_host_discover: out_discovery_json pointer is null",
        );
        return ERR_NULL_POINTER;
    }
    unsafe { *out_discovery_json = std::ptr::null_mut() };
    let options = match read_attach_options(options_json, "runtime_host_discover") {
        Ok(options) => options,
        Err(code) => return code,
    };
    let status = match daemon_status_from_options(&options) {
        Ok(status) => status,
        Err(err) => {
            set_last_error_code(ERR_GENERIC, format!("runtime_host_discover: {err}"));
            return ERR_GENERIC;
        }
    };
    let ptr = alloc_output_cstring(daemon_status_json(&status).to_string());
    if ptr.is_null() {
        set_last_error_code(
            ERR_GENERIC,
            "runtime_host_discover: out-of-memory allocating discovery string",
        );
        return ERR_GENERIC;
    }
    unsafe { *out_discovery_json = ptr };
    clear_last_error();
    RUNTIME_OK
}

/// Stop a daemon lifecycle handle.
///
/// The handle is removed only after the stop operation succeeds.
/// Unknown handles return `ERR_INVALID_HANDLE`.
#[no_mangle]
pub extern "C" fn runtime_host_stop(handle: RuntimeHostHandle) -> i32 {
    let Some(daemon) = get_host_handle(handle) else {
        set_last_error_code(
            ERR_INVALID_HANDLE,
            format!("runtime_host_stop: daemon handle {handle} is not registered"),
        );
        return ERR_INVALID_HANDLE;
    };
    let stop_result = daemon
        .inner
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .stop();
    if let Err(err) = stop_result {
        set_last_error_code(ERR_DAEMON_DOWN, format!("runtime_host_stop: {err}"));
        return ERR_DAEMON_DOWN;
    }
    let _ = remove_host_handle(handle);
    clear_last_error();
    RUNTIME_OK
}

/// Detach a daemon lifecycle handle without stopping the daemon.
#[no_mangle]
pub extern "C" fn runtime_host_detach(handle: RuntimeHostHandle) -> i32 {
    let Some(daemon) = remove_host_handle(handle) else {
        set_last_error_code(
            ERR_INVALID_HANDLE,
            format!("runtime_host_detach: daemon handle {handle} is not registered"),
        );
        return ERR_INVALID_HANDLE;
    };
    daemon
        .inner
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .detach();
    clear_last_error();
    RUNTIME_OK
}

/// Return daemon liveness and endpoint status as JSON.
///
/// The returned string is caller-owned and must be freed with
/// `runtime_string_free`.
///
/// # Safety
/// `out_status_json` must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn runtime_host_status(
    handle: RuntimeHostHandle,
    out_status_json: *mut *mut c_char,
) -> i32 {
    if out_status_json.is_null() {
        set_last_error_code(
            ERR_NULL_POINTER,
            "runtime_host_status: out_status_json pointer is null",
        );
        return ERR_NULL_POINTER;
    }
    unsafe { *out_status_json = std::ptr::null_mut() };

    let Some(daemon) = get_host_handle(handle) else {
        set_last_error_code(
            ERR_INVALID_HANDLE,
            format!("runtime_host_status: daemon handle {handle} is not registered"),
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
            "runtime_host_status: out-of-memory allocating status string",
        );
        return ERR_GENERIC;
    }
    unsafe { *out_status_json = ptr };
    clear_last_error();
    RUNTIME_OK
}

/// Return the daemon Axon Invocation endpoint path.
///
/// The returned string is caller-owned and must be freed with
/// `runtime_string_free`.
///
/// # Safety
/// `out_endpoint` must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn runtime_host_invocation_endpoint(
    handle: RuntimeHostHandle,
    out_endpoint: *mut *mut c_char,
) -> i32 {
    if out_endpoint.is_null() {
        set_last_error_code(
            ERR_NULL_POINTER,
            "runtime_host_invocation_endpoint: out_endpoint pointer is null",
        );
        return ERR_NULL_POINTER;
    }
    unsafe { *out_endpoint = std::ptr::null_mut() };

    let Some(daemon) = get_host_handle(handle) else {
        set_last_error_code(
            ERR_INVALID_HANDLE,
            format!("runtime_host_invocation_endpoint: daemon handle {handle} is not registered"),
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
            "runtime_host_invocation_endpoint: out-of-memory allocating endpoint",
        );
        return ERR_GENERIC;
    }
    unsafe { *out_endpoint = ptr };
    clear_last_error();
    RUNTIME_OK
}

/// Return all daemon endpoints as JSON.
///
/// # Safety
/// `out_endpoints_json` must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn runtime_host_endpoints(
    handle: RuntimeHostHandle,
    out_endpoints_json: *mut *mut c_char,
) -> i32 {
    if out_endpoints_json.is_null() {
        set_last_error_code(
            ERR_NULL_POINTER,
            "runtime_host_endpoints: out_endpoints_json pointer is null",
        );
        return ERR_NULL_POINTER;
    }
    unsafe { *out_endpoints_json = std::ptr::null_mut() };

    let Some(daemon) = get_host_handle(handle) else {
        set_last_error_code(
            ERR_INVALID_HANDLE,
            format!("runtime_host_endpoints: daemon handle {handle} is not registered"),
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
            "runtime_host_endpoints: out-of-memory allocating endpoints string",
        );
        return ERR_GENERIC;
    }
    unsafe { *out_endpoints_json = ptr };
    clear_last_error();
    RUNTIME_OK
}

/// Open an Invocation-capable client handle from a daemon lifecycle
/// handle.
///
/// This is the binding-friendly bridge between the process lifecycle
/// ABI and the Invocation ABI: callers may start or attach to a daemon,
/// then call this function and pass the returned `RuntimeHandle` to
/// `runtime_invocation_*`. The returned handle is released with
/// `runtime_shutdown`.
///
/// # Safety
/// `out_handle` must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn runtime_host_open_client(
    host_handle: RuntimeHostHandle,
    out_handle: *mut RuntimeHandle,
) -> i32 {
    if out_handle.is_null() {
        set_last_error_code(
            ERR_NULL_POINTER,
            "runtime_host_open_client: out_handle pointer is null",
        );
        return ERR_NULL_POINTER;
    }
    unsafe { *out_handle = 0 };

    let Some(daemon) = get_host_handle(host_handle) else {
        set_last_error_code(
            ERR_INVALID_HANDLE,
            format!("runtime_host_open_client: daemon handle {host_handle} is not registered"),
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
                "runtime_host_open_client: daemon is not ready; control_accepting={}, invocation_accepting={}",
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
    RUNTIME_OK
}

#[derive(Debug)]
struct ActiveDaemonHandle {
    inner: Mutex<DaemonHandle>,
}

#[derive(Debug)]
struct DaemonHandleRegistry {
    next: AtomicU64,
    entries: Mutex<std::collections::HashMap<RuntimeHostHandle, Arc<ActiveDaemonHandle>>>,
}

fn host_handle_registry() -> &'static DaemonHandleRegistry {
    static REGISTRY: OnceLock<DaemonHandleRegistry> = OnceLock::new();
    REGISTRY.get_or_init(|| DaemonHandleRegistry {
        next: AtomicU64::new(1),
        entries: Mutex::new(std::collections::HashMap::new()),
    })
}

fn lock_daemon_entries(
    registry: &DaemonHandleRegistry,
) -> MutexGuard<'_, std::collections::HashMap<RuntimeHostHandle, Arc<ActiveDaemonHandle>>> {
    registry
        .entries
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn insert_host_handle(handle: DaemonHandle) -> RuntimeHostHandle {
    let registry = host_handle_registry();
    let id = registry.next.fetch_add(1, Ordering::Relaxed);
    lock_daemon_entries(registry).insert(
        id,
        Arc::new(ActiveDaemonHandle {
            inner: Mutex::new(handle),
        }),
    );
    id
}

fn get_host_handle(handle: RuntimeHostHandle) -> Option<Arc<ActiveDaemonHandle>> {
    if handle == 0 {
        return None;
    }
    lock_daemon_entries(host_handle_registry())
        .get(&handle)
        .cloned()
}

fn remove_host_handle(handle: RuntimeHostHandle) -> Option<Arc<ActiveDaemonHandle>> {
    if handle == 0 {
        return None;
    }
    lock_daemon_entries(host_handle_registry()).remove(&handle)
}

#[derive(Debug)]
struct DaemonStartConfigJson {
    mode: RuntimeHostStartMode,
    runtime_instance_id: Option<String>,
    realm: Option<String>,
    runtime_bin: Option<String>,
    working_dir: Option<String>,
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
            mode: RuntimeHostStartMode::parse(required_string(obj, "mode")?)?,
            runtime_instance_id: optional_string(obj, "runtime_instance_id")?,
            realm: optional_string(obj, "realm")?,
            runtime_bin: optional_string(obj, "runtime_bin")?,
            working_dir: optional_string(obj, "working_dir")?,
            log_path: optional_string(obj, "log_path")?,
            detached: optional_bool(obj, "detached")?,
            env: parse_env(obj)?,
        })
    }

    fn build(self) -> Result<DaemonStartConfig, DaemonStartConfigError> {
        let mut config = match self.mode {
            RuntimeHostStartMode::Edge => {
                let runtime_instance_id = self
                    .runtime_instance_id
                    .ok_or(DaemonStartConfigError::MissingField("runtime_instance_id"))?;
                DaemonStartConfig::device(runtime_instance_id)?
            }
            RuntimeHostStartMode::Authority => DaemonStartConfig::hub(),
        };
        if let Some(realm) = self.realm {
            config = config.with_realm(realm);
        }
        if let Some(path) = self.runtime_bin {
            config = config.with_daemon_bin(path)?;
        }
        if let Some(path) = self.working_dir {
            config = config.with_working_dir(path)?;
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
enum RuntimeHostStartMode {
    Edge,
    Authority,
}

impl RuntimeHostStartMode {
    fn parse(raw: String) -> Result<Self, DaemonStartConfigError> {
        match raw.as_str() {
            "edge" => Ok(Self::Edge),
            "authority" => Ok(Self::Authority),
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
    #[error("unsupported runtime host mode `{0}`")]
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

#[derive(Debug, Default)]
struct DaemonAttachOptions {
    control_path: Option<PathBuf>,
    control_endpoint: Option<PathBuf>,
    invocation_endpoint: Option<PathBuf>,
}

fn read_attach_options(
    options_json: *const c_char,
    function_name: &'static str,
) -> Result<DaemonAttachOptions, i32> {
    if options_json.is_null() {
        return Ok(DaemonAttachOptions::default());
    }
    let raw = match read_cstr(options_json) {
        Ok(raw) => raw,
        Err(StringError::NotUtf8) => {
            set_last_error_code(
                ERR_INVALID_UTF8,
                format!("{function_name}: options_json is not valid UTF-8"),
            );
            return Err(ERR_INVALID_UTF8);
        }
        Err(StringError::Null) => return Ok(DaemonAttachOptions::default()),
    };
    parse_attach_options(raw).map_err(|err| {
        set_last_error_code(ERR_INVALID_ARG, format!("{function_name}: {err}"));
        ERR_INVALID_ARG
    })
}

fn parse_attach_options(raw: &str) -> Result<DaemonAttachOptions, DaemonStartConfigError> {
    let value: serde_json::Value = serde_json::from_str(raw)?;
    if value.is_null() {
        return Ok(DaemonAttachOptions::default());
    }
    let object = value
        .as_object()
        .ok_or(DaemonStartConfigError::ExpectedObject)?;
    let string_path = |key: &str| {
        object
            .get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(normalize_local_endpoint_path)
    };
    Ok(DaemonAttachOptions {
        control_path: string_path("control_path"),
        control_endpoint: string_path("control_endpoint"),
        invocation_endpoint: string_path("invocation_endpoint"),
    })
}

fn normalize_local_endpoint_path(raw: &str) -> PathBuf {
    let normalized = raw.strip_prefix("unix://").unwrap_or(raw);
    PathBuf::from(normalized)
}

fn daemon_status_from_options(
    options: &DaemonAttachOptions,
) -> crate::daemon::Result<crate::daemon::DaemonStatus> {
    match daemon_endpoints_from_options(options)? {
        Some(endpoints) => Ok(crate::daemon::DaemonStatus::from_explicit_endpoints(
            endpoints,
        )),
        None => crate::daemon::DaemonStatus::try_current(),
    }
}

fn attach_daemon_from_options(
    options: &DaemonAttachOptions,
) -> crate::daemon::Result<DaemonHandle> {
    match daemon_endpoints_from_options(options)? {
        Some(endpoints) => DaemonHandle::attach_endpoints(endpoints),
        None => DaemonHandle::attach_current(),
    }
}

fn daemon_endpoints_from_options(
    options: &DaemonAttachOptions,
) -> crate::daemon::Result<Option<DaemonEndpoints>> {
    let Some(control_path) = options
        .control_path
        .as_ref()
        .or(options.control_endpoint.as_ref())
    else {
        return Ok(None);
    };
    let discovery_path = crate::daemon::control::discovery::resolve_control_json_path(control_path)
        .map_err(
            |source| crate::daemon::DaemonError::DaemonStateRootUnavailable {
                context: "explicit daemon attach discovery",
                source,
            },
        )?;
    let discovery = crate::daemon::control::discovery::read(&discovery_path)
        .map_err(
            |source| crate::daemon::DaemonError::DaemonStateRootUnavailable {
                context: "explicit daemon attach discovery read",
                source,
            },
        )?
        .ok_or_else(|| crate::daemon::DaemonError::DaemonStateRootUnavailable {
            context: "explicit daemon attach discovery missing",
            source: anyhow::anyhow!(
                "control discovery {} does not exist",
                discovery_path.display()
            ),
        })?;
    let control = options
        .control_endpoint
        .clone()
        .or(discovery.socket_path)
        .unwrap_or_else(|| control_path.to_path_buf());
    let invocation = options
        .invocation_endpoint
        .clone()
        .or(discovery.invocation_endpoint)
        .ok_or_else(|| crate::daemon::DaemonError::InvocationEndpointMissing {
            control: discovery_path.clone(),
        })?;
    require_absolute_endpoint(&control, "explicit daemon control endpoint")?;
    require_absolute_endpoint(&invocation, "explicit daemon invocation endpoint")?;
    Ok(Some(DaemonEndpoints {
        control,
        invocation,
    }))
}

fn require_absolute_endpoint(path: &Path, label: &'static str) -> crate::daemon::Result<()> {
    if !path.is_absolute() {
        return Err(crate::daemon::DaemonError::DaemonStateRootUnavailable {
            context: label,
            source: anyhow::anyhow!("{} must be absolute: {}", label, path.display()),
        });
    }
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
    fn parse_start_config_builds_edge_config() {
        let config = DaemonStartConfigJson::parse(
            r#"{
                "mode": "edge",
                "runtime_instance_id": "dev-a",
                "runtime_bin": "/tmp/runtime-host",
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
    fn parse_start_config_rejects_edge_without_runtime_instance_id() {
        let err = DaemonStartConfigJson::parse(r#"{"mode":"edge"}"#)
            .unwrap()
            .build()
            .unwrap_err();
        assert!(
            err.to_string().contains("runtime_instance_id"),
            "missing runtime_instance_id must be reported: {err}"
        );
    }

    #[test]
    fn daemon_start_rejects_null_out_handle_before_io() {
        let raw = CString::new(r#"{"mode":"authority"}"#).unwrap();
        let code = unsafe { runtime_host_start(raw.as_ptr(), std::ptr::null_mut()) };
        assert_eq!(code, ERR_NULL_POINTER);
    }

    #[test]
    fn parse_start_config_rejects_retired_product_modes() {
        for mode in ["device", "hub", "both", "combined"] {
            let err = DaemonStartConfigJson::parse(&format!(r#"{{"mode":"{mode}"}}"#))
                .expect_err("retired or unsupported host mode must fail at the C ABI boundary");
            assert!(
                err.to_string().contains("unsupported runtime host mode"),
                "unexpected mode error for {mode}: {err}"
            );
        }
    }

    #[test]
    fn daemon_start_rejects_malformed_json_after_zeroing_handle() {
        let raw = CString::new("{not-json").unwrap();
        let mut handle: RuntimeHostHandle = 42;
        let code = unsafe { runtime_host_start(raw.as_ptr(), &mut handle) };
        assert_eq!(code, ERR_INVALID_ARG);
        assert_eq!(handle, 0);
    }

    #[test]
    fn daemon_attach_rejects_malformed_options_after_zeroing_handle() {
        let raw = CString::new("{not-json").unwrap();
        let mut handle: RuntimeHostHandle = 42;
        let code = unsafe { runtime_host_attach(raw.as_ptr(), &mut handle) };
        assert_eq!(code, ERR_INVALID_ARG);
        assert_eq!(handle, 0);
    }

    #[test]
    fn daemon_attach_options_select_explicit_control_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let control_json = dir.path().join("control.json");
        let control_sock = dir.path().join("selected-control.sock");
        let invocation_sock = dir.path().join("selected-daemon.sock");
        crate::daemon::control::discovery::write(
            &control_json,
            &crate::daemon::control::discovery::ControlDiscovery {
                socket_path: Some(control_sock.clone()),
                pipe_name: None,
                invocation_endpoint: Some(dir.path().join("discovered-daemon.sock")),
                daemon_identity: Some(crate::daemon::control::discovery::DaemonIdentity {
                    mode: "hub".to_string(),
                    realm: "localhost".to_string(),
                    node_id: None,
                }),
                pid: 9_999,
                daemon_version: env!("CARGO_PKG_VERSION").to_string(),
                supported_ipc_versions: crate::daemon::control::discovery::IpcVersionRange::single(
                    1,
                ),
                capability_flags: Vec::new(),
                pages_port: None,
            },
        )
        .expect("write control discovery");

        let options = parse_attach_options(
            &serde_json::json!({
                "control_path": control_sock,
                "invocation_endpoint": format!("unix://{}", invocation_sock.display()),
            })
            .to_string(),
        )
        .expect("parse attach options");
        let endpoints = daemon_endpoints_from_options(&options)
            .expect("resolve explicit endpoints")
            .expect("explicit endpoint selection");

        assert_eq!(endpoints.control(), control_sock);
        assert_eq!(endpoints.invocation(), invocation_sock);
    }

    #[test]
    fn daemon_discover_rejects_null_output() {
        let code = unsafe { runtime_host_discover(std::ptr::null(), std::ptr::null_mut()) };
        assert_eq!(code, ERR_NULL_POINTER);
    }

    #[test]
    fn daemon_status_rejects_invalid_handle_after_zeroing_output() {
        let mut out: *mut c_char = std::ptr::dangling_mut();
        let code = unsafe { runtime_host_status(9_999_999, &mut out) };
        assert_eq!(code, ERR_INVALID_HANDLE);
        assert!(out.is_null());

        let error = read_last_error_json();
        assert_eq!(error["code"], "INVALID_HANDLE");
        assert_eq!(error["details"]["abi_code"], ERR_INVALID_HANDLE);
        assert_eq!(error["details"]["abi_symbol"], "ERR_INVALID_HANDLE");
    }

    #[test]
    fn daemon_invocation_endpoint_rejects_invalid_handle_after_zeroing_output() {
        let mut out: *mut c_char = std::ptr::dangling_mut();
        let code = unsafe { runtime_host_invocation_endpoint(9_999_999, &mut out) };
        assert_eq!(code, ERR_INVALID_HANDLE);
        assert!(out.is_null());
    }

    #[test]
    fn daemon_endpoints_rejects_invalid_handle_after_zeroing_output() {
        let mut out: *mut c_char = std::ptr::dangling_mut();
        let code = unsafe { runtime_host_endpoints(9_999_999, &mut out) };
        assert_eq!(code, ERR_INVALID_HANDLE);
        assert!(out.is_null());
    }

    #[test]
    fn daemon_detach_rejects_invalid_handle() {
        let code = runtime_host_detach(9_999_999);
        assert_eq!(code, ERR_INVALID_HANDLE);
    }

    #[test]
    fn daemon_open_client_rejects_null_out_handle_before_registry_lookup() {
        let code = unsafe { runtime_host_open_client(9_999_999, std::ptr::null_mut()) };
        assert_eq!(code, ERR_NULL_POINTER);
    }

    #[test]
    fn daemon_open_client_rejects_invalid_handle_after_zeroing_output() {
        let mut out: RuntimeHandle = 42;
        let code = unsafe { runtime_host_open_client(9_999_999, &mut out) };
        assert_eq!(code, ERR_INVALID_HANDLE);
        assert_eq!(out, 0);
    }

    #[test]
    fn daemon_stop_rejects_invalid_handle() {
        let code = runtime_host_stop(9_999_999);
        assert_eq!(code, ERR_INVALID_HANDLE);
    }

    fn read_last_error_json() -> serde_json::Value {
        let mut out: *mut c_char = std::ptr::null_mut();
        let code = unsafe { crate::ffi::errors::runtime_last_error_json(&mut out) };
        assert_eq!(code, RUNTIME_OK);
        assert!(!out.is_null());
        let value = unsafe { serde_json::from_str(CStr::from_ptr(out).to_str().unwrap()).unwrap() };
        unsafe { crate::ffi::strings::runtime_string_free(out) };
        value
    }
}
