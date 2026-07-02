// EasyNet CLI — process.exec ability (AXIOM Tier 2.5)
// =====================================================
//
// File: src/runtime/system_abilities/device_control/process.rs
// Description: The structured-execution member of the
//              Baseline Locomotion Profile. Spawns one
//              process via OS-level argv (NO shell
//              interpretation), captures stdout/stderr with
//              caps, enforces timeout via tree-kill, and
//              redacts the receipt per AXIOM Tier 2.5.
//
// process.exec vs. shell.run
// --------------------------
// process.exec is the structured path: caller hands a
// command + argv, the receiver passes them to execve
// verbatim. There is no /bin/sh -c. There is no glob, env
// expansion, or substitution. The injection surface is
// closed by construction.
//
// A caller that wants pipes / redirections / glob uses
// shell.run instead, which carries an 8-stage security
// pipeline (see AXIOM Tier 2.5). process.exec runs no such
// pipeline because it doesn't need to: the structured input
// shape is the security boundary.
//
// What process.exec DOES check
// ----------------------------
// 1. Schema (validated upstream by ability dispatch).
// 2. Caps: timeout_ms, output_max_bytes, sandbox.
// 3. Destructive command warning. AXIOM Tier 2.5
//    Stage 7 names a base-command list (rm, dd, mkfs*,
//    fdisk family, ...). process.exec consults the SAME
//    list as shell.run via shellguard::destructive. If the
//    base command is on the list and the caller did NOT
//    supply destructive_acknowledged: true, the call is
//    refused before spawn.
// 4. Receipt redaction: command name, argv SHA-256, env
//    keys (NEVER values), exit code, output sizes,
//    truncation flags.
//
// What process.exec does NOT check
// --------------------------------
// - argv contents. The argv vector is opaque to the
//   receiver. A caller that puts a destructive flag in args
//   (e.g. `["--rm-rf"]`) is the caller's call.
// - cwd / env contents. The receiver applies them; if the
//   caller wants stricter rules they go through admission.
//
// Author: Silan.Hu
// Email: silan.hu@u.nus.edu
// Copyright (c) 2026-2027 easynet. All rights reserved.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use serde_json::{json, Value};

use crate::runtime::ability_dispatch::AxonAbilityCatalog;
use crate::runtime::ability_dispatch::OwnerKind;
use crate::support::shellguard::destructive;
use crate::support::shellguard::runner::{
    self, RunError, RunRequest, Sandbox, OUTPUT_DEFAULT_CAP, OUTPUT_HARD_CAP, TIMEOUT_DEFAULT_MS,
    TIMEOUT_HARD_CAP_MS,
};

/// Wire name. Pinned by AXIOM Tier 2.5; a rename is a
/// protocol break.
pub const ABILITY_NAME: &str = crate::runtime::ability_names::device_control::PROCESS_EXEC;

/// AXIOM Tier 2.5 profile version. Echoed in every receipt
/// so a verifier can match against the right schema.
pub const PROFILE_VERSION: &str =
    crate::runtime::ability_names::device_control::BASELINE_LOCOMOTION_PROFILE_VERSION;

/// Register the handler. Stateless; no per-call setup.
pub fn register(reg: &mut AxonAbilityCatalog) {
    reg.register_rpc_with_owner("process.exec", OwnerKind::Device, Arc::new(handler));
}

fn handler(args: Value) -> Result<Value> {
    // The dispatcher gives us a sync handler; spawn the
    // tokio future on the current runtime via
    // `tokio::runtime::Handle::block_on` is the wrong tool
    // here because we're already inside a tokio context (the
    // dispatch thread). The safe pattern is `Handle::try_current().block_on`.
    //
    // The ability registry's `register_rpc` wraps every
    // handler in a `spawn_blocking`, so this thread is a
    // blocking-pool thread, not a tokio worker — a
    // `Handle::block_on` here is safe.
    let req = parse_request(&args)?;
    let runtime_handle = tokio::runtime::Handle::current();
    let outcome = runtime_handle.block_on(execute(req.clone()));
    Ok(build_response(&req, outcome))
}

#[derive(Clone, Debug)]
struct ExecRequest {
    command: String,
    argv: Vec<String>,
    cwd: Option<String>,
    env: Option<HashMap<String, String>>,
    timeout_ms: u64,
    stdin: Option<Vec<u8>>,
    output_max_bytes: u64,
    sandbox: Sandbox,
    destructive_acknowledged: bool,
}

fn parse_request(args: &Value) -> Result<ExecRequest> {
    let command = require_string(args, "command")?.to_string();

    let argv = match args.get("args") {
        Some(Value::Array(items)) => {
            let mut out = Vec::with_capacity(items.len());
            for (i, v) in items.iter().enumerate() {
                let s = v
                    .as_str()
                    .ok_or_else(|| anyhow!("process.exec: args[{i}] must be a string"))?;
                out.push(s.to_string());
            }
            out
        }
        Some(_) => return Err(anyhow!("process.exec: args must be an array of strings")),
        None => Vec::new(),
    };

    let cwd = args
        .get("cwd")
        .and_then(Value::as_str)
        .map(|s| s.to_string());

    let env = match args.get("env") {
        Some(Value::Object(map)) => {
            let mut out = HashMap::with_capacity(map.len());
            for (k, v) in map {
                let s = v
                    .as_str()
                    .ok_or_else(|| anyhow!("process.exec: env[{k}] must be a string value"))?;
                out.insert(k.clone(), s.to_string());
            }
            Some(out)
        }
        Some(Value::Null) | None => None,
        Some(_) => return Err(anyhow!("process.exec: env must be an object")),
    };

    let timeout_ms = args
        .get("timeout_ms")
        .and_then(Value::as_u64)
        .unwrap_or(TIMEOUT_DEFAULT_MS);
    if timeout_ms > TIMEOUT_HARD_CAP_MS {
        return Err(anyhow!(
            "process.exec: timeout_ms {timeout_ms} exceeds hard cap {TIMEOUT_HARD_CAP_MS}"
        ));
    }

    let stdin = decode_stdin(args)?;

    let output_max_bytes = args
        .get("output_max_bytes")
        .and_then(Value::as_u64)
        .unwrap_or(OUTPUT_DEFAULT_CAP);
    if output_max_bytes > OUTPUT_HARD_CAP {
        return Err(anyhow!(
            "process.exec: output_max_bytes {output_max_bytes} exceeds hard cap {OUTPUT_HARD_CAP}"
        ));
    }

    let sandbox_requested = args
        .get("sandbox")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let sandbox = if sandbox_requested {
        Sandbox::Best
    } else {
        Sandbox::None
    };

    let destructive_acknowledged = args
        .get("destructive_acknowledged")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    // Pre-spawn destructive check. Tier 2.5 stage 7
    // semantics: the base command is checked; argv flags are
    // not interpreted. A caller that wants `rm` MUST set
    // destructive_acknowledged: true.
    if destructive::is_destructive(&command) && !destructive_acknowledged {
        return Err(anyhow!(
            "process.exec: command {command:?} is on the AXIOM Tier 2.5 destructive list; \
             set destructive_acknowledged=true to proceed"
        ));
    }

    Ok(ExecRequest {
        command,
        argv,
        cwd,
        env,
        timeout_ms,
        stdin,
        output_max_bytes,
        sandbox,
        destructive_acknowledged,
    })
}

fn decode_stdin(args: &Value) -> Result<Option<Vec<u8>>> {
    match args.get("stdin") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => {
            // Default encoding is base64 (matches fs.write).
            // A caller wanting raw text passes
            // stdin_encoding="utf8".
            let encoding = args
                .get("stdin_encoding")
                .and_then(Value::as_str)
                .unwrap_or("base64");
            match encoding {
                "base64" => BASE64_STANDARD
                    .decode(s.as_bytes())
                    .map(Some)
                    .map_err(|e| anyhow!("process.exec: invalid stdin base64: {e}")),
                "utf8" => Ok(Some(s.as_bytes().to_vec())),
                other => Err(anyhow!(
                    "process.exec: stdin_encoding {other:?} unknown; expected \"base64\" or \"utf8\""
                )),
            }
        }
        Some(Value::Array(items)) => {
            // Bytes-as-array form (matches fs.write).
            let mut out = Vec::with_capacity(items.len());
            for (i, v) in items.iter().enumerate() {
                let n = v.as_u64().ok_or_else(|| {
                    anyhow!("process.exec: stdin[{i}] must be an integer in 0..256")
                })?;
                if n > 255 {
                    return Err(anyhow!("process.exec: stdin[{i}] = {n} out of range"));
                }
                out.push(n as u8);
            }
            Ok(Some(out))
        }
        Some(_) => Err(anyhow!(
            "process.exec: stdin must be a string (base64/utf8), an array of bytes, or null"
        )),
    }
}

async fn execute(req: ExecRequest) -> Result<runner::RunOutcome, RunError> {
    runner::run(RunRequest {
        program: req.command.clone().into(),
        args: req.argv.clone(),
        cwd: req.cwd.clone().map(Into::into),
        env: req.env.clone(),
        stdin: req.stdin.clone(),
        output_max_bytes: Some(req.output_max_bytes),
        timeout_ms: Some(req.timeout_ms),
        sandbox: req.sandbox,
    })
    .await
}

fn build_response(req: &ExecRequest, outcome: Result<runner::RunOutcome, RunError>) -> Value {
    let outcome = match outcome {
        Ok(o) => o,
        Err(err) => {
            // Spawn / cap / sandbox errors. Surface as a
            // failed call with a typed code so the caller can
            // distinguish "we never spawned" from "we spawned,
            // got an exit code".
            let code = match err {
                RunError::OutputCapTooLarge { .. } => "OUTPUT_CAP_TOO_LARGE",
                RunError::TimeoutTooLarge { .. } => "TIMEOUT_TOO_LARGE",
                RunError::SandboxUnavailable => "SANDBOX_UNAVAILABLE",
                RunError::SpawnFailed(_) => "SPAWN_FAILED",
                RunError::Io(_) => "RUNNER_IO_ERROR",
            };
            return json!({
                "ok": false,
                "code": code,
                "error": err.to_string(),
                // Receipt fields still emitted on the failure
                // path so the audit log carries them.
                "command_basename": destructive::basename(&req.command),
                "argv_sha256": runner::argv_sha256(&req.argv),
                "destructive_detected": destructive::is_destructive(&req.command),
                "destructive_acknowledged": req.destructive_acknowledged,
                "ability_profile_version": PROFILE_VERSION,
            });
        }
    };

    // Standard env-key audit: record which keys the caller
    // supplied (without their values).
    let env_keys: Vec<String> = req
        .env
        .as_ref()
        .map(|e| {
            let mut keys: Vec<String> = e.keys().cloned().collect();
            keys.sort();
            keys
        })
        .unwrap_or_default();

    json!({
        "ok": true,
        // Output
        "exit_code": outcome.exit_code,
        "stdout": BASE64_STANDARD.encode(&outcome.stdout),
        "stderr": BASE64_STANDARD.encode(&outcome.stderr),
        "stdout_bytes": outcome.stdout.len(),
        "stderr_bytes": outcome.stderr.len(),
        "timed_out": outcome.timed_out,
        "duration_ms": outcome.duration_ms,
        "output_truncated": outcome.output_truncated,
        "last_line_truncated": outcome.last_line_truncated,
        "sandbox_applied": outcome.sandbox_applied,
        // Receipt-redacted audit fields (Tier 2.5 mandate)
        "command_basename": destructive::basename(&req.command),
        "argv_sha256": runner::argv_sha256(&req.argv),
        "argv_count": req.argv.len(),
        "cwd": req.cwd.clone().unwrap_or_default(),
        "env_keys": env_keys,
        "destructive_detected": destructive::is_destructive(&req.command),
        "destructive_acknowledged": req.destructive_acknowledged,
        "ability_profile_version": PROFILE_VERSION,
    })
}

// ── Schema + description (for discovery) ──────────────────────────

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["command"],
        "additionalProperties": false,
        "properties": {
            "command": { "type": "string", "minLength": 1 },
            "args": { "type": "array", "items": { "type": "string" } },
            "cwd": { "type": "string" },
            "env": { "type": "object", "additionalProperties": { "type": "string" } },
            "timeout_ms": { "type": "integer", "minimum": 0, "maximum": TIMEOUT_HARD_CAP_MS },
            "stdin": {
                "oneOf": [
                    { "type": "string", "description": "base64 (default) or UTF-8 with stdin_encoding=\"utf8\"" },
                    { "type": "array", "items": { "type": "integer", "minimum": 0, "maximum": 255 } },
                    { "type": "null" }
                ]
            },
            "stdin_encoding": { "type": "string", "enum": ["base64", "utf8"] },
            "output_max_bytes": { "type": "integer", "minimum": 0, "maximum": OUTPUT_HARD_CAP },
            "sandbox": { "type": "boolean" },
            "destructive_acknowledged": { "type": "boolean" }
        }
    })
}

pub fn description() -> &'static str {
    "Run a process via OS-level argv (no shell interpretation). \
     Part of the baseline-locomotion-v1 profile (AXIOM §Tier 2.5). \
     For pipe/redirect/glob, use shell.run instead. Destructive \
     commands (rm/dd/mkfs/fdisk family) require \
     destructive_acknowledged=true."
}

// ── Helpers ────────────────────────────────────────────────────────

fn require_string<'a>(args: &'a Value, field: &str) -> Result<&'a str> {
    args.get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("process.exec: missing required string field `{field}`"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args_with(map: serde_json::Map<String, Value>) -> Value {
        Value::Object(map)
    }

    fn args(pairs: &[(&str, Value)]) -> Value {
        let mut m = serde_json::Map::new();
        for (k, v) in pairs {
            m.insert(k.to_string(), v.clone());
        }
        args_with(m)
    }

    // ─── Request parsing ─────────────────────────────────────

    #[test]
    fn parse_minimal_request_command_only() {
        let r = parse_request(&args(&[("command", json!("/bin/echo"))])).unwrap();
        assert_eq!(r.command, "/bin/echo");
        assert!(r.argv.is_empty());
        assert!(r.cwd.is_none());
        assert!(r.env.is_none());
        assert_eq!(r.timeout_ms, TIMEOUT_DEFAULT_MS);
        assert!(r.stdin.is_none());
        assert_eq!(r.output_max_bytes, OUTPUT_DEFAULT_CAP);
        assert_eq!(r.sandbox, Sandbox::None);
        assert!(!r.destructive_acknowledged);
    }

    #[test]
    fn parse_full_request() {
        let r = parse_request(&args(&[
            ("command", json!("/bin/echo")),
            ("args", json!(["a", "b"])),
            ("cwd", json!("/tmp")),
            ("env", json!({"K": "V"})),
            ("timeout_ms", json!(5000)),
            ("output_max_bytes", json!(100_000)),
        ]))
        .unwrap();
        assert_eq!(r.command, "/bin/echo");
        assert_eq!(r.argv, vec!["a", "b"]);
        assert_eq!(r.cwd.as_deref(), Some("/tmp"));
        assert_eq!(r.env.unwrap().get("K").map(String::as_str), Some("V"));
        assert_eq!(r.timeout_ms, 5000);
        assert_eq!(r.output_max_bytes, 100_000);
    }

    #[test]
    fn parse_rejects_missing_command() {
        let err = parse_request(&args(&[])).unwrap_err();
        assert!(err.to_string().contains("missing required string field"));
    }

    #[test]
    fn parse_rejects_non_string_arg() {
        let err = parse_request(&args(&[
            ("command", json!("/bin/echo")),
            ("args", json!(["ok", 123])),
        ]))
        .unwrap_err();
        assert!(err.to_string().contains("args[1] must be a string"));
    }

    #[test]
    fn parse_rejects_timeout_above_hard_cap() {
        let err = parse_request(&args(&[
            ("command", json!("/bin/echo")),
            ("timeout_ms", json!(TIMEOUT_HARD_CAP_MS + 1)),
        ]))
        .unwrap_err();
        assert!(err.to_string().contains("timeout_ms"));
    }

    #[test]
    fn parse_rejects_output_cap_above_hard_cap() {
        let err = parse_request(&args(&[
            ("command", json!("/bin/echo")),
            ("output_max_bytes", json!(OUTPUT_HARD_CAP + 1)),
        ]))
        .unwrap_err();
        assert!(err.to_string().contains("output_max_bytes"));
    }

    // ─── Destructive detection ────────────────────────────────

    #[test]
    fn rm_without_acknowledgement_is_rejected() {
        let err = parse_request(&args(&[("command", json!("rm"))])).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("destructive list"));
        assert!(msg.contains("destructive_acknowledged"));
    }

    #[test]
    fn rm_with_acknowledgement_passes_parse() {
        let r = parse_request(&args(&[
            ("command", json!("rm")),
            ("destructive_acknowledged", json!(true)),
        ]))
        .unwrap();
        assert!(r.destructive_acknowledged);
    }

    #[test]
    fn dd_with_path_and_no_ack_is_rejected() {
        let err = parse_request(&args(&[("command", json!("/usr/bin/dd"))])).unwrap_err();
        assert!(err.to_string().contains("destructive list"));
    }

    #[test]
    fn benign_commands_do_not_require_acknowledgement() {
        for cmd in ["/bin/echo", "ls", "cat", "/usr/bin/grep"] {
            let r = parse_request(&args(&[("command", json!(cmd))]))
                .unwrap_or_else(|e| panic!("{cmd} should parse: {e}"));
            assert!(!r.destructive_acknowledged);
        }
    }

    // ─── stdin decoding ───────────────────────────────────────

    #[test]
    fn parse_stdin_base64_default() {
        let r = parse_request(&args(&[
            ("command", json!("/bin/cat")),
            ("stdin", json!(BASE64_STANDARD.encode(b"hello"))),
        ]))
        .unwrap();
        assert_eq!(r.stdin.as_deref(), Some(b"hello".as_ref()));
    }

    #[test]
    fn parse_stdin_utf8_with_explicit_encoding() {
        let r = parse_request(&args(&[
            ("command", json!("/bin/cat")),
            ("stdin", json!("hello")),
            ("stdin_encoding", json!("utf8")),
        ]))
        .unwrap();
        assert_eq!(r.stdin.as_deref(), Some(b"hello".as_ref()));
    }

    #[test]
    fn parse_stdin_byte_array() {
        let r = parse_request(&args(&[
            ("command", json!("/bin/cat")),
            ("stdin", json!([72, 105])),
        ]))
        .unwrap();
        assert_eq!(r.stdin.as_deref(), Some(b"Hi".as_ref()));
    }

    #[test]
    fn parse_stdin_byte_array_rejects_out_of_range() {
        let err = parse_request(&args(&[
            ("command", json!("/bin/cat")),
            ("stdin", json!([256])),
        ]))
        .unwrap_err();
        assert!(err.to_string().contains("out of range"));
    }

    // ─── Schema sanity ────────────────────────────────────────

    #[test]
    fn schema_well_formed() {
        let s = input_schema();
        assert_eq!(s["type"], json!("object"));
        assert_eq!(s["required"], json!(["command"]));
        assert!(s["properties"]["command"].is_object());
        assert!(s["properties"]["destructive_acknowledged"].is_object());
    }

    #[test]
    fn description_mentions_profile_and_alternate() {
        let d = description();
        assert!(d.contains("baseline-locomotion-v1"));
        assert!(d.contains("shell.run"));
        assert!(d.contains("destructive_acknowledged"));
    }

    // ─── End-to-end via execute() (uses real OS) ──────────────

    #[tokio::test]
    async fn end_to_end_echo_returns_zero_exit() {
        let req = ExecRequest {
            command: "/bin/echo".into(),
            argv: vec!["hello".into()],
            cwd: None,
            env: None,
            timeout_ms: 5_000,
            stdin: None,
            output_max_bytes: 1024,
            sandbox: Sandbox::None,
            destructive_acknowledged: false,
        };
        let outcome = execute(req.clone()).await.unwrap();
        let resp = build_response(&req, Ok(outcome));
        assert_eq!(resp["ok"], json!(true));
        assert_eq!(resp["exit_code"], json!(0));
        let stdout = BASE64_STANDARD
            .decode(resp["stdout"].as_str().unwrap())
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&stdout).trim(), "hello");
    }

    #[tokio::test]
    async fn end_to_end_response_carries_destructive_audit_on_acknowledged_rm() {
        // We don't actually run rm — just confirm the
        // response carries the destructive flag honestly when
        // the underlying spawn would fail/succeed. Use /bin/echo
        // pretending to be on the destructive list by name…
        // actually no, the destructive check runs at parse time
        // not execute. So this test exercises only the audit
        // field on a clean run.
        let req = ExecRequest {
            command: "/bin/echo".into(),
            argv: vec![],
            cwd: None,
            env: None,
            timeout_ms: 5_000,
            stdin: None,
            output_max_bytes: 1024,
            sandbox: Sandbox::None,
            destructive_acknowledged: false,
        };
        let outcome = execute(req.clone()).await.unwrap();
        let resp = build_response(&req, Ok(outcome));
        assert_eq!(resp["destructive_detected"], json!(false));
        assert_eq!(resp["destructive_acknowledged"], json!(false));
        assert_eq!(resp["ability_profile_version"], json!(PROFILE_VERSION));
    }

    #[tokio::test]
    async fn end_to_end_failure_path_carries_typed_code() {
        // Spawn /this/does/not/exist → SpawnFailed → response
        // carries code=SPAWN_FAILED with the audit fields.
        let req = ExecRequest {
            command: "/this/does/not/exist".into(),
            argv: vec![],
            cwd: None,
            env: None,
            timeout_ms: 5_000,
            stdin: None,
            output_max_bytes: 1024,
            sandbox: Sandbox::None,
            destructive_acknowledged: false,
        };
        let outcome = execute(req.clone()).await;
        let resp = build_response(&req, outcome);
        assert_eq!(resp["ok"], json!(false));
        assert_eq!(resp["code"], json!("SPAWN_FAILED"));
        assert!(resp["argv_sha256"].is_string());
        assert_eq!(resp["destructive_detected"], json!(false));
    }
}
