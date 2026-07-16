// EasyNet CLI — a2a.client.send_task ability handler (C-M10-iii)
// =================================================================
//
// File: src/daemon/ability/builtins/integrations/a2a/client.rs
//
// Outbound A2A: lets a local caller dispatch an Invoke against a
// remote node's ability, surfaced as a first-class ability
// (`a2a.client.send_task`) so the caller doesn't reach into
// AxonAbilityCatalog directly. Pairs with `a2a.bridge.send_task`
// (the inbound side) — both surfaces ride the same Invoke
// pipeline; the difference is which direction crosses the wire.
//
// Why an ability and not a CLI subcommand
// ---------------------------------------
// Anything an interactive operator can do, an in-process planner
// (or a hosted Agent) should also be able to do. Naming the
// outbound surface as an ability means a future LLM-driven
// orchestrator can compose `meta.list_abilities` → pick a target
// → `a2a.client.send_task` to a remote node — same call shape as
// dispatching against a local ability, no special-case planner
// glue.
//
// Why this ISN'T `send_a2a_task`
// ------------------------------
// AXON-RFC-001 P1.5 deleted the underlying `send_a2a_task` axon
// helper (it now bails with a deprecation message). The new
// canonical path is "use Invoke against the appropriate Agent
// ability." This adapter projects A2A arguments into a complete
// descriptor-bound request and submits it through the daemon-hosted
// `Invocation::Invoke` service, so outbound A2A uses the same Axon
// service path as CLI/EAL remote invocation.
//
// What lives here
// ---------------
//   * a2a.client.send_task — { target_node_ura, agent_name,
//                              skill_name, args }. Resolves to
//                              ability `<agent_name>.<skill_name>`
//                              on the named remote node.
//
// What does NOT live here yet
// ---------------------------
//   * Streaming / bidi outbound — same handler shape, different Axon
//     call mode. Land when an actual remote streaming caller surfaces;
//     the unary surface covers every concrete request known today.
//   * a2a.client.list — outbound discovery. The realm hub's
//     `federation.subscribe_directory` (C-M11) is the right
//     surface for "what nodes can I talk to"; this ability
//     focuses on the dispatch primitive.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet.

use std::sync::Arc;

use serde_json::{json, Value};

#[cfg(feature = "axon-pb")]
use crate::daemon::ability::builtins::agents::discover::DiscoverFederationResolveError;
use crate::daemon::ability::builtins::agents::discover::{
    DiscoverFederationResolver, SharedDiscoverFederationResolver,
};
use crate::daemon::ability::dispatch::AxonAbilityCatalog;

use crate::daemon::ability::dispatch::OwnerKind;
pub const ABILITY_SEND_TASK: &str =
    crate::daemon::ability::names::integrations::A2A_CLIENT_SEND_TASK;

/// Register `a2a.client.send_task` on the registry. Stateless;
/// every call submits a signed request to the local daemon's
/// `Invocation::Invoke` service — the same wire path the rest of the CLI's
/// cross-device dispatch takes after the joint-plan unification.
pub fn register(
    reg: &mut AxonAbilityCatalog,
    federation_resolver: SharedDiscoverFederationResolver,
) {
    // Registered envelope-aware so the handler can read the inbound
    // invocation's causal context and chain it onto the forward hop
    // (refactor SPEC §15.1-1, bug-1 Slice B). The plain args-only
    // registration could not see the envelope, so an A2A forward always
    // re-rooted the receipt DAG.
    reg.register_rpc_with_envelope_and_owner(
        ABILITY_SEND_TASK,
        OwnerKind::Device,
        Arc::new(
            move |env: crate::daemon::ability::dispatch::EnvelopeContext, args: Value| {
                send_task_handler(args, &env, federation_resolver.as_ref())
            },
        ),
    );
}

/// Extract the causal parent anchors (`{receipt_ura, receipt_hash}` objects)
/// from an inbound `EnvelopeContext.causal_context`. The adapter serialises
/// the typed `CausalContext` as `{"kind":"none"}`, `{"kind":"scalar",..}`, or
/// `{"kind":"list","receipts":[..]}` (see `ability_dispatch::causal_context_to_json`).
/// Returns the parent list to chain onto the forward hop; empty for a root
/// invocation.
#[cfg(any(feature = "axon-pb", test))]
fn causal_parents_from_env(env: &crate::daemon::ability::dispatch::EnvelopeContext) -> Vec<Value> {
    let cc = env.causal_context();
    match cc.get("kind").and_then(Value::as_str) {
        Some("scalar") => vec![json!({
            "receipt_ura": cc.get("receipt_ura").cloned().unwrap_or(Value::Null),
            "receipt_hash": cc.get("receipt_hash").cloned().unwrap_or(Value::Null),
        })],
        Some("list") => cc
            .get("receipts")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        // "none", "merkle", or anything unrecognised: no chainable receipt
        // anchors, so this forward hop roots a fresh causal context.
        _ => Vec::new(),
    }
}

/// `a2a.client.send_task` handler.
///
/// Args: `{ "target_node_ura": "<URA>", "agent_name": "<agent>",
///          "skill_name": "<verb>", "args": <json-value> }`.
///
/// Routes through the canonical `Invocation::Invoke` service for the target
/// device URA — the same unified path
/// `easynet ability invoke --node <URA>` and EAL
/// `IrTarget::Device` use after the joint-plan cut over. Pre-cut
/// this handler tried to drive the dispatcher's now-deleted
/// `TargetScope::Remote` branch, which had no network implementation.
///
/// Returns: `{ ok, result?, error? }` — same envelope shape
/// `a2a.bridge.send_task` (the inbound side) returns, so a planner
/// that handles the inbound shape handles the outbound shape too.
fn send_task_handler(
    args: Value,
    _env: &crate::daemon::ability::dispatch::EnvelopeContext,
    federation_resolver: &dyn DiscoverFederationResolver,
) -> anyhow::Result<Value> {
    let target_node = match target_node_field(&args) {
        Ok(s) => s,
        Err(msg) => return Ok(error_response(&msg)),
    };
    let agent_name = match required_nonempty_string(&args, "agent_name") {
        Ok(s) => s,
        Err(msg) => return Ok(error_response(&msg)),
    };
    let skill_name = match required_nonempty_string(&args, "skill_name") {
        Ok(s) => s,
        Err(msg) => return Ok(error_response(&msg)),
    };
    let task_args = args.get("args").cloned().unwrap_or(Value::Null);

    #[cfg(feature = "axon-pb")]
    {
        let target_ura = if crate::core::ura::parse_ura(target_node.trim()).is_ok() {
            match crate::daemon::invocation::routing::remote_invoke::parse_node_ura(&target_node) {
                Ok(ura) => ura,
                Err(e) => return Ok(error_response(&format!("parse target_node_ura: {e}"))),
            }
        } else {
            // Bare uuid path: wrap in the local daemon's realm.
            // Without credentials we cannot do this safely — surface
            // a structured error so the caller knows to pass a URA.
            match crate::daemon::persistence::config::load_credentials() {
                Ok(c) if !c.realm.trim().is_empty() => {
                    crate::core::ura::device_ura(&c.realm, target_node.trim())
                }
                _ => {
                    return Ok(error_response(
                        "target_node_ura must be a canonical \
                         `easynet:///r/<realm>/device/<id>` URA when no local \
                         credentials are available",
                    ));
                }
            }
        };

        let caller_ura = crate::daemon::persistence::config::load_credentials()
            .ok()
            .filter(|c| !c.realm.trim().is_empty() && !c.node_id.trim().is_empty())
            .map(|c| crate::core::ura::device_ura(c.realm.trim(), c.node_id.trim()));
        if let Some(message) = local_daemon_transport_error() {
            return Ok(error_response(&message));
        }
        let resolve_caller = caller_ura.as_deref().unwrap_or_else(|| _env.caller());
        let target_call = match resolve_a2a_target(
            &target_ura,
            &agent_name,
            &skill_name,
            federation_resolver,
            resolve_caller,
        ) {
            Ok(target_call) => target_call,
            Err(e) => return Ok(error_response(&format!("{e}"))),
        };
        // Chain the inbound invocation's causal parents onto the forward
        // hop so an A2A relay preserves the receipt DAG instead of re-rooting
        // it (SPEC §15.1-1, bug-1 Slice B). Root invocations yield no parents.
        let causal_parents = causal_parents_from_env(_env);
        match crate::daemon::invocation::routing::remote_invoke::invoke_remote_target_with_causal_parents(
            &target_call,
            task_args,
            caller_ura.as_deref(),
            &causal_parents,
        ) {
            Ok(value) => Ok(json!({ "ok": true, "result": value })),
            Err(e) => Ok(error_response(&format!("{e}"))),
        }
    }
    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = (
            agent_name,
            skill_name,
            task_args,
            target_node,
            federation_resolver,
        );
        Ok(error_response(
            "a2a.client.send_task requires the `axon-pb` feature; \
             rebuild with `--features axon-pb` (production builds always do).",
        ))
    }
}

/// Pull a required, non-empty string field out of `args`. Returns
/// the string on success; returns the caller-visible error message
/// on absence/wrong-type/empty so the call site can wrap it in an
/// error_response without a separate format!.
fn required_nonempty_string(args: &Value, key: &str) -> Result<String, String> {
    match args.get(key).and_then(Value::as_str) {
        Some(s) if !s.is_empty() => Ok(s.to_string()),
        _ => Err(format!(
            "`{key}` is required and must be a non-empty string"
        )),
    }
}

fn error_response(message: &str) -> Value {
    json!({ "ok": false, "error": message })
}

#[cfg(feature = "axon-pb")]
fn local_daemon_transport_error() -> Option<String> {
    let socket_path = crate::support::platform::local_daemon_grpc::resolve_socket_path();
    if crate::support::platform::local_daemon_grpc::probe_accepting(&socket_path) {
        return None;
    }
    Some(format!(
        "daemon not running (local gRPC listener unreachable at {}). Start it with `easynet runtime start`.",
        socket_path.display()
    ))
}

#[cfg(feature = "axon-pb")]
fn resolve_a2a_target(
    execution_target_ura: &str,
    agent_name: &str,
    skill_name: &str,
    federation_resolver: &dyn DiscoverFederationResolver,
    caller_ura: &str,
) -> anyhow::Result<crate::daemon::invocation::routing::remote_invoke::RemoteAbilityInvocationTarget>
{
    let target = crate::core::ura::parse_ura(execution_target_ura)
        .map_err(|err| anyhow::anyhow!("parse target_node_ura: {err}"))?;
    let realm = target.realm.clone();
    let agents = federation_resolver
        .resolve_agents(&realm, &realm, caller_ura.to_string(), Some(realm.clone()))
        .map_err(|err| match err {
            DiscoverFederationResolveError::NotJoined(message) => {
                anyhow::anyhow!("federation directory is not joined: {message}")
            }
            DiscoverFederationResolveError::Unavailable(message) => {
                anyhow::anyhow!("federation directory is unavailable: {message}")
            }
        })?;
    let agent = select_a2a_agent(&agents, &target, execution_target_ura, agent_name)?;
    let descriptor_ref = descriptor_ref_for_a2a_skill(agent, skill_name)?;
    crate::daemon::invocation::routing::remote_invoke::RemoteAbilityInvocationTarget::from_descriptor_ref(
        execution_target_ura,
        &descriptor_ref,
    )
}

#[cfg(feature = "axon-pb")]
fn select_a2a_agent<'a>(
    agents: &'a [crate::daemon::federation::client::ability_contract::ResolvedAgent],
    target: &easynet_axon::ura::ParsedURA,
    execution_target_ura: &str,
    agent_name: &str,
) -> anyhow::Result<&'a crate::daemon::federation::client::ability_contract::ResolvedAgent> {
    let mut matches = agents
        .iter()
        .filter(|agent| a2a_agent_matches_target(agent, target, agent_name));
    let selected = matches.next().ok_or_else(|| {
        anyhow::anyhow!(
            "federation directory has no hosted Agent {agent_name:?} on target {execution_target_ura}"
        )
    })?;
    if matches.next().is_some() {
        anyhow::bail!(
            "federation directory returned multiple hosted Agents named {agent_name:?} on target {execution_target_ura}"
        );
    }
    Ok(selected)
}

#[cfg(feature = "axon-pb")]
fn a2a_agent_matches_target(
    agent: &crate::daemon::federation::client::ability_contract::ResolvedAgent,
    target: &easynet_axon::ura::ParsedURA,
    agent_name: &str,
) -> bool {
    let Ok(agent_ura) = crate::core::ura::parse_ura(&agent.ura) else {
        return false;
    };
    if agent_ura.kind != crate::core::ura::URAKind::Agent {
        return false;
    }
    let Some((_owner, advertised_agent_id)) = agent_ura
        .agent_ids()
        .or_else(|| agent_ura.device_agent_ids())
    else {
        return false;
    };
    if advertised_agent_id != agent_name {
        return false;
    }
    match target.kind {
        crate::core::ura::URAKind::Device => target
            .device_id()
            .is_some_and(|target_node_id| agent.host_node_id.as_deref() == Some(target_node_id)),
        crate::core::ura::URAKind::Hub => agent_ura.realm == target.realm,
        _ => false,
    }
}

#[cfg(feature = "axon-pb")]
fn descriptor_ref_for_a2a_skill(
    agent: &crate::daemon::federation::client::ability_contract::ResolvedAgent,
    skill_name: &str,
) -> anyhow::Result<String> {
    let mut matches = agent
        .ability_summaries
        .iter()
        .filter_map(crate::daemon::federation::read_model::owner_projection::summary_from_value)
        .filter(|summary| {
            crate::daemon::federation::read_model::owner_projection::summary_public_name(summary)
                .as_deref()
                == Some(skill_name)
        });
    let summary = matches.next().ok_or_else(|| {
        anyhow::anyhow!(
            "federation catalog for Agent `{}` has no RPC ability {skill_name:?}",
            agent.ura
        )
    })?;
    if matches.next().is_some() {
        anyhow::bail!(
            "federation catalog for Agent `{}` returned multiple ability summaries named {skill_name:?}",
            agent.ura
        );
    }
    crate::daemon::federation::read_model::owner_projection::descriptor_ref_for_summary_call_mode(
        &summary,
        crate::daemon::ability::descriptors::CallMode::Rpc,
    )
}

// ── Discovery surfaces ────────────────────────────────────────

pub fn send_task_input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["target_node_ura", "agent_name", "skill_name"],
        "properties": {
            "target_node_ura": {"type": "string", "minLength": 1},
            "agent_name": {"type": "string", "minLength": 1},
            "skill_name": {"type": "string", "minLength": 1},
            "args": {
                "description": "Free-form per-skill args; shape per the remote skill's input_schema."
            },
        },
        "additionalProperties": false,
    })
}

fn target_node_field(args: &Value) -> Result<String, String> {
    required_nonempty_string(args, "target_node_ura")
}

pub fn send_task_description() -> &'static str {
    "Outbound A2A: dispatch a signed descriptor-bound RPC Invoke against a \
     remote node's `<agent_name>.<skill_name>` ability through the daemon's \
     canonical Invocation service. Returns {ok:true,result} on success; \
     remote failures surface as {ok:false,error}. The `args` payload remains \
     opaque to the routing layer."
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tests run without a local daemon socket. The handler should
    /// return ok:false instead of panicking; the populated path is
    /// exercised by daemon/axon integration tests.
    fn fresh_registry() -> Arc<AxonAbilityCatalog> {
        let mut reg = AxonAbilityCatalog::new();
        register(
            &mut reg,
            Arc::new(crate::daemon::ability::builtins::agents::discover::DetachedDiscoverFederationResolver),
        );
        Arc::new(reg)
    }

    fn detached_resolver(
    ) -> crate::daemon::ability::builtins::agents::discover::DetachedDiscoverFederationResolver
    {
        crate::daemon::ability::builtins::agents::discover::DetachedDiscoverFederationResolver
    }

    /// A root (parent-less) inbound envelope. `send_task_handler` reads the
    /// causal context off this to chain it onto the forward hop.
    fn root_env() -> crate::daemon::ability::dispatch::EnvelopeContext {
        crate::daemon::ability::dispatch::EnvelopeContext::for_test_ability(
            "easynet:///r/acme/device/local",
            ABILITY_SEND_TASK,
            "easynet:///r/acme/device/local",
        )
    }

    #[test]
    fn registration_makes_send_task_dispatchable() {
        let arc = fresh_registry();
        // Registered envelope-aware (rpc_with_env), so it answers has_rpc
        // (which checks both handler families), not the args-only get_rpc.
        assert!(arc.has_rpc(ABILITY_SEND_TASK));
    }

    /// SPEC §15.1-1 (bug-1 Slice B): the A2A forward must chain the inbound
    /// invocation's causal parents. This pins the extraction from each
    /// `EnvelopeContext.causal_context` serialised shape so a relayed task
    /// preserves the receipt DAG instead of re-rooting it.
    #[test]
    fn causal_parents_extracted_from_each_causal_context_shape() {
        use crate::daemon::ability::dispatch::EnvelopeContext;

        // Root: {"kind":"none"} -> no parents.
        let none_env = EnvelopeContext::for_test_ability(
            "easynet:///r/acme/device/local",
            ABILITY_SEND_TASK,
            "easynet:///r/acme/device/local",
        )
        .with_causal_context(json!({"kind": "none"}));
        assert!(causal_parents_from_env(&none_env).is_empty());

        // Scalar: one receipt anchor.
        let scalar_env = none_env.clone().with_causal_context(json!({
            "kind": "scalar",
            "receipt_ura": "easynet:///r/acme/resource/agent.a2a.forwarder/invocation/r1/receipt",
            "receipt_hash": "aa",
        }));
        let scalar = causal_parents_from_env(&scalar_env);
        assert_eq!(scalar.len(), 1);
        assert_eq!(
            scalar[0]["receipt_ura"],
            "easynet:///r/acme/resource/agent.a2a.forwarder/invocation/r1/receipt"
        );

        // List: fan-in parents pass through verbatim.
        let list_env = none_env.with_causal_context(json!({
            "kind": "list",
            "receipts": [
                {"receipt_ura": "easynet:///r/acme/resource/agent.a2a.forwarder/invocation/r1/receipt", "receipt_hash": "aa"},
                {"receipt_ura": "easynet:///r/acme/resource/agent.a2a.forwarder/invocation/r2/receipt", "receipt_hash": "bb"},
            ],
        }));
        let list = causal_parents_from_env(&list_env);
        assert_eq!(list.len(), 2);
        assert_eq!(
            list[1]["receipt_ura"],
            "easynet:///r/acme/resource/agent.a2a.forwarder/invocation/r2/receipt"
        );
    }

    #[test]
    fn send_task_missing_target_node_ura_returns_ok_false() {
        let resp = send_task_handler(
            json!({
                "agent_name": "claude",
                "skill_name": "chat",
            }),
            &root_env(),
            &detached_resolver(),
        )
        .unwrap();
        assert_eq!(resp["ok"], false);
        let err = resp["error"].as_str().unwrap();
        assert!(
            err.contains("`target_node_ura`"),
            "error must name the missing field; got {err:?}"
        );
    }

    #[test]
    fn send_task_missing_agent_name_returns_ok_false() {
        let resp = send_task_handler(
            json!({
                "target_node_ura": "easynet:///r/acme/node/N1",
                "skill_name": "chat",
            }),
            &root_env(),
            &detached_resolver(),
        )
        .unwrap();
        assert_eq!(resp["ok"], false);
        assert!(resp["error"].as_str().unwrap().contains("`agent_name`"));
    }

    #[test]
    fn send_task_missing_skill_name_returns_ok_false() {
        let resp = send_task_handler(
            json!({
                "target_node_ura": "easynet:///r/acme/node/N1",
                "agent_name": "claude",
            }),
            &root_env(),
            &detached_resolver(),
        )
        .unwrap();
        assert_eq!(resp["ok"], false);
        assert!(resp["error"].as_str().unwrap().contains("`skill_name`"));
    }

    #[test]
    fn send_task_empty_string_field_returns_ok_false() {
        let resp = send_task_handler(
            json!({
                "target_node_ura": "",
                "agent_name": "claude",
                "skill_name": "chat",
            }),
            &root_env(),
            &detached_resolver(),
        )
        .unwrap();
        assert_eq!(resp["ok"], false);
    }

    #[test]
    fn send_task_without_daemon_socket_returns_ok_false_no_panic() {
        // Joint-plan phase 4: a2a.client.send_task no longer
        // requires a process-wide dispatcher handle — every call
        // dials the canonical `Invocation::Invoke` service over the daemon UDS
        // fresh. Tests run without a daemon, so the call path
        // surfaces a structured `ok: false` envelope (NOT panic)
        // with a message naming the missing daemon transport or
        // the parse arm if the URA shape rejects first.
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let resp = send_task_handler(
            json!({
                "target_node_ura": "easynet:///r/acme/device/N1",
                "agent_name": "claude",
                "skill_name": "chat",
            }),
            &root_env(),
            &detached_resolver(),
        )
        .unwrap();
        assert_eq!(resp["ok"], false);
        let msg = resp["error"].as_str().unwrap();
        assert!(
            msg.contains("daemon")
                || msg.contains("federation")
                || msg.contains("credentials")
                || msg.contains("axon-pb"),
            "must surface a structured transport / config error; got: {msg}"
        );
    }

    #[test]
    fn send_task_rejects_unknown_retired_target_alias() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let resp = send_task_handler(
            json!({
                "target_node_legacy": "easynet:///r/acme/device/N1",
                "agent_name": "claude",
                "skill_name": "chat",
            }),
            &root_env(),
            &detached_resolver(),
        )
        .unwrap();
        assert_eq!(resp["ok"], false);
        let msg = resp["error"].as_str().unwrap();
        assert!(
            msg.contains("`target_node_ura`"),
            "retired aliases must be rejected at validation; got: {msg}"
        );
    }

    #[test]
    fn send_task_input_schema_requires_canonical_target_node_ura() {
        let s = send_task_input_schema();
        let req = s["required"].as_array().unwrap();
        for field in ["target_node_ura", "agent_name", "skill_name"] {
            assert!(
                req.iter().any(|v| v == field),
                "required field {field} missing from schema"
            );
            assert_eq!(s["properties"][field]["minLength"], 1);
        }
        assert!(s["properties"].get("target_node_legacy").is_none());
        assert!(s.get("anyOf").is_none());
        assert_eq!(s["additionalProperties"], false);
    }
}
