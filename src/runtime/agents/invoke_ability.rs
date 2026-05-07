// EasyNet CLI — <self>.invoke ability handler
// =================================================================
//
// File: src/runtime/agents/invoke_ability.rs
//
// Per-agent ability invocation entry point. Companion to
// `<self>.discover` (see discover_ability.rs): once an LLM has picked
// a candidate from the discovery ladder, it calls `<self>.invoke` to
// actually run the chosen ability — same wire shape regardless of
// whether the target is the calling agent itself or a peer on this
// device.
//
// Why per-agent registration
// --------------------------
// The ability-only model says every ability has an owner. Registering
// `easynet.invoke` once at the daemon level (the pre-RFC-001 shape)
// made the entry point look anonymous and obscured the "the caller is
// THIS agent" fact from the handler — needed for the `[access]`
// check, audit, and self/device scope filtering. Each agent gets its
// own `<self>.invoke` so the handler's closure carries the caller
// identity by construction.
//
// `easynet.invoke` survives as a thin compat alias that ignores the
// caller identity (treats it as "anyone"); new skills (the `delegate`
// SKILL.md) teach the owner-namespaced form so a fresh install lands
// on the canonical name.
//
// Wire shape
// ----------
//   args:  { target?: string, ability: string, args?: object }
//          - target  optional. Omit → calls the calling agent's
//                    own `<self>.<ability>`. Pass `"<peer>"` to
//                    cross-call.
//          - ability required. Bare verb (no `<owner>.` prefix);
//                    the prefix is constructed from `target` (or
//                    self if absent).
//          - args    forwarded as-is to the resolved handler.
//                    Default `{}` so common calls can omit it.
//
//   returns: { result: <handler return>, fulfilled_by, target,
//              ability, qualified_name, elapsed_ms }
//
// Errors are typed and returned as Err on the dispatch surface so
// the caller's tool-use loop sees `is_error: true`. Codes:
//   - `ability_not_found`     no handler for the qualified name
//   - `permission_denied`     caller scope < ability visibility
//   - `target_not_registered` target agent name not in registry
//   - `invalid_args`          args failed shape validation
//
// What this handler does NOT do
// -----------------------------
// * Federation routing — `target` must resolve to a local agent. A
//   future `<self>.invoke` extension can recognise federation-shaped
//   targets (e.g. `<user>:<agent>`) and route through the federation
//   layer once it ships; today targets are bare local agent names.
// * Streaming — invoke is the synchronous one-shot RPC surface. For
//   streaming consumption, callers reach `<agent>.chat` directly via
//   the dispatcher's stream mode (`InvokeStream` / Subscribe), as
//   established by chat_ability.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet.

use std::sync::Arc;
use std::time::Instant;

use serde_json::{json, Value};

use crate::core::ability_spec::Visibility;
use crate::registry::agents::AgentRegistry;
use crate::runtime::ability_dispatch::LocalAbilityRegistry;

/// Verb portion of the per-agent invoke ability. Combined with the
/// owning agent's name to form the wire-level `<agent>.invoke`.
pub const ABILITY_VERB: &str = "invoke";

/// Register `<agent_name>.invoke` on the registry.
///
/// `agent_registry_provider` returns the live agent registry at
/// handler-call time so a hot-added peer becomes a valid `target`
/// without re-registration.
///
/// `dispatch_registry_handle` is a `OnceLock` populated by the
/// daemon's boot path AFTER `Arc::new(reg)`. The handler reads
/// through it so it can dispatch into peer abilities that were
/// registered after this `register_for_agent` call ran (the agent
/// registration order is deterministic, but mission_ability /
/// per-agent fallback resolvers register later in the boot
/// sequence).
pub fn register_for_agent<F>(
    reg: &mut LocalAbilityRegistry,
    agent_name: String,
    agent_registry_provider: F,
    dispatch_registry_handle: Arc<std::sync::OnceLock<Arc<LocalAbilityRegistry>>>,
) where
    F: Fn() -> AgentRegistry + Send + Sync + 'static,
{
    use crate::runtime::ability_dispatch::OwnerKind;
    let provider: Arc<dyn Fn() -> AgentRegistry + Send + Sync> = Arc::new(agent_registry_provider);
    let qualified = format!("{agent_name}.{ABILITY_VERB}");
    let caller = agent_name.clone();
    reg.register_rpc_with_owner(
        &qualified,
        OwnerKind::Agent(agent_name),
        Arc::new(move |args: Value| dispatch(&caller, &provider, &dispatch_registry_handle, args)),
    );
}

/// Public per-call entry point. Validates args, applies access policy,
/// resolves the target handler, dispatches.
///
/// Exposed so the dynamic per-agent fallback resolver in
/// `chat_ability::register_dynamic_agent_fallback` can synthesise a
/// handler for a hot-added agent without re-running this module's
/// register_for_agent (which requires `&mut LocalAbilityRegistry`).
pub fn dispatch(
    caller: &str,
    agent_registry_provider: &Arc<dyn Fn() -> AgentRegistry + Send + Sync>,
    dispatch_registry_handle: &Arc<std::sync::OnceLock<Arc<LocalAbilityRegistry>>>,
    args: Value,
) -> anyhow::Result<Value> {
    let parsed = InvokeArgs::parse(&args)?;
    let target = parsed.target.as_deref().unwrap_or(caller);
    let qualified = format!("{target}.{}", parsed.ability);

    // Determine the caller's scope relative to the target ability.
    // self when the call stays inside one agent's own bundle,
    // device otherwise. Federation targets aren't recognised yet —
    // when they arrive they'll resolve into a `Visibility::Public`
    // caller scope here.
    let caller_scope = if target == caller {
        Visibility::Selfish
    } else {
        Visibility::Device
    };

    // Validate the target agent exists. Returning `target_not_registered`
    // here (rather than letting the registry miss surface as a
    // generic `ability_not_found`) gives the LLM a typed signal it
    // can act on — re-discover the registry, or surface a clearer
    // error to the user.
    let agents = agent_registry_provider();
    if target != caller && !agents.agents.contains_key(target) {
        // RFC-002 §5.2: when the target is not local but a CLI
        // forward invoker is registered AND the target shape is a
        // federation URA, route through `federation.forward_invoke`
        // before bailing. This keeps the local-only fast path
        // unchanged (no fed daemon = no remote dispatch attempts)
        // while letting peer-aware deployments transparently route
        // cross-device invokes.
        if crate::runtime::keyring::forward::is_federation_target(target) {
            if let Some(invoker) = crate::runtime::keyring::forward::forward_invoker() {
                if invoker.knows_target(target) {
                    // Emit the trace ID at stderr so an operator
                    // tailing the daemon log can correlate the
                    // forward hop with the originating HTTP request.
                    // Matches the rest of this file's "best-effort
                    // observability via eprintln!" convention — see
                    // audit_invoke. Empty request_id → emit nothing.
                    if !parsed.metadata.request_id.is_empty() {
                        eprintln!(
                            "[easynet trace] forward_invoke begin request_id={} target={} ability={}",
                            parsed.metadata.request_id, target, parsed.ability
                        );
                    }
                    let started_fwd = Instant::now();
                    let result = invoker.invoke(target, &parsed.ability, parsed.args.clone());
                    let elapsed_ms = started_fwd.elapsed().as_millis() as u64;
                    audit_invoke(
                        caller,
                        target,
                        &qualified,
                        &parsed.args,
                        result.as_ref().map_err(|e| e),
                        elapsed_ms,
                        &parsed.metadata,
                    );
                    let inner = result?;
                    return Ok(json!({
                        "result": inner,
                        "fulfilled_by": "federation_forward",
                        "target": target,
                        "ability": parsed.ability,
                        "qualified_name": qualified,
                        "elapsed_ms": elapsed_ms,
                    }));
                }
            }
        }
        anyhow::bail!(
            "target_not_registered: agent {target:?} is not registered on this device; \
             call <self>.discover first to see what's reachable"
        );
    }

    // Access policy check is a best-effort layer: it consults the
    // target ability's `[access]` block when the manifest is
    // discoverable on disk. Builtin self-bundle abilities (chat,
    // discover, invoke, run, …) have no on-disk manifest and skip
    // this check — they are inherently per-agent and trust the
    // dispatch layer.
    if let Some(policy) = lookup_access_policy(&agents, target, &parsed.ability) {
        if !policy.allows_caller(caller_scope) {
            anyhow::bail!(
                "permission_denied: {qualified} has visibility = {:?}; \
                 caller scope = {:?} is not permitted",
                policy.visibility,
                caller_scope
            );
        }
        // Fine-grained name check. Order matters: `deny_callers`
        // wins over `allow_callers`, and an empty `allow_callers`
        // means "no whitelist applied" rather than "deny everyone".
        if !policy.allows_caller_name(caller) {
            anyhow::bail!(
                "permission_denied: caller {caller:?} is not permitted to invoke \
                 {qualified} (deny_callers / allow_callers rule)"
            );
        }
    }

    // Resolve + dispatch in one wrapper so the audit log captures
    // both `ability_not_found` (handler resolution failed) and a
    // successful handler that returned an error from the executor.
    // Audit MUST be the last step before the typed error escapes —
    // otherwise an `ability_not_found` short-circuit would skip the
    // log line and an operator wouldn't see "agent X tried to call
    // missing ability Y", which is the most useful audit signal.
    let started = Instant::now();
    let dispatch_result: anyhow::Result<Value> = (|| {
        let registry = dispatch_registry_handle.get().ok_or_else(|| {
            anyhow::anyhow!(
                "internal_error: dispatch registry handle not yet set; \
                 this is a daemon boot ordering bug, not a caller-side issue"
            )
        })?;
        let handler = registry.resolve_rpc(&qualified).ok_or_else(|| {
            anyhow::anyhow!(
                "ability_not_found: no handler registered for {qualified}; \
                 call <self>.discover to see what's available"
            )
        })?;
        handler(parsed.args.clone())
    })();
    let elapsed_ms = started.elapsed().as_millis() as u64;

    // Best-effort audit log. Errors writing the audit line never fail
    // the invocation — the audit subsystem is observability, not
    // policy. The shape is one JSONL row per call so the file can be
    // streamed with `tail -F | jq`.
    audit_invoke(
        caller,
        target,
        &qualified,
        &parsed.args,
        dispatch_result.as_ref(),
        elapsed_ms,
        &parsed.metadata,
    );

    let inner = dispatch_result?;

    Ok(json!({
        "result": inner,
        "fulfilled_by": "registry_dispatch",
        "target": target,
        "ability": parsed.ability,
        "qualified_name": qualified,
        "elapsed_ms": elapsed_ms,
    }))
}

/// Append one invocation record to `~/.easynet/logs/ability-audit.jsonl`.
///
/// Why JSONL not a structured logger
/// ---------------------------------
/// One ability call → one line. A `tail -F | jq` pipeline is the
/// minimum viable observability surface; the file is also small
/// enough that an operator can `cat` it during debugging without
/// pulling in a log aggregator. Future PRs can add structured
/// shipping (OpenTelemetry, ndjson → S3) without changing the
/// canonical line shape — the JSONL representation is the contract.
///
/// Why best-effort
/// ---------------
/// An IO error writing the audit line MUST NOT fail the underlying
/// ability call. The user invoked `claude.weather`; the daemon
/// returned the weather; whether the audit line landed is an
/// operator concern, not a caller concern. We log to stderr on
/// failure so the daemon log surfaces audit drops, but propagate
/// nothing.
///
/// Privacy note
/// ------------
/// `args` and `result` are recorded by SHA-256 of their JSON-encoded
/// form, NOT verbatim. This keeps the audit line constant-size,
/// avoids leaking secrets that may have ridden through `args` into
/// a long-lived disk artifact, and still gives an operator the
/// equality test "this call's args matched the previous one's" for
/// replay-style debugging. Verbatim logging belongs in a separate
/// debug-only knob — not the default.
fn audit_invoke(
    caller: &str,
    target: &str,
    qualified: &str,
    args: &Value,
    dispatch_result: Result<&Value, &anyhow::Error>,
    elapsed_ms: u64,
    metadata: &InvokeMetadata,
) {
    use std::io::Write;
    let (outcome, error_message, result_hash) = match dispatch_result {
        Ok(v) => ("ok", None, Some(sha256_hex_of_json(v))),
        Err(e) => ("error", Some(format!("{e}")), None),
    };
    // Emit metadata fields only when populated. An empty `request_id`
    // is the legacy / non-HTTP path (e.g. an EAL step or a peer agent
    // calling directly without going through the backend); recording
    // a literal `""` would make grep noisier without adding signal.
    let request_id = if metadata.request_id.is_empty() {
        Value::Null
    } else {
        Value::String(metadata.request_id.clone())
    };
    let caller_uri = if metadata.caller_uri.is_empty() {
        Value::Null
    } else {
        Value::String(metadata.caller_uri.clone())
    };
    let line = json!({
        "ts": chrono::Utc::now().to_rfc3339(),
        "caller": caller,
        "target": target,
        "qualified": qualified,
        "args_sha256": sha256_hex_of_json(args),
        "result_sha256": result_hash,
        "outcome": outcome,
        "error": error_message,
        "elapsed_ms": elapsed_ms,
        "request_id": request_id,
        "caller_uri": caller_uri,
    });

    let path = audit_log_path();
    let parent_ok = path
        .parent()
        .map(|p| std::fs::create_dir_all(p).is_ok())
        .unwrap_or(false);
    if !parent_ok {
        eprintln!(
            "[easynet audit] failed to create parent dir for {}",
            path.display()
        );
        return;
    }
    let f = match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        Ok(f) => f,
        Err(e) => {
            eprintln!("[easynet audit] open {} failed: {e}", path.display());
            return;
        }
    };
    let mut writer = std::io::BufWriter::new(f);
    if let Err(e) = writeln!(writer, "{}", line) {
        eprintln!("[easynet audit] write {} failed: {e}", path.display());
    }
}

fn audit_log_path() -> std::path::PathBuf {
    crate::persistence::config::state_dir()
        .join("logs")
        .join("ability-audit.jsonl")
}

fn sha256_hex_of_json(v: &Value) -> String {
    use sha2::{Digest, Sha256};
    // Canonical-ish: serde_json's compact form. Two args that differ
    // only in whitespace round-trip to the same digest, which is
    // what we want for "did the same call run twice" debugging.
    let bytes = serde_json::to_vec(v).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let digest = hasher.finalize();
    let mut out = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write;
        let _ = write!(out, "{:02x}", b);
    }
    out
}

/// Look up the target ability's access policy by reading its on-disk
/// manifest. Returns `None` for any ability whose manifest is not
/// discoverable (builtin self-bundle abilities, abilities served by
/// the agent fallback resolver, etc.) — callers treat None as
/// "skip the check, trust the dispatch layer".
fn lookup_access_policy(
    agents: &AgentRegistry,
    target_agent: &str,
    bare_ability: &str,
) -> Option<crate::core::ability_spec::AccessPolicy> {
    let entry = agents.agents.get(target_agent)?;
    let manifests = crate::runtime::abilities::manifests_for(target_agent, entry);
    manifests
        .into_iter()
        .find(|m| m.name() == bare_ability)
        .map(|m| m.access())
}

/// Parsed invocation args. Pulled out so parse-time validation lives
/// in one place and the test cases can construct it directly.
///
/// `metadata` carries `_`-prefixed sidecar fields that backends pass
/// through the IPC frame but that DO NOT enter the inner ability call
/// (so they don't perturb args_digest / canonical bytes). The
/// EasyNet-backend cliipc adapter sets at minimum:
///
///   * `_caller_uri`       — original HTTP caller's URA
///   * `_request_id`       — `req-…` correlation token from the HTTP
///                            edge; flows into the audit row + the
///                            forward_invoke hop
///   * `_idempotency_key`  — RFC-001 idempotency key (M1 metadata)
///   * `_timeout_ms`       — RFC-001 timeout (M2 metadata)
///
/// Unknown `_`-prefixed fields are accepted silently and ignored —
/// this is the forward-compat slot for future metadata. Unknown
/// fields WITHOUT the underscore prefix still hard-fail per the
/// canonical schema (see `parse`).
#[derive(Debug, Clone, PartialEq)]
struct InvokeArgs {
    target: Option<String>,
    ability: String,
    args: Value,
    metadata: InvokeMetadata,
}

/// Subset of sidecar metadata the handler actually consults. Other
/// `_`-prefixed fields are accepted and dropped — keeping this struct
/// small avoids growing the parse code for fields nobody reads.
#[derive(Debug, Clone, Default, PartialEq)]
struct InvokeMetadata {
    /// Frontend-minted correlation ID (`req-` + 16 hex). Empty when
    /// the caller did not pass one. Not validated for charset here —
    /// the HTTP middleware already constrains the inbound shape; we
    /// just guard against empty / non-string.
    request_id: String,
    /// Original HTTP caller's URA. Recorded in the audit row so the
    /// device-side log can identify the operator behind a backend
    /// request without grepping the backend log.
    caller_uri: String,
}

impl InvokeArgs {
    fn parse(raw: &Value) -> anyhow::Result<Self> {
        let obj = raw
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("invalid_args: invoke args must be a JSON object"))?;

        let target = match obj.get("target") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) if s.is_empty() => {
                anyhow::bail!("invalid_args: `target` must not be the empty string")
            }
            Some(Value::String(s)) => Some(s.clone()),
            Some(other) => {
                anyhow::bail!("invalid_args: `target` must be a string (agent name); got {other}")
            }
        };

        let ability = obj
            .get("ability")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("invalid_args: `ability` (string) is required"))?
            .to_string();
        if ability.is_empty() {
            anyhow::bail!("invalid_args: `ability` must not be empty");
        }
        // The bare verb must NOT contain a dot — the caller passes
        // the verb separate from the target, and the handler
        // constructs `<target>.<verb>`. A dotted verb is almost
        // always a copy-paste mistake (the LLM passed
        // `"claude.weather"` as `ability` instead of splitting), so
        // catch it loud at parse rather than dispatching to a
        // never-registered `claude.claude.weather`.
        if ability.contains('.') {
            anyhow::bail!(
                "invalid_args: `ability` must be the bare verb only \
                 (no `<owner>.` prefix); got {ability:?}. Pass the \
                 owner via `target` instead."
            );
        }

        let args = match obj.get("args") {
            None | Some(Value::Null) => json!({}),
            Some(v) if v.is_object() => v.clone(),
            Some(other) => {
                anyhow::bail!("invalid_args: `args` must be a JSON object; got {other}")
            }
        };

        // Reject any unrecognised top-level fields. Pre-fix the
        // chat ability accepted unknown top-levels silently and
        // operators would write `arguments:` instead of `args:`
        // and wonder why the call did nothing — fail loud to make
        // typos surface here.
        //
        // Exception: `_`-prefixed fields are reserved as the
        // sidecar-metadata slot (the EasyNet backend adapter uses
        // `_caller_uri`, `_request_id`, `_idempotency_key`,
        // `_timeout_ms`). These flow through IPC for observability
        // and routing context but DO NOT enter args_digest or the
        // signed envelope. Unknown `_`-prefixed fields are
        // accepted-and-dropped so adding new metadata at a later
        // backend version doesn't require a CLI bump.
        const KNOWN: &[&str] = &["target", "ability", "args"];
        for key in obj.keys() {
            if KNOWN.contains(&key.as_str()) {
                continue;
            }
            if key.starts_with('_') {
                continue;
            }
            anyhow::bail!(
                "invalid_args: unknown field {key:?}; known: {:?} \
                 (sidecar metadata fields must start with `_`)",
                KNOWN
            );
        }

        // Pluck the two sidecar fields the handler actually reads.
        // Tolerate non-string values silently (caller passed garbage
        // → we drop it), since metadata is non-load-bearing.
        let metadata = InvokeMetadata {
            request_id: obj
                .get("_request_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            caller_uri: obj
                .get("_caller_uri")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        };

        Ok(InvokeArgs {
            target,
            ability,
            args,
            metadata,
        })
    }
}

// ── Discovery surfaces ────────────────────────────────────────

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["ability"],
        "additionalProperties": false,
        "properties": {
            "target": {
                "type": "string",
                "description": "Optional agent name. Omit to call your own \
                                ability of that name; pass `\"<peer>\"` to \
                                call a peer's. The full wire name \
                                `<target>.<ability>` is built for you."
            },
            "ability": {
                "type": "string",
                "description": "Bare verb (no owner prefix). e.g. `\"weather\"`, \
                                NOT `\"claude.weather\"`."
            },
            "args": {
                "type": "object",
                "description": "Arguments forwarded to the target ability's \
                                handler. Must satisfy the target ability's \
                                input_schema; the daemon validates before the \
                                executor runs.",
                "additionalProperties": true
            }
        }
    })
}

pub fn description() -> &'static str {
    "Invoke a discovered ability by name. Pair with <self>.discover \
     once you've picked a candidate from the discovery ladder. Returns \
     {result, fulfilled_by, target, ability, qualified_name, elapsed_ms}. \
     Typed errors: ability_not_found / permission_denied / \
     target_not_registered / invalid_args."
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ability_spec::{AbilityManifest, AccessPolicy, Visibility};
    use crate::registry::agents::{AgentEntry, AgentRegistry, AgentType};
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn obj_schema() -> Value {
        json!({"type": "object"})
    }

    fn workspace_with_manifest(
        agent_name: &str,
        verb: &str,
        manifest: AbilityManifest,
    ) -> (TempDir, AgentEntry) {
        let dir = TempDir::new().unwrap();
        let root = dir.path().to_path_buf();
        let abilities_dir = root.join("abilities");
        std::fs::create_dir_all(&abilities_dir).unwrap();
        std::fs::write(
            root.join("agent.toml"),
            format!("name = \"{agent_name}\"\nruntime = \"claude-code\"\n"),
        )
        .unwrap();
        std::fs::write(
            abilities_dir.join(format!("{verb}.ability.toml")),
            manifest.to_toml_string().unwrap().as_bytes(),
        )
        .unwrap();
        let mut entry = AgentEntry::new(AgentType::ClaudeCode, None);
        entry.root_path = Some(root);
        (dir, entry)
    }

    /// Build a registry + dispatch handle pair, register a `claude.invoke`
    /// against them, and seed the registry with the supplied target
    /// handlers. The returned closure dispatches `claude.invoke({...})`
    /// for you so each test stays focused on the assertion.
    fn fixture(
        target_handlers: &[(&str, crate::runtime::ability_dispatch::LocalRpcHandler)],
        agents: AgentRegistry,
    ) -> impl Fn(Value) -> anyhow::Result<Value> {
        let mut reg = LocalAbilityRegistry::new();
        for (name, h) in target_handlers {
            reg.register_rpc(*name, Arc::clone(h));
        }
        let handle: Arc<std::sync::OnceLock<Arc<LocalAbilityRegistry>>> =
            Arc::new(std::sync::OnceLock::new());
        let h_for_register = Arc::clone(&handle);
        let agents_clone = agents.clone();
        register_for_agent(
            &mut reg,
            "claude".into(),
            move || agents_clone.clone(),
            h_for_register,
        );
        let arc_reg = Arc::new(reg);
        // `OnceLock::set` returns `Err(Arc<...>)` on second-set, but
        // `LocalAbilityRegistry` doesn't implement `Debug`, so the
        // ergonomic `.expect("...")` won't compile. Match the Result
        // explicitly — first-set always succeeds in this test fixture.
        if handle.set(Arc::clone(&arc_reg)).is_err() {
            panic!("handle set once");
        }
        let dispatch = arc_reg
            .resolve_rpc("claude.invoke")
            .expect("invoke registered");
        move |args| dispatch(args)
    }

    #[test]
    fn parse_rejects_missing_ability() {
        let err = InvokeArgs::parse(&json!({})).unwrap_err();
        assert!(format!("{err}").contains("ability"));
    }

    #[test]
    fn parse_rejects_dotted_ability() {
        let err = InvokeArgs::parse(&json!({"ability": "claude.weather"})).unwrap_err();
        assert!(format!("{err}").contains("bare verb"));
    }

    #[test]
    fn parse_rejects_empty_ability() {
        let err = InvokeArgs::parse(&json!({"ability": ""})).unwrap_err();
        assert!(format!("{err}").contains("empty"));
    }

    #[test]
    fn parse_rejects_unknown_field() {
        // Catches the common typo of `arguments:` instead of `args:`.
        let err = InvokeArgs::parse(&json!({
            "ability": "weather",
            "arguments": {"x": 1}
        }))
        .unwrap_err();
        assert!(format!("{err}").contains("unknown field"));
    }

    #[test]
    fn parse_accepts_underscore_prefixed_sidecar_fields() {
        // Backend cliipc adapter ships `_caller_uri`, `_request_id`,
        // `_idempotency_key`, `_timeout_ms` as IPC-only sidecars.
        // The handler must accept them silently — they don't enter
        // args_digest / canonical bytes / signed envelope.
        let parsed = InvokeArgs::parse(&json!({
            "ability": "weather",
            "_caller_uri": "easynet:///r/silan.localhost/hub",
            "_request_id": "req-deadbeef00112233",
            "_idempotency_key": "idem-abc",
            "_timeout_ms": 5000,
            "_future_field_we_dont_read_yet": "value",
        }))
        .expect("sidecar fields must not be rejected");
        assert_eq!(parsed.metadata.request_id, "req-deadbeef00112233");
        assert_eq!(
            parsed.metadata.caller_uri,
            "easynet:///r/silan.localhost/hub",
        );
        // Sidecars MUST NOT bleed into the inner ability args; the
        // signed args_digest covers `args` exclusively.
        assert_eq!(parsed.args, json!({}));
    }

    #[test]
    fn parse_drops_unread_underscore_fields_silently() {
        // Forward-compat: a future backend version adding new
        // `_*` metadata must not require a CLI bump.
        let parsed = InvokeArgs::parse(&json!({
            "ability": "weather",
            "_brand_new_metadata_v3": {"nested": true},
        }))
        .unwrap();
        // Default empty strings, not panic / not error.
        assert_eq!(parsed.metadata.request_id, "");
        assert_eq!(parsed.metadata.caller_uri, "");
    }

    #[test]
    fn parse_tolerates_non_string_sidecar_values() {
        // Caller passed garbage (e.g. number where string expected).
        // Metadata is non-load-bearing — silently coerce to "" rather
        // than crash, since rejecting would mean a typo in operator
        // tooling could lock out the entire invoke surface.
        let parsed = InvokeArgs::parse(&json!({
            "ability": "weather",
            "_request_id": 12345,
            "_caller_uri": null,
        }))
        .unwrap();
        assert_eq!(parsed.metadata.request_id, "");
        assert_eq!(parsed.metadata.caller_uri, "");
    }

    #[test]
    fn parse_still_rejects_unknown_field_without_underscore_prefix() {
        // Underscore-prefix carve-out must not weaken the typo guard.
        let err = InvokeArgs::parse(&json!({
            "ability": "weather",
            "arguments": {"x": 1},
        }))
        .unwrap_err();
        assert!(format!("{err}").contains("unknown field"));
        // Hint should mention the underscore convention so a future
        // operator reading the error knows the proper slot.
        assert!(
            format!("{err}").contains("_"),
            "error should hint at the `_` prefix convention; got: {err}"
        );
    }

    #[test]
    fn parse_defaults_args_to_empty_object_when_absent() {
        let parsed = InvokeArgs::parse(&json!({"ability": "discover"})).unwrap();
        assert_eq!(parsed.args, json!({}));
        assert_eq!(parsed.target, None);
    }

    #[test]
    fn sha256_hex_of_json_is_deterministic_and_whitespace_invariant() {
        // The audit log uses this digest for `args_sha256` /
        // `result_sha256`. Two calls with logically equal args must
        // produce the same digest so an operator's "find duplicate
        // calls" grep works.
        let a = sha256_hex_of_json(&json!({"location": "Beijing"}));
        let b = sha256_hex_of_json(&json!({ "location" : "Beijing" }));
        assert_eq!(a, b);
        // 64-char lowercase hex
        assert_eq!(a.len(), 64);
        assert!(a
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()));
    }

    #[test]
    fn sha256_hex_of_json_distinguishes_different_payloads() {
        let a = sha256_hex_of_json(&json!({"location": "Beijing"}));
        let b = sha256_hex_of_json(&json!({"location": "Singapore"}));
        assert_ne!(a, b);
    }

    #[test]
    fn invoke_self_ability_dispatches_through_registry() {
        // Caller is claude; target unspecified → resolves to claude.weather.
        let weather: crate::runtime::ability_dispatch::LocalRpcHandler = Arc::new(|args: Value| {
            let loc = args.get("location").and_then(Value::as_str).unwrap_or("");
            Ok(json!({"summary": format!("{loc}: clear 18C")}))
        });
        let dispatch = fixture(&[("claude.weather", weather)], AgentRegistry::default());
        let resp = dispatch(json!({
            "ability": "weather",
            "args": {"location": "Beijing"}
        }))
        .unwrap();
        assert_eq!(resp["target"], "claude");
        assert_eq!(resp["qualified_name"], "claude.weather");
        assert_eq!(resp["result"]["summary"], json!("Beijing: clear 18C"));
        assert!(resp["elapsed_ms"].is_u64());
    }

    #[test]
    fn invoke_unknown_ability_returns_typed_not_found() {
        let dispatch = fixture(&[], AgentRegistry::default());
        let err = dispatch(json!({"ability": "nope"})).unwrap_err();
        assert!(format!("{err}").contains("ability_not_found"));
    }

    #[test]
    fn invoke_unknown_target_returns_typed_target_not_registered() {
        let dispatch = fixture(&[], AgentRegistry::default());
        let err = dispatch(json!({
            "target": "phantom",
            "ability": "weather"
        }))
        .unwrap_err();
        assert!(format!("{err}").contains("target_not_registered"));
    }

    #[test]
    fn invoke_peer_with_self_visibility_is_denied() {
        // codex publishes a private helper; claude (a peer) tries to
        // invoke it. The access policy check must reject before the
        // handler runs.
        let private = AbilityManifest::new("internal", "private", obj_schema())
            .unwrap()
            .with_access(AccessPolicy {
                visibility: Visibility::Selfish,
                ..Default::default()
            })
            .unwrap();
        let (_dir, codex_entry) = workspace_with_manifest("codex", "internal", private);
        let mut agents = AgentRegistry::default();
        agents.agents.insert("codex".into(), codex_entry);

        // The handler is registered (the dispatch layer would reach
        // it) — the access check has to be the gate, not the
        // registry miss.
        let h: crate::runtime::ability_dispatch::LocalRpcHandler =
            Arc::new(|_| Ok(json!("should not run")));
        let dispatch = fixture(&[("codex.internal", h)], agents);
        let err = dispatch(json!({
            "target": "codex",
            "ability": "internal"
        }))
        .unwrap_err();
        assert!(format!("{err}").contains("permission_denied"));
    }

    #[test]
    fn invoke_peer_with_device_visibility_is_allowed() {
        let weather = AbilityManifest::new("weather", "Fetch weather", obj_schema())
            .unwrap()
            .with_access(AccessPolicy {
                visibility: Visibility::Device,
                ..Default::default()
            })
            .unwrap();
        let (_dir, codex_entry) = workspace_with_manifest("codex", "weather", weather);
        let mut agents = AgentRegistry::default();
        agents.agents.insert("codex".into(), codex_entry);

        let h: crate::runtime::ability_dispatch::LocalRpcHandler =
            Arc::new(|_| Ok(json!({"summary": "Beijing: 18C"})));
        let dispatch = fixture(&[("codex.weather", h)], agents);
        let resp = dispatch(json!({
            "target": "codex",
            "ability": "weather"
        }))
        .unwrap();
        assert_eq!(resp["qualified_name"], "codex.weather");
    }

    #[test]
    fn invoke_peer_in_deny_callers_is_rejected() {
        // Fine-grained per-caller policy: even a `device`-visible
        // ability rejects a specific caller listed in `deny_callers`.
        // Pin the wire-message form ("permission_denied: caller …")
        // so a script that filters on it stays stable.
        let weather = AbilityManifest::new("weather", "Fetch weather", obj_schema())
            .unwrap()
            .with_access(AccessPolicy {
                visibility: Visibility::Device,
                deny_callers: Some(vec!["claude".into()]),
                allow_callers: None,
            })
            .unwrap();
        let (_dir, codex_entry) = workspace_with_manifest("codex", "weather", weather);
        let mut agents = AgentRegistry::default();
        agents.agents.insert("codex".into(), codex_entry);

        let h: crate::runtime::ability_dispatch::LocalRpcHandler =
            Arc::new(|_| Ok(json!("should not run")));
        let dispatch = fixture(&[("codex.weather", h)], agents);
        let err = dispatch(json!({
            "target": "codex",
            "ability": "weather"
        }))
        .unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("permission_denied"), "got {msg}");
        assert!(msg.contains("claude"), "got {msg}");
    }

    #[test]
    fn invoke_peer_outside_allow_callers_is_rejected() {
        let weather = AbilityManifest::new("weather", "Fetch weather", obj_schema())
            .unwrap()
            .with_access(AccessPolicy {
                visibility: Visibility::Device,
                allow_callers: Some(vec!["alice".into(), "bob".into()]),
                deny_callers: None,
            })
            .unwrap();
        let (_dir, codex_entry) = workspace_with_manifest("codex", "weather", weather);
        let mut agents = AgentRegistry::default();
        agents.agents.insert("codex".into(), codex_entry);

        let h: crate::runtime::ability_dispatch::LocalRpcHandler =
            Arc::new(|_| Ok(json!("should not run")));
        let dispatch = fixture(&[("codex.weather", h)], agents);
        let err = dispatch(json!({
            "target": "codex",
            "ability": "weather"
        }))
        .unwrap_err();
        assert!(format!("{err}").contains("permission_denied"));
    }

    #[test]
    fn invoke_peer_inside_allow_callers_is_admitted() {
        let weather = AbilityManifest::new("weather", "Fetch weather", obj_schema())
            .unwrap()
            .with_access(AccessPolicy {
                visibility: Visibility::Device,
                // claude is the caller in the test fixture (the
                // `claude.invoke` registration). Putting it in the
                // allow list should let the call through.
                allow_callers: Some(vec!["claude".into()]),
                deny_callers: None,
            })
            .unwrap();
        let (_dir, codex_entry) = workspace_with_manifest("codex", "weather", weather);
        let mut agents = AgentRegistry::default();
        agents.agents.insert("codex".into(), codex_entry);

        let h: crate::runtime::ability_dispatch::LocalRpcHandler =
            Arc::new(|_| Ok(json!({"summary": "Beijing"})));
        let dispatch = fixture(&[("codex.weather", h)], agents);
        let resp = dispatch(json!({
            "target": "codex",
            "ability": "weather"
        }))
        .unwrap();
        assert_eq!(resp["qualified_name"], "codex.weather");
    }

    #[test]
    fn invoke_target_without_manifest_skips_access_check() {
        // claude's own `chat` is a builtin handler with no on-disk
        // manifest in this fixture. The invoke handler must NOT
        // reject the call just because lookup_access_policy returns
        // None — that path covers all builtin self-bundle abilities.
        let chat: crate::runtime::ability_dispatch::LocalRpcHandler =
            Arc::new(|_| Ok(json!({"reply": "hi"})));
        let dispatch = fixture(&[("claude.chat", chat)], AgentRegistry::default());
        let resp = dispatch(json!({"ability": "chat"})).unwrap();
        assert_eq!(resp["result"]["reply"], "hi");
    }

    #[test]
    fn invoke_propagates_handler_error() {
        let failing: crate::runtime::ability_dispatch::LocalRpcHandler =
            Arc::new(|_| anyhow::bail!("upstream_failed: wttr.in returned 503"));
        let dispatch = fixture(&[("claude.weather", failing)], AgentRegistry::default());
        let err = dispatch(json!({"ability": "weather"})).unwrap_err();
        assert!(format!("{err}").contains("upstream_failed"));
    }

    // ── EXP-3: federation forward_invoke routing ────────────────────
    //
    // These tests install a fake CliForwardInvoker via the test sink
    // and verify the dispatch layer routes federation-shaped targets
    // through it instead of returning target_not_registered.

    use crate::runtime::keyring::forward as fwd;

    #[test]
    fn forward_invoke_routes_federation_target_through_invoker() {
        let _g = fwd::test_lock();
        fwd::set_test_knower(|uri| uri.starts_with("easynet:///r/"));
        fwd::set_test_router(|target, ability, args| {
            assert_eq!(target, "easynet:///r/exp-realm/device/alice-node");
            assert_eq!(ability, "ping");
            assert_eq!(args["from"], "silan");
            Ok(json!({"echo": "from-alice", "ability": ability}))
        });

        let dispatch = fixture(&[], AgentRegistry::default());
        let resp = dispatch(json!({
            "target": "easynet:///r/exp-realm/device/alice-node",
            "ability": "ping",
            "args": {"from": "silan"}
        }))
        .unwrap();
        // Forward path returns the inner result Value verbatim — the
        // dispatch layer wraps that with the standard envelope.
        assert_eq!(resp["target"], "easynet:///r/exp-realm/device/alice-node");
        assert_eq!(
            resp["qualified_name"],
            "easynet:///r/exp-realm/device/alice-node.ping"
        );
        assert_eq!(resp["result"]["echo"], "from-alice");

        fwd::clear_test_routing();
    }

    #[test]
    fn forward_invoke_falls_through_when_invoker_does_not_know_target() {
        let _g = fwd::test_lock();
        fwd::set_test_knower(|_uri| false); // invoker rejects every target
        fwd::set_test_router(|_t, _a, _x| panic!("router must not be called"));

        let dispatch = fixture(&[], AgentRegistry::default());
        let err = dispatch(json!({
            "target": "easynet:///r/exp-realm/device/unknown",
            "ability": "ping",
        }))
        .unwrap_err();
        assert!(format!("{err}").contains("target_not_registered"));

        fwd::clear_test_routing();
    }

    #[test]
    fn forward_invoke_propagates_typed_remote_error() {
        let _g = fwd::test_lock();
        fwd::set_test_knower(|uri| uri.starts_with("easynet:///r/"));
        fwd::set_test_router(|_t, _a, _x| {
            Err(anyhow::anyhow!("AXON_TARGET_OFFLINE: peer unreachable"))
        });

        let dispatch = fixture(&[], AgentRegistry::default());
        let err = dispatch(json!({
            "target": "easynet:///r/exp-realm/device/down",
            "ability": "ping"
        }))
        .unwrap_err();
        assert!(format!("{err}").contains("AXON_TARGET_OFFLINE"));

        fwd::clear_test_routing();
    }

    #[test]
    fn forward_invoke_skipped_for_non_federation_targets() {
        // Bare names (legacy local target) MUST stay on the
        // target_not_registered path even if the router claims to
        // know them, because is_federation_target rejects bare names.
        let _g = fwd::test_lock();
        fwd::set_test_knower(|_uri| true);
        fwd::set_test_router(|_t, _a, _x| {
            panic!("router must not be called for non-federation targets")
        });

        let dispatch = fixture(&[], AgentRegistry::default());
        let err = dispatch(json!({
            "target": "phantom-bare-name",
            "ability": "ping"
        }))
        .unwrap_err();
        assert!(format!("{err}").contains("target_not_registered"));

        fwd::clear_test_routing();
    }

    #[test]
    fn invoke_unset_dispatch_handle_is_internal_error() {
        // Direct construction so we skip the fixture's `handle.set()`
        // call. This pins the daemon-boot-ordering check.
        let mut reg = LocalAbilityRegistry::new();
        let handle: Arc<std::sync::OnceLock<Arc<LocalAbilityRegistry>>> =
            Arc::new(std::sync::OnceLock::new());
        register_for_agent(
            &mut reg,
            "claude".into(),
            AgentRegistry::default,
            Arc::clone(&handle),
        );
        let dispatch = reg.resolve_rpc("claude.invoke").unwrap();
        let err = dispatch(json!({"ability": "discover"})).unwrap_err();
        assert!(format!("{err}").contains("internal_error"));
        let _ = PathBuf::from("/keep-imports-happy"); // tempfile import lives in fixture
    }
}
