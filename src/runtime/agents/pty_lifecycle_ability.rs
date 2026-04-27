// EasyNet CLI — fleet.pty_session_{create,close} ability handlers
// =================================================================
//
// File: src/runtime/agents/pty_lifecycle_ability.rs
//
// Per RFC §18 + the C-M3a/b/c plan, PTY-hosted sessions are exposed
// through three abilities:
//
//   * fleet.pty_session_create  (this file, RPC) — open a PTY,
//                                spawn a child, return session_id
//   * fleet.pty_session_close   (this file, RPC) — kill the child,
//                                drop the session row
//   * fleet.pty_session_attach  (C-M3c, BIDI)    — wire stdin/stdout
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

use serde_json::{json, Value};

use crate::runtime::ability_dispatch::LocalAbilityRegistry;
use crate::runtime::execution::pty::{PtyCreateSpec, PtyService, PtySessionId};

pub const ABILITY_PTY_SESSION_CREATE: &str = "fleet.pty_session_create";
pub const ABILITY_PTY_SESSION_CLOSE: &str = "fleet.pty_session_close";

/// Default PTY size when the caller doesn't specify. 80×24 is the
/// classic VT100; matches what most terminal emulators open with so
/// shells render readably even before a `_resize` lands.
const DEFAULT_COLS: u16 = 80;
const DEFAULT_ROWS: u16 = 24;

/// Register both PTY lifecycle abilities on the registry. The
/// service handle is shared with the future `_attach` registration
/// so the three handlers see the same session table.
pub fn register(reg: &mut LocalAbilityRegistry, pty: Arc<PtyService>) {
    let svc_for_create = Arc::clone(&pty);
    reg.register_rpc(
        ABILITY_PTY_SESSION_CREATE,
        Arc::new(move |args: Value| create_handler(&svc_for_create, args)),
    );
    reg.register_rpc(
        ABILITY_PTY_SESSION_CLOSE,
        Arc::new(move |args: Value| close_handler(&pty, args)),
    );
}

/// `fleet.pty_session_create` handler.
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

/// `fleet.pty_session_close` handler.
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
fn close_handler(pty: &Arc<PtyService>, args: Value) -> anyhow::Result<Value> {
    let id = args
        .get("session_id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("pty_session_close: `session_id` required"))?;
    let outcome = pty.close(&PtySessionId::new(id));
    match outcome.exit_status {
        Some(code) => Ok(json!({ "ack": outcome.ack, "exit_status": code })),
        None => Ok(json!({ "ack": outcome.ack })),
    }
}

/// Parse the caller's args into a PtyCreateSpec, applying defaults.
///
/// Validation policy: drop unknown fields silently (forward
/// compatibility — a future schema addition mustn't break old
/// callers), but reject malformed values (a `cols: "not-a-number"`
/// is a caller bug, not a forward-compat scenario).
fn parse_create_spec(args: &Value) -> anyhow::Result<PtyCreateSpec> {
    fn u16_field(args: &Value, key: &str, default: u16) -> anyhow::Result<u16> {
        match args.get(key) {
            None | Some(Value::Null) => Ok(default),
            Some(Value::Number(n)) => n
                .as_u64()
                .and_then(|v| u16::try_from(v).ok())
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "pty_session_create: `{key}` must fit in u16 (got {n})"
                    )
                }),
            Some(other) => anyhow::bail!(
                "pty_session_create: `{key}` must be a number, got {other}"
            ),
        }
    }

    let cols = u16_field(args, "cols", DEFAULT_COLS)?;
    let rows = u16_field(args, "rows", DEFAULT_ROWS)?;
    if cols == 0 || rows == 0 {
        anyhow::bail!("pty_session_create: cols and rows must be > 0");
    }

    let command = args
        .get("command")
        .and_then(Value::as_str)
        .map(str::to_string);

    let command_args: Vec<String> = match args.get("command_args") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(arr)) => arr
            .iter()
            .map(|v| {
                v.as_str().map(str::to_string).ok_or_else(|| {
                    anyhow::anyhow!(
                        "pty_session_create: `command_args` entries must be strings"
                    )
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?,
        Some(other) => anyhow::bail!(
            "pty_session_create: `command_args` must be an array, got {other}"
        ),
    };

    let cwd = args
        .get("cwd")
        .and_then(Value::as_str)
        .map(str::to_string);

    let env: HashMap<String, String> = match args.get("env") {
        None | Some(Value::Null) => HashMap::new(),
        Some(Value::Object(map)) => map
            .iter()
            .map(|(k, v)| {
                v.as_str().map(|s| (k.clone(), s.to_string())).ok_or_else(|| {
                    anyhow::anyhow!("pty_session_create: env values must be strings")
                })
            })
            .collect::<anyhow::Result<HashMap<_, _>>>()?,
        Some(other) => anyhow::bail!(
            "pty_session_create: `env` must be an object, got {other}"
        ),
    };

    Ok(PtyCreateSpec {
        cols,
        rows,
        command,
        command_args,
        cwd,
        env,
    })
}

// ── Discovery surfaces ────────────────────────────────────────

pub fn create_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "cols": {"type": "integer", "minimum": 1, "maximum": 65535},
            "rows": {"type": "integer", "minimum": 1, "maximum": 65535},
            "command": {"type": "string"},
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
     callers hand to fleet.pty_session_attach (bidi) and \
     fleet.pty_session_close (close+reap)."
}

pub fn close_input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["session_id"],
        "properties": {
            "session_id": {"type": "string"},
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

    #[test]
    fn registration_makes_both_dispatchable() {
        let mut reg = LocalAbilityRegistry::new();
        register(&mut reg, fresh_service());
        assert!(reg.get_rpc(ABILITY_PTY_SESSION_CREATE).is_some());
        assert!(reg.get_rpc(ABILITY_PTY_SESSION_CLOSE).is_some());
    }

    #[test]
    fn create_returns_session_id_and_inserts_row() {
        let svc = fresh_service();
        let resp = create_handler(
            &svc,
            json!({"command": true_command()}),
        )
        .expect("create true");
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
        let err = create_handler(
            &svc,
            json!({"command": "/this/binary/does/not/exist"}),
        )
        .unwrap_err();
        assert!(format!("{err}").contains("spawn"));
    }

    #[test]
    fn close_known_session_returns_ack_true() {
        let svc = fresh_service();
        let resp = create_handler(&svc, json!({"command": true_command()})).unwrap();
        let id = resp["session_id"].as_str().unwrap().to_string();
        let close_resp =
            close_handler(&svc, json!({"session_id": id.clone()})).unwrap();
        assert_eq!(close_resp["ack"], true);
        assert_eq!(svc.live_count(), 0);
    }

    #[test]
    fn close_unknown_session_returns_ack_false_without_error() {
        let svc = fresh_service();
        let resp =
            close_handler(&svc, json!({"session_id": "ghost-id"})).unwrap();
        assert_eq!(resp["ack"], false);
        // exit_status absent when ack=false (no child to wait on).
        assert!(resp.get("exit_status").is_none());
    }

    #[test]
    fn close_rejects_missing_session_id() {
        let svc = fresh_service();
        let err = close_handler(&svc, json!({})).unwrap_err();
        assert!(format!("{err}").contains("session_id"));
    }

    #[test]
    fn close_is_idempotent_second_call_is_ack_false() {
        let svc = fresh_service();
        let resp = create_handler(&svc, json!({"command": true_command()})).unwrap();
        let id = resp["session_id"].as_str().unwrap().to_string();
        let first = close_handler(&svc, json!({"session_id": id.clone()})).unwrap();
        assert_eq!(first["ack"], true);
        let second = close_handler(&svc, json!({"session_id": id})).unwrap();
        assert_eq!(
            second["ack"], false,
            "second close on same id must ack=false"
        );
    }

    #[test]
    fn parse_create_spec_drops_unknown_fields_silently() {
        // Forward compat: a future schema addition (e.g. `tty_name`)
        // must NOT break old daemons. The parser ignores unknown
        // top-level keys.
        let spec = parse_create_spec(&json!({
            "cols": 100,
            "future_field_we_dont_know": true
        }))
        .expect("unknown fields must be tolerated");
        assert_eq!(spec.cols, 100);
        assert_eq!(spec.rows, DEFAULT_ROWS);
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
    }

    #[test]
    fn create_input_schema_pins_field_constraints() {
        let s = create_input_schema();
        assert_eq!(s["type"], "object");
        assert_eq!(s["additionalProperties"], false);
        assert_eq!(s["properties"]["cols"]["type"], "integer");
        assert_eq!(s["properties"]["env"]["type"], "object");
    }

    #[test]
    fn close_input_schema_requires_session_id() {
        let s = close_input_schema();
        let req = s["required"].as_array().unwrap();
        assert!(req.iter().any(|v| v == "session_id"));
    }
}
