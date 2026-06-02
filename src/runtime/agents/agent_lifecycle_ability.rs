// EasyNet CLI — device.agent.{start,stop} ability handlers
// =================================================================
//
// File: src/runtime/agents/agent_lifecycle_ability.rs
//
// Per RFC §18, the device-profile owns these abilities. They match
// the operator-facing `easynet agent add` / `easynet agent remove`
// CLI subcommands but reach the same registry through Invoke instead
// of stdin parsing — so a remote operator (or another local Agent)
// can manage the local agent registry without spawning a shell.
//
// Lifecycle model
// ---------------
// LLM sub-agents in EasyNet are *registry rows*, not resident
// processes. Per `~/.easynet/agents.json`, an entry records the
// runtime kind (claude-code / codex / …), the model selector, and
// optional label. The actual claude/codex process is spawned per
// invocation by `agent send` / chat_ability and exits when the
// invocation completes — there is no long-running daemon to start
// or stop.
//
// So `device.agent.start` is "register a new agent row", and
// `device.agent.stop` is "remove the row." The ability names match
// the §18 registry; the verbs map onto today's reality.
//
// What lives here
// ---------------
//   * device.agent.start — { name, agent_type, model? } →
//                          { agent_ura, replaced_prior, runtime_* }
//                          replaced_prior=true means the call
//                          overwrote an existing row of the same
//                          name (operator-visible event).
//   * device.agent.stop  — { name | agent_ura } → { ack: bool }
//                          ack=false when the row didn't exist
//                          (idempotent: callers can retry without
//                          triggering an error).
//
// What does NOT live here
// -----------------------
//   * Workspace cleanup. `easynet agent remove --purge` deletes
//     `~/.easynet/workspaces/<name>/`; the ability deliberately
//     doesn't, so a remote stop_agent can't accidentally wipe an
//     operator's local files. Workspace lifecycle stays under the
//     CLI subcommand.
//   * Process kill signals. There are no resident agent processes
//     today (see Lifecycle model above); a future per-agent
//     long-runner would land its own a future device.session operation.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet.

use std::str::FromStr;
use std::sync::Arc;

use serde_json::{json, Value};

use crate::core::agent_spec::{AgentSpec, RuntimeKind};
use crate::persistence::config;
use crate::registry::agents::{self, AgentEntry, AgentType, CURRENT_REGISTRY_SCHEMA};
use crate::runtime::ability_dispatch::AxonAbilityCatalog;
use crate::runtime::directory::{AgentDirectory, Location};

use crate::runtime::ability_dispatch::OwnerKind;
pub const ABILITY_START_AGENT: &str = "device.agent.start";
pub const ABILITY_STOP_AGENT: &str = "device.agent.stop";
pub const ABILITY_REFRESH_AGENTS: &str = "device.agent.refresh";

/// Late-wired `Arc<HotAgentRegistrar>` shared between boot and the
/// `device.agent.start` / `device.agent.stop` handler closures.
///
/// Boot constructs the registrar AFTER the `LocalRuntime` and
/// `dispatch_handle` OnceLock are both wired. The agent-lifecycle
/// register call runs EARLIER (during static-registry build), so
/// the handler closures can't capture the registrar directly. We
/// thread a `OnceLock<Arc<HotAgentRegistrar>>` instead: the
/// handlers read through `get()` at dispatch time, and boot
/// populates the cell once everything else is in place. Pre-set
/// reads see `None` and skip runtime sync. The disk-side row is
/// still written; the next daemon boot registers it into
/// `LocalRuntime` through the normal catalogue build.
pub type SharedHotRegistrarCell =
    std::sync::OnceLock<Arc<crate::runtime::axon_bridge::hot_agent_registrar::HotAgentRegistrar>>;

pub fn register(reg: &mut AxonAbilityCatalog, hot_registrar: Arc<SharedHotRegistrarCell>) {
    let registrar_for_start = Arc::clone(&hot_registrar);
    reg.register_rpc_with_owner(
        "device.agent.start",
        OwnerKind::Device,
        Arc::new(move |args: Value| start_agent_handler(args, &registrar_for_start)),
    );
    let registrar_for_stop = Arc::clone(&hot_registrar);
    reg.register_rpc_with_owner(
        "device.agent.stop",
        OwnerKind::Device,
        Arc::new(move |args: Value| stop_agent_handler(args, &registrar_for_stop)),
    );
    let registrar_for_refresh = Arc::clone(&hot_registrar);
    reg.register_rpc_with_owner(
        "device.agent.refresh",
        OwnerKind::Device,
        Arc::new(move |args: Value| refresh_agents_handler(args, &registrar_for_refresh)),
    );
}

/// `device.agent.start` handler.
///
/// Args: `{ "name": "claude", "agent_type": "claude-code", "model": "sonnet"? }`
/// or `{ "name": "claude", "entry": AgentEntry }`.
/// Behaviour:
///   1. Validate `name` (non-empty) and resolve the requested
///      runtime type from top-level `agent_type` or `entry.agent_type`.
///   2. Load the registry. This daemon handler is the canonical
///      writer for `agents.json`: the CLI sends primitive fields,
///      while direct ability callers may supply a full `entry` v2
///      row for advanced import/migration flows.
///   3. If no `entry` is supplied, insert a minimal
///      `AgentEntry::new(agent_type, model)` when missing and leave
///      an existing row untouched. This preserves the programmatic
///      `device.agent.start` contract for direct ability callers.
///   4. Register `<name>.{chat,discover,invoke}` into `LocalRuntime`
///      via the `HotAgentRegistrar` so subsequent Axon-routed
///      dispatch lands ledger rows (Phase 5c).
///   5. Return the Agent URA plus the runtime registration counters.
fn start_agent_handler(
    args: Value,
    hot_registrar: &SharedHotRegistrarCell,
) -> anyhow::Result<Value> {
    let name = args
        .get("name")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("device.agent.start: `name` (non-empty string) required"))?
        .to_string();
    let model = args
        .get("model")
        .and_then(Value::as_str)
        .map(str::to_string);
    let model_present = args
        .get("model_present")
        .and_then(Value::as_bool)
        .unwrap_or_else(|| args.get("model").is_some());
    let label = args
        .get("label")
        .and_then(Value::as_str)
        .map(str::to_string);
    let materialize_directory = args
        .get("materialize_directory")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let update_existing_spec = args
        .get("update_existing_spec")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let project_workspace = args
        .get("project_workspace")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let requested_root = args
        .get("root_path")
        .and_then(Value::as_str)
        .map(std::path::PathBuf::from);
    let provided_entry: Option<AgentEntry> = args
        .get("entry")
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|err| anyhow::anyhow!("device.agent.start: invalid `entry`: {err}"))?;
    let top_level_agent_type = args
        .get("agent_type")
        .and_then(Value::as_str)
        .map(AgentType::from_str)
        .transpose()?;
    let entry_agent_type = provided_entry.as_ref().map(|entry| entry.agent_type);
    let agent_type = match (top_level_agent_type, entry_agent_type) {
        (Some(top_level), Some(from_entry)) if top_level != from_entry => anyhow::bail!(
            "device.agent.start: top-level `agent_type` ({top_level}) does not match \
             `entry.agent_type` ({from_entry})"
        ),
        (Some(agent_type), _) => agent_type,
        (None, Some(agent_type)) => agent_type,
        (None, None) => anyhow::bail!(
            "device.agent.start: either top-level `agent_type` or `entry.agent_type` is required"
        ),
    };

    // Two real caller shapes reach this handler:
    //
    //   1. **CLI `easynet agent add`** - sends primitive fields plus
    //      materialization flags. The daemon creates/updates
    //      `agent.toml`, writes `agents.json`, and syncs runtime rows;
    //      the CLI no longer mutates those files directly.
    //   2. **Direct `device.agent.start` invocation** (rare:
    //      programmatic registration, integration tests) — may omit
    //      `entry`; in that case we keep an existing row or create a
    //      minimal one.
    let mut registry = agents::load_agents().unwrap_or_default();
    let existing_entry = registry.agents.get(&name).cloned();
    let replaced_prior = existing_entry.is_some();

    let mut materialized_directory: Option<AgentDirectory> = None;
    let mut created_directory = false;
    let mut updated_spec = false;
    if materialize_directory {
        let root = requested_root
            .or_else(|| {
                existing_entry
                    .as_ref()
                    .and_then(|entry| entry.root_path.clone())
            })
            .unwrap_or_else(|| config::agents_root().join(&name));
        if root.join("agent.toml").exists() && !replaced_prior {
            anyhow::bail!(
                "agent root at {} already carries an 'agent.toml' but no registry row. \
                 Import it by hand (add a registry row pointing at this path) or remove \
                 the directory before running `agent add`.",
                root.display()
            );
        }

        let directory = if root.join("agent.toml").exists() {
            let mut directory = AgentDirectory::open(&root)?;
            if update_existing_spec && model_present {
                directory.set_model(model.clone())?;
                updated_spec = true;
            }
            directory
        } else {
            let mut spec = AgentSpec::new(&name, runtime_kind_from(agent_type));
            if model_present {
                spec.model = model.clone();
            }
            if let Some(label) = label.as_ref() {
                spec.description = Some(label.clone());
            }
            created_directory = true;
            AgentDirectory::create(&Location::Local { root }, spec)?
        };
        materialized_directory = Some(directory);
    }

    let mut entry = if let Some(entry) = provided_entry {
        entry
    } else if let Some(existing) = existing_entry {
        if materialize_directory {
            let mut updated = existing;
            updated.agent_type = agent_type;
            if model_present {
                updated.with_model(model.clone());
            }
            if label.is_some() {
                updated.with_label(label.clone());
            }
            updated
        } else {
            existing
        }
    } else {
        AgentEntry::new(agent_type, model.clone())
    };

    if let Some(directory) = materialized_directory.as_ref() {
        normalize_v2_entry(&mut entry);
        entry.root_path = Some(directory.root().to_path_buf());
    }
    if label.is_some() {
        entry.with_label(label.clone());
    }
    registry.agents.insert(name.clone(), entry.clone());
    agents::save_agents(&registry)?;

    let mut workspace_projected = false;
    let mut workspace_projection_error: Option<String> = None;
    if project_workspace {
        if let Some(directory) = materialized_directory.as_ref() {
            match crate::runtime::workspace::ensure_from_directory(directory) {
                Ok(_) => workspace_projected = true,
                Err(err) => workspace_projection_error = Some(format!("{err:#}")),
            }
        }
    }

    // ── Phase 5c runtime registration ─────────────────────────────
    //
    // The runtime registration invariant: every name in
    // `agents.json` must own its `<name>.{chat,discover,invoke}`
    // triple in `LocalRuntime`. Without this, dispatch on a
    // hot-added agent is not visible to Axon's dispatch table and
    // `invocations.redb` never grows even though chat completes.
    //
    // The registrar produces byte-identical handlers to the
    // fallback (they share the same `build_*_handler_for`
    // factory closures in `chat_ability`), so the two paths are
    // observably equivalent for dispatch. Axon's `LocalRuntime`
    // owns the live call path and ledger write; the catalogue side
    // remains registration metadata.
    //
    // If the OnceLock is empty (boot not done) we log + skip.
    // The agent still lands in `agents.json`, so a daemon
    // restart picks it up via the boot-time
    // `chat_ability::register_for_agent` path. The window is small and only matters for the very
    // first agent added during boot - operators normally run
    // `agent add` post-boot.
    // Test path: tests in this file construct the handler with an
    // `empty_hot_registrar()` whose `OnceLock` is unset; the outer
    // `else` here emits a `hot_registrar_not_yet_wired_at_boot`
    // event and returns `None`, which is the documented test seam.
    // The production path goes through `Some(registrar)` below —
    // and once the registrar IS wired, the absence of a tokio runtime
    // is a real bug, not a legitimate skip. The helper also handles
    // current-thread tokio runtimes by offloading to a fresh runtime
    // thread, so all registrar sync sites share one bridge policy.
    let runtime_sync_outcome = if let Some(registrar) = hot_registrar.get() {
        let registrar = Arc::clone(registrar);
        let name_for_registrar = name.clone();
        let entry_for_registrar = entry.clone();
        let outcome = block_on_hot_registrar(async move {
            registrar
                .register_agent(&name_for_registrar, &entry_for_registrar)
                .await
        })
        .ok_or_else(|| {
            anyhow::anyhow!(
                "device.agent.start: hot_registrar is wired but no tokio runtime is \
                 available on the calling thread — handler must be driven from a \
                 tokio worker (see daemon boot in `axon_serve::boot`)"
            )
        })?;
        Some(outcome)
    } else {
        crate::op_event!(
            component = agent_lifecycle,
            kind = hot_agent_runtime_sync_skipped,
            agent_name = name.as_str(),
            reason = "hot_registrar_not_yet_wired_at_boot",
            message = "agent landed in agents.json but `LocalRuntime` did not get \
                       <agent>.{chat,discover,invoke} — daemon restart or next \
                       boot will pick it up via the static registration path",
        );
        None
    };

    let (runtime_registered, runtime_failed) = runtime_sync_outcome
        .map(|o| (o.registered, o.failed))
        .unwrap_or((0, 0));
    let agent_ura = agent_ura_for_name(&name);

    Ok(json!({
        "agent_ura": agent_ura,
        "replaced_prior": replaced_prior,
        "runtime_registered": runtime_registered,
        "runtime_failed": runtime_failed,
        "created_directory": created_directory,
        "updated_spec": updated_spec,
        "workspace_projected": workspace_projected,
        "workspace_projection_error": workspace_projection_error,
        "root_path": entry.root_path.as_ref().map(|path| path.to_string_lossy().to_string()),
        "agent_type": entry.agent_type.to_string(),
        "model": entry.model.clone(),
        "entry": entry.clone(),
    }))
}

fn runtime_kind_from(t: AgentType) -> RuntimeKind {
    match t {
        AgentType::ClaudeCode => RuntimeKind::ClaudeCode,
        AgentType::Codex => RuntimeKind::Codex,
        AgentType::CodexAppServer => RuntimeKind::CodexAppServer,
    }
}

fn normalize_v2_entry(entry: &mut AgentEntry) {
    entry.command.clear();
    entry.args.clear();
    entry.env.clear();
    entry.timeout_secs = agents::default_timeout_for_new_rows();
    entry.max_output_bytes = agents::default_max_output_for_new_rows();
    entry.schema_version = CURRENT_REGISTRY_SCHEMA;
}

/// `device.agent.stop` handler.
///
/// Args: `{ "name": "claude" }` or `{ "agent_ura": "easynet:///r/<realm>/agent/<user>.claude" }`.
/// Behaviour: remove the registry row. Idempotent — `ack=false` if
/// the row didn't exist; never errors on missing target.
fn stop_agent_handler(
    args: Value,
    hot_registrar: &SharedHotRegistrarCell,
) -> anyhow::Result<Value> {
    let mut registry = agents::load_agents().unwrap_or_default();
    let name = stop_agent_name_from_args(&args)?;
    let removed_entry = registry.agents.remove(&name);
    let ack = removed_entry.is_some();
    if ack {
        agents::save_agents(&registry)?;
    }

    // Phase 5c runtime-sync reverse: tear down every `<name>.*`
    // row from `LocalRuntime` in one atomic
    // `unregister_ability_by_prefix` call. Ordering is
    // "persist then update runtime" on the create side, so we follow the
    // same shape here: persist the removal first, then drop the
    // runtime rows. Doing it that way means a crash between
    // steps leaves the runtime in a "host registered but
    // agents.json doesn't know" state — equivalent to a stale
    // boot-time registration, which is harmless and self-heals
    // on next daemon restart (the boot path only registers
    // agents present in agents.json).
    // Symmetric to `start_agent_handler`: an unset registrar cell
    // is the documented test seam (with a warn-level event so a
    // production occurrence is operator-visible); a wired registrar
    // without a tokio runtime is a hard error rather than a silent
    // skip. Current-thread tokio runtimes are handled by the same
    // helper-thread runtime bridge as start/refresh.
    let runtime_removed = if ack {
        if let Some(registrar) = hot_registrar.get() {
            let registrar = Arc::clone(registrar);
            let name_for_registrar = name.clone();
            Some(
                block_on_hot_registrar(async move {
                    registrar.unregister_agent(&name_for_registrar).await
                })
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "device.agent.stop: hot_registrar is wired but no tokio runtime is \
                     available on the calling thread — handler must be driven from a \
                     tokio worker (see daemon boot in `axon_serve::boot`)"
                    )
                })?,
            )
        } else {
            crate::op_event!(
                component = agent_lifecycle,
                kind = hot_agent_runtime_sync_skipped,
                agent_name = name.as_str(),
                reason = "hot_registrar_not_yet_wired_at_boot",
                message = "agent row removed from agents.json but `LocalRuntime` did \
                           not drop its handlers — daemon restart will resync",
            );
            None
        }
    } else {
        None
    };

    Ok(json!({
        "ack": ack,
        "runtime_removed": runtime_removed.unwrap_or(0),
        "removed_entry": removed_entry,
    }))
}

/// `device.agent.refresh` handler.
///
/// Args: `{ "name": "claude"? }`.
/// Behaviour: re-register the selected agent, or every row in
/// `agents.json`, into the daemon-owned `LocalRuntime`. This is the
/// daemon-hosted replacement for the old CLI-side
/// runtime refresh sweep.
fn refresh_agents_handler(
    args: Value,
    hot_registrar: &SharedHotRegistrarCell,
) -> anyhow::Result<Value> {
    let requested_name = args
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let registry = agents::load_agents().unwrap_or_default();
    let rows: Vec<(String, AgentEntry)> = match requested_name.as_ref() {
        Some(name) => {
            let entry = registry.agents.get(name).cloned().ok_or_else(|| {
                anyhow::anyhow!("device.agent.refresh: agent {name:?} is not registered")
            })?;
            vec![(name.clone(), entry)]
        }
        None => registry
            .agents
            .iter()
            .map(|(name, entry)| (name.clone(), entry.clone()))
            .collect(),
    };

    let Some(registrar) = hot_registrar.get() else {
        // Boot-window state: agents.json exists but LocalRuntime is
        // not yet wired. Operator-visible event so a stuck cell is
        // not invisible.
        crate::op_event!(
            component = agent_lifecycle,
            kind = hot_agent_runtime_sync_skipped,
            reason = "hot_registrar_not_yet_wired_at_boot",
            message = "device.agent.refresh invoked before LocalRuntime hot \
                       registrar was wired",
        );
        return Ok(json!({
            "ok": false,
            "runtime_not_ready": true,
            "agents_scanned": rows.len(),
            "runtime_registered": 0,
            "runtime_failed": 0,
            "agents": [],
        }));
    };
    let registrar = Arc::clone(registrar);

    // Registrar IS wired — no tokio runtime here is a real bug
    // (the daemon always drives RPC handlers from a tokio worker).
    // Surface it as a hard error instead of returning a
    // `runtime_not_ready` envelope the operator might confuse with
    // the boot-window case above.
    let Some(agent_results) = block_on_hot_registrar(async move {
        let mut agent_results = Vec::with_capacity(rows.len());
        for (name, entry) in rows {
            let outcome = registrar.register_agent(&name, &entry).await;
            agent_results.push(json!({
                "name": name,
                "runtime_registered": outcome.registered,
                "runtime_failed": outcome.failed,
                "runtime_not_ready": outcome.runtime_not_ready,
            }));
        }
        agent_results
    }) else {
        return Err(anyhow::anyhow!(
            "device.agent.refresh: hot_registrar is wired but no tokio runtime is \
             available on the calling thread — handler must be driven from a \
             tokio worker (see daemon boot in `axon_serve::boot`)"
        ));
    };

    let runtime_registered = agent_results
        .iter()
        .filter_map(|row| row.get("runtime_registered").and_then(Value::as_u64))
        .sum::<u64>();
    let runtime_failed = agent_results
        .iter()
        .filter_map(|row| row.get("runtime_failed").and_then(Value::as_u64))
        .sum::<u64>();
    let runtime_not_ready = agent_results.iter().any(|row| {
        row.get("runtime_not_ready")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    });
    Ok(json!({
        "ok": !runtime_not_ready && runtime_failed == 0,
        "runtime_not_ready": runtime_not_ready,
        "agents_scanned": agent_results.len(),
        "runtime_registered": runtime_registered,
        "runtime_failed": runtime_failed,
        "agents": agent_results,
    }))
}

fn block_on_hot_registrar<F, T>(future: F) -> Option<T>
where
    F: std::future::Future<Output = T> + Send,
    T: Send,
{
    crate::support::async_bridge::try_run_blocking_in_tokio(future)
}

fn agent_ura_for_name(name: &str) -> String {
    if let Some(ura) = local_agents_ura_for_name(name) {
        return ura;
    }
    let (realm, user_id) = crate::persistence::config::load_credentials()
        .ok()
        .and_then(|creds| {
            let user_id = creds.username.filter(|s| !s.is_empty())?;
            (!creds.tenant_id.is_empty()).then_some((creds.tenant_id, user_id))
        })
        .unwrap_or_else(|| ("<unjoined>".to_string(), "<unjoined>".to_string()));
    crate::ura::agent_ura(&realm, &user_id, name)
}

fn local_agents_ura_for_name(name: &str) -> Option<String> {
    crate::persistence::local_agents::load()
        .ok()?
        .hosted_agents
        .into_iter()
        .find(|entry| {
            entry.profile == "llm"
                && entry.name == name
                && matches!(
                    crate::ura::parse_ura(&entry.agent_ura).map(|parsed| parsed.kind),
                    Ok(crate::ura::URAKind::Agent)
                )
        })
        .map(|entry| entry.agent_ura)
}

fn stop_agent_name_from_args(args: &Value) -> anyhow::Result<String> {
    let name = args
        .get("name")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty());
    let agent_ura = args
        .get("agent_ura")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty());
    match (name, agent_ura) {
        (Some(name), None) => Ok(name.to_string()),
        (None, Some(ura)) => agent_name_from_ura(ura),
        (Some(name), Some(ura)) => {
            let from_ura = agent_name_from_ura(ura)?;
            if from_ura != name {
                anyhow::bail!(
                    "device.agent.stop: `name` ({name}) does not match `agent_ura` ({from_ura})"
                );
            }
            Ok(name.to_string())
        }
        (None, None) => {
            anyhow::bail!("device.agent.stop: either `name` or `agent_ura` is required")
        }
    }
}

fn agent_name_from_ura(ura: &str) -> anyhow::Result<String> {
    let parsed = crate::ura::parse_ura(ura)
        .map_err(|err| anyhow::anyhow!("device.agent.stop: invalid `agent_ura`: {err}"))?;
    if parsed.kind != crate::ura::URAKind::Agent {
        anyhow::bail!("device.agent.stop: `agent_ura` must be an Agent URA");
    }
    if let Some(entry) = crate::persistence::local_agents::load()
        .ok()
        .and_then(|file| {
            file.hosted_agents
                .into_iter()
                .find(|entry| entry.profile == "llm" && entry.agent_ura == ura)
        })
    {
        return Ok(entry.name);
    }
    Ok(parsed.agent_id)
}

// ── Discovery surfaces ────────────────────────────────────────

pub fn start_agent_input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["name"],
        "anyOf": [
            { "required": ["agent_type"] },
            { "required": ["entry"] }
        ],
        "properties": {
            "name":       { "type": "string", "minLength": 1 },
            "agent_type": { "type": "string",
                            "enum": ["claude-code", "claude", "codex",
                                     "codex-app-server", "codex-appserver"] },
            "model":      { "type": ["string", "null"] },
            "model_present": { "type": "boolean" },
            "label":      { "type": "string" },
            "root_path":  { "type": "string" },
            "entry": agent_entry_input_schema(),
            "materialize_directory": { "type": "boolean" },
            "update_existing_spec": { "type": "boolean" },
            "project_workspace": { "type": "boolean" },
        },
        "additionalProperties": false,
    })
}

fn agent_entry_input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["agent_type"],
        "properties": {
            "schema_version": { "type": "integer", "minimum": 0 },
            "root_path": { "type": ["string", "null"] },
            "agent_type": {
                "type": "string",
                "enum": ["claude-code", "claude", "codex", "codex-app-server", "codex-appserver"]
            },
            "command": { "type": "string" },
            "args": {
                "type": "array",
                "items": { "type": "string" }
            },
            "model": { "type": ["string", "null"] },
            "label": { "type": ["string", "null"] },
            "env": {
                "type": "object",
                "additionalProperties": { "type": "string" }
            },
            "timeout_secs": { "type": "integer", "minimum": 0 },
            "max_output_bytes": { "type": "integer", "minimum": 0 }
        },
        "additionalProperties": false
    })
}

pub fn start_agent_description() -> &'static str {
    "Register a new LLM sub-agent (claude/codex/…) in the device's \
     agent registry. This is the daemon-hosted surface behind \
     `easynet agent add`; callers may send primitive fields or a rich \
     entry object. Returns agent_ura and replaced_prior=true means the \
     call overwrote an existing row."
}

pub fn stop_agent_input_schema() -> Value {
    json!({
        "type": "object",
        "anyOf": [
            { "required": ["name"] },
            { "required": ["agent_ura"] }
        ],
        "properties": {
            "name": { "type": "string", "minLength": 1 },
            "agent_ura": { "type": "string", "minLength": 1 },
        },
        "additionalProperties": false,
    })
}

pub fn stop_agent_description() -> &'static str {
    "Remove an LLM sub-agent registry row by name or Agent URA. \
     Idempotent: ack=false when the row didn't exist. Workspace files \
     are deliberately NOT deleted — use `easynet agent remove --purge` for that."
}

pub fn refresh_agents_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "name": { "type": "string", "minLength": 1 },
        },
        "additionalProperties": false,
    })
}

pub fn refresh_agents_description() -> &'static str {
    "Re-register the LocalRuntime handlers for one or all LLM \
     sub-agents declared in agents.json. With `name` set, only that \
     agent's `<agent>.*` handlers and ability TOMLs are rebuilt; \
     with `name` omitted, every row in agents.json is rebuilt. This \
     is the daemon-hosted surface behind `easynet agent refresh` and \
     re-reads `<agent-root>/abilities/*.ability.toml` from disk."
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test fixture: route `~/.easynet/` at a fresh tempdir for the
    /// duration of `f`. Uses the canonical `test_support::HomeGuard`
    /// — same fixture as registry::agents tests and the dispatch suite
    /// — which (a) acquires a process-global mutex so two HomeGuards
    /// never run concurrently and (b) sets `HOME` (the var
    /// `config::home_dir()` actually reads), not `EASYNET_HOME`.
    ///
    /// History: an earlier draft of this fixture set `EASYNET_HOME`
    /// (which `config::home_dir()` ignores) and only PID+nanos-keyed
    /// the tempdir, leaving the env-var racing under parallel tests.
    /// See `docs/rfc/AXON-RFC-001-flake-localization.md` (2026-04-27).
    fn with_isolated_home<F: FnOnce()>(f: F) {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        f();
    }

    /// Empty hot-registrar cell for unit tests. The handlers see
    /// `get() == None` and skip `LocalRuntime` registration entirely
    /// — every test in this module only validates the disk-side
    /// (`agents.json`) semantics.
    fn empty_hot_registrar() -> SharedHotRegistrarCell {
        SharedHotRegistrarCell::new()
    }

    #[test]
    fn registration_makes_lifecycle_abilities_dispatchable() {
        let mut reg = AxonAbilityCatalog::new();
        register(&mut reg, Arc::new(empty_hot_registrar()));
        assert!(reg.get_rpc(ABILITY_START_AGENT).is_some());
        assert!(reg.get_rpc(ABILITY_STOP_AGENT).is_some());
        assert!(reg.get_rpc(ABILITY_REFRESH_AGENTS).is_some());
    }

    #[test]
    fn start_agent_persists_row_and_returns_runtime_counters() {
        with_isolated_home(|| {
            let resp = start_agent_handler(
                json!({
                    "name": "claude",
                    "agent_type": "claude-code",
                    "model": "sonnet",
                }),
                &empty_hot_registrar(),
            )
            .unwrap();
            assert_eq!(resp["replaced_prior"], false);
            assert_eq!(resp["runtime_registered"], 0);
            assert_eq!(resp["runtime_failed"], 0);
            assert_eq!(
                resp["agent_ura"],
                json!(crate::ura::agent_ura("<unjoined>", "<unjoined>", "claude"))
            );

            // Round-trip: the registry now has the row.
            let registry = agents::load_agents().unwrap();
            assert!(registry.agents.contains_key("claude"));
        });
    }

    #[test]
    fn start_agent_preserves_existing_row_and_signals_replaced_prior() {
        // Phase 5c semantic: a second `device.agent.start` call on
        // an existing row LEAVES the stored entry alone (so the CLI
        // `agent add` flow's rich v2 row survives the daemon's
        // notify-back hop) but still flags `replaced_prior=true`
        // so the operator can see the row existed.
        with_isolated_home(|| {
            start_agent_handler(
                json!({
                    "name": "claude",
                    "agent_type": "claude-code",
                }),
                &empty_hot_registrar(),
            )
            .unwrap();
            let resp = start_agent_handler(
                json!({
                    "name": "claude",
                    "agent_type": "codex",
                }),
                &empty_hot_registrar(),
            )
            .unwrap();
            assert_eq!(
                resp["replaced_prior"], true,
                "second insertion of same name MUST flag replaced_prior=true"
            );
            // Preserve invariant: agent_type stays `claude-code`
            // even though the second call asked for `codex`. The
            // stored row is authoritative; the daemon-side notify
            // is runtime-registration-only.
            let registry = agents::load_agents().unwrap();
            assert_eq!(
                registry.agents.get("claude").unwrap().agent_type,
                AgentType::ClaudeCode,
                "second start of same name MUST NOT overwrite the stored row's agent_type"
            );
        });
    }

    #[test]
    fn start_agent_rejects_missing_name() {
        let err = start_agent_handler(json!({"agent_type": "claude-code"}), &empty_hot_registrar())
            .unwrap_err();
        assert!(format!("{err}").contains("name"));
    }

    #[test]
    fn start_agent_accepts_rich_entry_without_top_level_agent_type() {
        with_isolated_home(|| {
            let resp = start_agent_handler(
                json!({
                    "name": "codex-rich",
                    "entry": {
                        "agent_type": "codex",
                        "model": "gpt-5",
                        "label": "Codex rich row"
                    }
                }),
                &empty_hot_registrar(),
            )
            .unwrap();
            assert_eq!(resp["agent_type"], "codex");
            assert_eq!(resp["model"], "gpt-5");

            let registry = agents::load_agents().unwrap();
            let stored = registry.agents.get("codex-rich").unwrap();
            assert_eq!(stored.agent_type, AgentType::Codex);
            assert_eq!(stored.model.as_deref(), Some("gpt-5"));
        });
    }

    #[test]
    fn start_agent_rejects_conflicting_top_level_and_entry_agent_type() {
        let err = start_agent_handler(
            json!({
                "name": "conflict",
                "agent_type": "claude-code",
                "entry": {
                    "agent_type": "codex"
                }
            }),
            &empty_hot_registrar(),
        )
        .unwrap_err();
        assert!(format!("{err}").contains("does not match"));
    }

    #[test]
    fn start_agent_rejects_missing_agent_type_when_entry_absent() {
        let err = start_agent_handler(json!({"name": "x"}), &empty_hot_registrar()).unwrap_err();
        assert!(format!("{err}").contains("agent_type"));
    }

    #[test]
    fn start_agent_materialize_reuses_existing_root_path_when_root_omitted() {
        with_isolated_home(|| {
            let custom_root = crate::persistence::config::home_dir()
                .join("project")
                .join("agents")
                .join("claude");
            start_agent_handler(
                json!({
                    "name": "claude",
                    "agent_type": "claude-code",
                    "model": "sonnet",
                    "root_path": custom_root,
                    "materialize_directory": true,
                }),
                &empty_hot_registrar(),
            )
            .unwrap();

            let resp = start_agent_handler(
                json!({
                    "name": "claude",
                    "agent_type": "claude-code",
                    "model": "opus",
                    "model_present": true,
                    "materialize_directory": true,
                    "update_existing_spec": true,
                }),
                &empty_hot_registrar(),
            )
            .unwrap();

            let stored_root = agents::load_agents().unwrap().agents["claude"]
                .root_path
                .clone()
                .unwrap();
            assert_eq!(
                resp["root_path"],
                json!(stored_root.to_string_lossy().to_string())
            );
            assert!(
                stored_root.ends_with("project/agents/claude"),
                "must preserve project-local root, got {}",
                stored_root.display()
            );
            let spec = AgentSpec::from_toml_str(
                &std::fs::read_to_string(stored_root.join("agent.toml")).unwrap(),
            )
            .unwrap();
            assert_eq!(spec.model.as_deref(), Some("opus"));
        });
    }

    #[test]
    fn start_agent_rejects_unknown_agent_type() {
        let err = start_agent_handler(
            json!({
                "name": "x",
                "agent_type": "totally-not-a-runtime",
            }),
            &empty_hot_registrar(),
        )
        .unwrap_err();
        assert!(format!("{err}").contains("unknown agent type"));
    }

    #[test]
    fn stop_agent_by_name_acks_true_and_removes_row() {
        with_isolated_home(|| {
            start_agent_handler(
                json!({
                    "name": "claude",
                    "agent_type": "claude-code",
                }),
                &empty_hot_registrar(),
            )
            .unwrap();

            let resp =
                stop_agent_handler(json!({"name": "claude"}), &empty_hot_registrar()).unwrap();
            assert_eq!(resp["ack"], true);
            assert!(!agents::load_agents().unwrap().agents.contains_key("claude"));
        });
    }

    #[test]
    fn stop_agent_idempotent_returns_ack_false_when_row_missing() {
        with_isolated_home(|| {
            // Never registered; stop should report ack=false (not error).
            let resp =
                stop_agent_handler(json!({"name": "ghost"}), &empty_hot_registrar()).unwrap();
            assert_eq!(resp["ack"], false);
        });
    }

    #[test]
    fn stop_agent_by_agent_ura_resolves_agent_tail() {
        with_isolated_home(|| {
            start_agent_handler(
                json!({
                    "name": "claude",
                    "agent_type": "claude-code",
                }),
                &empty_hot_registrar(),
            )
            .unwrap();
            let agent_ura = crate::ura::agent_ura("acme", "u1", "claude");
            let resp = stop_agent_handler(json!({"agent_ura": agent_ura}), &empty_hot_registrar())
                .unwrap();
            assert_eq!(resp["ack"], true);
            assert!(!agents::load_agents().unwrap().agents.contains_key("claude"));
        });
    }

    #[test]
    fn stop_agent_rejects_non_agent_ura() {
        let err = stop_agent_handler(
            json!({"agent_ura": crate::ura::device_ura("acme", "device-1")}),
            &empty_hot_registrar(),
        )
        .unwrap_err();
        assert!(format!("{err}").contains("Agent URA"));
    }

    #[test]
    fn input_schemas_have_required_fields_pinned() {
        let s = start_agent_input_schema();
        let req = s["required"].as_array().unwrap();
        assert!(req.iter().any(|v| v == "name"));
        assert!(
            !req.iter().any(|v| v == "agent_type"),
            "top-level agent_type must stay optional because rich `entry` can carry it"
        );
        let alternatives = s["anyOf"].as_array().expect("start anyOf");
        assert!(alternatives
            .iter()
            .any(|shape| shape["required"][0] == "agent_type"));
        assert!(alternatives
            .iter()
            .any(|shape| shape["required"][0] == "entry"));
        assert!(s["properties"].get("entry").is_some());
        assert_eq!(s["properties"]["entry"]["required"][0], "agent_type");
        assert_eq!(s["additionalProperties"], false);

        let s = stop_agent_input_schema();
        let alternatives = s["anyOf"].as_array().expect("stop anyOf");
        assert!(alternatives
            .iter()
            .any(|shape| shape["required"][0] == "name"));
        assert!(alternatives
            .iter()
            .any(|shape| shape["required"][0] == "agent_ura"));
        assert!(s["properties"].get("name").is_some());
        assert!(s["properties"].get("agent_ura").is_some());
        assert_eq!(s["additionalProperties"], false);
        assert!(refresh_agents_description().contains("re-reads"));
    }
}
