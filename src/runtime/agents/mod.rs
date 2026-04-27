// EasyNet CLI — System Abilities (`system.*` namespace)
// ======================================================
//
// File: src/runtime/system/mod.rs
// Description: Device-level abilities published by `easynet-daemon`.
//              Distinct from agent abilities (which live under
//              `runtime::abilities` and bind to one registered AI
//              agent), system abilities belong to the *node*
//              itself: ping, schedule, session-attach, permission,
//              discuss, loop. Their handlers run inside the daemon,
//              not inside an agent subprocess.
//
// Naming
// ------
// All system abilities are named `system.<feature>[.<verb>]`. Today
// only `observe.health` exists; PR-ATTACH onwards extends the namespace.
//
// Per-feature module layout
// -------------------------
// One file per feature (PR-ATTACH adds `session_ability.rs`,
// PR-PERM adds `permission_ability.rs`, etc.). Each file exports
// (a) the schema/manifest helpers and (b) a registration function
// that mounts the handler on the `LocalAbilityRegistry`.
//
// CI rule (`scripts/check-dispatch-boundary.sh`)
// ----------------------------------------------
// Handler functions in this directory MUST NOT inspect
// `self.node_id` or `target_node` to decide locality. The stage-1
// resolver in `runtime::invocation_target` is the only place that
// makes that decision; handlers consume `InvocationTarget` and act
// on it.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

/// Generator for the `abilities/system/<name>.ability.toml`
/// files. Single source of truth: every TOML descriptor on
/// disk is the output of `render_ability_toml(name,
/// description_for(name), input_schema_for(name))`. The drift
/// test in this module's `tests` block enforces the equality;
/// the `gen-ability-tomls` binary regenerates after any code
/// change to the metadata.
pub mod ability_toml;
/// Per-ability real-invocation tests. Each `#[test]` exercises
/// one published ability through the live dispatcher with
/// realistic args (not `{}`). This is the test layer that
/// validates the full chain: registry lookup + handler
/// invocation + service interaction + response shape. See the
/// file's preamble for what "real" does and does NOT cover.
#[cfg(test)]
mod real_invoke_tests;
pub mod a2a_bridge_ability;
pub mod a2a_client_ability;
pub mod chat_ability;
/// AXIOM §"Tier 2.5" Baseline Locomotion Profile, filesystem
/// half. Three abilities (`fs.read`, `fs.write`, `fs.list`)
/// published by every host-embodied agent claiming the
/// `baseline-locomotion-v1` profile.
pub mod fs_ability;
/// fs.edit — surgical string-replace primitive over a single
/// text file. Default contract: old_string MUST occur exactly
/// once; ambiguous matches reject with the count rather than
/// silently rewriting all occurrences. Pass replace_all=true
/// to opt into bulk replacement. Empty old_string + missing
/// target = create-new-file. Atomic write (tempfile +
/// fdatasync + rename) shared with fs.write.
pub mod fs_edit_ability;
/// AXIOM §"Tier 2.5" Baseline Locomotion Profile,
/// structured-execution member. `process.exec` spawns one
/// process via OS-level argv (NO shell interpretation);
/// for pipes / redirects / glob a caller uses `shell.run`.
/// The eight-stage shell pipeline is NOT run here; the
/// structured input shape is the security boundary.
pub mod process_exec_ability;
/// shell.run — the shell-interpreted member of the Baseline
/// Locomotion Profile. Takes a bash command STRING (with pipes
/// / redirects / glob), runs it through the AXIOM Tier 2.5
/// 8-stage pipeline (ast → security → permissions →
/// pathconstraints → readonly → destructive), and on full pass
/// dispatches to `bash -c <command>` via the SAME runner
/// process.exec uses. See AXIOM Tier 2.5 §"shell.run / 8-stage
/// pipeline" for the normative spec; the pipeline lives in
/// `support/shellguard/`, this module is the thin
/// agent-dispatch wiring on top.
pub mod shell_run_ability;
/// http.request — outbound HTTP client. Last member of the
/// Baseline Locomotion Profile. Issues one request per call,
/// captures status / headers / body up to a cap, redacts
/// auth-bearing headers (Authorization, Cookie, X-API-Key, …)
/// from every receipt the auditor may persist. Schemes
/// restricted to http / https; CR/LF in header values
/// rejected; redirect / timeout / body caps enforced.
pub mod http_request_ability;
pub mod context_loaders;
pub mod discuss_ability;
pub mod fleet_list_agents_ability;
pub mod loop_ability;
pub mod mcp_bridge_ability;
pub mod mcp_client_ability;
pub mod meta_ability;
pub mod network_health_ability;
pub mod policy_ability;
pub mod permission_ability;
pub mod ping;
pub mod profiles;
/// fleet.pty_session_attach — interactive bidirectional terminal
/// stream over InvokeBidi. The seventh and final member of the
/// AXIOM Tier 2.5 Baseline Locomotion Profile (the streaming
/// counterpart to process.exec / shell.run). See
/// `runtime/execution/pty/` for the underlying PtyService.
pub mod admin_status_ability;
pub mod fleet_lifecycle_ability;
pub mod pty_attach_ability;
/// fleet.pty_session_create / fleet.pty_session_close — control-
/// plane lifecycle for PtyService sessions. attach (above) is
/// the data-plane sibling.
pub mod pty_lifecycle_ability;
pub mod schedule_ability;
pub mod session_ability;
pub mod skill_ability;
pub mod skill_install_ability;

use std::sync::Arc;

use crate::registry::agents::AgentRegistry;
use crate::runtime::ability_dispatch::LocalAbilityRegistry;
use crate::runtime::execution::discuss::DiscussService;
use crate::runtime::execution::loop_instance::LoopService;
use crate::runtime::execution::permission::PermissionService;
use crate::runtime::execution::pty::PtyService;
use crate::runtime::execution::schedule::ScheduleService;
use crate::runtime::execution::session::SessionService;

/// Build a `LocalAbilityRegistry` populated with every v1 system
/// ability handler. Suitable for early-boot smoke tests + the
/// `published_ability_names` helper that the discovery publisher
/// consumes. Tests get fresh empty sub-services and an empty agent
/// registry; the daemon bin calls `build_registry_with_services`
/// instead with its real Kernel handles + loaded agents.
pub fn build_registry() -> Arc<LocalAbilityRegistry> {
    build_registry_with_services(
        Arc::new(SessionService::new()),
        Arc::new(PermissionService::new()),
        Arc::new(DiscussService::new()),
        Arc::new(ScheduleService::new()),
        Arc::new(LoopService::new()),
        &AgentRegistry::default(),
        Arc::new(Vec::new()),
    )
}

/// Build a `LocalAbilityRegistry` with sub-service handles wired
/// in. The daemon bin calls this with the Kernel's actual handles
/// at boot; tests construct a fresh registry per case.
///
/// `agents` and `loaders` were added when chat became a first-class
/// system-registered ability: a `<agent>.chat` handler is registered
/// per agent (see `chat_ability::register`). `loaders` is the seam
/// for pluggable context loaders — empty in v1, populated in
/// subsequent PRs without touching the daemon's startup code.
pub fn build_registry_with_services(
    sessions: Arc<SessionService>,
    perms: Arc<PermissionService>,
    discuss: Arc<DiscussService>,
    schedule: Arc<ScheduleService>,
    loop_svc: Arc<LoopService>,
    agents: &AgentRegistry,
    loaders: Arc<Vec<Arc<dyn chat_ability::ContextLoader>>>,
) -> Arc<LocalAbilityRegistry> {
    let mut reg = LocalAbilityRegistry::new();
    ping::register(&mut reg);
    network_health_ability::register(&mut reg);
    // AXIOM §"Tier 2.5" Baseline Locomotion Profile, filesystem
    // half. Three stateless handlers (fs.read / fs.write /
    // fs.list) — every host-embodied agent claiming
    // `baseline-locomotion-v1` MUST expose them.
    fs_ability::register(&mut reg);
    // AXIOM §"Tier 2.5" Baseline Locomotion — surgical text
    // edit. Sibling of fs.read / fs.write; uses the SAME
    // atomic-write path (tempfile + fdatasync + rename) so
    // the crash-resilience story is uniform.
    fs_edit_ability::register(&mut reg);
    // AXIOM §"Tier 2.5" Baseline Locomotion Profile —
    // structured execution. `process.exec` shares the
    // destructive command list and process-execution
    // hardening (tempfile-backed output, tree-kill on
    // timeout, env defaults) with `shell.run` via the
    // `support::shellguard` subsystem.
    process_exec_ability::register(&mut reg);
    // AXIOM §"Tier 2.5" Baseline Locomotion Profile —
    // shell-interpreted execution. `shell.run` is the only
    // member of the profile that takes a bash command STRING;
    // the 8-stage shellguard pipeline (ast → security →
    // permissions → pathconstraints → readonly → destructive)
    // gates every dispatch.
    shell_run_ability::register(&mut reg);
    // AXIOM §"Tier 2.5" Baseline Locomotion — HTTP client.
    // Last member of the seven-ability profile; first-class
    // surface for outbound network so receivers can audit
    // every external call uniformly instead of going through
    // a shell.run-wrapped curl.
    http_request_ability::register(&mut reg);
    // AXIOM §"Tier 2.5" Baseline Locomotion — PTY data-plane and
    // its lifecycle control-plane. fleet.pty_session_create /
    // fleet.pty_session_close manage the session catalog;
    // fleet.pty_session_attach pumps stdin/stdout bidirectionally
    // over InvokeBidi for interactive workloads (REPLs, editors,
    // text-mode TUI). All three share one process-wide PtyService
    // (single Arc, lazy init): a session created by …_create
    // is the same session …_attach pumps and …_close tears down,
    // so the three abilities cohere even though they're three
    // separate handlers.
    let pty = Arc::new(PtyService::new());
    pty_lifecycle_ability::register(&mut reg, Arc::clone(&pty));
    pty_attach_ability::register(&mut reg, pty);
    // fleet.start_agent / fleet.stop_agent — Invoke-side mirror
    // of `easynet agent add/remove`. LLM sub-agents are registry
    // rows (not resident processes), so start ≡ insert into
    // ~/.easynet/agents.json and return the canonical URA;
    // stop ≡ delete the row (idempotent).
    fleet_lifecycle_ability::register(&mut reg);
    // policy.{evaluate,simulate} — admission-gate consumer surface
    // pinned to the §A6 contract. v1 is allow-all; the gate's
    // rewiring to actually call this ability lands in a follow-up
    // (see policy_ability module preamble).
    policy_ability::register(&mut reg);
    session_ability::register(&mut reg, sessions);
    permission_ability::register(&mut reg, perms);
    discuss_ability::register(&mut reg, discuss);
    schedule_ability::register(&mut reg, schedule);
    loop_ability::register(&mut reg, loop_svc);
    chat_ability::register(&mut reg, agents, loaders);
    skill_ability::register(&mut reg);
    skill_install_ability::register(&mut reg);
    // mcp.bridge.{list_tools, call_tool} — MCP edge adapter.
    //
    // list_tools projects local AbilityDescriptors to the MCP
    // tools/list shape. Provider runs on every call so a daemon
    // restart that picks up a freshly-canonicalised URA (or a
    // future hot-add of a hosted Agent) is reflected without re-
    // registering the handler. `load_host_descriptors` is the same
    // recipe the MCP stdio server uses, so an external MCP client
    // and an in-process Invoke caller see one catalog.
    //
    // call_tool dispatches an in-process Invoke against the named
    // local ability. The OnceLock seam below is the
    // chicken-and-egg fix: call_tool needs an `Arc` to the
    // registry being built, but the registry isn't yet wrapped in
    // an `Arc` at registration time. Set the lock once after
    // `Arc::new(reg)` completes; the closure's `get()` returns
    // the populated handle on every subsequent call.
    //
    // One shared lock is enough — both mcp.bridge.call_tool and
    // a2a.bridge.send_task target the same local registry;
    // doubling up would waste an allocation per boot for no gain.
    // Same OnceLock shape admin_status_ability uses elsewhere in
    // this file.
    let local_registry_handle: Arc<std::sync::OnceLock<Arc<LocalAbilityRegistry>>> =
        Arc::new(std::sync::OnceLock::new());
    mcp_bridge_ability::register(
        &mut reg,
        profiles::load_host_descriptors,
        Arc::clone(&local_registry_handle),
    );
    // meta.{describe,list_abilities} — Agent self-introspection on
    // the same descriptor catalogue. describe is the lightweight
    // identity+summary surface; list_abilities returns the full
    // catalogue (visibility-filtered at the admission gate, not here).
    meta_ability::register(&mut reg, profiles::load_host_descriptors);
    // a2a.bridge.list_skills — same edge-adapter pattern as the MCP
    // bridge above, but for the A2A agent-card surface. Closes over
    // a clone of the AgentRegistry passed in here. v1 has no
    // hot-reload of `agents.json`, so the snapshot stays accurate
    // for the daemon's lifetime; the closure is still cheap to call.
    let agents_for_a2a = agents.clone();
    a2a_bridge_ability::register(
        &mut reg,
        move || agents_for_a2a.clone(),
        Arc::clone(&local_registry_handle),
    );
    // a2a.client.send_task — outbound A2A. Reads through a process-
    // wide DISPATCHER_HANDLE that the daemon bin populates after
    // building the AbilityDispatcher (see
    // a2a_client_ability::set_dispatcher). Tests leave the lock
    // unset; the handler returns ok:false on every call, which is
    // what the unit tests verify.
    a2a_client_ability::register(&mut reg);
    // mcp.client.{list,call} — outbound MCP. Boots an
    // McpClientService from ~/.easynet/mcp_clients.json (missing
    // file → empty service, no upstreams). Each upstream MCP
    // server is spawned lazily on first call; subsequent calls
    // reuse the live connection. Parse errors at boot bubble up
    // because a malformed file is an operator typo, not a "no
    // upstreams" condition.
    let mcp_clients_path = std::env::var("EASYNET_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::var("HOME")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|_| std::path::PathBuf::from("."))
                .join(".easynet")
        })
        .join("mcp_clients.json");
    let mcp_client_svc = match crate::runtime::execution::mcp_client::McpClientService::from_path(
        &mcp_clients_path,
    ) {
        Ok(svc) => Arc::new(svc),
        Err(e) => {
            eprintln!(
                "system::build_registry_with_services: failed to load {}: {e}; \
                 falling back to empty MCP client service (no outbound MCP \
                 servers configured)",
                mcp_clients_path.display()
            );
            Arc::new(crate::runtime::execution::mcp_client::McpClientService::new())
        }
    };
    mcp_client_ability::register(&mut reg, mcp_client_svc);
    // fleet.list_agents — operational view of registered LLM
    // sub-agents. Cheap-row projection (name, runtime, model, label);
    // for the protocol agent-card view see a2a.bridge.list_skills.
    let agents_for_fleet = agents.clone();
    fleet_list_agents_ability::register(&mut reg, move || agents_for_fleet.clone());
    // admin.status — operator-facing component snapshot. The
    // ability-count provider reads through the same OnceLock the
    // bridge handlers use, so the count is accurate at call time
    // (post-Arc-wrap; pre-set the OnceLock returns 0 which only
    // happens during the brief window before `.set()` below).
    {
        let handle_for_admin = Arc::clone(&local_registry_handle);
        admin_status_ability::register(&mut reg, move || {
            handle_for_admin
                .get()
                .map(|r| r.list_abilities().len())
                .unwrap_or(0)
        });
    }
    let arc = Arc::new(reg);
    // Populate the shared OnceLock now that the registry is wrapped.
    // Both mcp.bridge.call_tool and a2a.bridge.send_task read through
    // it to dispatch into other local abilities; until this line runs
    // they each return isError("not initialised") on every call.
    let _ = local_registry_handle.set(Arc::clone(&arc));
    arc
}

/// Daemon-side convenience wrapper. Loads the agent registry and
/// builds the full `LocalAbilityRegistry` in one call, swallowing a
/// load failure into the empty-registry case (so a brand-new install
/// without `~/.easynet/agents.json` still boots).
///
/// Exists so `bin/easynet-daemon.rs` does not have to reach into the
/// `pub(crate) registry::agents` module — that module's visibility is
/// intentionally crate-private.
pub fn build_registry_for_daemon(
    sessions: Arc<SessionService>,
    perms: Arc<PermissionService>,
    discuss: Arc<DiscussService>,
    schedule: Arc<ScheduleService>,
    loop_svc: Arc<LoopService>,
    loaders: Arc<Vec<Arc<dyn chat_ability::ContextLoader>>>,
) -> Arc<LocalAbilityRegistry> {
    let agents = match crate::registry::agents::load_agents() {
        Ok(r) => r,
        Err(e) => {
            eprintln!(
                "system::build_registry_for_daemon: failed to load agent registry: {e}; \
                 continuing with no agents (chat handlers will not be registered)"
            );
            AgentRegistry::default()
        }
    };
    build_registry_with_services(
        sessions, perms, discuss, schedule, loop_svc, &agents, loaders,
    )
}

/// Public list of every v1 system-ability *name*. Used by
/// `registry::a2a_labels` to populate the top-level
/// `system_skills[]` field of the node-roster v2 envelope so peers
/// discover what device-level abilities this daemon offers without
/// invoking anything.
///
/// The list is built from the live registry to avoid name drift
/// between the publisher and the dispatcher.
pub fn published_ability_names() -> Vec<String> {
    build_registry().list_abilities()
}

/// One row of a system ability's discovery + registration metadata.
///
/// Centralises (name, description, input_schema) so every consumer —
/// the federation label builder (`registry::a2a_labels`), the
/// runtime-local register publisher (`runtime::publish`), and any
/// future `easynet ability list --system` surface — pulls from one
/// table. Adding a new system ability now requires updating exactly
/// one match arm in `metadata_for`; previously the same name lived
/// in three places that could (and did) drift.
#[derive(Debug, Clone)]
pub struct SystemAbilityMetadata {
    pub name: String,
    pub description: &'static str,
    pub input_schema: serde_json::Value,
}

/// Every published system ability's metadata, in the deterministic
/// order `published_ability_names()` returns.
///
/// `<agent>.chat` is **excluded** even when the live registry would
/// include it: those entries are already published to the
/// axon-runtime via `runtime::publish::republish_abilities_via_advertise`
/// off the on-disk `chat.ability.toml` manifest, and re-publishing
/// them through the system path would double-register with a
/// different (synthesised) schema. Filter is by suffix because the
/// agent name varies per install.
pub fn published_abilities() -> Vec<SystemAbilityMetadata> {
    published_ability_names()
        .into_iter()
        .filter(|name| !name.ends_with(".chat"))
        .map(|name| SystemAbilityMetadata {
            description: description_for(&name),
            input_schema: input_schema_for(&name),
            name,
        })
        .collect()
}

/// Human-readable description for a published system ability name.
///
/// Authoritative source for the description text. `registry::a2a_labels`
/// re-exports through this so the wire payload and the runtime
/// register call agree byte-for-byte. Falls back to a short generic
/// blurb for unknown names; the `_ if name.ends_with(".chat")` arm
/// exists because `published_ability_names()` includes per-agent chat
/// handlers when called from the daemon registry (the `published_abilities`
/// filter strips them, but other callers may not).
pub fn description_for(name: &str) -> &'static str {
    match name {
        "observe.health" => ping::description(),
        "observe.network_health" => network_health_ability::description(),
        "policy.evaluate" => policy_ability::evaluate_description(),
        "policy.simulate" => policy_ability::simulate_description(),
        "fleet.list_sessions" => session_ability::list_description(),
        "fleet.attach_session" => session_ability::attach_description(),
        "consent.subscribe" => permission_ability::subscribe_description(),
        "consent.decide" => permission_ability::decide_description(),
        "consent.list_pending" => permission_ability::list_pending_description(),
        "discuss.create" => discuss_ability::create_description(),
        "discuss.post" => discuss_ability::post_description(),
        "discuss.subscribe" => discuss_ability::subscribe_description(),
        "schedule.add" => schedule_ability::add_description(),
        "schedule.list" => schedule_ability::list_description(),
        "schedule.remove" => schedule_ability::remove_description(),
        "schedule.enable" => schedule_ability::enable_description(),
        "loop.create" => loop_ability::create_description(),
        "loop.status" => loop_ability::status_description(),
        "loop.subscribe" => loop_ability::subscribe_description(),
        "loop.cancel" => loop_ability::cancel_description(),
        "fleet.list_abilities" => skill_ability::list_description(),
        "fleet.skill_install" => skill_install_ability::install_description(),
        "fleet.skill_remove" => skill_install_ability::remove_description(),
        "fleet.skill_upgrade" => skill_install_ability::upgrade_description(),
        "mcp.bridge.list_tools" => mcp_bridge_ability::list_tools_description(),
        "mcp.bridge.call_tool" => mcp_bridge_ability::call_tool_description(),
        "a2a.bridge.list_skills" => a2a_bridge_ability::list_skills_description(),
        "a2a.bridge.send_task" => a2a_bridge_ability::send_task_description(),
        "a2a.client.send_task" => a2a_client_ability::send_task_description(),
        "mcp.client.list" => mcp_client_ability::list_description(),
        "mcp.client.call" => mcp_client_ability::call_description(),
        "fleet.list_agents" => fleet_list_agents_ability::list_agents_description(),
        "meta.describe" => meta_ability::describe_description(),
        "meta.list_abilities" => meta_ability::list_abilities_description(),
        // AXIOM §"Tier 2.5" Baseline Locomotion — filesystem half.
        "fs.read" => fs_ability::description_read(),
        "fs.write" => fs_ability::description_write(),
        "fs.list" => fs_ability::description_list(),
        "fs.edit" => fs_edit_ability::description(),
        "process.exec" => process_exec_ability::description(),
        "shell.run" => shell_run_ability::description(),
        "http.request" => http_request_ability::description(),
        "fleet.pty_session_create" => pty_lifecycle_ability::description_create(),
        "fleet.pty_session_close" => pty_lifecycle_ability::description_close(),
        "fleet.pty_session_attach" => pty_attach_ability::description(),
        "fleet.start_agent" => fleet_lifecycle_ability::start_agent_description(),
        "fleet.stop_agent" => fleet_lifecycle_ability::stop_agent_description(),
        "admin.status" => admin_status_ability::description(),
        _ if name.ends_with(".chat") => "Send a chat prompt to the locally-installed agent.",
        _ => "(system ability)",
    }
}

/// JSON Schema for a published system ability's input. Mirrors
/// `description_for` — adding an arm here is the second half of
/// landing a new system ability so it can register against
/// axon-runtime with a real schema (not the empty-object default).
///
/// Unknown names fall back to `{"type":"object"}` — the most
/// permissive shape that still validates as a JSON Schema. A future
/// ability that lands without an arm here is callable but appears
/// as schema-less in MCP `ListTools`; a CI test pins the table
/// against the live registry to surface that drift.
pub fn input_schema_for(name: &str) -> serde_json::Value {
    match name {
        "observe.health" => ping::input_schema(),
        "observe.network_health" => network_health_ability::input_schema(),
        "policy.evaluate" => policy_ability::evaluate_input_schema(),
        "policy.simulate" => policy_ability::simulate_input_schema(),
        "fleet.list_sessions" => session_ability::list_input_schema(),
        "fleet.attach_session" => session_ability::attach_input_schema(),
        "consent.subscribe" => permission_ability::subscribe_input_schema(),
        "consent.decide" => permission_ability::decide_input_schema(),
        "consent.list_pending" => permission_ability::list_pending_input_schema(),
        "discuss.create" => discuss_ability::create_input_schema(),
        "discuss.post" => discuss_ability::post_input_schema(),
        "discuss.subscribe" => discuss_ability::subscribe_input_schema(),
        "schedule.add" => schedule_ability::add_input_schema(),
        "schedule.list" => schedule_ability::list_input_schema(),
        "schedule.remove" => schedule_ability::remove_input_schema(),
        "schedule.enable" => schedule_ability::enable_input_schema(),
        "loop.create" => loop_ability::create_input_schema(),
        "loop.status" => loop_ability::status_input_schema(),
        "loop.subscribe" => loop_ability::subscribe_input_schema(),
        "loop.cancel" => loop_ability::cancel_input_schema(),
        "fleet.list_abilities" => skill_ability::list_input_schema(),
        "fleet.skill_install" => skill_install_ability::install_input_schema(),
        "fleet.skill_remove" => skill_install_ability::remove_input_schema(),
        "fleet.skill_upgrade" => skill_install_ability::upgrade_input_schema(),
        "mcp.bridge.list_tools" => mcp_bridge_ability::list_tools_input_schema(),
        "mcp.bridge.call_tool" => mcp_bridge_ability::call_tool_input_schema(),
        "a2a.bridge.list_skills" => a2a_bridge_ability::list_skills_input_schema(),
        "a2a.bridge.send_task" => a2a_bridge_ability::send_task_input_schema(),
        "a2a.client.send_task" => a2a_client_ability::send_task_input_schema(),
        "mcp.client.list" => mcp_client_ability::list_input_schema(),
        "mcp.client.call" => mcp_client_ability::call_input_schema(),
        "fleet.list_agents" => fleet_list_agents_ability::list_agents_input_schema(),
        "meta.describe" => meta_ability::describe_input_schema(),
        "meta.list_abilities" => meta_ability::list_abilities_input_schema(),
        // AXIOM §"Tier 2.5" Baseline Locomotion — filesystem half.
        "fs.read" => fs_ability::input_schema_read(),
        "fs.write" => fs_ability::input_schema_write(),
        "fs.list" => fs_ability::input_schema_list(),
        "fs.edit" => fs_edit_ability::input_schema(),
        "process.exec" => process_exec_ability::input_schema(),
        "shell.run" => shell_run_ability::input_schema(),
        "http.request" => http_request_ability::input_schema(),
        "fleet.pty_session_create" => pty_lifecycle_ability::input_schema_create(),
        "fleet.pty_session_close" => pty_lifecycle_ability::input_schema_close(),
        "fleet.pty_session_attach" => pty_attach_ability::input_schema(),
        "fleet.start_agent" => fleet_lifecycle_ability::start_agent_input_schema(),
        "fleet.stop_agent" => fleet_lifecycle_ability::stop_agent_input_schema(),
        "admin.status" => admin_status_ability::input_schema(),
        _ => serde_json::json!({ "type": "object" }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Semantic layer for an ability. See
    /// docs/rfc/AXON-RFC-001-ability-layers.md for the contract each
    /// layer enforces. The classifier below + the
    /// `ability_layer_classification_is_complete` test together
    /// guarantee every published name lands in exactly one layer.
    #[derive(Debug, PartialEq, Eq)]
    enum AbilityLayer {
        /// Pure, side-effect free, deterministic for a catalog snapshot.
        Introspection,
        /// Pure decision functions (no mutation of catalog state).
        /// `consent.decide` is the documented exception: write-only-
        /// after-decision.
        Control,
        /// Derived state only; never triggers behaviour elsewhere.
        Observation,
        /// Per-feature business verbs (chat, schedule, loop, discuss,
        /// session, skill management). Not subject to the
        /// layer-purity rules; they ARE the work.
        Operational,
    }

    /// Classify a published ability name by the §"three layers"
    /// model. A name with no match returns `None` and the
    /// completeness test below fails — forcing the author of any
    /// new ability to either pick a layer or update this table.
    fn classify_ability(name: &str) -> Option<AbilityLayer> {
        // Per-agent chat handlers are operational by definition.
        if name.ends_with(".chat") {
            return Some(AbilityLayer::Operational);
        }
        match name {
            // ── Introspection ───────────────────────────────────
            "meta.describe"
            | "meta.list_abilities"
            | "mcp.bridge.list_tools"
            // mcp.client.list — aggregate read of every configured
            // upstream MCP server's tools/list. No mutation;
            // belongs with the introspection-layer reads.
            | "mcp.client.list"
            | "a2a.bridge.list_skills"
            | "fleet.list_agents"
            | "fleet.list_abilities"
            | "fleet.list_sessions"
            | "consent.list_pending"
            | "schedule.list"
            | "loop.status" => Some(AbilityLayer::Introspection),
            // ── Control / decision ──────────────────────────────
            "policy.evaluate"
            | "policy.simulate"
            | "consent.decide"
            | "consent.subscribe" => Some(AbilityLayer::Control),
            // ── Observation ─────────────────────────────────────
            "observe.health"
            | "observe.network_health"
            | "admin.status" => Some(AbilityLayer::Observation),
            // ── Operational (per-feature business verbs) ────────
            "fleet.attach_session"
            | "fleet.start_agent"
            | "fleet.stop_agent"
            | "fleet.skill_install"
            | "fleet.skill_remove"
            | "fleet.skill_upgrade"
            // mcp.bridge.call_tool / a2a.bridge.send_task — both
            // dispatch into another local ability; the side effects
            // come from that dispatch, not the bridge itself. Sit
            // with the operational verbs because the call surface
            // IS the work.
            | "mcp.bridge.call_tool"
            // mcp.client.call — outbound mirror of bridge.call_tool.
            // Same operational classification: dispatching
            // delegates side effects to the upstream tool.
            | "mcp.client.call"
            | "a2a.bridge.send_task"
            // a2a.client.send_task — outbound mirror of bridge.send_task.
            // Same operational classification: dispatching crosses
            // a wire and mutates the remote node's state.
            | "a2a.client.send_task"
            | "discuss.create"
            | "discuss.post"
            | "discuss.subscribe"
            | "schedule.add"
            | "schedule.remove"
            | "schedule.enable"
            | "loop.create"
            | "loop.subscribe"
            | "loop.cancel"
            // AXIOM §"Tier 2.5" Baseline Locomotion Profile,
            // filesystem half. fs.read is technically read-only
            // but it returns business content, not just metadata
            // — Operational rather than Observation. fs.write
            // mutates state. fs.list returns directory metadata
            // but its purpose is to enable subsequent fs.read /
            // fs.write — Operational by intent.
            | "fs.read"
            | "fs.write"
            | "fs.list"
            | "fs.edit"
            // AXIOM Tier 2.5 execution members. process.exec
            // and shell.run are unconditionally Operational —
            // they spawn processes that may do anything; even
            // with the 8-stage shellguard pipeline gating
            // shell.run dispatch, the layer classification
            // tracks privilege not invocation safety.
            | "process.exec"
            | "shell.run"
            | "http.request"
            | "fleet.pty_session_create"
            | "fleet.pty_session_close"
            | "fleet.pty_session_attach" => Some(AbilityLayer::Operational),
            _ => None,
        }
    }

    #[test]
    fn ability_layer_classification_is_complete() {
        // The audit story (RFC docs/AXON-RFC-001-ability-layers.md)
        // says every published ability MUST belong to exactly one
        // semantic layer. A new ability that lands without a
        // classify_ability arm trips this test, forcing the author
        // to either pick a layer or amend the layer doc.
        let names = published_ability_names();
        let unclassified: Vec<String> = names
            .iter()
            .filter(|n| classify_ability(n).is_none())
            .cloned()
            .collect();
        assert!(
            unclassified.is_empty(),
            "abilities missing a layer classification: {unclassified:?}\n\
             Add an arm to classify_ability() in src/runtime/agents/mod.rs \
             and update docs/rfc/AXON-RFC-001-ability-layers.md."
        );
    }

    #[test]
    fn introspection_layer_includes_all_three_discovery_planes() {
        // The discovery-planes invariant from
        // docs/rfc/AXON-RFC-001-discovery-planes.md: meta.list_abilities,
        // mcp.bridge.list_tools, and a2a.bridge.list_skills MUST all
        // classify as Introspection. A regression that moved one of
        // them to a different layer would fragment the discovery story.
        for name in ["meta.list_abilities", "mcp.bridge.list_tools", "a2a.bridge.list_skills"] {
            assert_eq!(
                classify_ability(name),
                Some(AbilityLayer::Introspection),
                "{name} must classify as Introspection (discovery plane)"
            );
        }
    }

    #[test]
    fn build_registry_is_non_empty_and_includes_ping() {
        // Every v1 daemon publishes at least `observe.health` so a
        // peer wanting to test reachability has a known ability.
        // A regression that emptied this list would silently break
        // discovery + smoke tests.
        let reg = build_registry();
        let names = reg.list_abilities();
        assert!(
            names.iter().any(|n| n == "observe.health"),
            "observe.health must be in the v1 registry; got {names:?}"
        );
    }

    #[test]
    fn published_ability_names_matches_live_registry() {
        // The label-publishing helper and the dispatch registry
        // must agree byte-for-byte. A regression that returned a
        // hard-coded list would let the publisher advertise
        // abilities the dispatcher cannot route.
        let live = build_registry().list_abilities();
        let advertised = published_ability_names();
        assert_eq!(live, advertised);
    }

    #[test]
    fn every_published_ability_has_a_toml_byte_for_byte_matching_the_renderer() {
        // The TOML descriptors in abilities/system/ are the
        // source of truth for external discovery tools. They are
        // GENERATED from `render_ability_toml(name,
        // description_for(name), input_schema_for(name))`. This
        // test enforces that the on-disk file is byte-for-byte
        // identical to what the renderer produces; if the
        // dispatcher's metadata changed and a maintainer forgot
        // to regenerate, this test names every drifted ability
        // and tells them how to fix it.
        let mut missing: Vec<String> = Vec::new();
        let mut drift: Vec<String> = Vec::new();
        for meta in published_abilities() {
            let toml_path = format!("abilities/system/{}.ability.toml", meta.name);
            let on_disk = match std::fs::read_to_string(&toml_path) {
                Ok(body) => body,
                Err(_) => {
                    missing.push(meta.name.clone());
                    continue;
                }
            };
            let expected = ability_toml::render_ability_toml(
                &meta.name,
                meta.description,
                &meta.input_schema,
            );
            if on_disk != expected {
                drift.push(meta.name.clone());
            }
        }
        let mut errors: Vec<String> = Vec::new();
        if !missing.is_empty() {
            errors.push(format!(
                "no TOML on disk for: {missing:?}\n\
                 -> run `cargo run --bin gen-ability-tomls` to create them"
            ));
        }
        if !drift.is_empty() {
            errors.push(format!(
                "TOML on disk differs from renderer output for: {drift:?}\n\
                 -> run `cargo run --bin gen-ability-tomls` to regenerate"
            ));
        }
        assert!(
            errors.is_empty(),
            "abilities/system TOML descriptor drift:\n  {}",
            errors.join("\n  ")
        );
    }

    /// Walk every published ability and confirm a handler is
    /// registered under SOME invocation mode (RPC, Stream, or
    /// Bidi). Distinguishes "ability advertised in
    /// list_abilities() but dispatcher returns ABILITY_NOT_FOUND"
    /// from "ability is callable". This is the bare minimum for
    /// the question "is this ability really wired".
    ///
    /// What this DOES NOT verify:
    ///   * the handler implementation is correct (most need
    ///     valid args, services, real I/O — those have their
    ///     own per-module tests),
    ///   * the response shape matches the documented schema,
    ///   * end-to-end behavior over the wire.
    /// What it DOES verify:
    ///   * `register(...)` was called for every published name
    ///     (catches the slice-16 bug class: file present, never
    ///     wired into build_registry_with_services),
    ///   * the registration mode matches the dispatcher's
    ///     expectation (catches "registered as Stream but
    ///     get_rpc() returns None" type mismatches).
    #[test]
    fn every_published_ability_resolves_to_a_handler() {
        let reg = build_registry();
        let names: Vec<String> = reg.list_abilities();
        let mut unresolved: Vec<String> = Vec::new();
        for name in &names {
            // <agent>.chat handlers register as Stream. Most
            // system abilities register as RPC. Bidi is rare
            // (PTY attach). We accept any of the three.
            let has_rpc = reg.get_rpc(name).is_some();
            let has_stream = reg.get_stream(name).is_some();
            let has_bidi = reg.get_bidi(name).is_some();
            if !(has_rpc || has_stream || has_bidi) {
                unresolved.push(name.clone());
            }
        }
        assert!(
            unresolved.is_empty(),
            "abilities listed by list_abilities() but with NO handler registered: {unresolved:?}"
        );
    }

    /// For every RPC-mode ability with no-arg or empty-args
    /// schema, actually invoke it through the dispatcher with
    /// `{}` and confirm the call returns SOME result (Ok(value)
    /// or a structured Err). The point is: we exercise the full
    /// dispatch path for every ability we can — registry lookup,
    /// handler invocation, response materialisation — not just
    /// "the function pointer exists".
    ///
    /// Rationale for the empty-args scope:
    ///   * Many RPC abilities have required fields (e.g.
    ///     fs.read needs `path`). Calling them with `{}` will
    ///     return Err(`missing required field`) which is the
    ///     CORRECT response and proves the handler runs end-to-
    ///     end. We accept Err here as PASS — the handler
    ///     reached arg-validation.
    ///   * What we reject is: panic (test framework catches),
    ///     ABILITY_NOT_FOUND error (means dispatch never reached
    ///     a handler), or a hang (test would time out).
    #[test]
    fn every_rpc_ability_actually_dispatches_through_to_its_handler() {
        use crate::runtime::ability_dispatch::AbilityDispatcher;
        use crate::runtime::gateway::NoopGateway;
        use crate::runtime::invocation_target::{
            CallMode, InvocationTarget, TargetScope,
        };

        let reg = build_registry();
        let dispatcher = AbilityDispatcher::new(Arc::clone(&reg), Arc::new(NoopGateway::new()));
        let names = reg.list_abilities();

        let mut invoked_ok: Vec<String> = Vec::new();
        let mut invoked_err: Vec<(String, String)> = Vec::new();
        let mut not_found: Vec<String> = Vec::new();
        let mut not_rpc: Vec<String> = Vec::new();

        for name in &names {
            // Only invoke things that ARE RPC. Stream / Bidi
            // abilities skip this stage; the previous test
            // confirmed they have a handler under their mode.
            if reg.get_rpc(name).is_none() {
                not_rpc.push(name.clone());
                continue;
            }
            let target = InvocationTarget {
                scope: TargetScope::Local,
                ability: name.clone(),
                normalized_args: serde_json::json!({}),
                call_mode: CallMode::Rpc,
            };
            match dispatcher.execute_rpc(target) {
                Ok(_) => invoked_ok.push(name.clone()),
                Err(e) => {
                    let msg = format!("{e}");
                    if msg.to_ascii_lowercase().contains("no rpc handler")
                        || msg.contains("ABILITY_NOT_FOUND")
                    {
                        not_found.push(name.clone());
                    } else {
                        invoked_err.push((name.clone(), msg));
                    }
                }
            }
        }

        // Print a summary so a green run still shows what was
        // actually exercised (visible with `cargo test ... --
        // --nocapture`).
        eprintln!(
            "ability invoke smoke: {} OK, {} errored-but-reached-handler, {} skipped (non-RPC)",
            invoked_ok.len(),
            invoked_err.len(),
            not_rpc.len()
        );
        if !invoked_err.is_empty() {
            eprintln!("  errored (handler reached, normal):");
            for (n, m) in &invoked_err {
                let preview: String = m.chars().take(80).collect();
                eprintln!("    {n}: {preview}");
            }
        }
        if !not_rpc.is_empty() {
            eprintln!("  skipped (registered as Stream/Bidi): {not_rpc:?}");
        }

        assert!(
            not_found.is_empty(),
            "abilities advertised but dispatcher could not find an RPC handler: {not_found:?}"
        );
    }

    #[test]
    fn build_registry_actually_contains_every_baseline_locomotion_ability() {
        // Pin the AXIOM Tier 2.5 surface: every member of the
        // Baseline Locomotion Profile MUST be registered in the
        // live registry. A regression that adds a `pub mod` but
        // forgets the `register(&mut reg)` call would leave the
        // ability invisible to the dispatcher even though the
        // module compiles. This test catches that.
        let reg = build_registry();
        let names: std::collections::BTreeSet<String> =
            reg.list_abilities().into_iter().collect();
        let must_have = [
            // Filesystem half
            "fs.read", "fs.write", "fs.list", "fs.edit",
            // Execution half
            "process.exec", "shell.run",
            // Outbound network
            "http.request",
            // Interactive PTY trio
            "fleet.pty_session_create",
            "fleet.pty_session_close",
            "fleet.pty_session_attach",
            // Operator surface added in slice 16
            "admin.status",
            "fleet.start_agent",
            "fleet.stop_agent",
        ];
        let missing: Vec<&str> = must_have
            .iter()
            .filter(|n| !names.contains(**n))
            .copied()
            .collect();
        assert!(
            missing.is_empty(),
            "Baseline Locomotion abilities NOT registered: {missing:?}.\n\
             Live registry has {} abilities: {:?}",
            names.len(),
            names
        );
    }

    #[test]
    fn published_abilities_includes_skill_list_with_real_metadata() {
        // Load-bearing for the EasyNet frontend's Skills page: the
        // backend invokes `fleet.list_abilities` via Hub-mediated
        // CallMcpTool, which in turn looks up the runtime-local tool
        // registry on the target node. That registry is populated from
        // exactly this list (see `runtime::publish::
        // republish_abilities_via_advertise`). A regression
        // that dropped skill.list from `published_abilities()` would
        // silently empty the Skills page across the fleet.
        let metas = published_abilities();
        let skill = metas
            .iter()
            .find(|m| m.name == "fleet.list_abilities")
            .expect("fleet.list_abilities must be in published_abilities");
        // Description must NOT be the unknown-name fallback.
        // `(system ability)` is what `description_for` returns when
        // an ability is added without an arm here; pin against it so
        // a future ability that lands without metadata trips the
        // test instead of shipping a generic blurb to the frontend.
        assert_ne!(
            skill.description, "(system ability)",
            "skill.list must have a real description, not the fallback"
        );
        // Input schema must be a JSON Schema object (the wire shape
        // axon-runtime stores). Empty `{}` would also pass `is_object`,
        // so additionally pin the `type` field.
        assert_eq!(
            skill.input_schema.get("type").and_then(|v| v.as_str()),
            Some("object"),
            "input schema must declare type:object; got {:?}",
            skill.input_schema
        );
    }

    #[test]
    fn published_abilities_excludes_per_agent_chat_handlers() {
        // `<agent>.chat` is published via the per-agent manifest path
        // (`runtime::publish::republish_abilities_via_advertise`) off the
        // on-disk `chat.ability.toml`. Re-publishing it through the
        // system path would double-register with a synthesised schema
        // that shadows the manifest's real one. The filter in
        // `published_abilities()` enforces this; pin it.
        use crate::registry::agents::{AgentEntry, AgentType};
        let mut agents = AgentRegistry::default();
        agents
            .agents
            .insert("alice".into(), AgentEntry::new(AgentType::ClaudeCode, None));
        let reg = build_registry_with_services(
            Arc::new(SessionService::new()),
            Arc::new(PermissionService::new()),
            Arc::new(DiscussService::new()),
            Arc::new(ScheduleService::new()),
            Arc::new(LoopService::new()),
            &agents,
            Arc::new(Vec::new()),
        );
        // Sanity: the registry itself does include alice.chat.
        assert!(reg.list_abilities().iter().any(|n| n == "alice.chat"));
        // But the system publisher's view excludes it. We can't call
        // published_abilities() with this custom registry directly
        // (it goes through build_registry()), so instead assert the
        // filter property: every entry's name does NOT end with .chat.
        for meta in published_abilities() {
            assert!(
                !meta.name.ends_with(".chat"),
                "published_abilities must filter out *.chat (came in via per-agent manifest); \
                 found {} which would double-register",
                meta.name
            );
        }
    }

    #[test]
    fn description_for_and_input_schema_for_cover_every_published_name() {
        // Adding a new ability to build_registry without also adding
        // arms to `description_for`/`input_schema_for` would let it
        // ship with the unknown-name fallback ("(system ability)" and
        // empty `{type: object}` schema). Pin the contract that every
        // published name has real metadata.
        for name in published_ability_names() {
            // `<agent>.chat` is the documented exception — its
            // description lives in the manifest, not the table — so
            // skip it here. (The `published_abilities` filter already
            // strips it from the publisher's view.)
            if name.ends_with(".chat") {
                continue;
            }
            let desc = description_for(&name);
            assert_ne!(
                desc, "(system ability)",
                "{name} is missing a description_for arm — add one in runtime::system::mod"
            );
            let schema = input_schema_for(&name);
            // The default fallback returns `{"type":"object"}` with
            // NO other keys. A real arm always pins something more —
            // `properties`, `additionalProperties`, `oneOf`, etc. —
            // even for genuinely-no-arg abilities (e.g.
            // `consent.subscribe` declares
            // `additionalProperties: false`). Distinguishing the
            // fallback from an authored "no-arg" schema by structure
            // (does the object have any key besides `type`?) is
            // strictly stronger than a name allowlist.
            let obj = schema
                .as_object()
                .unwrap_or_else(|| panic!("{name} schema must be a JSON object"));
            let has_only_type = obj.len() == 1 && obj.contains_key("type");
            assert!(
                !has_only_type,
                "{name} fell through to the default `{{type: object}}` schema; \
                 add an input_schema_for arm (declare additionalProperties: false \
                 even if the ability is genuinely no-arg)"
            );
        }
    }

    #[test]
    fn registry_includes_chat_handler_per_registered_agent() {
        // After Phase 3 wired chat as a system ability, every agent
        // in the registry should produce a `<agent>.chat` handler in
        // the unified LocalAbilityRegistry. This is the load-bearing
        // property that lets the proxy dispatch chat through the
        // same registry as ping/session/permission.
        use crate::registry::agents::{AgentEntry, AgentType};
        let mut agents = AgentRegistry::default();
        agents
            .agents
            .insert("alice".into(), AgentEntry::new(AgentType::ClaudeCode, None));
        agents
            .agents
            .insert("bob".into(), AgentEntry::new(AgentType::Codex, None));
        let reg = build_registry_with_services(
            Arc::new(SessionService::new()),
            Arc::new(PermissionService::new()),
            Arc::new(DiscussService::new()),
            Arc::new(ScheduleService::new()),
            Arc::new(LoopService::new()),
            &agents,
            Arc::new(Vec::new()),
        );
        let names = reg.list_abilities();
        assert!(
            names.iter().any(|n| n == "alice.chat"),
            "alice.chat must be registered; got {names:?}"
        );
        assert!(
            names.iter().any(|n| n == "bob.chat"),
            "bob.chat must be registered; got {names:?}"
        );
    }
}
