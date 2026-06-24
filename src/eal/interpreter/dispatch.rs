// Step dispatch plane: agent-aware dispatcher, daemon and
// remote forward_invoke routes (split from interpreter.rs,
// T4.4 / F-021; bodies are move-only).

// EasyNet CLI — EAL Interpreter
// =============================
//
// File: src/eal/interpreter.rs
// Description: Client-side execution engine for Mission IR v2 (temporary — target: MissionControl v2).
//
// Execution Model:
//   Phases execute sequentially (data-flow barriers between them).
//   Steps within a phase execute in parallel via rayon work-stealing threadpool.
//   When a dispatcher cannot be cloned across worker threads, falls back to sequential.
//
// Core Capabilities:
//   1. True parallel dispatch — rayon::scope + clone_for_thread() per step.
//   2. Structured ExecutionTrace — per-step audit log with timestamps, result hashes, retry history.
//   3. Retry with exponential backoff — delay = min(base * 2^attempt, max) + deterministic jitter.
//   4. Cross-phase data flow — results captured in HashMap, substituted into downstream input_refs.
//   5. Lock-free result collection — crossbeam SegQueue eliminates collector contention.
//   6. Connection pool reuse — BridgePool with adaptive sizing based on CPU cores.
//
// Dispatch Abstraction:
//   `trait StepDispatcher` decouples execution from transport. Production uses
//   AgentAwareDispatcher; tests inject MockDispatcher or a non-cloneable dispatcher.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::Arc;

use serde_json::Value;

use super::*;

/// without changing the EAL surface.
fn dispatch_remote_via_forward_invoke(
    tenant: &str,
    node_id: &str,
    ability_name: &str,
    arguments: &Value,
) -> Result<Value, EalError> {
    #[cfg(feature = "axon-pb")]
    {
        let trimmed = node_id.trim();

        // Local short-circuit: `local`, empty, or this device's own
        // node id all dispatch through the local daemon's control
        // socket, the same surface every other in-process invocation
        // uses. Skip the forward_invoke envelope entirely — the
        // self-target shortcut on the daemon side covers a different
        // case (canonical self URI), not the keyword `local`.
        let self_node = crate::persistence::config::load_credentials()
            .ok()
            .map(|c| c.node_id);
        let is_local = trimmed.is_empty()
            || trimmed.eq_ignore_ascii_case("local")
            || self_node
                .as_deref()
                .is_some_and(|n| !n.is_empty() && trimmed == n);
        if is_local {
            return crate::support::local_invoke::invoke_local_ability(
                ability_name,
                arguments.clone(),
            )
            .map_err(|e| {
                EalError::Unavailable(format!("invoke_local_ability {ability_name} (local): {e}"))
            });
        }

        let target_ura = if crate::ura::parse_ura(trimmed).is_ok() {
            crate::services::invocation_transport::federation_invoke::parse_node_ura(trimmed)
                .map_err(|e| EalError::Validation(format!("parse target URA: {e}")))?
        } else if !tenant.is_empty() {
            crate::ura::device_ura(tenant, trimmed)
        } else {
            return Err(EalError::Validation(format!(
                "cannot resolve EAL device target {trimmed:?}: no tenant in scope; \
                 pass a canonical `easynet:///r/<realm>/device/<id>` URA"
            )));
        };

        let caller_ura = crate::persistence::config::load_credentials()
            .ok()
            .filter(|c| !c.realm.trim().is_empty() && !c.node_id.trim().is_empty())
            .map(|c| crate::ura::device_ura(c.realm.trim(), c.node_id.trim()));
        let target_call = crate::services::invocation_transport::federation_invoke::RemoteAbilityInvocationTarget::for_target_owned_selector(
            &target_ura,
            ability_name,
        )
        .map_err(|e| EalError::Validation(format!("derive Ability URA for {ability_name}: {e}")))?;
        crate::services::invocation_transport::federation_invoke::invoke_via_federation_forward_target(
            &target_call,
            arguments.clone(),
            caller_ura.as_deref(),
        )
        .map_err(|e| {
            EalError::Unavailable(format!("forward_invoke {ability_name} → {target_ura}: {e}"))
        })
    }
    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = (tenant, node_id, ability_name, arguments);
        Err(EalError::Unavailable(
            "EAL device-targeted dispatch requires the `axon-pb` feature; \
             rebuild with `--features axon-pb` (production builds always do)."
                .to_string(),
        ))
    }
}

/// Per-mission-run dispatch context.
///
/// `trace_id` is the mission run's id (minted once per run in
/// `execute_with_dispatcher`, equal to `ExecutionTrace::mission_id`).
/// Dispatchers that lower steps onto the Axon Invocation surface stamp
/// it on every envelope's `trace_id` operational-metadata field, so the
/// daemon ledger groups one mission run's invocations and
/// `easynet invocation trace <mission_id>` reconstructs the run's
/// receipt graph. It is not an Invocation axiom field — causal
/// placement stays in `causal_context`.
pub struct AgentAwareDispatcher {
    registry: Arc<crate::registry::agents::AgentRegistry>,
}

impl AgentAwareDispatcher {
    pub fn new(_endpoint: &str, _timeout_ms: u64) -> Self {
        let registry = load_registry_or_warn();
        Self {
            registry: Arc::new(registry),
        }
    }
}

/// Load the agent registry, logging a visible warning if the load fails.
///
/// Previously this was `load_agents().unwrap_or_default()`, which turned
/// "registry file is corrupt / home dir missing / permission denied"
/// into "you have no registered agents", so an EAL member-call like
/// `claude.chat(...)` would fail downstream with `agent '…' not found
/// in registry` — a classic false-negative that sends operators hunting
/// for a mis-registered agent when the real problem is upstream.
///
/// We still want a usable dispatcher when no agents are registered
/// (that is a legitimate first-run state), so we return an empty
/// registry on failure *after* logging. The distinction between
/// "empty by design" and "empty by failure" is preserved in operator-
/// visible logs rather than hidden from the caller.
fn load_registry_or_warn() -> crate::registry::agents::AgentRegistry {
    match crate::registry::agents::load_agents() {
        Ok(r) => r,
        Err(e) => {
            eprintln!(
                "[easynet eal] warning: agent registry load failed ({e}); \
                 dispatching with an empty registry. Any agent-target call \
                 will fail with `not_found` until the registry is repaired."
            );
            crate::registry::agents::AgentRegistry::default()
        }
    }
}

impl StepDispatcher for AgentAwareDispatcher {
    fn dispatch(
        &self,
        run: RunContext<'_>,
        target: &IrTarget,
        ability: &AbilityName,
        arguments: &Value,
        timeout_ms: Option<u64>,
        causal_parents: &[Value],
    ) -> Result<StepDispatchOutcome, EalError> {
        match target {
            IrTarget::Agent(agent_id) => dispatch_to_agent(
                &self.registry,
                agent_id,
                ability,
                arguments,
                causal_parents,
                run.trace_id,
            ),
            IrTarget::Device { node_id } => {
                let _ = timeout_ms;
                dispatch_remote_via_forward_invoke(run.tenant, node_id, ability.as_str(), arguments)
                    .map(Into::into)
            }
        }
    }

    fn clone_for_thread(&self) -> Result<Box<dyn StepDispatcher + Send>, EalError> {
        Ok(Box::new(AgentAwareDispatcher {
            registry: Arc::clone(&self.registry),
        }))
    }
}

/// Shared agent dispatch logic used by AgentAwareDispatcher.
pub(super) fn dispatch_to_agent(
    registry: &crate::registry::agents::AgentRegistry,
    agent_id: &crate::core::agent_id::AgentId,
    ability: &AbilityName,
    arguments: &Value,
    causal_parents: &[Value],
    trace_id: &str,
) -> Result<StepDispatchOutcome, EalError> {
    // Registry is keyed by string today (see Step 4
    // follow-up: registry will be keyed by AgentId itself).
    // For now, look up by the canonical Display form.
    let key = agent_id.to_string();
    let entry = registry
        .agents
        .get(&key)
        .or_else(|| {
            // Backwards-compat: registry files written
            // before the migration may use the bare name
            // form (`"claude"` instead of `"default/claude"`).
            // Fall back to the bare name when the agent
            // is in the default tenant.
            if agent_id.tenant == crate::core::agent_id::DEFAULT_TENANT {
                registry.agents.get(&agent_id.name)
            } else {
                None
            }
        })
        // Missing agent in registry is `not_found`, not `unavailable` —
        // the caller's identifier doesn't resolve and a retry of the
        // same id will not help.
        .ok_or_else(|| EalError::NotFound(format!("agent '{key}' not found in registry")))?;

    // Fast path: if the target ability has an `[exec]` binding in its
    // on-disk manifest, run the executor directly and skip spawning
    // the LLM. This is the EAL counterpart of the dispatcher's
    // shell-exec short-circuit in `chat_ability::build_agent_ability_handler`
    // — both paths converge on `manifests_for(...) → run_shell_exec(...)`
    // for a deterministic ability. Without this branch, EAL's
    // `agent.ability(...)` syntax would always go through the chat
    // CLI even when the manifest pinned a concrete argv, and a
    // weather lookup that should take 200 ms would burn 30 s of LLM
    // tool-search latency.
    let bare_ability = ability.as_str();
    let manifest_match = crate::runtime::abilities::manifests_for(&agent_id.name, entry)
        .into_iter()
        .find(|m| m.name() == bare_ability);
    if let Some(manifest) = manifest_match {
        if let Some(exec) = manifest.exec() {
            // Lower the step onto the daemon's Axon Invocation surface
            // first: the daemon executes the same `[exec]` manifest but
            // the call becomes a ledger-recorded seven-tuple invocation
            // (caller/callee/ability/subject/nonce/causal_context/args)
            // whose receipt anchors downstream steps reference as their
            // causal parents. The in-process executor below remains the
            // offline path (daemon down / ability not registered yet)
            // and is recorded with `invocation: None` — no fabricated
            // receipts.
            //
            // `adapter_fault: "drop_causal_context"` is the phase-1
            // benchmark fault-injection knob (Easynet-Semantic-Operator-
            // Integration negative missions): it models a binding that
            // fails to preserve causal placement, so the invocation is
            // emitted with an empty causal_context while the argument
            // still reaches the adapter.
            let display_name = format!("{}.{}", agent_id.name, bare_ability);
            let timeout = manifest
                .timeout_seconds()
                .map(std::time::Duration::from_secs);
            let effective_parents: &[Value] =
                if arguments.get("adapter_fault").and_then(Value::as_str)
                    == Some("drop_causal_context")
                {
                    &[]
                } else {
                    causal_parents
                };
            match crate::support::local_invoke::invoke_local_ability_with_invocation_meta(
                bare_ability,
                arguments.clone(),
                None,
                effective_parents,
                timeout,
                Some(trace_id),
                Some(&agent_id.name),
            ) {
                Ok((value, meta)) => {
                    return Ok(StepDispatchOutcome {
                        value,
                        invocation: Some(meta),
                    });
                }
                Err(err) => {
                    use crate::support::local_invoke::{
                        classify_invoke_error, LocalInvokeErrorKind,
                    };
                    match classify_invoke_error(&err) {
                        // Nothing ran (daemon down / ability not
                        // registered there) — the in-process executor
                        // below may legitimately take over.
                        LocalInvokeErrorKind::DaemonOffline
                        | LocalInvokeErrorKind::AbilityUnregistered => {}
                        // The daemon ran the same manifest and failed for
                        // real; re-running in-process would double-execute
                        // a side-effecting ability to mask a true error.
                        LocalInvokeErrorKind::Failed => {
                            return Err(EalError::Unavailable(format!(
                                "daemon invoke {display_name}: {err}"
                            )));
                        }
                    }
                }
            }
            return (match exec {
                crate::core::ability_spec::AbilityExec::Shell(spec) => {
                    crate::runtime::agents::shell_executor::run_shell_exec(spec, arguments, timeout)
                        .map_err(|e| EalError::Unavailable(format!("shell exec: {e}")))
                }
                crate::core::ability_spec::AbilityExec::Http(spec) => {
                    crate::runtime::agents::http_executor::run_http_exec(spec, arguments, timeout)
                        .map_err(|e| EalError::Unavailable(format!("http exec: {e}")))
                }
                crate::core::ability_spec::AbilityExec::Eal(spec) => {
                    crate::runtime::agents::eal_executor::run_eal_exec(spec, arguments, timeout)
                        .map_err(|e| EalError::Unavailable(format!("eal exec: {e}")))
                }
                crate::core::ability_spec::AbilityExec::Mcp(spec) => {
                    let _ = timeout;
                    crate::runtime::agents::mcp_executor::run_mcp_exec(spec, arguments)
                        .map_err(|e| EalError::Unavailable(format!("mcp exec: {e}")))
                }
                crate::core::ability_spec::AbilityExec::HostStream(_) => {
                    // host_stream is a server-stream executor; an EAL step
                    // is a unary child invocation and cannot carry its
                    // many-frame output. Such an ability registers as
                    // stream-mode and is reached via the stream dispatch
                    // path, never here — surface a clear error if an EAL
                    // program nonetheless targets one as a unary step.
                    Err(EalError::Unavailable(
                        "host_stream exec is server-stream; it cannot run as a \
                         unary EAL step — call it as a stream invocation"
                            .to_string(),
                    ))
                }
            })
            .map(Into::into);
        }
    }

    // `<agent>.chat` is special: when an EAL mission desugars
    // `easynet agent send` it wants the driver's live stderr
    // timeline in the *current* CLI process. Routing chat through
    // the daemon's unary Invoke RPC would hide that live output in
    // the daemon process and reduce the caller to a final snapshot.
    // Keep chat local by reusing the daemon handler's own parsing /
    // context / resume logic directly in-process.
    if bare_ability == crate::runtime::agents::chat_ability::ABILITY_VERB {
        return crate::runtime::agents::chat_ability::invoke_direct_with_progress(
            &agent_id.name,
            entry,
            &[],
            arguments.clone(),
            None,
        )
        .map(Into::into)
        .map_err(|e| EalError::Unavailable(format!("agent chat: {e}")));
    }

    // Second fast path: try the local daemon's ability registry over
    // the control socket. The daemon's public function name is the
    // owner-local ability (`echo`, `discover`, `invoke`, ...); the
    // hosted agent name is passed separately as callee/delegation
    // context so Axon can bind the call to the canonical owner Ability
    // URA. Keeping `<agent>.<verb>` out of the wire function_name is
    // the convergence point with DescriptorBoundEnvelope dispatch:
    // display names are not dispatch keys.
    //
    // We do the IPC round-trip here only when the manifest path
    // above did NOT short-circuit. Failure modes are explicit:
    //
    //   * daemon down → propagate as Unavailable (caller may retry).
    //   * daemon returned `ability_not_found` → fall through to the
    //     chat path (preserves the legacy "manifest declares an
    //     ability but only chat can fulfil it" behaviour).
    //   * daemon returned any other typed error → propagate.
    let display_name = format!("{}.{}", agent_id.name, ability.as_str());
    match try_dispatch_via_daemon(&agent_id.name, ability.as_str(), arguments) {
        DaemonDispatch::Result(value) => return Ok(value.into()),
        DaemonDispatch::AbilityNotFound => { /* fall through to chat */ }
        DaemonDispatch::DaemonDown(reason) => {
            return Err(EalError::Unavailable(format!(
                "daemon {display_name}: {reason}"
            )));
        }
        DaemonDispatch::Error(reason) => {
            return Err(EalError::Unavailable(format!(
                "daemon {display_name}: {reason}"
            )));
        }
    }

    let prompt = build_agent_prompt(ability.as_str(), arguments);

    // Agent CLI dispatch failures (process spawn, IO, model error) are
    // transport-class — `unavailable` is the right bucket so the
    // interpreter's retry policy can fire when configured.
    //
    // IMPORTANT: pass the BARE agent name to send_to_agent, not the
    // namespaced AgentId form (`default/claude`). Downstream
    // workspace::ensure_workspace runs the registry name validator
    // against this string, and the validator legitimately rejects
    // anything containing `/`. The namespaced form is the registry
    // *lookup* identity above; the *runtime* identity is the bare
    // name. Using the wrong one here was the root cause of every
    // EAL→agent dispatch failing with
    // "workspace provisioning failed: agent.toml: name = "default/claude"
    //  must contain only lowercase ASCII letters, …" before the
    // CLI could even spawn.
    let response =
        crate::runtime::dispatch::send_to_agent(&agent_id.name, entry, &prompt, None, None)
            .map_err(|e| EalError::Unavailable(format!("agent dispatch: {e}")))?;

    Ok(serde_json::json!({
        "ok": true,
        "agent": response.agent,
        "output": response.content,
        "model": response.model,
        "duration_ms": response.duration_ms,
    })
    .into())
}

/// Outcome of attempting to dispatch a `<agent>.<verb>` call through
/// the local daemon's control socket.
///
/// Why a custom enum (rather than `Result<Option<Value>, ...>`)
/// -----------------------------------------------------------
/// `dispatch_to_agent` needs to make three decisions on the result:
///   1. Got a value → return it directly.
///   2. Daemon told us "no such ability" → silently fall through to
///      the chat path. Legacy abilities that exist in name only (a
///      manifest declares the verb but the daemon never registered
///      a handler) STILL need the chat translation, and the caller
///      must not see a stack trace just because the daemon was
///      consulted first.
///   3. Daemon down / daemon errored → propagate as Unavailable and
///      stop. Continuing to chat in this case would mask transport
///      failures.
///
/// A flat `Result<Option<Value>, ...>` collapses (2) and (3) into the
/// "Err" axis, which the chat-fall-through code can't distinguish
/// without string-matching the error message — fragile.
enum DaemonDispatch {
    Result(Value),
    AbilityNotFound,
    DaemonDown(String),
    Error(String),
}

/// Dispatch an owner-local ability against the local daemon through
/// Axon's local Invocation gRPC surface, with the hosted agent carried
/// as explicit callee context. Returns one of the four outcome variants
/// the caller branches on.
fn try_dispatch_via_daemon(
    agent_name: &str,
    ability_name: &str,
    arguments: &Value,
) -> DaemonDispatch {
    use crate::support::local_invoke::{classify_invoke_error, LocalInvokeErrorKind};
    match crate::support::local_invoke::invoke_local_ability_with_invocation_meta(
        ability_name,
        arguments.clone(),
        None,
        &[],
        None,
        None,
        Some(agent_name),
    ) {
        Ok((value, _meta)) => DaemonDispatch::Result(value),
        Err(err) => match classify_invoke_error(&err) {
            LocalInvokeErrorKind::DaemonOffline => DaemonDispatch::DaemonDown(format!("{err}")),
            LocalInvokeErrorKind::AbilityUnregistered => DaemonDispatch::AbilityNotFound,
            LocalInvokeErrorKind::Failed => DaemonDispatch::Error(format!("{err}")),
        },
    }
}

/// Build a prompt for an agent from an EAL step's `function_name` and arguments.
///
/// The convention is: `function_name` becomes the task description,
/// arguments become context. The `prompt` argument, if present, is used directly.
fn build_agent_prompt(function_name: &str, arguments: &Value) -> String {
    // If there's a "prompt" key, use it directly.
    if let Some(prompt) = arguments.get("prompt").and_then(|v| v.as_str()) {
        return prompt.to_string();
    }

    // Otherwise, build from function name + all argument key-values.
    let mut parts = vec![format!("Task: {function_name}")];
    if let Some(obj) = arguments.as_object() {
        for (key, val) in obj {
            match val {
                Value::String(s) => parts.push(format!("{key}: {s}")),
                other => parts.push(format!("{key}: {other}")),
            }
        }
    }
    parts.join("\n\n")
}
