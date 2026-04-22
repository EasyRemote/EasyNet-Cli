// EasyNet CLI — MCP Tool Handlers
// ================================
//
// File: src/mcp/handlers.rs
// Description: Implementation of Hub-level MCP tool handlers.
//
// Contract:
//   Each handler: `(bridge, tenant, args) → Result<Value, McpError>`.
//
//   The `McpError` variant picks the behaviour the calling agent should
//   take: `Validation` for bad input, `NotFound` for missing resources,
//   `Unavailable` for transient bridge/device failures, `Internal` for
//   bugs. The provider (`provider.rs`) renders the error into the
//   on-the-wire envelope `{"ok": false, "error_code": ..., "error": ...}`.
//
//   See `mcp/error.rs` for the full design note and the stability
//   guarantees on `error_code` strings.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use crate::eal;
use crate::facade::mcp::error::McpError;
use crate::persistence::config;
use crate::support::node::is_online;
use easynet_axon::dendrite_bridge::DendriteBridge;
use serde_json::{json, Map, Value};

/// Crate-local convenience alias for the handler return shape. Not part
/// of the public MCP contract — that contract is governed by
/// `McpError::error_code` and the on-the-wire envelope produced by
/// `provider::into_tool_result`. Keeping this alias `pub(crate)` makes
/// the boundary explicit: callers outside `mcp::` must go through the
/// rendered `ToolResult`, not the raw Rust type.
pub(crate) type HandlerResult = Result<Value, McpError>;

// ── Timeouts ────────────────────────────────────────────────────────────────
// MCP-level call budgets, in milliseconds to match the bridge API
// (commit 14115fa unified the unit across call sites). Kept next to the
// handlers they govern so the budget is visible at the call site rather
// than buried in shared/*.

/// Default deadline for a single `invoke_ability` call. Callers that need
/// a different budget should pass `timeout` through the CLI flag rather
/// than retuning this floor.
const INVOKE_ABILITY_TIMEOUT_MS: u64 = 60_000;

/// Default deadline for `execute_command`. One shot of shell is cheap;
/// the 60 s ceiling is for slow-starting interpreters. Long-running
/// commands belong in an ability, not in `execute_command`.
const EXECUTE_COMMAND_TIMEOUT_MS: u64 = 60_000;

// ── A2A listing bounds ──────────────────────────────────────────────────────
// The authoritative range lives in `super::specs` where the JSON Schema
// that advertises it to callers is generated. We pull the same constants
// here so the validator and the schema cannot drift — changing either
// side alone would silently produce a validator / advertised-bound
// mismatch and confuse every LLM that trusts the schema.

use super::specs::{LIST_A2A_LIMIT_DEFAULT, LIST_A2A_LIMIT_MAX, LIST_A2A_LIMIT_MIN};

fn req<'a>(args: &'a Map<String, Value>, key: &str) -> Result<&'a str, McpError> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| McpError::Validation(format!("missing required field `{key}`")))
}

fn parse_list_a2a_limit(args: &Map<String, Value>) -> Result<u32, McpError> {
    let err = || {
        McpError::Validation(format!(
            "field `limit` must be an integer in [{LIST_A2A_LIMIT_MIN}, {LIST_A2A_LIMIT_MAX}]"
        ))
    };
    #[allow(clippy::cast_possible_truncation)]
    let default_u32 = LIST_A2A_LIMIT_DEFAULT as u32;
    match args.get("limit") {
        None => Ok(default_u32),
        Some(Value::Number(n)) => n
            .as_u64()
            .filter(|v| (LIST_A2A_LIMIT_MIN..=LIST_A2A_LIMIT_MAX).contains(v))
            .map(|v| v as u32)
            .ok_or_else(err),
        Some(_) => Err(err()),
    }
}

/// Extract `node_id` for `invoke_ability`. The empty string is the
/// auto-route sentinel (runtime picks the first activated install);
/// any non-empty value is validated against the `NodeId` grammar so
/// a typo is caught at the MCP boundary instead of surfacing as a
/// bridge-side "not found".
fn parse_invoke_node_id(args: &Map<String, Value>) -> Result<&str, McpError> {
    let raw = match args.get("node_id") {
        None | Some(Value::Null) => return Ok(""),
        // `trim()` collapses both "explicit empty" and "whitespace-only"
        // into the same auto-route sentinel. The CLI layer (invoke.rs)
        // rejects `--node ""` earlier; at the MCP boundary we accept it
        // as auto-route so a programmatic caller can send an empty
        // string without first having to strip the field.
        Some(Value::String(s)) => s.trim(),
        Some(_) => {
            return Err(McpError::Validation(
                "field `node_id` must be a string".into(),
            ));
        }
    };
    if raw.is_empty() {
        return Ok("");
    }
    // Validate format at the boundary: a malformed node_id at this
    // point is a caller contract violation, not a transport issue.
    crate::core::agent_id::NodeId::parse(raw).map_err(|e| {
        McpError::Validation(format!("field `node_id`: {e}"))
    })?;
    Ok(raw)
}

/// Extract a required `node_id` field with format validation.
///
/// Unlike `req(args, "node_id")`, this runs the input through
/// [`NodeId::parse`] so handlers that accept a node id as a hard
/// requirement (every device-targeted verb) reject malformed input
/// uniformly. The returned `&str` is the caller-supplied text,
/// borrowed from `args`, and is safe to pass to bridge APIs that
/// expect `&str`. Passing the typed `NodeId` all the way through is
/// deferred until the bridge SDK grows a typed node-id argument; the
/// validation boundary here is the point where a programmatic error
/// would otherwise escape as a misleading "not found".
fn req_node_id(args: &Map<String, Value>) -> Result<&str, McpError> {
    let raw = req(args, "node_id")?;
    crate::core::agent_id::NodeId::parse(raw).map_err(|e| {
        McpError::Validation(format!("field `node_id`: {e}"))
    })?;
    Ok(raw)
}

fn parse_invoke_arguments(args: &Map<String, Value>) -> Result<Value, McpError> {
    match args.get("arguments") {
        None | Some(Value::Null) => Ok(json!({})),
        Some(v) if v.is_object() => Ok(v.clone()),
        Some(_) => Err(McpError::Validation(
            "field `arguments` must be a JSON object".into(),
        )),
    }
}

/// Wrap a shell command in a Python subprocess template that returns JSON.
///
/// Uses `python3 -` (stdin) to avoid shell-quoting issues with `-c`.
/// The command is JSON-encoded and embedded as a Python string literal,
/// so no shell interpretation of user input can occur.
///
/// Mirrors `easynet_axon` remote-control preset semantics (Python + Rust).
fn build_python_subprocess_template(command: &str) -> String {
    let quoted = serde_json::to_string(command).unwrap_or_else(|_| "\"\"".to_string());
    let script = format!(
        "import json,subprocess,sys; \
         cmd = {quoted}; \
         proc = subprocess.run(['/bin/sh', '-c', cmd], text=True, capture_output=True); \
         combined = (proc.stdout + proc.stderr).strip(); \
         print(json.dumps({{'entries': [combined], 'command': cmd, \
         'exit_code': proc.returncode, 'stdout': proc.stdout, 'stderr': proc.stderr}}))"
    );
    format!(
        "printf '%s' {json_script} | python3 -",
        json_script = shell_escape_posix(&script)
    )
}

/// POSIX-safe shell escaping: wraps in single quotes, escapes embedded single quotes.
fn shell_escape_posix(raw: &str) -> String {
    format!("'{}'", raw.replace('\'', "'\"'\"'"))
}

/// Convert an untyped error value (`anyhow::Error`,
/// `Box<dyn Error>`, …) into an [`McpError::Unavailable`].
///
/// # Why this is the *only* fallback — not the default path
///
/// The SDK now surfaces `Result<_, AxonError>` and routes through
/// [`From<AxonError> for McpError`](super::error::McpError). That is
/// the typed, category-preserving path — every `br.xxx().map_err(
/// McpError::from)` in this file classifies a `NotInstalled` as
/// `NotFound`, a `DeadlineExceeded` as `DeadlineExceeded`, and so
/// on. This function exists solely for the few call sites whose
/// error type is genuinely opaque — today that is
/// [`crate::eal::interpreter::execute_pooled_shared`], which
/// returns `anyhow::Error` because it spans the whole mission
/// pipeline (parse + plan + dispatch + trace-write) and no single
/// category fits the union.
///
/// When a caller uses this helper, it is promising one of two
/// things:
///
///   1. *I already categorised the typed errors before this call
///      and what's left is legitimately transport-class.*
///   2. *The error is genuinely opaque (a composition of many
///      things) and `Unavailable` is the safest default — agents
///      get a retryable signal, operator-facing message preserved.*
///
/// Any new call site should justify which case it falls under. If
/// the source is an [`easynet_axon::AxonError`], do NOT use this
/// helper — route through `McpError::from` for a precise category.
fn anyhow_to_mcp(e: impl std::fmt::Display) -> McpError {
    McpError::Unavailable(e.to_string())
}

pub fn hub_status(br: &DendriteBridge, tenant: &str, _: &Map<String, Value>) -> HandlerResult {
    let nodes = br.list_nodes(tenant, None).map_err(McpError::from)?;
    let on = nodes.iter().filter(|n| is_online(n)).count();
    Ok(json!({"nodes_online": on, "nodes_offline": nodes.len() - on}))
}

pub fn list_devices(br: &DendriteBridge, tenant: &str, args: &Map<String, Value>) -> HandlerResult {
    let nodes = br.list_nodes(tenant, None).map_err(McpError::from)?;
    let sf = args.get("state_filter").and_then(|v| v.as_str());
    let filtered: Vec<_> = nodes
        .into_iter()
        .filter(|n| {
            sf.is_none_or(|f| match f {
                "online" => is_online(n),
                "offline" => !is_online(n),
                _ => true,
            })
        })
        .collect();
    Ok(json!({"devices": filtered, "count": filtered.len()}))
}

pub fn get_device_detail(
    br: &DendriteBridge,
    tenant: &str,
    args: &Map<String, Value>,
) -> HandlerResult {
    let node_id = req_node_id(args)?;
    let nodes = br.list_nodes(tenant, None).map_err(McpError::from)?;
    // The device may legitimately not exist (typo, wrong tenant). Caller
    // needs `not_found` — not `unavailable` — because retrying won't help.
    let node = nodes
        .iter()
        .find(|n| n.get("node_id").and_then(|v| v.as_str()) == Some(node_id))
        .ok_or_else(|| McpError::NotFound(format!("device '{node_id}' not registered")))?;
    // `list_mcp_tools` failure here used to be swallowed into an empty
    // vec via `unwrap_or_default()`, which lied to the calling agent:
    // it would see `{"abilities": []}` and conclude the device hosts no
    // abilities, when in fact we just couldn't fetch the list. That is
    // exactly the class of silent false-negative the structured-error
    // migration was supposed to kill. Bridge failure here is transient
    // (transport / timeout / remote 5xx) → `Unavailable` so agents can
    // retry, and the device record is still surfaced for UX.
    let abilities = br
        .list_mcp_tools(tenant, "", node_id)
        .map_err(McpError::from)?;
    Ok(json!({"node": node, "abilities": abilities}))
}

/// Discover abilities across the federation, optionally filtered.
///
/// Historically this was two separate tools (`list_all_abilities`
/// for glob-filtered discovery and `search_abilities` for free-text
/// search). The bridge SDK exposes a single endpoint for both —
/// `list_mcp_tools(tenant, pattern, node_id)` treats the pattern as
/// a substring/glob — so the two tools were behaviourally
/// indistinguishable at the handler layer. Presenting them as
/// distinct MCP tools forced every LLM caller to pick between
/// functions that did the same thing; the redundant tool has been
/// removed and this handler absorbs the single merged spec.
///
/// Contract:
/// - `node_id` absent → federation-wide view
///   (`list_mcp_tools` returns entries keyed by tool_name with
///   `node_ids[]` showing every server)
/// - `node_id` present → scoped to that one device
/// - `name_pattern` absent → no filter
/// - `name_pattern` present → substring/glob match on tool_name
pub fn list_all_abilities(
    br: &DendriteBridge,
    tenant: &str,
    args: &Map<String, Value>,
) -> HandlerResult {
    let node = args.get("node_id").and_then(|v| v.as_str()).unwrap_or("");
    let pat = args
        .get("name_pattern")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let t = br.list_mcp_tools(tenant, pat, node).map_err(McpError::from)?;
    Ok(json!({"abilities": t, "count": t.len()}))
}

pub fn list_a2a_agents(
    br: &DendriteBridge,
    tenant: &str,
    args: &Map<String, Value>,
) -> HandlerResult {
    let tag_strings: Vec<String> = args
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default();
    let tag_refs: Vec<&str> = tag_strings.iter().map(String::as_str).collect();

    let owner_id = args
        .get("owner_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(ToString::to_string);
    let limit = parse_list_a2a_limit(args)?;

    let agents = br
        .list_a2a_agents(tenant, &tag_refs, owner_id.as_deref(), limit)
        .map_err(McpError::from)?;
    Ok(json!({"agents": agents, "count": agents.len()}))
}

pub fn get_a2a_agent_card(
    br: &DendriteBridge,
    tenant: &str,
    args: &Map<String, Value>,
) -> HandlerResult {
    let node_id = req_node_id(args)?;
    br.get_a2a_agent_card(tenant, node_id).map_err(McpError::from)
}

pub fn send_a2a_task(
    br: &DendriteBridge,
    tenant: &str,
    args: &Map<String, Value>,
) -> HandlerResult {
    let target_agent_id = req(args, "target_agent_id")?;
    let skill_id = req(args, "skill_id")?;

    let input_json = args.get("input_json").cloned().unwrap_or(json!({}));
    let task_id = args
        .get("task_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(ToString::to_string);
    let idempotency_key = args
        .get("idempotency_key")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(ToString::to_string);

    br.send_a2a_task_with_options(
        tenant,
        target_agent_id,
        skill_id,
        input_json,
        task_id.as_deref(),
        idempotency_key.as_deref(),
    )
    .map_err(McpError::from)
}

pub fn deploy_ability(
    br: &DendriteBridge,
    tenant: &str,
    args: &Map<String, Value>,
) -> HandlerResult {
    let node_id = req_node_id(args)?;
    let tool_name = req(args, "tool_name")?;
    let command = req(args, "command")?;
    let desc = args
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let signature = config::load_credentials()
        .ok()
        .map(|c| c.deploy_signature)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| {
            eprintln!(
                "deploy_ability: no deploy signature, using ephemeral placeholder (dev only)"
            );
            easynet_axon::EPHEMERAL_SIGNATURE.to_string()
        });

    let mut pkg_args = Map::<String, Value>::new();
    pkg_args.insert(
        "ability_name".to_string(),
        Value::String(tool_name.to_string()),
    );
    pkg_args.insert(
        "tool_name".to_string(),
        Value::String(tool_name.to_string()),
    );
    pkg_args.insert("description".to_string(), Value::String(desc.to_string()));
    pkg_args.insert(
        "command_template".to_string(),
        Value::String(build_python_subprocess_template(command)),
    );
    pkg_args.insert("version".to_string(), Value::String("1.0.0".to_string()));

    // `build_deploy_package` fails only on malformed caller input (bad
    // schema, missing required field) — that is a contract violation,
    // so it reads back as `Validation`. `deploy_package` can fail for
    // either input reasons or transport; we keep it as `Unavailable`
    // and let the SDK's own diagnostic string surface the detail.
    let descriptor = easynet_axon::ability::build_deploy_package(&pkg_args, &signature)
        .map_err(|e| McpError::Validation(e.to_string()))?;
    let deploy = easynet_axon::ability::deploy_package(br, tenant, node_id, &descriptor, true)
        .map_err(McpError::from)?;
    let deploy_value = easynet_axon::presets::remote_control::deploy_to_value(&deploy, &descriptor);

    Ok(json!({
        "ok": true,
        "node_id": node_id,
        "tool_name": descriptor.tool_name,
        "install_id": deploy.install_id,
        "deploy": deploy_value,
    }))
}

pub fn execute_command(
    br: &DendriteBridge,
    tenant: &str,
    args: &Map<String, Value>,
) -> HandlerResult {
    let node_id = req_node_id(args)?;
    let command = req(args, "command")?;
    br.call_mcp_tool_with_timeout(
        tenant,
        "session_bridge",
        node_id,
        &json!({"action": "exec", "command": command}),
        Some(EXECUTE_COMMAND_TIMEOUT_MS),
    )
    .map_err(McpError::from)
}

// ── ADR: stream_resume adoption (NOT applicable at current Cli scope) ──────
//
// The Axon SDK publishes `easynet_axon::stream_resume::{ResumingStream,
// StreamChunkSource, StreamFactory, ResumePolicy}` — an auto-reopening
// wrapper for server-streaming bridge calls. It is the right primitive
// for a consumer that reads chunks from a long-lived RPC and wants
// transparent reconnect across transport failures.
//
// Cli does not currently have such a consumer:
//
//   - `invoke_ability` (below) uses the unary
//     `DendriteBridge::call_mcp_tool_with_timeout` — one request in,
//     one response out, no streaming.
//   - `run_mission` runs an in-process EAL interpreter; its
//     fan-out uses pooled unary bridges, not a server stream.
//   - The chat-ability path (`facade::mcp::agent_dispatch`),
//     which surfaces each registered agent's default input as a
//     `<agent>.chat` MCP tool, dispatches to a local subprocess
//     — not a bridge call at all.
//
// Retrofitting `ResumingStream` into the unary invoke path would mean
// *introducing* a streaming consumer where none exists, i.e. feature
// work (e.g. adopting `call_mcp_tool_stream` for progress events on a
// long-running ability). That is out of scope for the SDK-alignment
// pass — ResumingStream is a resume abstraction; it is not a retrofit
// over a unary RPC.
//
// When Cli does gain a streaming consumer, *that call site* is where
// `ResumingStream<S>` with a `StreamChunkSource` adapter belongs.
// Expected signs that the moment has arrived:
//
//   * `DendriteBridge::call_mcp_tool_stream` appears in a handler's
//     signature here.
//   * A long-running ability (video processing, streaming inference)
//     motivates progress events.
//   * The MCP contract grows a streaming response shape.
//
// Adopting `ResumingStream` pre-emptively on unary calls would hide
// the actual retry primitive (`ReconnectingBridge::with_bridge`, which
// is the correct path for retry-once-on-transport-error semantics on
// a unary RPC, see `provider.rs::with_bridge`) and create two retry
// mechanisms for one problem. One retry mechanism per problem; the
// right abstraction for the right shape.

/// Invoke an ability. Omitting `node_id` triggers runtime-side auto-routing:
/// the runtime resolves the first activated install exposing this tool
/// within the caller's tenant and returns `selected_node_id` in the
/// response so callers see where the call landed. Passing a non-empty
/// `node_id` pins execution to that device.
///
/// # Tracing
///
/// When `trace = true`, the response is wrapped in an envelope that
/// carries a [`PhaseReceipt`] alongside the raw result:
///
/// ```json
/// { "result": <raw runtime result>, "trace": {
///     "phase": "invoke", "status": "ok"|"error",
///     "tenant_id": "...", "node_id": "...", "ability_id": "...",
///     "started_ms": ..., "ended_ms": ..., "duration_ms": ...
/// } }
/// ```
///
/// On failure, the outer error envelope is still returned (so agents
/// that branch on `error_code` keep working unchanged), but the
/// receipt is attached under the top-level `"trace"` key of the
/// error payload so telemetry is not lost. This mirrors the deploy
/// path's `DeployTrace` shape (see
/// `presets::remote_control::deploy_to_value`), the difference being
/// that invoke tracing is opt-in because invoke is a hot-path tool
/// call — unconditional embedding would break every pre-existing
/// consumer that does not expect the envelope.
///
/// `trace = false` (and absent) yields the pre-existing raw-result
/// shape — pinned by `invoke_ability_without_trace_preserves_raw_shape`
/// in the tests below.
///
/// [`PhaseReceipt`]: easynet_axon::receipt::PhaseReceipt
pub fn invoke_ability(
    br: &DendriteBridge,
    tenant: &str,
    args: &Map<String, Value>,
) -> HandlerResult {
    let ability = req(args, "ability")?;
    let node_id = parse_invoke_node_id(args)?;
    let arguments = parse_invoke_arguments(args)?;
    let trace_requested = parse_invoke_trace_flag(args)?;

    let call = || {
        br.call_mcp_tool_with_timeout(
            tenant,
            ability,
            node_id,
            &arguments,
            Some(INVOKE_ABILITY_TIMEOUT_MS),
        )
    };

    if !trace_requested {
        return call().map_err(McpError::from);
    }

    // Trace path: time the call with a `PhaseReceipt` that wraps
    // either the success value or the error's canonical code. The
    // receipt is always emitted once `trace=true` — even on failure,
    // because the fail-fast *duration* is itself telemetry.
    //
    // Node id on the receipt uses the caller-supplied value when
    // pinned, or "<auto>" when the caller left it empty (the runtime
    // will have resolved it server-side, but we don't see which
    // specific node was picked from here — the raw result carries
    // `selected_node_id` for that).
    let receipt_node_id = if node_id.is_empty() { "<auto>" } else { node_id };
    let builder = easynet_axon::receipt::PhaseReceipt::begin(
        easynet_axon::receipt::Phase::Invoke,
        tenant,
        receipt_node_id,
        ability,
    );

    // When `trace=true`, the caller has opted into the richer
    // `{result, trace}` envelope. We honour that on both the success
    // *and* the failure side: on failure we return `Ok(envelope)`
    // where the envelope is shaped like the standard error payload
    // (`{ok: false, error_code, error, trace}`). The caller still
    // branches on `error_code` exactly as before — only the top-level
    // `ok`/`is_error` distinction is lost on the failure path, which
    // is the honest trade for richer telemetry. The raw-result path
    // (`trace=false`) is unchanged for every non-tracing caller.
    match call() {
        Ok(value) => {
            let receipt = builder.finish_ok(None, None);
            let trace = serialize_receipt(&receipt)?;
            Ok(json!({
                "result": value,
                "trace": trace,
            }))
        }
        Err(axon_err) => {
            let receipt = builder.finish_err(&axon_err);
            let trace = serialize_receipt(&receipt)?;
            // Classify first so the error_code the caller branches on
            // is identical to what the non-trace path would have
            // produced — the only visible change is the extra
            // `trace` field and the `ok: false` wrapping that
            // `to_payload()` already builds.
            let mcp_err: McpError = axon_err.into();
            let mut payload = mcp_err.to_payload();
            payload
                .as_object_mut()
                .expect("McpError::to_payload always returns an object")
                .insert("trace".into(), trace);
            Ok(payload)
        }
    }
}

/// Serialize a [`PhaseReceipt`] into a JSON value for the invoke-path
/// trace envelope. The receipt type is serde-derived from plain data,
/// so `to_value` only fails on a theoretical serializer defect; we
/// surface that path as `Internal` to match the policy in every other
/// handler ("unexpected condition, probably a bug") rather than hide
/// it behind an `.expect(..)` that would take the stdio loop down on
/// regression.
fn serialize_receipt(
    receipt: &easynet_axon::receipt::PhaseReceipt,
) -> Result<Value, McpError> {
    serde_json::to_value(receipt)
        .map_err(|e| McpError::Internal(format!("PhaseReceipt serialize: {e}")))
}

/// Parse the optional `trace` boolean from the tool-call arguments.
/// Absent and `null` mean "no trace, raw response shape". Non-boolean
/// types are rejected as `Validation` — no silent coercion.
fn parse_invoke_trace_flag(args: &Map<String, Value>) -> Result<bool, McpError> {
    match args.get("trace") {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(b)) => Ok(*b),
        Some(other) => Err(McpError::Validation(format!(
            "`trace` must be a boolean if present, got {}",
            json_type_name(other)
        ))),
    }
}

/// Human-readable JSON type name for validation error messages.
fn json_type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Execute a mission reusing a shared BridgePool for parallel phase execution.
///
/// This is the primary path for MCP server calls. The pool is persisted across
/// the MCP session lifetime, so connections are amortized across missions.
pub fn run_mission_with_pool(
    pool: std::sync::Arc<crate::support::bridge_pool::BridgePool>,
    tenant: &str,
    args: &Map<String, Value>,
) -> HandlerResult {
    let source = req(args, "eal_source")?;
    let emit_only = args
        .get("emit_ir_only")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    // EAL parse/compile failures are always caller-input problems
    // (malformed mission source). Surface them as `Validation` so an
    // agent can tell them apart from transient transport errors.
    let program = eal::parser::parse(source)
        .map_err(|e| McpError::Validation(format!("EAL parse: {e}")))?;
    let ir = eal::planner::compile(&program)
        .map_err(|e| McpError::Validation(format!("EAL compile: {e}")))?;

    if emit_only {
        // IR serialization is infallible in practice (every field is
        // serde-derived from plain data types), but a silent fallback
        // to `null` was the previous behaviour and would have the
        // agent see `{"ok": true, "emit_ir_only": null}` indistinguishable
        // from a successfully-emitted empty IR. Surface the real error
        // through `Internal` so the caller can report it instead of
        // treating "null" as success.
        return serde_json::to_value(&ir)
            .map_err(|e| McpError::Internal(format!("IR serialize: {e}")));
    }

    let r = eal::interpreter::execute_pooled_shared(pool, tenant, &ir).map_err(anyhow_to_mcp)?;
    Ok(json!({
        "ok": true,
        "mission": ir.name,
        "steps_completed": r.steps_completed,
        "steps_failed": r.steps_failed,
        "elapsed_ms": r.total_elapsed_ms,
    }))
}

pub fn manage_device(
    br: &DendriteBridge,
    tenant: &str,
    args: &Map<String, Value>,
) -> HandlerResult {
    let node_id = req_node_id(args)?;
    let action = req(args, "action")?;
    match action {
        "drain" => br.drain_node(tenant, node_id, "CLI").map_err(McpError::from),
        "disconnect" => br
            .deregister_node(tenant, node_id, "CLI")
            .map_err(McpError::from),
        // Unknown action is a caller-input bug — Validation, not Internal.
        _ => Err(McpError::Validation(format!(
            "unknown action `{action}` (supported: drain, disconnect)"
        ))),
    }
}

pub fn uninstall_ability(
    br: &DendriteBridge,
    tenant: &str,
    args: &Map<String, Value>,
) -> HandlerResult {
    let node_id = req_node_id(args)?;
    let install_id = req(args, "install_id")?;
    let r = br
        .uninstall_capability(tenant, node_id, install_id)
        .map_err(McpError::from)?;
    Ok(json!({"ok": true, "result": r}))
}

/// Send a prompt to a registered AI agent (pre-bridge fast path — no DendriteBridge needed).
pub fn send_to_agent(args: &Map<String, Value>) -> HandlerResult {
    use crate::runtime::dispatch;
    use crate::registry::agents;

    let agent_name = req(args, "agent")?;
    let prompt = req(args, "prompt")?;
    let context = args.get("context").and_then(|v| v.as_str());

    let registry = agents::load_agents()
        .map_err(|e| McpError::Internal(format!("agent registry read failed: {e}")))?;
    let entry = registry.agents.get(agent_name).ok_or_else(|| {
        McpError::NotFound(format!("agent '{agent_name}' not in local registry"))
    })?;

    let response = dispatch::send_to_agent(agent_name, entry, prompt, context, None)
        .map_err(|e| McpError::Unavailable(format!("agent dispatch: {e}")))?;

    Ok(json!({
        "ok": true,
        "agent": response.agent,
        "content": response.content,
        "model": response.model,
        "duration_ms": response.duration_ms,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn obj(v: Value) -> Map<String, Value> {
        v.as_object().expect("test input must be object").clone()
    }

    #[test]
    fn parse_list_a2a_limit_defaults_and_accepts_valid_range() {
        assert_eq!(parse_list_a2a_limit(&obj(json!({}))).unwrap(), 100);
        assert_eq!(parse_list_a2a_limit(&obj(json!({"limit": 1}))).unwrap(), 1);
        assert_eq!(
            parse_list_a2a_limit(&obj(json!({"limit": 1000}))).unwrap(),
            1000
        );
    }

    #[test]
    fn parse_list_a2a_limit_rejects_invalid_values() {
        assert!(parse_list_a2a_limit(&obj(json!({"limit": 0}))).is_err());
        assert!(parse_list_a2a_limit(&obj(json!({"limit": 1001}))).is_err());
        assert!(parse_list_a2a_limit(&obj(json!({"limit": -1}))).is_err());
        assert!(parse_list_a2a_limit(&obj(json!({"limit": "100"}))).is_err());
    }

    #[test]
    fn parse_invoke_node_id_handles_auto_route_and_explicit_id() {
        assert_eq!(parse_invoke_node_id(&obj(json!({}))).unwrap(), "");
        assert_eq!(
            parse_invoke_node_id(&obj(json!({"node_id": ""}))).unwrap(),
            ""
        );
        assert_eq!(
            parse_invoke_node_id(&obj(json!({"node_id": "   "}))).unwrap(),
            ""
        );
        assert_eq!(
            parse_invoke_node_id(&obj(json!({"node_id": "node-1"}))).unwrap(),
            "node-1"
        );
        assert_eq!(
            parse_invoke_node_id(&obj(json!({"node_id": " node-2 "}))).unwrap(),
            "node-2"
        );
    }

    #[test]
    fn parse_invoke_node_id_rejects_non_string_values() {
        assert!(parse_invoke_node_id(&obj(json!({"node_id": 1}))).is_err());
        assert!(parse_invoke_node_id(&obj(json!({"node_id": {"x": 1}}))).is_err());
    }

    #[test]
    fn parse_invoke_arguments_defaults_and_rejects_non_objects() {
        assert_eq!(parse_invoke_arguments(&obj(json!({}))).unwrap(), json!({}));
        assert_eq!(
            parse_invoke_arguments(&obj(json!({"arguments": null}))).unwrap(),
            json!({})
        );
        assert_eq!(
            parse_invoke_arguments(&obj(json!({"arguments": {"k": "v"}}))).unwrap(),
            json!({"k": "v"})
        );
        assert!(parse_invoke_arguments(&obj(json!({"arguments": []}))).is_err());
        assert!(parse_invoke_arguments(&obj(json!({"arguments": "x"}))).is_err());
    }

    /// Input-validation failures must categorise as `validation_error`,
    /// not `unavailable` — an agent seeing `validation_error` knows to
    /// fix the payload, not retry. Regression-guards the whole point of
    /// the structured-error migration.
    #[test]
    fn input_validation_failures_map_to_validation_error_code() {
        let err = req(&obj(json!({})), "node_id").unwrap_err();
        assert_eq!(err.error_code(), "validation_error");

        let err = parse_invoke_node_id(&obj(json!({"node_id": 42}))).unwrap_err();
        assert_eq!(err.error_code(), "validation_error");

        let err = parse_invoke_arguments(&obj(json!({"arguments": "x"}))).unwrap_err();
        assert_eq!(err.error_code(), "validation_error");

        let err = parse_list_a2a_limit(&obj(json!({"limit": 0}))).unwrap_err();
        assert_eq!(err.error_code(), "validation_error");
    }

    // ── invoke_ability `trace` flag + PhaseReceipt envelope ─────────────────
    //
    // The `invoke_ability` handler itself requires a live DendriteBridge,
    // which we cannot stand up in a pure-Rust unit test. These tests
    // cover the two parts we *can* exercise in isolation: the `trace`
    // arg parser, and the receipt-envelope construction. Together they
    // pin every observable property of the trace feature short of the
    // actual RPC round-trip (which is covered by the manual smoke in
    // the plan file).

    #[test]
    fn parse_invoke_trace_flag_defaults_to_false_when_absent() {
        assert!(!parse_invoke_trace_flag(&obj(json!({}))).unwrap());
    }

    #[test]
    fn parse_invoke_trace_flag_accepts_explicit_boolean() {
        assert!(!parse_invoke_trace_flag(&obj(json!({"trace": false}))).unwrap());
        assert!(parse_invoke_trace_flag(&obj(json!({"trace": true}))).unwrap());
    }

    #[test]
    fn parse_invoke_trace_flag_treats_null_as_absent() {
        // A templated JSON caller that leaves `trace` unset sometimes
        // emits `{"trace": null}`; semantically "no trace", not an
        // error. Pinned so the parser does not conflate null with
        // "invalid type".
        assert!(!parse_invoke_trace_flag(&obj(json!({"trace": null}))).unwrap());
    }

    #[test]
    fn parse_invoke_trace_flag_rejects_non_boolean() {
        for bad in [
            json!({"trace": "true"}),
            json!({"trace": 1}),
            json!({"trace": 0}),
            json!({"trace": []}),
            json!({"trace": {"x": 1}}),
        ] {
            let err = parse_invoke_trace_flag(&obj(bad)).unwrap_err();
            assert_eq!(err.error_code(), "validation_error");
            assert!(
                err.message().contains("trace"),
                "validation message must name the offending field, got {}",
                err.message()
            );
            assert!(
                err.message().contains("boolean"),
                "validation message must name the expected type, got {}",
                err.message()
            );
        }
    }

    /// The serializable PhaseReceipt shape is a wire contract for every
    /// consumer of the `trace` envelope. Pin the keys and their types
    /// so a serde-derive change on the SDK side does not silently alter
    /// what federated telemetry pipelines see.
    #[test]
    fn phase_receipt_serialized_shape_is_pinned() {
        use easynet_axon::receipt::{Phase, PhaseReceipt};
        let receipt = PhaseReceipt::begin(Phase::Invoke, "tenant", "node-x", "ability-y")
            .finish_ok(None, None);
        let v = serialize_receipt(&receipt).unwrap();
        let obj = v.as_object().expect("receipt JSON must be an object");
        // Required keys — every consumer can rely on these.
        for key in [
            "phase",
            "status",
            "started_ms",
            "ended_ms",
            "duration_ms",
            "tenant_id",
            "node_id",
            "ability_id",
        ] {
            assert!(
                obj.contains_key(key),
                "receipt JSON missing key `{key}`, got {v}"
            );
        }
        assert_eq!(obj["phase"], "invoke");
        assert_eq!(obj["status"], "ok");
        assert_eq!(obj["tenant_id"], "tenant");
        assert_eq!(obj["node_id"], "node-x");
        assert_eq!(obj["ability_id"], "ability-y");
    }

    #[test]
    fn phase_receipt_duration_is_monotonic_non_negative() {
        // `duration_ms = ended_ms - started_ms` must be ≥ 0. A
        // pathological clock running backwards across `begin` /
        // `finish_ok` would expose itself here.
        use easynet_axon::receipt::{Phase, PhaseReceipt};
        let receipt = PhaseReceipt::begin(Phase::Invoke, "t", "n", "a").finish_ok(None, None);
        assert!(receipt.duration_ms >= 0, "duration must be non-negative");
        assert!(
            receipt.ended_ms >= receipt.started_ms,
            "ended must be ≥ started"
        );
        assert_eq!(
            receipt.duration_ms,
            receipt.ended_ms - receipt.started_ms,
            "duration must equal ended - started (no rounding slop)"
        );
    }

    /// `finish_err` must preserve the canonical `error_code` the
    /// non-trace path produces. A mismatch would mean an agent that
    /// branches on the top-level `error_code` sees one category while
    /// the receipt attributes the failure to a different one —
    /// exactly the "operator confusion" scenario `McpError`'s
    /// taxonomy exists to prevent.
    #[test]
    fn phase_receipt_error_code_agrees_with_mcp_error_code() {
        use easynet_axon::receipt::{Phase, PhaseReceipt};
        use easynet_axon::AxonError;
        // `AxonError` is not `Clone` (some variants wrap non-clone
        // payloads), so each case constructs the variant twice — once
        // for the receipt, once for the classifier. This is acceptable
        // in a test; it pins that the two paths agree on error
        // categorisation without relying on an AxonError-wide cloning
        // contract we don't control.
        let cases: &[(fn() -> AxonError, &str)] = &[
            (|| AxonError::Validation("bad".into()), "validation_error"),
            (|| AxonError::NotInstalled("inst-x".into()), "not_found"),
            (|| AxonError::DeadlineExceeded("slow".into()), "deadline_exceeded"),
            (|| AxonError::Bridge("peer closed".into()), "unavailable"),
        ];
        for (make_axon, expected_mcp_code) in cases {
            let receipt =
                PhaseReceipt::begin(Phase::Invoke, "t", "n", "a").finish_err(&make_axon());
            // The bridge: `From<AxonError> for McpError` selects by
            // variant tag, and `finish_err` records `AxonError::code()`
            // on the receipt. They must produce the same category for
            // a caller reading either surface.
            let mcp: McpError = make_axon().into();
            assert_eq!(
                mcp.error_code(),
                *expected_mcp_code,
                "mismatch for {expected_mcp_code}"
            );
            // The receipt must round-trip through serde cleanly with
            // an `error_code` field populated.
            let v = serialize_receipt(&receipt).unwrap();
            assert_eq!(v["status"], "error");
            assert!(
                v.get("error_code").and_then(Value::as_str).is_some(),
                "error receipt must serialize error_code, got {v}"
            );
        }
    }

    /// Simulate what the trace-on-failure branch assembles: an
    /// `McpError::to_payload()` object with a `trace` key merged in.
    /// The invariant is "base payload shape unchanged, plus `trace`" —
    /// a regression here would break agents that parse
    /// `{ok, error_code, error}` strictly.
    #[test]
    fn error_payload_gains_trace_key_without_losing_base_keys() {
        let err = McpError::Unavailable("peer gone".into());
        let mut payload = err.to_payload();
        payload.as_object_mut().unwrap().insert(
            "trace".into(),
            json!({"phase": "invoke", "status": "error"}),
        );
        // Base keys still present and unchanged.
        assert_eq!(payload["ok"], json!(false));
        assert_eq!(payload["error_code"], "unavailable");
        assert_eq!(payload["error"], "peer gone");
        // New telemetry key present.
        assert_eq!(payload["trace"]["phase"], "invoke");
        assert_eq!(payload["trace"]["status"], "error");
        // Exactly four keys — we haven't leaked anything extra.
        assert_eq!(payload.as_object().unwrap().len(), 4);
    }
}
