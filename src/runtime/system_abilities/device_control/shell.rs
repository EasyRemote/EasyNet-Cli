// EasyNet CLI — shell.run ability (AXIOM Tier 2.5)
// =================================================
//
// File: src/runtime/system_abilities/device_control/shell.rs
// Description: The shell-interpreted member of the Baseline
//              Locomotion Profile. Takes a single bash command
//              string, runs the AXIOM Tier 2.5 8-stage pipeline
//              (empty / pre-checks / AST / security / permissions
//              / pathconstraints / readonly / destructive),
//              and on full pass dispatches to the SAME runner
//              process.exec uses — same caps, same redaction,
//              same receipt shape — with `bash -c` as the
//              transport.
//
// Pipeline order
// --------------
//
//   Stage 1  empty rejection       — handled by parser entry
//   Stage 2  pre-checks            — ast::parse_for_security
//   Stage 3  AST extraction        — ast::parse_for_security
//   Stage 4  security catalogue    — security::evaluate
//   Stage 5  permission rules      — permissions::evaluate
//   Stage 6  path constraints      — pathconstraints::evaluate_or_skip
//   Stage 7  readonly classifier   — readonly::evaluate_or_skip
//   Stage 8  destructive list      — destructive::is_destructive
//                                    on every argv[0]
//
// First failure short-circuits and returns a typed error
// response. Only when ALL eight stages pass does the
// dispatcher hand the original command string to bash via
// `runner::run` with `bash -c <command>`.
//
// Why bash -c, not /bin/sh -c
// ---------------------------
// AliveCode and the AXIOM spec both pin bash specifically.
// `/bin/sh` aliases to dash on Debian-family systems and
// to ash/busybox on Alpine — both of which lack array
// support and have subtly different word-splitting. The
// 8-stage pipeline reasons about bash semantics; running
// the validated command under a different interpreter
// would let differential parsing reopen the bypasses we
// just closed.
//
// Receipt fields
// --------------
// In addition to the process.exec receipt fields (exit_code,
// stdout, stderr, durations, env_keys, argv_sha256), the
// shell.run receipt carries:
//
//   * `command_sha256` — SHA-256 of the original command
//     string. The runner's argv_sha256 is computed over
//     `["bash", "-c", command]` which gives a different
//     fingerprint; auditors want both.
//   * `extracted_commands_count` — how many SimpleCommands
//     the AST stage produced. Helps debug cases where
//     pathconstraints or readonly fired on a sub-command
//     of a pipeline.
//   * `pipeline_stage` — the stage that rejected, when the
//     call did NOT pass. Operators see "rejected at stage 5
//     (permissions): no allow rule matched argv[0] `rm`".
//
// Author: Silan.Hu
// Email: silan.hu@u.nus.edu
// Copyright (c) 2026-2027 easynet. All rights reserved.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::runtime::ability_dispatch::AxonAbilityCatalog;
use crate::runtime::ability_dispatch::OwnerKind;
use crate::support::shellguard::ast::{parse_for_security, ParseForSecurityResult, SimpleCommand};
use crate::support::shellguard::destructive;
use crate::support::shellguard::pathconstraints::{self, Constraints, PathVerdict};
use crate::support::shellguard::permissions::{
    self, MatchMode, PermissionRejection, PermissionVerdict, Rule, RuleSet,
};
use crate::support::shellguard::readonly::{self, ReadOnlyRejection, ReadOnlyVerdict};
use crate::support::shellguard::runner::{
    self, RunError, RunRequest, Sandbox, OUTPUT_DEFAULT_CAP, OUTPUT_HARD_CAP, TIMEOUT_DEFAULT_MS,
    TIMEOUT_HARD_CAP_MS,
};
use crate::support::shellguard::security::{self, SecurityVerdict};

/// Wire name. Pinned by AXIOM Tier 2.5; rename = protocol break.
pub const ABILITY_NAME: &str = crate::runtime::ability_names::device_control::SHELL_RUN;

/// AXIOM Tier 2.5 profile version. Echoed in every receipt.
pub const PROFILE_VERSION: &str =
    crate::runtime::ability_names::device_control::BASELINE_LOCOMOTION_PROFILE_VERSION;

/// Bash binary the receiver always uses. Paths checked in
/// order — first existing one wins. Not configurable: the
/// 8-stage pipeline reasons about bash semantics, so the
/// dispatch interpreter MUST be bash (see module-level note).
const BASH_PATHS: &[&str] = &["/bin/bash", "/usr/bin/bash", "/usr/local/bin/bash"];

pub fn register(reg: &mut AxonAbilityCatalog) {
    reg.register_rpc_with_owner("shell.run", OwnerKind::Device, Arc::new(handler));
}

fn handler(args: Value) -> Result<Value> {
    let req = parse_request(&args)?;
    // Pipeline.
    let pipeline = match run_pipeline(&req) {
        Ok(stage_data) => stage_data,
        Err(rejection) => {
            return Ok(rejection_response(&req, rejection));
        }
    };
    // Pipeline passed — dispatch.
    let runtime_handle = tokio::runtime::Handle::current();
    let outcome = runtime_handle.block_on(execute(&req));
    Ok(build_response(&req, &pipeline, outcome))
}

#[derive(Clone, Debug)]
struct ShellRunRequest {
    command: String,
    cwd: Option<String>,
    env: Option<HashMap<String, String>>,
    timeout_ms: u64,
    stdin: Option<Vec<u8>>,
    output_max_bytes: u64,
    sandbox: Sandbox,
    destructive_acknowledged: bool,
    // Policy from the caller. None means "no constraint" for
    // the corresponding stage.
    permission_rules: Option<RuleSet>,
    write_allowed_roots: Option<Vec<PathBuf>>,
    read_only_only: bool,
}

fn parse_request(args: &Value) -> Result<ShellRunRequest> {
    let command = require_string(args, "command")?.trim().to_string();
    if command.is_empty() {
        return Err(anyhow!("shell.run: command must not be empty"));
    }
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
                    .ok_or_else(|| anyhow!("shell.run: env[{k}] must be a string value"))?;
                out.insert(k.clone(), s.to_string());
            }
            Some(out)
        }
        Some(Value::Null) | None => None,
        Some(_) => return Err(anyhow!("shell.run: env must be an object")),
    };
    let timeout_ms = args
        .get("timeout_ms")
        .and_then(Value::as_u64)
        .unwrap_or(TIMEOUT_DEFAULT_MS);
    if timeout_ms > TIMEOUT_HARD_CAP_MS {
        return Err(anyhow!(
            "shell.run: timeout_ms {timeout_ms} exceeds hard cap {TIMEOUT_HARD_CAP_MS}"
        ));
    }
    let stdin = decode_stdin(args)?;
    let output_max_bytes = args
        .get("output_max_bytes")
        .and_then(Value::as_u64)
        .unwrap_or(OUTPUT_DEFAULT_CAP);
    if output_max_bytes > OUTPUT_HARD_CAP {
        return Err(anyhow!(
            "shell.run: output_max_bytes {output_max_bytes} exceeds hard cap {OUTPUT_HARD_CAP}"
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
    let permission_rules = parse_permission_rules(args)?;
    let write_allowed_roots = parse_write_roots(args)?;
    let read_only_only = args
        .get("read_only_only")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok(ShellRunRequest {
        command,
        cwd,
        env,
        timeout_ms,
        stdin,
        output_max_bytes,
        sandbox,
        destructive_acknowledged,
        permission_rules,
        write_allowed_roots,
        read_only_only,
    })
}

fn parse_permission_rules(args: &Value) -> Result<Option<RuleSet>> {
    let allow = args
        .get("allow_rules")
        .and_then(Value::as_array)
        .map(|arr| parse_rule_array(arr, "allow_rules"))
        .transpose()?;
    let deny = args
        .get("deny_rules")
        .and_then(Value::as_array)
        .map(|arr| parse_rule_array(arr, "deny_rules"))
        .transpose()?;
    if allow.is_none() && deny.is_none() {
        return Ok(None);
    }
    Ok(Some(RuleSet {
        allow: allow.unwrap_or_default(),
        deny: deny.unwrap_or_default(),
    }))
}

fn parse_rule_array(arr: &[Value], field: &str) -> Result<Vec<Rule>> {
    let mut out = Vec::with_capacity(arr.len());
    for (i, v) in arr.iter().enumerate() {
        let obj = v
            .as_object()
            .ok_or_else(|| anyhow!("shell.run: {field}[{i}] must be an object"))?;
        let argv0_prefix = obj
            .get("argv0")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("shell.run: {field}[{i}].argv0 missing"))?
            .to_string();
        let mode = obj.get("match").and_then(Value::as_str).unwrap_or("exact");
        let match_mode = match mode {
            "exact" => MatchMode::Exact,
            "prefix" => MatchMode::Prefix,
            other => {
                return Err(anyhow!(
                    "shell.run: {field}[{i}].match {other:?} unknown; expected exact|prefix"
                ));
            }
        };
        let allowed_flags = match obj.get("flags") {
            Some(Value::Array(items)) => {
                let mut flags = Vec::with_capacity(items.len());
                for (j, item) in items.iter().enumerate() {
                    let s = item.as_str().ok_or_else(|| {
                        anyhow!("shell.run: {field}[{i}].flags[{j}] must be a string")
                    })?;
                    flags.push(s.to_string());
                }
                Some(flags)
            }
            Some(Value::Null) | None => None,
            Some(_) => {
                return Err(anyhow!(
                    "shell.run: {field}[{i}].flags must be an array of strings"
                ));
            }
        };
        out.push(Rule {
            argv0_prefix,
            match_mode,
            allowed_flags,
        });
    }
    Ok(out)
}

fn parse_write_roots(args: &Value) -> Result<Option<Vec<PathBuf>>> {
    match args.get("write_allowed_roots") {
        Some(Value::Array(items)) => {
            let mut out = Vec::with_capacity(items.len());
            for (i, v) in items.iter().enumerate() {
                let s = v.as_str().ok_or_else(|| {
                    anyhow!("shell.run: write_allowed_roots[{i}] must be a string")
                })?;
                out.push(PathBuf::from(s));
            }
            Ok(Some(out))
        }
        Some(Value::Null) | None => Ok(None),
        Some(_) => Err(anyhow!(
            "shell.run: write_allowed_roots must be an array of strings"
        )),
    }
}

fn decode_stdin(args: &Value) -> Result<Option<Vec<u8>>> {
    match args.get("stdin") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => {
            let encoding = args
                .get("stdin_encoding")
                .and_then(Value::as_str)
                .unwrap_or("base64");
            match encoding {
                "base64" => BASE64_STANDARD
                    .decode(s.as_bytes())
                    .map(Some)
                    .map_err(|e| anyhow!("shell.run: invalid stdin base64: {e}")),
                "utf8" => Ok(Some(s.as_bytes().to_vec())),
                other => Err(anyhow!(
                    "shell.run: stdin_encoding {other:?} unknown; expected base64|utf8"
                )),
            }
        }
        Some(Value::Array(items)) => {
            let mut out = Vec::with_capacity(items.len());
            for (i, v) in items.iter().enumerate() {
                let n = v
                    .as_u64()
                    .ok_or_else(|| anyhow!("shell.run: stdin[{i}] must be an integer in 0..256"))?;
                if n > 255 {
                    return Err(anyhow!("shell.run: stdin[{i}] = {n} out of range"));
                }
                out.push(n as u8);
            }
            Ok(Some(out))
        }
        Some(_) => Err(anyhow!(
            "shell.run: stdin must be a string, an array of bytes, or null"
        )),
    }
}

#[derive(Debug)]
struct PipelinePass {
    commands: Vec<SimpleCommand>,
}

#[derive(Debug)]
enum PipelineRejection {
    Parse {
        reason: String,
        node_type: Option<String>,
    },
    Security {
        violation: security::SecurityViolation,
    },
    Permission {
        argv_index: usize,
        reason: PermissionRejection,
    },
    Path {
        argv_index: usize,
        target: String,
        op: String,
    },
    ReadOnly {
        argv_index: usize,
        reason: ReadOnlyRejection,
    },
    Destructive {
        argv_index: usize,
        argv0: String,
    },
}

fn run_pipeline(req: &ShellRunRequest) -> std::result::Result<PipelinePass, PipelineRejection> {
    // Stages 1-3: parse_for_security folds empty / pre-checks / AST extraction.
    let parsed = parse_for_security(&req.command);
    let commands = match parsed {
        ParseForSecurityResult::Simple { commands } => commands,
        ParseForSecurityResult::TooComplex { reason, node_type } => {
            return Err(PipelineRejection::Parse { reason, node_type });
        }
        ParseForSecurityResult::ParseUnavailable => {
            return Err(PipelineRejection::Parse {
                reason: "tree-sitter-bash unavailable".to_string(),
                node_type: None,
            });
        }
    };
    if commands.is_empty() {
        // Empty / comment-only command — nothing to dispatch.
        // Fall through; the runner will see `bash -c ""` which
        // exits 0 immediately. Receipts will show
        // extracted_commands_count=0.
        return Ok(PipelinePass { commands });
    }
    // Stage 4: security catalogue.
    if let SecurityVerdict::Reject(violation) = security::evaluate(&commands) {
        return Err(PipelineRejection::Security { violation });
    }
    // Stage 5: permissions.
    if let Some(rules) = &req.permission_rules {
        if let PermissionVerdict::Rejected { argv_index, reason } =
            permissions::evaluate(&commands, rules)
        {
            return Err(PipelineRejection::Permission { argv_index, reason });
        }
    }
    // Stage 6: path constraints.
    if let Some(roots) = &req.write_allowed_roots {
        let cwd = req
            .cwd
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/"));
        let constraints = Constraints {
            write_allowed_roots: roots.clone(),
            cwd,
        };
        if let PathVerdict::Rejected {
            argv_index,
            target,
            op,
        } = pathconstraints::evaluate(&commands, &constraints)
        {
            return Err(PipelineRejection::Path {
                argv_index,
                target: target.to_string_lossy().into_owned(),
                op,
            });
        }
    }
    // Stage 7: readonly.
    if let ReadOnlyVerdict::Rejected { argv_index, reason } =
        readonly::evaluate_or_skip(&commands, req.read_only_only)
    {
        return Err(PipelineRejection::ReadOnly { argv_index, reason });
    }
    // Stage 8: destructive list. Apply per command argv[0]; any
    // hit without destructive_acknowledged rejects.
    if !req.destructive_acknowledged {
        for (i, cmd) in commands.iter().enumerate() {
            if let Some(argv0) = cmd.argv.first() {
                if destructive::is_destructive(argv0) {
                    return Err(PipelineRejection::Destructive {
                        argv_index: i,
                        argv0: argv0.clone(),
                    });
                }
            }
        }
    }
    Ok(PipelinePass { commands })
}

async fn execute(req: &ShellRunRequest) -> Result<runner::RunOutcome, RunError> {
    let bash = locate_bash().ok_or_else(|| {
        RunError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "bash binary not found in any of /bin /usr/bin /usr/local/bin",
        ))
    })?;
    runner::run(RunRequest {
        program: bash.into(),
        args: vec!["-c".to_string(), req.command.clone()],
        cwd: req.cwd.clone().map(Into::into),
        env: req.env.clone(),
        stdin: req.stdin.clone(),
        output_max_bytes: Some(req.output_max_bytes),
        timeout_ms: Some(req.timeout_ms),
        sandbox: req.sandbox,
    })
    .await
}

fn locate_bash() -> Option<&'static str> {
    BASH_PATHS
        .iter()
        .copied()
        .find(|p| std::path::Path::new(p).exists())
}

fn build_response(
    req: &ShellRunRequest,
    pipeline: &PipelinePass,
    outcome: Result<runner::RunOutcome, RunError>,
) -> Value {
    let outcome = match outcome {
        Ok(o) => o,
        Err(err) => {
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
                "command_sha256": command_sha256(&req.command),
                "extracted_commands_count": pipeline.commands.len(),
                "destructive_acknowledged": req.destructive_acknowledged,
                "ability_profile_version": PROFILE_VERSION,
            });
        }
    };
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
        "command_sha256": command_sha256(&req.command),
        "extracted_commands_count": pipeline.commands.len(),
        "cwd": req.cwd.clone().unwrap_or_default(),
        "env_keys": env_keys,
        "destructive_acknowledged": req.destructive_acknowledged,
        "ability_profile_version": PROFILE_VERSION,
    })
}

fn rejection_response(req: &ShellRunRequest, rejection: PipelineRejection) -> Value {
    let (stage, code, detail) = match rejection {
        PipelineRejection::Parse { reason, node_type } => (
            "ast",
            "AST_REJECTED",
            json!({ "reason": reason, "node_type": node_type }),
        ),
        PipelineRejection::Security { violation } => (
            "security",
            "SECURITY_REJECTED",
            json!({
                "category": violation.category.to_string(),
                "name": violation.name,
                "argv_index": violation.argv_index,
                "detail": violation.detail,
            }),
        ),
        PipelineRejection::Permission { argv_index, reason } => {
            let (kind, payload) = match reason {
                PermissionRejection::DeniedByRule { matched_prefix } => {
                    ("DeniedByRule", json!({ "matched_prefix": matched_prefix }))
                }
                PermissionRejection::NotAllowed => ("NotAllowed", json!({})),
                PermissionRejection::FlagNotAllowed {
                    matched_prefix,
                    offending_flag,
                } => (
                    "FlagNotAllowed",
                    json!({
                        "matched_prefix": matched_prefix,
                        "offending_flag": offending_flag,
                    }),
                ),
            };
            (
                "permissions",
                "PERMISSION_REJECTED",
                json!({
                    "argv_index": argv_index,
                    "kind": kind,
                    "payload": payload,
                }),
            )
        }
        PipelineRejection::Path {
            argv_index,
            target,
            op,
        } => (
            "pathconstraints",
            "PATH_REJECTED",
            json!({
                "argv_index": argv_index,
                "target": target,
                "op": op,
            }),
        ),
        PipelineRejection::ReadOnly { argv_index, reason } => {
            let (kind, payload) = match reason {
                ReadOnlyRejection::UnknownCommand { argv0 } => {
                    ("UnknownCommand", json!({ "argv0": argv0 }))
                }
                ReadOnlyRejection::GitNotReadOnly { subcommand } => {
                    ("GitNotReadOnly", json!({ "subcommand": subcommand }))
                }
                ReadOnlyRejection::WriteFlag { argv0, flag } => {
                    ("WriteFlag", json!({ "argv0": argv0, "flag": flag }))
                }
                ReadOnlyRejection::WriteRedirect { op, target } => {
                    ("WriteRedirect", json!({ "op": op, "target": target }))
                }
            };
            (
                "readonly",
                "READONLY_REJECTED",
                json!({
                    "argv_index": argv_index,
                    "kind": kind,
                    "payload": payload,
                }),
            )
        }
        PipelineRejection::Destructive { argv_index, argv0 } => (
            "destructive",
            "DESTRUCTIVE_REJECTED",
            json!({
                "argv_index": argv_index,
                "argv0": argv0,
            }),
        ),
    };
    json!({
        "ok": false,
        "code": code,
        "pipeline_stage": stage,
        "detail": detail,
        "command_sha256": command_sha256(&req.command),
        "destructive_acknowledged": req.destructive_acknowledged,
        "ability_profile_version": PROFILE_VERSION,
    })
}

fn command_sha256(command: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(command.as_bytes());
    hex::encode(hasher.finalize())
}

// ── Schema + description (for discovery) ──────────────────────────

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["command"],
        "additionalProperties": false,
        "properties": {
            "command": { "type": "string", "minLength": 1 },
            "cwd": { "type": "string" },
            "env": { "type": "object", "additionalProperties": { "type": "string" } },
            "timeout_ms": { "type": "integer", "minimum": 0, "maximum": TIMEOUT_HARD_CAP_MS },
            "stdin": {
                "oneOf": [
                    { "type": "string" },
                    { "type": "array", "items": { "type": "integer", "minimum": 0, "maximum": 255 } },
                    { "type": "null" }
                ]
            },
            "stdin_encoding": { "type": "string", "enum": ["base64", "utf8"] },
            "output_max_bytes": { "type": "integer", "minimum": 0, "maximum": OUTPUT_HARD_CAP },
            "sandbox": { "type": "boolean" },
            "destructive_acknowledged": { "type": "boolean" },
            "read_only_only": { "type": "boolean" },
            "allow_rules": {
                "type": "array",
                "items": {
                    "type": "object",
                    "required": ["argv0"],
                    "properties": {
                        "argv0": { "type": "string" },
                        "match": { "type": "string", "enum": ["exact", "prefix"] },
                        "flags": { "type": "array", "items": { "type": "string" } }
                    }
                }
            },
            "deny_rules": {
                "type": "array",
                "items": {
                    "type": "object",
                    "required": ["argv0"],
                    "properties": {
                        "argv0": { "type": "string" },
                        "match": { "type": "string", "enum": ["exact", "prefix"] },
                        "flags": { "type": "array", "items": { "type": "string" } }
                    }
                }
            },
            "write_allowed_roots": { "type": "array", "items": { "type": "string" } }
        }
    })
}

pub fn description() -> &'static str {
    "Run a bash command string through the AXIOM Tier 2.5 8-stage \
     security pipeline (empty / pre-check / AST / security catalogue / \
     permission rules / path constraints / read-only classifier / \
     destructive list) before dispatching to bash -c. Part of the \
     baseline-locomotion-v1 profile."
}

fn require_string<'a>(args: &'a Value, field: &str) -> Result<&'a str> {
    args.get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("shell.run: missing required string field `{field}`"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(pairs: &[(&str, Value)]) -> Value {
        let mut m = serde_json::Map::new();
        for (k, v) in pairs {
            m.insert(k.to_string(), v.clone());
        }
        Value::Object(m)
    }

    // ─── parse_request ───

    #[test]
    fn parse_minimal_request() {
        let r = parse_request(&args(&[("command", json!("ls"))])).unwrap();
        assert_eq!(r.command, "ls");
        assert!(r.permission_rules.is_none());
        assert!(r.write_allowed_roots.is_none());
        assert!(!r.read_only_only);
    }

    #[test]
    fn empty_command_rejected() {
        let err = parse_request(&args(&[("command", json!(""))])).unwrap_err();
        assert!(err.to_string().contains("must not be empty"));
    }

    #[test]
    fn whitespace_only_command_rejected() {
        let err = parse_request(&args(&[("command", json!("  \t  "))])).unwrap_err();
        assert!(err.to_string().contains("must not be empty"));
    }

    #[test]
    fn parse_allow_rules() {
        let r = parse_request(&args(&[
            ("command", json!("ls")),
            (
                "allow_rules",
                json!([{"argv0":"ls","match":"exact","flags":["-la"]}]),
            ),
        ]))
        .unwrap();
        let rs = r.permission_rules.unwrap();
        assert_eq!(rs.allow.len(), 1);
        assert_eq!(rs.allow[0].argv0_prefix, "ls");
        assert_eq!(rs.allow[0].allowed_flags.as_ref().unwrap(), &vec!["-la"]);
    }

    #[test]
    fn parse_write_roots() {
        let r = parse_request(&args(&[
            ("command", json!("ls")),
            ("write_allowed_roots", json!(["/tmp", "/var/log"])),
        ]))
        .unwrap();
        assert_eq!(
            r.write_allowed_roots.unwrap(),
            vec![PathBuf::from("/tmp"), PathBuf::from("/var/log")]
        );
    }

    #[test]
    fn invalid_match_mode_rejects() {
        let err = parse_request(&args(&[
            ("command", json!("ls")),
            ("allow_rules", json!([{"argv0":"ls","match":"glob"}])),
        ]))
        .unwrap_err();
        assert!(err.to_string().contains("expected exact|prefix"));
    }

    // ─── pipeline (no bash, no spawn) ───

    fn req_with(command: &str) -> ShellRunRequest {
        ShellRunRequest {
            command: command.to_string(),
            cwd: None,
            env: None,
            timeout_ms: TIMEOUT_DEFAULT_MS,
            stdin: None,
            output_max_bytes: OUTPUT_DEFAULT_CAP,
            sandbox: Sandbox::None,
            destructive_acknowledged: false,
            permission_rules: None,
            write_allowed_roots: None,
            read_only_only: false,
        }
    }

    #[test]
    fn pipeline_passes_simple_ls() {
        let r = req_with("ls -la");
        let p = run_pipeline(&r).unwrap();
        assert_eq!(p.commands.len(), 1);
    }

    #[test]
    fn pipeline_rejects_at_ast_for_command_substitution() {
        let r = req_with("rm $(echo /tmp/x)");
        match run_pipeline(&r) {
            Err(PipelineRejection::Parse { node_type, .. }) => {
                assert_eq!(node_type.as_deref(), Some("command_substitution"));
            }
            other => panic!("expected Parse rejection, got {other:?}"),
        }
    }

    #[test]
    fn pipeline_rejects_at_security_for_eval() {
        let r = req_with("eval rm");
        match run_pipeline(&r) {
            Err(PipelineRejection::Security { violation }) => {
                assert_eq!(violation.name, "eval");
            }
            other => panic!("expected Security, got {other:?}"),
        }
    }

    #[test]
    fn pipeline_rejects_at_security_for_bash_dash_c() {
        let r = req_with("bash -c 'rm -rf /'");
        match run_pipeline(&r) {
            Err(PipelineRejection::Security { violation }) => {
                assert_eq!(violation.name, "bash -c");
            }
            other => panic!("expected Security, got {other:?}"),
        }
    }

    #[test]
    fn pipeline_rejects_at_destructive_without_ack() {
        let r = req_with("rm /tmp/x");
        match run_pipeline(&r) {
            Err(PipelineRejection::Destructive { argv0, .. }) => {
                assert_eq!(argv0, "rm");
            }
            other => panic!("expected Destructive, got {other:?}"),
        }
    }

    #[test]
    fn pipeline_passes_destructive_with_ack() {
        let mut r = req_with("rm /tmp/x");
        r.destructive_acknowledged = true;
        // No allow_rules → permission stage is skipped, runs OK.
        run_pipeline(&r).unwrap();
    }

    #[test]
    fn pipeline_rejects_with_permission_default_deny() {
        let mut r = req_with("ls");
        r.permission_rules = Some(RuleSet {
            allow: vec![Rule::exact("cat")],
            deny: vec![],
        });
        match run_pipeline(&r) {
            Err(PipelineRejection::Permission {
                reason: PermissionRejection::NotAllowed,
                ..
            }) => {}
            other => panic!("expected Permission(NotAllowed), got {other:?}"),
        }
    }

    #[test]
    fn pipeline_passes_with_matching_allow_rule() {
        let mut r = req_with("ls -la");
        r.permission_rules = Some(RuleSet {
            allow: vec![Rule::exact("ls")],
            deny: vec![],
        });
        run_pipeline(&r).unwrap();
    }

    #[test]
    fn pipeline_rejects_at_pathconstraints_outside_root() {
        let mut r = req_with("echo hi > /etc/hosts");
        r.write_allowed_roots = Some(vec![PathBuf::from("/tmp")]);
        r.cwd = Some("/tmp".to_string());
        // Permission stage off → pathconstraints fires.
        match run_pipeline(&r) {
            Err(PipelineRejection::Path { target, op, .. }) => {
                assert_eq!(target, "/etc/hosts");
                assert_eq!(op, ">");
            }
            other => panic!("expected Path, got {other:?}"),
        }
    }

    #[test]
    fn pipeline_passes_path_inside_root() {
        let mut r = req_with("echo hi > /tmp/log");
        r.write_allowed_roots = Some(vec![PathBuf::from("/tmp")]);
        r.cwd = Some("/tmp".to_string());
        run_pipeline(&r).unwrap();
    }

    #[test]
    fn pipeline_rejects_readonly_when_enabled() {
        let mut r = req_with("rm /tmp/x");
        r.destructive_acknowledged = true; // wouldn't fire dest stage anyway
        r.read_only_only = true;
        match run_pipeline(&r) {
            Err(PipelineRejection::ReadOnly { reason, .. }) => match reason {
                ReadOnlyRejection::UnknownCommand { argv0 } => {
                    assert_eq!(argv0, "rm");
                }
                other => panic!("expected UnknownCommand, got {other:?}"),
            },
            other => panic!("expected ReadOnly, got {other:?}"),
        }
    }

    #[test]
    fn pipeline_passes_readonly_for_ls() {
        let mut r = req_with("ls -la");
        r.read_only_only = true;
        run_pipeline(&r).unwrap();
    }

    #[test]
    fn pipeline_empty_after_comment_only_passes() {
        let r = req_with("# just a comment");
        let p = run_pipeline(&r).unwrap();
        assert_eq!(p.commands.len(), 0);
    }

    // ─── rejection_response shape ───

    #[test]
    fn rejection_response_has_pipeline_stage_field() {
        let r = req_with("eval rm");
        let rejection = run_pipeline(&r).unwrap_err();
        let resp = rejection_response(&r, rejection);
        assert_eq!(resp["ok"], json!(false));
        assert_eq!(resp["pipeline_stage"], json!("security"));
        assert_eq!(resp["code"], json!("SECURITY_REJECTED"));
    }

    #[test]
    fn rejection_response_carries_command_sha256() {
        let r = req_with("rm /tmp");
        let rejection = run_pipeline(&r).unwrap_err();
        let resp = rejection_response(&r, rejection);
        let sha = resp["command_sha256"].as_str().unwrap();
        assert_eq!(sha.len(), 64);
        assert!(sha.chars().all(|c| c.is_ascii_hexdigit()));
    }

    // ─── command_sha256 ───

    #[test]
    fn command_sha256_is_stable_and_64_hex() {
        let a = command_sha256("ls -la");
        let b = command_sha256("ls -la");
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
    }

    #[test]
    fn command_sha256_differs_on_different_input() {
        assert_ne!(command_sha256("ls"), command_sha256("ls -la"));
    }

    // ─── input_schema ───

    #[test]
    fn input_schema_is_object_with_command_required() {
        let s = input_schema();
        assert_eq!(s["type"], json!("object"));
        let required: Vec<&str> = s["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(required.contains(&"command"));
    }

    // ─── locate_bash ───

    #[test]
    fn locate_bash_finds_bash_on_test_host() {
        // Most CI / dev hosts have bash at /bin/bash.
        // Skip the assertion if none of the candidates exist
        // (would only happen on a stripped-down container).
        if BASH_PATHS.iter().any(|p| std::path::Path::new(p).exists()) {
            assert!(locate_bash().is_some());
        }
    }
}
