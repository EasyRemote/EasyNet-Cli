// EasyNet CLI — a2a.bridge.list_skills ability handler
// =====================================================
//
// File: src/daemon/ability/builtins/integrations/a2a/bridge.rs
//
// Edge-adapter ability that surfaces the host's local A2A agent card
// catalogue from the daemon's committed ability control plane. Per the
// consensus "A2A at the edge, NOT node-to-node" (RFC §A3 / plan §13),
// this ability is how an in-process caller (or a co-located A2A
// adapter) reaches the local A2A view through the same Invoke pipeline
// every other ability uses, instead of cracking the label string.
//
// What lives here
// ---------------
//   * a2a.bridge.list_skills — joins local Agent roster metadata with
//                              the live publication snapshot to build
//                              the v2 A2A agent-card envelope:
//                              `{ "agents": [{ name, runtime,
//                                              skills: [...], ... }] }`.
//                              Identical shape to the wire label, so
//                              callers parsing the label and callers
//                              hitting this ability see one truth.
//
// What lives here (continued)
// ---------------------------
//   * a2a.bridge.send_task — translates an incoming A2A `tasks/send`
//                            request into an in-process Invocation.
//                            Args: { agent_name, skill_name,
//                            args }. Resolves to local registry name
//                            `<agent_name>.<skill_name>` (matches
//                            how chat_ability::register installs
//                            `<agent>.chat`). Same registry-self-
//                            reference seam as mcp.bridge.call_tool.
//
// What does NOT live here yet
// ---------------------------
//   * a2a.client.* — outgoing A2A. Lives where the existing Axon
//                    `send_a2a_task` helper does, surfaced as an
//                    ability in a follow-up.
//
// Why this is unary, not bidi
// ---------------------------
// A2A `agents/list` (and the legacy node-roster query) is request /
// response. Streaming applies to A2A `tasks/send` not the catalogue
// fetch — that ability lands separately on Invoke (or InvokeStream
// once a streaming-friendly A2A task takes shape).
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::daemon::ability::dispatch::AxonAbilityCatalog;
use crate::daemon::invocation::routing::target::{CallMode, InvocationTarget};
use crate::daemon::persistence::agent_registry::AgentRegistry;

use crate::daemon::ability::dispatch::OwnerKind;
pub const ABILITY_LIST_SKILLS: &str =
    crate::daemon::ability::names::integrations::A2A_BRIDGE_LIST_SKILLS;
pub const ABILITY_SEND_TASK: &str =
    crate::daemon::ability::names::integrations::A2A_BRIDGE_SEND_TASK;

/// Register both bridge abilities on the registry.
///
/// `registry_provider` is invoked at handler-call time for roster metadata.
/// Ability rows are captured from `local_registry_handle` at the same instant;
/// manifests are never reopened by this edge projection.
///
/// `local_registry_handle` is a `OnceLock` populated by the build
/// site after `Arc::new(reg)`. send_task reads through it to
/// dispatch into the named local ability (`<agent_name>.<skill_name>`).
/// Same seam mcp.bridge.call_tool uses; see that file for the
/// chicken-and-egg justification.
pub fn register<F>(
    reg: &mut AxonAbilityCatalog,
    registry_provider: F,
    local_registry_handle: Arc<std::sync::OnceLock<Arc<AxonAbilityCatalog>>>,
) where
    F: Fn() -> anyhow::Result<AgentRegistry> + Send + Sync + 'static,
{
    let provider: Arc<dyn Fn() -> anyhow::Result<AgentRegistry> + Send + Sync> =
        Arc::new(registry_provider);
    let provider_for_list = Arc::clone(&provider);
    let local_registry_for_list = Arc::clone(&local_registry_handle);
    reg.register_rpc_with_owner(
        ABILITY_LIST_SKILLS,
        OwnerKind::Device,
        Arc::new(move |_args: Value| {
            list_skills_handler(&provider_for_list, &local_registry_for_list)
        }),
    );
    reg.register_rpc_with_owner(
        ABILITY_SEND_TASK,
        OwnerKind::Device,
        Arc::new(move |args: Value| send_task_handler(&provider, &local_registry_handle, args)),
    );
}

/// `a2a.bridge.list_skills` handler.
///
/// Returns the v2 `{"agents": [...]}` envelope as structured JSON
/// (NOT the stringified label form). Keeping the structured shape
/// here means an in-process caller doesn't have to re-parse the
/// label encoding to read it back.
fn list_skills_handler(
    registry_provider: &Arc<dyn Fn() -> anyhow::Result<AgentRegistry> + Send + Sync>,
    local_registry_handle: &Arc<std::sync::OnceLock<Arc<AxonAbilityCatalog>>>,
) -> anyhow::Result<Value> {
    let registry = registry_provider().map_err(|error| {
        anyhow::anyhow!("a2a.bridge.list_skills: load agent registry: {error:#}")
    })?;
    let local = local_registry_handle.get().ok_or_else(|| {
        anyhow::anyhow!(
            "a2a.bridge.list_skills: live ability catalog is not initialized; daemon assembly is incomplete"
        )
    })?;
    let publication =
        crate::daemon::ability::catalog::LocalAbilityPublicationSnapshot::capture(local.as_ref());
    Ok(
        crate::daemon::federation::read_model::a2a_labels::build_agents_envelope(
            &registry,
            &publication,
        ),
    )
}

/// `a2a.bridge.send_task` handler.
///
/// Args: `{ "agent_name": "<agent>", "skill_name": "<verb>",
///          "args": <json-value> }`.
///
/// Resolves to the local registry name `<agent_name>.<skill_name>`
/// — matches how chat_ability::register installs `<agent>.chat` per
/// loaded agent. Visibility re-check: agent_name MUST appear in
/// the live `AgentRegistry` snapshot (the same source list_skills
/// projects). A registered agent's skills array names a tool by
/// `name`; we re-check that too.
///
/// Returns: `{ ok, result?, error? }` — A2A's task model is async
/// in the spec, but for the in-process v1 bridge a unary
/// success/error envelope is what callers actually want; the
/// streaming flavour ships when an A2A peer needs it.
fn send_task_handler(
    registry_provider: &Arc<dyn Fn() -> anyhow::Result<AgentRegistry> + Send + Sync>,
    local_registry_handle: &Arc<std::sync::OnceLock<Arc<AxonAbilityCatalog>>>,
    args: Value,
) -> anyhow::Result<Value> {
    let agent_name = match args.get("agent_name").and_then(Value::as_str) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => {
            return Ok(error_response(
                "`agent_name` is required and must be a non-empty string",
            ));
        }
    };
    let requested_skill_name = match args.get("skill_name").and_then(Value::as_str) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => {
            return Ok(error_response(
                "`skill_name` is required and must be a non-empty string",
            ));
        }
    };
    let task_args = args.get("args").cloned().unwrap_or(Value::Null);

    // Visibility re-check against the live AgentRegistry. If the
    // agent isn't registered, send_task refuses — the catalogue is
    // the source of truth for "which agents accept tasks here."
    let registry = registry_provider()
        .map_err(|error| anyhow::anyhow!("a2a.bridge.send_task: load agent registry: {error:#}"))?;
    if !registry.agents.contains_key(&agent_name) {
        return Ok(error_response(&format!(
            "agent `{agent_name}` not found in the local registry"
        )));
    }

    let local = local_registry_handle.get().ok_or_else(|| {
        anyhow::anyhow!(
            "a2a.bridge.send_task: live ability catalog is not initialized; daemon assembly is incomplete"
        )
    })?;
    let qualified_prefix = format!("{agent_name}.");
    let public_name = requested_skill_name
        .strip_prefix(&qualified_prefix)
        .unwrap_or(&requested_skill_name);
    let publication =
        crate::daemon::ability::catalog::LocalAbilityPublicationSnapshot::capture(local.as_ref());
    let descriptor = publication
        .hosted_agent_descriptors(&agent_name)
        .into_iter()
        .find(|descriptor| {
            descriptor.call_mode() == crate::daemon::ability::CallMode::Rpc
                && descriptor.public_name() == public_name
                && !matches!(public_name, "discover" | "invoke")
        });
    let Some(descriptor) = descriptor else {
        return Ok(error_response(&format!(
            "skill `{agent_name}.{public_name}` is not present in the live A2A capability publication"
        )));
    };
    let Some(ability_ura) = descriptor.canonical_ability_ura() else {
        return Ok(error_response(&format!(
            "skill `{agent_name}.{public_name}` has no canonical Ability URA"
        )));
    };
    let selector = match crate::core::ura::AbilitySelector::parse(&ability_ura) {
        Ok(selector) => selector,
        Err(error) => {
            return Ok(error_response(&format!(
                "skill `{agent_name}.{public_name}` has invalid Ability URA `{ability_ura}`: {error}"
            )));
        }
    };
    let target =
        crate::daemon::invocation::routing::target::LocalAbilityTarget::from_selector(&selector);
    if !local.has_rpc(target.dispatch_name()) {
        return Ok(error_response(&format!(
            "skill `{agent_name}.{public_name}` maps to ability `{}`, which is not invocable as a unary RPC",
            target.dispatch_name()
        )));
    }

    let invocation_target = InvocationTarget::local_daemon_system_with_subject(
        target.ability_ura().to_string(),
        task_args,
        CallMode::Rpc,
        target.default_subject_ura().to_string(),
    );
    match local.invoke_rpc_target_json(invocation_target) {
        Ok(value) => Ok(json!({
            "ok": true,
            "result": value,
        })),
        Err(e) => Ok(error_response(&format!(
            "task `{agent_name}.{public_name}` returned an error: {e}"
        ))),
    }
}

fn error_response(message: &str) -> Value {
    json!({
        "ok": false,
        "error": message,
    })
}

// ── Discovery surfaces ────────────────────────────────────────

pub fn list_skills_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {},
        "additionalProperties": false,
    })
}

pub fn list_skills_description() -> &'static str {
    "List the host's local A2A agent cards in the v2 envelope shape \
     (`{agents: [...]}`). Skills are projected from the committed live \
     ability catalog, so discovery and dispatch share one truth."
}

pub fn send_task_input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["agent_name", "skill_name"],
        "properties": {
            "agent_name": {"type": "string", "minLength": 1},
            "skill_name": {"type": "string", "minLength": 1},
            "args": {
                "description": "Free-form per-skill args; shape per the skill's own input schema."
            },
        },
        "additionalProperties": false,
    })
}

pub fn send_task_description() -> &'static str {
    "Dispatch an A2A task against a locally-hosted agent. Resolves \
     to the in-process registry name `<agent_name>.<skill_name>`; \
     visibility is re-checked against the live capability publication on \
     each call. Returns {ok:true, result} or {ok:false, error}."
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::persistence::agent_registry::{AgentEntry, AgentRegistry, AgentType};
    use std::sync::OnceLock;

    fn executable_test_runtime() -> Arc<easynet_axon::invocation::LocalRuntime> {
        crate::daemon::axon_bridge::runtime_factory::build_local_runtime(None, None)
    }

    fn manifest_for(name: &str) -> crate::daemon::ability::manifest::AbilityManifest {
        crate::daemon::ability::manifest::AbilityManifest::new(
            name.rsplit('.').next().unwrap_or(name),
            "test A2A bridge ability",
            json!({"type": "object"}),
        )
        .and_then(|manifest| {
            manifest.with_access(crate::daemon::ability::manifest::AccessPolicy {
                visibility: crate::daemon::ability::manifest::ManifestAccessScope::Public,
                allow_callers: None,
                deny_callers: None,
            })
        })
        .and_then(|manifest| {
            manifest.with_admission_action(
                crate::daemon::ability::descriptors::AdmissionAction::Invoke.as_str(),
            )
        })
        .expect("test manifest")
    }

    /// Test fixture: build a registry with both bridge abilities
    /// registered, and pre-register `<agent>.echo` for each agent
    /// so send_task tests have something real to dispatch into.
    fn build_bridge_registry<F>(provider: F, echo_agents: &[&str]) -> Arc<AxonAbilityCatalog>
    where
        F: Fn() -> anyhow::Result<AgentRegistry> + Send + Sync + 'static,
    {
        let hosted_agent_uras = echo_agents
            .iter()
            .map(|agent| crate::core::ura::agent_ura("test", "user", agent))
            .collect::<Vec<_>>();
        let runtime = executable_test_runtime();
        let authority = crate::daemon::ability::dispatch::AbilityAuthorityContext::for_device_authority_root_with_hosted_agents(
            crate::core::ura::device_ura("test", "device"),
            hosted_agent_uras,
        )
        .expect("test hosted-Agent authority context");
        let mut reg =
            AxonAbilityCatalog::new_with_runtime_and_authority_context(runtime, authority);
        let handle: Arc<OnceLock<Arc<AxonAbilityCatalog>>> = Arc::new(OnceLock::new());
        register(&mut reg, provider, Arc::clone(&handle));
        let arc = Arc::new(reg);
        let _ = handle.set(arc.clone());
        for a in echo_agents {
            let name = format!("{a}.echo");
            arc.hot_register_rpc_with_spec(
                name.clone(),
                OwnerKind::Agent((*a).to_string()),
                manifest_for(&name),
                Arc::new(|args: Value| Ok(json!({"echoed": args}))),
            )
            .expect("test hosted-Agent echo ability registers dynamically");
        }
        arc
    }

    fn registry_with(agent_name: &str) -> AgentRegistry {
        let mut r = AgentRegistry::default();
        r.agents.insert(
            agent_name.to_string(),
            AgentEntry::new(AgentType::ClaudeCode, None),
        );
        r
    }

    #[test]
    fn registration_makes_both_dispatchable() {
        let arc = build_bridge_registry(|| Ok(AgentRegistry::default()), &[]);
        assert!(arc.get_rpc(ABILITY_LIST_SKILLS).is_some());
        assert!(arc.get_rpc(ABILITY_SEND_TASK).is_some());
    }

    #[test]
    fn list_skills_returns_v2_agents_envelope_for_empty_registry() {
        let arc = build_bridge_registry(|| Ok(AgentRegistry::default()), &[]);
        let handler = arc.get_rpc(ABILITY_LIST_SKILLS).unwrap();
        let resp = handler(json!({})).unwrap();
        let agents = resp["agents"]
            .as_array()
            .expect("envelope.agents is an array");
        assert!(agents.is_empty());
    }

    #[test]
    fn list_skills_projects_registered_live_rpc_descriptor() {
        let arc = build_bridge_registry(|| Ok(registry_with("claude")), &["claude"]);
        let handler = arc.get_rpc(ABILITY_LIST_SKILLS).unwrap();
        let response = handler(json!({})).unwrap();

        assert_eq!(response["agents"].as_array().unwrap().len(), 1);
        assert_eq!(response["agents"][0]["name"], "claude");
        assert_eq!(response["agents"][0]["skills"].as_array().unwrap().len(), 1);
        assert_eq!(response["agents"][0]["skills"][0]["name"], "claude.echo");
    }

    #[test]
    fn list_skills_input_schema_is_empty_object() {
        let s = list_skills_input_schema();
        assert_eq!(s["type"], "object");
        let props = s["properties"].as_object().expect("properties is object");
        assert!(props.is_empty(), "list_skills accepts no arguments");
        assert_eq!(s["additionalProperties"], false);
    }

    #[test]
    fn list_skills_does_not_fabricate_skills_from_roster_changes() {
        use std::sync::Mutex;
        let snapshot: Arc<Mutex<AgentRegistry>> = Arc::new(Mutex::new(AgentRegistry::default()));
        let snap_for_provider = Arc::clone(&snapshot);
        let arc = build_bridge_registry(move || Ok(snap_for_provider.lock().unwrap().clone()), &[]);
        let handler = arc.get_rpc(ABILITY_LIST_SKILLS).unwrap();

        let first = handler(json!({})).unwrap();
        assert_eq!(first["agents"].as_array().unwrap().len(), 0);

        snapshot.lock().unwrap().agents.insert(
            "probe".to_string(),
            AgentEntry::new(AgentType::ClaudeCode, None),
        );
        let second = handler(json!({})).unwrap();
        assert_eq!(second["agents"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn list_skills_propagates_registry_load_failure() {
        let arc = build_bridge_registry(|| Err(anyhow::anyhow!("corrupt agents.json")), &[]);
        let handler = arc.get_rpc(ABILITY_LIST_SKILLS).unwrap();
        let error = handler(json!({})).expect_err("durable registry failure must fail closed");
        assert!(error.to_string().contains("corrupt agents.json"));
    }

    // ── send_task ─────────────────────────────────────────────

    #[test]
    fn send_task_round_trips_through_registered_agent_skill() {
        // Happy path: agent `claude` is in the registry, `claude.echo`
        // is in the local registry; send_task forwards args and wraps
        // the response.
        let arc = build_bridge_registry(|| Ok(registry_with("claude")), &["claude"]);
        let handler = arc.get_rpc(ABILITY_SEND_TASK).unwrap();
        let resp = handler(json!({
            "agent_name": "claude",
            "skill_name": "echo",
            "args": {"hello": "world"}
        }))
        .unwrap();
        assert_eq!(resp["ok"], true);
        // Echo handler wraps args in {"echoed": ...}.
        assert_eq!(resp["result"]["echoed"]["hello"], "world");
    }

    #[test]
    fn send_task_accepts_the_qualified_name_returned_by_list_skills() {
        let arc = build_bridge_registry(|| Ok(registry_with("claude")), &["claude"]);
        let handler = arc.get_rpc(ABILITY_SEND_TASK).unwrap();
        let resp = handler(json!({
            "agent_name": "claude",
            "skill_name": "claude.echo",
            "args": {"hello": "world"}
        }))
        .unwrap();
        assert_eq!(resp["ok"], true);
        assert_eq!(resp["result"]["echoed"]["hello"], "world");
    }

    #[test]
    fn send_task_unknown_agent_returns_ok_false() {
        // Agent registry is empty → send_task refuses cleanly.
        let arc = build_bridge_registry(|| Ok(AgentRegistry::default()), &["claude"]);
        let handler = arc.get_rpc(ABILITY_SEND_TASK).unwrap();
        let resp = handler(json!({
            "agent_name": "ghost",
            "skill_name": "echo",
            "args": {}
        }))
        .unwrap();
        assert_eq!(resp["ok"], false);
        let err = resp["error"].as_str().unwrap();
        assert!(err.contains("agent `ghost` not found"));
    }

    #[test]
    fn send_task_known_agent_unknown_skill_returns_ok_false() {
        // Agent IS in the registry but the skill name doesn't
        // resolve to a registered RPC handler. send_task surfaces
        // the lookup target so an operator can grep for it.
        let arc = build_bridge_registry(|| Ok(registry_with("claude")), &[]); // no claude.echo
        let handler = arc.get_rpc(ABILITY_SEND_TASK).unwrap();
        let resp = handler(json!({
            "agent_name": "claude",
            "skill_name": "design",
            "args": {}
        }))
        .unwrap();
        assert_eq!(resp["ok"], false);
        let err = resp["error"].as_str().unwrap();
        assert!(
            err.contains("`claude.design`"),
            "error must name the resolved registry key for greppability; got {err:?}"
        );
    }

    #[test]
    fn send_task_missing_agent_name_returns_ok_false() {
        let arc = build_bridge_registry(|| Ok(AgentRegistry::default()), &[]);
        let handler = arc.get_rpc(ABILITY_SEND_TASK).unwrap();
        let resp = handler(json!({"skill_name": "echo"})).unwrap();
        assert_eq!(resp["ok"], false);
        assert!(resp["error"].as_str().unwrap().contains("`agent_name`"));
    }

    #[test]
    fn send_task_missing_skill_name_returns_ok_false() {
        let arc = build_bridge_registry(|| Ok(registry_with("claude")), &[]);
        let handler = arc.get_rpc(ABILITY_SEND_TASK).unwrap();
        let resp = handler(json!({"agent_name": "claude"})).unwrap();
        assert_eq!(resp["ok"], false);
        assert!(resp["error"].as_str().unwrap().contains("`skill_name`"));
    }

    #[test]
    fn send_task_handler_error_is_surfaced_as_ok_false() {
        let runtime = executable_test_runtime();
        let authority = crate::daemon::ability::dispatch::AbilityAuthorityContext::for_device_authority_root_with_hosted_agents(
            crate::core::ura::device_ura("test", "device"),
            vec![crate::core::ura::agent_ura("test", "user", "claude")],
        )
        .expect("test hosted-Agent authority context");
        let mut reg =
            AxonAbilityCatalog::new_with_runtime_and_authority_context(runtime, authority);
        let handle: Arc<OnceLock<Arc<AxonAbilityCatalog>>> = Arc::new(OnceLock::new());
        register(
            &mut reg,
            || Ok(registry_with("claude")),
            Arc::clone(&handle),
        );
        let arc = Arc::new(reg);
        let _ = handle.set(arc.clone());
        arc.hot_register_rpc_with_spec(
            "claude.fails",
            OwnerKind::Agent("claude".to_string()),
            manifest_for("claude.fails"),
            Arc::new(|_args: Value| anyhow::bail!("planned failure for the test")),
        )
        .expect("test hosted-Agent failing ability registers dynamically");

        let handler = arc.get_rpc(ABILITY_SEND_TASK).unwrap();
        let resp = handler(json!({
            "agent_name": "claude",
            "skill_name": "fails",
            "args": {}
        }))
        .unwrap();
        assert_eq!(resp["ok"], false);
        let err = resp["error"].as_str().unwrap();
        assert!(err.contains("planned failure"));
    }

    #[test]
    fn send_task_tolerates_missing_args_field() {
        let arc = build_bridge_registry(|| Ok(registry_with("claude")), &["claude"]);
        let handler = arc.get_rpc(ABILITY_SEND_TASK).unwrap();
        let resp = handler(json!({
            "agent_name": "claude",
            "skill_name": "echo"
        }))
        .unwrap();
        assert_eq!(resp["ok"], true);
        assert_eq!(resp["result"]["echoed"], Value::Null);
    }

    #[test]
    fn send_task_input_schema_requires_both_names() {
        let s = send_task_input_schema();
        let req = s["required"].as_array().unwrap();
        assert!(req.iter().any(|v| v == "agent_name"));
        assert!(req.iter().any(|v| v == "skill_name"));
    }

    #[test]
    fn send_task_unset_registry_handle_fails_closed() {
        let mut reg = AxonAbilityCatalog::new();
        let handle: Arc<OnceLock<Arc<AxonAbilityCatalog>>> = Arc::new(OnceLock::new());
        register(
            &mut reg,
            || Ok(registry_with("claude")),
            Arc::clone(&handle),
        );
        let arc = Arc::new(reg);
        // Deliberately do NOT set the handle.
        let handler = arc.get_rpc(ABILITY_SEND_TASK).unwrap();
        let error = handler(json!({
            "agent_name": "claude",
            "skill_name": "echo"
        }))
        .expect_err("an unwired catalog is a daemon assembly failure");
        assert!(error.to_string().contains("not initialized"));
    }
}
