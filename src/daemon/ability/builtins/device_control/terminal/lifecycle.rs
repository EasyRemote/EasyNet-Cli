// EasyNet CLI — terminal.{create,list,close} ability handlers
// =================================================================
//
// File: src/daemon/ability/builtins/device_control/terminal/lifecycle.rs
//
// Per RFC §18 + the C-M3a/b/c plan, PTY-hosted sessions are exposed
// through three abilities:
//
//   * terminal.create  (this file, RPC) — open a PTY,
//                                spawn a child, return session_id
//   * terminal.list    (this file, RPC) — snapshot live PTYs
//   * terminal.close   (this file, RPC) — kill the child,
//                                drop the session row
//   * terminal.attach  (C-M3c, BIDI)    — wire stdin/stdout
//                                between the IPC bidi pipe and the
//                                PTY master fd; the InvokeBidi
//                                machinery from C-M3a is the
//                                transport
//
// This file lands the unary half. attach lives separately because
// its handler signature is `LocalBidiHandler` not `LocalRpcHandler`,
// and bundling them would couple two distinct call modes in one
// register call.
//
// Layer (per AXON-RFC-001-ability-layers.md): Operational. PTY
// create + close are per-feature business verbs — they're not pure
// (create spawns a child process, close kills one) and they're not
// observation (a slow consumer attaching later is a different
// ability). The classifier in agents::tests::classify_ability
// has these in the Operational arm.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{json, Map, Value};

use crate::daemon::ability::dispatch::AxonAbilityCatalog;
use crate::daemon::ability::dispatch::OwnerKind;
use crate::daemon::execution::pty::{PtyCreateSpec, PtyService, PtySessionId};

pub const ABILITY_TERMINAL_CREATE: &str =
    crate::daemon::ability::names::device_control::TERMINAL_CREATE;
pub const ABILITY_TERMINAL_LIST: &str =
    crate::daemon::ability::names::device_control::TERMINAL_LIST;
pub const ABILITY_TERMINAL_CLOSE: &str =
    crate::daemon::ability::names::device_control::TERMINAL_CLOSE;

/// Description published by the dispatcher's `description_for`
/// arm. Mirrors AXIOM Tier 2.5 §"Baseline Locomotion / pty"
/// summary semantics so a discovery client sees the same blurb
/// here as in `meta.list_abilities`.
pub fn description_create() -> &'static str {
    "Create an interactive PTY session and return its opaque \
     session_id. Pair with terminal.attach (data plane) \
     and terminal.close (lifecycle teardown). Part of \
     the baseline-locomotion-v1 profile."
}

pub fn description_close() -> &'static str {
    "Tear down an interactive PTY session. Idempotent — passing \
     an unknown session_id returns ack=false rather than an \
     error so callers can poll without special-casing already- \
     gone sessions."
}

pub fn description_list() -> &'static str {
    "List live PTY sessions owned by this device daemon. Returns \
     daemon-minted session_id values suitable for terminal.attach, \
     input/read/resize, and close. Equivalent to the PTY-internal list \
     used by the lifecycle subsystem; the `terminal` namespace is the \
     stable operator-facing alias."
}

/// JSON Schema for terminal.create input. All fields
/// optional; the service fills VT100 defaults (80×24, terminal env,
/// host shell).
pub fn input_schema_create() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "cols": { "type": "integer", "minimum": 1, "maximum": 65535 },
            "rows": { "type": "integer", "minimum": 1, "maximum": 65535 },
            "command": { "type": "string", "minLength": 1 },
            "command_args": { "type": "array", "items": { "type": "string" } },
            "cwd": { "type": "string" },
            "env": { "type": "object", "additionalProperties": { "type": "string" } }
        }
    })
}

pub fn input_schema_close() -> Value {
    json!({
        "type": "object",
        "required": ["session_id"],
        "additionalProperties": false,
        "properties": {
            "session_id": { "type": "string", "minLength": 1 }
        }
    })
}

pub fn input_schema_list() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {}
    })
}

/// Default PTY size when the caller doesn't specify. 80×24 is the
/// classic VT100; matches what most terminal emulators open with so
/// shells render readably even before a `_resize` lands.
const DEFAULT_COLS: u16 = 80;
const DEFAULT_ROWS: u16 = 24;

/// Register both PTY lifecycle abilities on the registry. The
/// service handle is shared with the future `_attach` registration
/// so the three handlers see the same session table.
///
/// `io` is the optional companion I/O service (`terminal_io_ability::
/// PtyIoService`). When `Some`, the close handler also drops the
/// session's I/O row — releasing the cached writer fd and the
/// reader thread. None is acceptable for tests / fixtures that
/// don't exercise the unary I/O surface.
pub fn register(
    reg: &mut AxonAbilityCatalog,
    pty: Arc<PtyService>,
    io: Option<crate::daemon::ability::builtins::device_control::terminal::io::PtyIoService>,
) {
    use crate::daemon::ability::dispatch::LocalRpcHandler;
    let owner = OwnerKind::terminal_system();
    let svc_for_create = Arc::clone(&pty);
    let create_h: LocalRpcHandler =
        Arc::new(move |args: Value| create_handler(&svc_for_create, args));
    reg.register_rpc_with_owner("terminal.create", owner.clone(), create_h);

    let svc_for_list = Arc::clone(&pty);
    let list_h: LocalRpcHandler = Arc::new(move |args: Value| list_handler(&svc_for_list, args));
    reg.register_rpc_with_owner("terminal.list", owner.clone(), list_h);

    let pty_for_close = pty;
    let close_h = Arc::new(move |env, args: Value| {
        let close_args = TerminalCloseArgs::parse(args)?;
        super::authority::require_session_authority(
            &env,
            close_args.session_id(),
            "terminal.close",
        )?;
        close_session(&pty_for_close, io.as_ref(), close_args)
    });
    reg.register_rpc_with_envelope_and_owner("terminal.close", owner, close_h);
}

/// `terminal.create` handler.
///
/// Args: `{ cols?, rows?, command?, command_args?, cwd?, env? }`.
/// All fields optional; the service fills defaults.
///
/// Returns: `{ session_id }` — opaque string the caller hands to
/// _attach / _close. The shape mirrors §18's per-ability output
/// schema; production callers should NOT parse the id (it's a
/// UUIDv4 today, may change).
fn create_handler(pty: &Arc<PtyService>, args: Value) -> anyhow::Result<Value> {
    let spec = parse_create_spec(&args)?;
    let id = pty.create(spec)?;
    Ok(json!({ "session_id": id.as_str() }))
}

/// `terminal.list` handler.
///
/// Args: `{}`.
///
/// Returns: `{ sessions: [{ session_id, status, created_unix_ms, command?,
/// command_args?, cwd? }] }`.
fn list_handler(pty: &Arc<PtyService>, args: Value) -> anyhow::Result<Value> {
    require_lifecycle_args(&args, "terminal.list", &[])?;
    let sessions = pty
        .list()
        .into_iter()
        .map(|session| {
            json!({
                "session_id": session.id.as_str(),
                "status": "active",
                "created_unix_ms": session.created_unix_ms,
                "command": session.command,
                "command_args": session.command_args,
                "cwd": session.cwd,
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({ "sessions": sessions }))
}

/// `terminal.close` handler.
///
/// Args: `{ session_id }`.
///
/// Returns: `{ ack: bool, exit_status?: int }`.
///   * ack=true when the session was alive and is now removed.
///   * ack=false when the id was unknown — idempotent close per
///     PtyService::close's contract; callers can poll without
///     special-casing "already gone".
///   * exit_status is the child's exit code when waitable; absent
///     when the child was killed before the OS published a status
///     OR when ack=false (no child to wait on).
#[cfg(test)]
fn close_handler(
    pty: &Arc<PtyService>,
    io: Option<&crate::daemon::ability::builtins::device_control::terminal::io::PtyIoService>,
    args: Value,
) -> anyhow::Result<Value> {
    close_session(pty, io, TerminalCloseArgs::parse(args)?)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TerminalCloseArgs {
    session_id: String,
}

impl TerminalCloseArgs {
    fn parse(args: Value) -> anyhow::Result<Self> {
        let args = require_lifecycle_args(&args, "terminal.close", &["session_id"])?;
        let session_id = required_non_empty_string(args, "session_id", "terminal.close")?;
        Ok(Self { session_id })
    }

    fn session_id(&self) -> &str {
        &self.session_id
    }
}

fn required_non_empty_string(
    args: &Map<String, Value>,
    key: &str,
    ability: &str,
) -> anyhow::Result<String> {
    let value = args
        .get(key)
        .ok_or_else(|| anyhow::anyhow!("{ability}: `{key}` required"))?;
    let string = value
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("{ability}: `{key}` must be a string"))?
        .trim();
    if string.is_empty() {
        anyhow::bail!("{ability}: `{key}` must not be empty");
    }
    Ok(string.to_string())
}

fn close_session(
    pty: &Arc<PtyService>,
    io: Option<&crate::daemon::ability::builtins::device_control::terminal::io::PtyIoService>,
    close_args: TerminalCloseArgs,
) -> anyhow::Result<Value> {
    let session_id = PtySessionId::new(&close_args.session_id);
    let outcome = pty.close(&session_id);
    // Drop the I/O row AFTER the lifecycle close so the reader
    // thread sees the PTY EOF first (clean exit), then the
    // dropped flag (cooperative stop). Reverse order would race
    // with the reader thread on the master fd's last byte.
    if let Some(io) = io {
        io.drop_session(&session_id);
    }
    match outcome.exit_status {
        Some(code) => Ok(json!({ "ack": outcome.ack, "exit_status": code })),
        None => Ok(json!({ "ack": outcome.ack })),
    }
}

/// Parse the caller's args into a PtyCreateSpec, applying defaults.
///
/// Validation policy: reject unknown fields and malformed values. The
/// published ability schema is the control-plane contract; accepting
/// extra keys would let stale product/SDK argument shapes create live
/// PTY lifecycle state.
///
/// Error messages do NOT prefix the ability name; the dispatcher's
/// outer wrapper already attaches it. (Same SR-6+9 lesson the Go
/// half learned earlier — repeated prefixes are noise plus a
/// rename hazard.)
fn parse_create_spec(args: &Value) -> anyhow::Result<PtyCreateSpec> {
    let args = require_lifecycle_args(
        args,
        "terminal.create",
        &["cols", "rows", "command", "command_args", "cwd", "env"],
    )?;

    fn u16_field(args: &Map<String, Value>, key: &str, default: u16) -> anyhow::Result<u16> {
        match args.get(key) {
            None | Some(Value::Null) => Ok(default),
            Some(Value::Number(n)) => n
                .as_u64()
                .and_then(|v| u16::try_from(v).ok())
                .ok_or_else(|| anyhow::anyhow!("`{key}` must fit in u16 (got {n})")),
            Some(other) => anyhow::bail!("`{key}` must be a number, got {other}"),
        }
    }

    fn optional_string_field(
        args: &Map<String, Value>,
        key: &str,
    ) -> anyhow::Result<Option<String>> {
        match args.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(value)) => Ok(Some(value.clone())),
            Some(other) => anyhow::bail!("`{key}` must be a string, got {other}"),
        }
    }

    let cols = u16_field(args, "cols", DEFAULT_COLS)?;
    let rows = u16_field(args, "rows", DEFAULT_ROWS)?;
    if cols == 0 || rows == 0 {
        anyhow::bail!("cols and rows must be > 0");
    }

    let command = optional_string_field(args, "command")?;

    let command_args: Vec<String> = match args.get("command_args") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(arr)) => arr
            .iter()
            .map(|v| {
                v.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| anyhow::anyhow!("`command_args` entries must be strings"))
            })
            .collect::<anyhow::Result<Vec<_>>>()?,
        Some(other) => anyhow::bail!("`command_args` must be an array, got {other}"),
    };

    let cwd = optional_string_field(args, "cwd")?;

    let mut env: HashMap<String, String> = match args.get("env") {
        None | Some(Value::Null) => HashMap::new(),
        Some(Value::Object(map)) => map
            .iter()
            .map(|(k, v)| {
                v.as_str()
                    .map(|s| (k.clone(), s.to_string()))
                    .ok_or_else(|| anyhow::anyhow!("env values must be strings"))
            })
            .collect::<anyhow::Result<HashMap<_, _>>>()?,
        Some(other) => anyhow::bail!("`env` must be an object, got {other}"),
    };
    normalize_terminal_env(&mut env);

    Ok(PtyCreateSpec {
        cols,
        rows,
        command,
        command_args,
        cwd,
        env,
    })
}

fn require_lifecycle_args<'a>(
    args: &'a Value,
    ability: &str,
    allowed_keys: &[&str],
) -> anyhow::Result<&'a Map<String, Value>> {
    let object = args
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("{ability}: args must be an object"))?;
    for key in object.keys() {
        if !allowed_keys.contains(&key.as_str()) {
            anyhow::bail!("{ability}: unknown argument `{key}`");
        }
    }
    Ok(object)
}

fn normalize_terminal_env(env: &mut HashMap<String, String>) {
    let term_is_missing_or_dumb = env
        .get("TERM")
        .map(|value| {
            let trimmed = value.trim();
            trimmed.is_empty() || trimmed == "dumb"
        })
        .unwrap_or(true);
    if term_is_missing_or_dumb {
        env.insert("TERM".to_string(), "xterm-256color".to_string());
    }
    env.entry("COLORTERM".to_string())
        .or_insert_with(|| "truecolor".to_string());
}

// ── Discovery surfaces ────────────────────────────────────────

pub fn create_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "cols": {"type": "integer", "minimum": 1, "maximum": 65535},
            "rows": {"type": "integer", "minimum": 1, "maximum": 65535},
            "command": {"type": "string", "minLength": 1},
            "command_args": {"type": "array", "items": {"type": "string"}},
            "cwd": {"type": "string"},
            "env": {
                "type": "object",
                "additionalProperties": {"type": "string"}
            },
        },
        "additionalProperties": false,
    })
}

pub fn create_description() -> &'static str {
    "Open a new PTY-hosted child session. Returns the session_id \
     callers hand to terminal.attach (bidi) and \
     terminal.close (close+reap)."
}

pub fn list_description() -> &'static str {
    description_list()
}

pub fn list_input_schema() -> Value {
    input_schema_list()
}

pub fn close_input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["session_id"],
        "properties": {
            "session_id": {"type": "string", "minLength": 1},
        },
        "additionalProperties": false,
    })
}

pub fn close_description() -> &'static str {
    "Kill and reap a PTY session. Idempotent: returns ack=false \
     when the session_id is unknown so a polling caller can stop \
     without special-casing already-closed."
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_service() -> Arc<PtyService> {
        Arc::new(PtyService::new())
    }

    fn true_command() -> &'static str {
        if std::path::Path::new("/usr/bin/true").exists() {
            "/usr/bin/true"
        } else {
            "/bin/true"
        }
    }

    const TEST_DEVICE_URA: &str = "easynet:///r/test/device/terminal-lifecycle";

    fn metadata_test_catalog() -> AxonAbilityCatalog {
        AxonAbilityCatalog::new_test_metadata_for_device_authority(TEST_DEVICE_URA)
    }

    #[test]
    fn registration_makes_both_dispatchable() {
        let mut reg = metadata_test_catalog();
        register(&mut reg, fresh_service(), None);
        assert!(reg.get_rpc(ABILITY_TERMINAL_CREATE).is_some());
        assert!(reg.get_rpc(ABILITY_TERMINAL_LIST).is_some());
        assert!(reg.resolve_rpc_with_env(ABILITY_TERMINAL_CLOSE).is_some());
    }

    #[test]
    fn create_returns_session_id_and_inserts_row() {
        let svc = fresh_service();
        let resp = create_handler(&svc, json!({"command": true_command()})).expect("create true");
        let id = resp["session_id"].as_str().expect("session_id is string");
        assert!(!id.is_empty());
        assert_eq!(svc.live_count(), 1, "create must install a session row");
        // Clean up the spawned process.
        svc.close(&PtySessionId::new(id));
    }

    #[test]
    fn create_with_no_args_uses_default_shell() {
        // Args = {}. Service falls back to $SHELL / /bin/sh.
        // We don't assert on which shell — just that create returns
        // some session_id and registers the row.
        let svc = fresh_service();
        let resp = create_handler(&svc, json!({})).expect("create default shell");
        let id = resp["session_id"].as_str().unwrap();
        svc.close(&PtySessionId::new(id));
    }

    #[test]
    fn create_rejects_zero_cols_or_rows() {
        let svc = fresh_service();
        let err = create_handler(&svc, json!({"cols": 0, "rows": 24})).unwrap_err();
        assert!(format!("{err}").contains("must be > 0"));
        let err = create_handler(&svc, json!({"cols": 80, "rows": 0})).unwrap_err();
        assert!(format!("{err}").contains("must be > 0"));
    }

    #[test]
    fn create_rejects_non_numeric_cols() {
        let svc = fresh_service();
        let err = create_handler(&svc, json!({"cols": "wide"})).unwrap_err();
        assert!(format!("{err}").contains("must be a number"));
    }

    #[test]
    fn create_rejects_oversized_cols() {
        let svc = fresh_service();
        let err = create_handler(&svc, json!({"cols": 999_999})).unwrap_err();
        assert!(format!("{err}").contains("u16"));
    }

    #[test]
    fn create_rejects_unknown_command() {
        let svc = fresh_service();
        let err =
            create_handler(&svc, json!({"command": "/this/binary/does/not/exist"})).unwrap_err();
        assert!(format!("{err}").contains("spawn"));
    }

    #[test]
    fn close_known_session_returns_ack_true() {
        let svc = fresh_service();
        let resp = create_handler(&svc, json!({"command": true_command()})).unwrap();
        let id = resp["session_id"].as_str().unwrap().to_string();
        let close_resp = close_handler(&svc, None, json!({"session_id": id.clone()})).unwrap();
        assert_eq!(close_resp["ack"], true);
        assert_eq!(svc.live_count(), 0);
    }

    #[test]
    fn list_returns_live_sessions_and_close_removes_them() {
        let svc = fresh_service();
        let first = create_handler(&svc, json!({"command": true_command()})).unwrap();
        let second = create_handler(&svc, json!({"command": true_command()})).unwrap();
        let first_id = first["session_id"].as_str().unwrap().to_string();
        let second_id = second["session_id"].as_str().unwrap().to_string();

        let listed = list_handler(&svc, json!({})).unwrap();
        let sessions = listed["sessions"].as_array().expect("sessions array");
        let ids = sessions
            .iter()
            .map(|session| session["session_id"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec![first_id.as_str(), second_id.as_str()]);
        assert!(sessions
            .iter()
            .all(|session| session["status"].as_str() == Some("active")));

        close_handler(&svc, None, json!({"session_id": first_id})).unwrap();
        close_handler(&svc, None, json!({"session_id": second_id})).unwrap();
        let listed = list_handler(&svc, json!({})).unwrap();
        assert_eq!(listed["sessions"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn close_unknown_session_returns_ack_false_without_error() {
        let svc = fresh_service();
        let resp = close_handler(&svc, None, json!({"session_id": "ghost-id"})).unwrap();
        assert_eq!(resp["ack"], false);
        // exit_status absent when ack=false (no child to wait on).
        assert!(resp.get("exit_status").is_none());
    }

    #[test]
    fn close_rejects_missing_session_id() {
        let svc = fresh_service();
        let err = close_handler(&svc, None, json!({})).unwrap_err();
        assert!(format!("{err}").contains("session_id"));
    }

    #[test]
    fn close_rejects_non_object_args_before_service_lookup() {
        let svc = fresh_service();
        let err = close_handler(&svc, None, Value::Null).unwrap_err();
        let message = format!("{err}");
        assert!(
            message.contains("terminal.close: args must be an object"),
            "unexpected error: {message}"
        );
    }

    #[test]
    fn close_rejects_unknown_argument() {
        let svc = fresh_service();
        let err = close_handler(&svc, None, json!({"session_id": "ghost-id", "force": true}))
            .unwrap_err();
        assert!(
            format!("{err}").contains("terminal.close: unknown argument `force`"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn close_rejects_unknown_argument_before_idempotent_close_lookup() {
        let svc = fresh_service();
        let resp = create_handler(&svc, json!({"command": true_command()})).unwrap();
        let id = resp["session_id"].as_str().unwrap().to_string();
        let err = close_handler(
            &svc,
            None,
            json!({"session_id": id.clone(), "legacy_mode": true}),
        )
        .unwrap_err();
        let message = format!("{err}");
        assert!(
            message.contains("terminal.close: unknown argument `legacy_mode`"),
            "unexpected error: {message}"
        );
        assert_eq!(
            svc.live_count(),
            1,
            "unknown fields must fail before closing a live PTY"
        );
        svc.close(&PtySessionId::new(&id));
    }

    #[test]
    fn close_rejects_wrong_typed_session_id_before_service_lookup() {
        let svc = fresh_service();
        let err = close_handler(&svc, None, json!({"session_id": 42})).unwrap_err();
        let message = format!("{err}");
        assert!(
            message.contains("terminal.close: `session_id` must be a string"),
            "unexpected error: {message}"
        );
    }

    #[test]
    fn close_rejects_blank_session_id_before_idempotent_close_lookup() {
        let svc = fresh_service();
        let err = close_handler(&svc, None, json!({"session_id": "   "})).unwrap_err();
        let message = format!("{err}");
        assert!(
            message.contains("terminal.close: `session_id` must not be empty"),
            "unexpected error: {message}"
        );
    }

    #[test]
    fn close_is_idempotent_second_call_is_ack_false() {
        let svc = fresh_service();
        let resp = create_handler(&svc, json!({"command": true_command()})).unwrap();
        let id = resp["session_id"].as_str().unwrap().to_string();
        let first = close_handler(&svc, None, json!({"session_id": id.clone()})).unwrap();
        assert_eq!(first["ack"], true);
        let second = close_handler(&svc, None, json!({"session_id": id})).unwrap();
        assert_eq!(
            second["ack"], false,
            "second close on same id must ack=false"
        );
    }

    #[test]
    fn list_rejects_unknown_argument() {
        let svc = fresh_service();
        let err = list_handler(&svc, json!({"include_closed": true})).unwrap_err();
        assert!(
            format!("{err}").contains("terminal.list: unknown argument `include_closed`"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn parse_create_spec_rejects_unknown_fields() {
        let err = parse_create_spec(&json!({
            "cols": 100,
            "future_field_we_dont_know": true
        }))
        .expect_err("unknown terminal.create fields must fail closed");
        assert!(
            format!("{err}")
                .contains("terminal.create: unknown argument `future_field_we_dont_know`"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn parse_create_spec_rejects_non_string_command_and_cwd() {
        let err = parse_create_spec(&json!({"command": true})).unwrap_err();
        assert!(
            format!("{err}").contains("`command` must be a string"),
            "unexpected error: {err}"
        );
        let err = parse_create_spec(&json!({"cwd": 42})).unwrap_err();
        assert!(
            format!("{err}").contains("`cwd` must be a string"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn parse_create_spec_decodes_full_shape() {
        let spec = parse_create_spec(&json!({
            "cols": 120,
            "rows": 40,
            "command": "/bin/sh",
            "command_args": ["-c", "echo hi"],
            "cwd": "/tmp",
            "env": {"FOO": "bar"}
        }))
        .unwrap();
        assert_eq!(spec.cols, 120);
        assert_eq!(spec.rows, 40);
        assert_eq!(spec.command.as_deref(), Some("/bin/sh"));
        assert_eq!(spec.command_args, vec!["-c", "echo hi"]);
        assert_eq!(spec.cwd.as_deref(), Some("/tmp"));
        assert_eq!(spec.env.get("FOO").map(String::as_str), Some("bar"));
        assert_eq!(
            spec.env.get("TERM").map(String::as_str),
            Some("xterm-256color")
        );
        assert_eq!(
            spec.env.get("COLORTERM").map(String::as_str),
            Some("truecolor")
        );
    }

    #[test]
    fn parse_create_spec_preserves_explicit_terminal_env() {
        let spec = parse_create_spec(&json!({
            "env": {
                "TERM": "screen-256color",
                "COLORTERM": "24bit"
            }
        }))
        .unwrap();
        assert_eq!(
            spec.env.get("TERM").map(String::as_str),
            Some("screen-256color")
        );
        assert_eq!(spec.env.get("COLORTERM").map(String::as_str), Some("24bit"));
    }

    #[test]
    fn parse_create_spec_replaces_dumb_terminal_env() {
        let spec = parse_create_spec(&json!({
            "env": {
                "TERM": "dumb"
            }
        }))
        .unwrap();
        assert_eq!(
            spec.env.get("TERM").map(String::as_str),
            Some("xterm-256color")
        );
    }

    #[test]
    fn create_input_schema_pins_field_constraints() {
        let s = create_input_schema();
        assert_eq!(s["type"], "object");
        assert_eq!(s["additionalProperties"], false);
        assert_eq!(s["properties"]["cols"]["type"], "integer");
        assert_eq!(s["properties"]["command"]["minLength"], 1);
        assert_eq!(s["properties"]["env"]["type"], "object");
    }

    #[test]
    fn close_input_schema_requires_session_id() {
        let s = close_input_schema();
        let req = s["required"].as_array().unwrap();
        assert!(req.iter().any(|v| v == "session_id"));
        assert_eq!(s["properties"]["session_id"]["minLength"], 1);
    }
}
