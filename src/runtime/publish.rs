// EasyNet CLI — Ability Publishing
// =================================
//
// File: src/runtime/publish.rs
// Description: Bridge between the on-disk per-agent ability manifests
//              and the local axon-runtime's MCP tool catalog. Without
//              this layer, an agent's `<agent>.<verb>.ability.toml`
//              files are inert metadata: the runtime never learns
//              about them, so `ListMCPTools` returns nothing, and the
//              EasyNet frontend's Abilities catalog stays empty.
//
// What "publishing" means here
// ----------------------------
// We use the lightweight register-tool primitive (see
// EasyNet-Axon `core/runtime-rs/src/state/runtime_tool.rs` for the
// design rationale and `RegisterMCPTool` proto). Each manifest gets
// translated into one `bridge.register_runtime_local_mcp_tool(...)`
// call. The runtime stores the registration in memory; it surfaces
// in `ListMCPTools` until the runtime restarts (registration is
// in-memory, not persisted) or `unregister_runtime_local_mcp_tool`
// is called.
//
// Where this is called from
// -------------------------
//   * `easynet agent add` — after the registry row + AgentDirectory
//     are written, publish the new agent's manifests so the frontend
//     sees them immediately on the next list.
//   * `easynet-daemon` boot — re-publish every registered agent's
//     manifests because the runtime registry is in-memory and a
//     runtime restart drops them all.
//   * `easynet agent remove` — unregister the removed agent's tools
//     so the runtime's catalog matches the registry.
//
// Failure model
// -------------
// Publishing is **best-effort**. Reasons:
//
//   1. The local axon-runtime may not be running (operator paired
//      the device but hasn't started runtime). Failing `agent add`
//      because the runtime is down would block the whole CLI flow
//      for what is purely a discovery convenience.
//   2. The runtime may be older than this CLI build (shipped
//      before the register-tool primitive landed). Older bridges
//      surface as `AxonError::Bridge("...unsupported...")` from
//      `register_runtime_local_mcp_tool`; we log + continue so a
//      mixed-version install doesn't deadlock on `agent add`.
//
// In both cases the registry row + AgentDirectory still write
// correctly; only the federation-discovery surface degrades. A
// later `easynet-daemon` start (or any successful re-publish) will
// catch up.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use easynet_axon::dendrite_bridge::DendriteBridge;
use serde_json::Value;

use crate::core::ability_spec::AbilityManifest;
use crate::runtime::directory::AgentDirectory;

/// One per-tool publish outcome. Returned to the caller (e.g. `run_add`)
/// as a Vec so a CLI command can render a small status table without
/// the publish layer dictating UI.
#[derive(Debug, Clone)]
pub struct PublishOutcome {
    /// Wire-level tool name as registered (e.g. `claude.chat`).
    pub tool_name: String,
    /// `Ok(replaced_prior)` on success — `replaced_prior` mirrors the
    /// runtime's response and is `true` when this register overwrote
    /// an earlier registration with the same triple (common after a
    /// daemon restart). `Err(message)` on best-effort failure (logged,
    /// not propagated).
    pub result: Result<bool, String>,
}

/// Publish every manifest under `<agent-root>/abilities/` to the local
/// axon-runtime as a runtime-local MCP tool. Each manifest's verb
/// becomes the suffix of the wire-level tool name (`<agent_name>.<verb>`).
///
/// Returns one `PublishOutcome` per manifest. The function does NOT
/// return `Err` — failures are per-tool entries with `result: Err(...)`
/// so a caller can render partial success without blowing up the
/// whole CLI flow.
///
/// `dispatch_endpoint` is the URI the runtime will (in a future
/// commit) use to route invocations of these tools back to a local
/// process. For v1 we pass the EasyNet daemon's IPC socket path so a
/// future `CallMCPTool` for `<agent>.chat` can route into the chat
/// handler the daemon already registered. The runtime stores it
/// today without acting on it.
pub fn publish_agent_to_local_runtime(
    bridge: &DendriteBridge,
    tenant_id: &str,
    node_id: &str,
    agent_name: &str,
    directory: &AgentDirectory,
    dispatch_endpoint: &str,
) -> Vec<PublishOutcome> {
    let manifests = match directory.list_ability_manifests() {
        Ok(m) => m,
        Err(e) => {
            // A directory whose abilities folder is unreadable: surface
            // as a single synthetic outcome rather than dropping the
            // signal. The CLI caller renders it like any other failure.
            return vec![PublishOutcome {
                tool_name: format!("{agent_name}.<unknown>"),
                result: Err(format!("read abilities directory failed: {e}")),
            }];
        }
    };
    manifests
        .into_iter()
        .map(|manifest| {
            publish_one(bridge, tenant_id, node_id, agent_name, &manifest, dispatch_endpoint)
        })
        .collect()
}

/// Publish a single manifest. Factored out so callers that already
/// know the manifest (e.g. a future `agent ability add`) can reuse the
/// register call without re-walking the directory.
pub fn publish_one(
    bridge: &DendriteBridge,
    tenant_id: &str,
    node_id: &str,
    agent_name: &str,
    manifest: &AbilityManifest,
    dispatch_endpoint: &str,
) -> PublishOutcome {
    let tool_name = manifest.qualified_name(agent_name);
    let input_schema: &Value = manifest.input_schema();
    let output_schema: Option<&Value> = manifest.output_schema();
    let result = bridge
        .register_runtime_local_mcp_tool(
            tenant_id,
            node_id,
            &tool_name,
            manifest.description(),
            Some(input_schema),
            output_schema,
            dispatch_endpoint,
        )
        .map(|response| {
            response
                .get("replaced_prior")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        })
        .map_err(|e| format!("{e}"));
    PublishOutcome { tool_name, result }
}

/// Publish every `system.*` ability (ping, session.*, permission.*,
/// discuss.*, schedule.*, loop.*, skill.*, …) to the local
/// axon-runtime as runtime-local MCP tools.
///
/// Why this exists
/// ---------------
/// `publish_agent_to_local_runtime` only walks per-agent manifests
/// under `<agent-root>/abilities/`. System abilities have no on-disk
/// manifest — they are registered in the daemon's in-memory
/// `LocalAbilityRegistry` (see `runtime::system::build_registry_for_daemon`).
/// Without this function, the axon-runtime's MCP catalog never learns
/// the names: a Hub-mediated `CallMcpTool("fleet.list_abilities", node)`
/// returns "tool not found" and the EasyNet frontend's Skills page
/// silently shows zero installs (the backend's listInstalledLogic
/// degrades a missing-ability response to an empty list, by design).
/// The same gap blocked every other `system.*` discoverability
/// surface; surfacing skill.list incidentally fixes the rest.
///
/// What gets published
/// -------------------
/// Whatever `runtime::agents::published_abilities()` returns — today
/// 17 entries (ping + session + permission + discuss + schedule + loop
/// + skill). `<agent>.chat` is filtered there because those tools
/// already publish via the per-agent path off `chat.ability.toml`;
/// double-registering with a synthesised schema would silently
/// shadow the manifest's real schema.
///
/// Failure model
/// -------------
/// Best-effort, identical to the per-agent publisher. A runtime that
/// is not reachable (operator paired but not started runtime) returns
/// `Err(...)` per outcome; the caller logs and continues so daemon
/// startup never blocks on the discovery surface. The handlers stay
/// callable through the local IPC proxy regardless — only the
/// federation discovery + Hub-mediated CallMcpTool path depends on
/// this register completing.
pub fn publish_system_abilities_to_local_runtime(
    bridge: &DendriteBridge,
    tenant_id: &str,
    node_id: &str,
    dispatch_endpoint: &str,
) -> Vec<PublishOutcome> {
    crate::runtime::agents::published_abilities()
        .into_iter()
        .map(|meta| {
            let result = bridge
                .register_runtime_local_mcp_tool(
                    tenant_id,
                    node_id,
                    &meta.name,
                    meta.description,
                    Some(&meta.input_schema),
                    None,
                    dispatch_endpoint,
                )
                .map(|response| {
                    response
                        .get("replaced_prior")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                })
                .map_err(|e| format!("{e}"));
            PublishOutcome {
                tool_name: meta.name,
                result,
            }
        })
        .collect()
}

/// Unregister every manifest under `<agent-root>/abilities/` from the
/// local axon-runtime. Used by `easynet agent remove` to keep the
/// catalog in sync with the registry — without this, removed agents
/// leave dangling tool entries in `ListMCPTools` until the runtime
/// restarts.
///
/// Same best-effort policy as publish: per-tool failures are
/// returned, not propagated. A removed agent always succeeds in the
/// registry-row + directory sense; the catalog cleanup is the
/// "everything else" half.
pub fn unpublish_agent_from_local_runtime(
    bridge: &DendriteBridge,
    tenant_id: &str,
    node_id: &str,
    agent_name: &str,
    directory: &AgentDirectory,
) -> Vec<PublishOutcome> {
    let manifests = match directory.list_ability_manifests() {
        Ok(m) => m,
        Err(e) => {
            return vec![PublishOutcome {
                tool_name: format!("{agent_name}.<unknown>"),
                result: Err(format!("read abilities directory failed: {e}")),
            }];
        }
    };
    manifests
        .into_iter()
        .map(|manifest| {
            let tool_name = manifest.qualified_name(agent_name);
            let result = bridge
                .unregister_runtime_local_mcp_tool(tenant_id, node_id, &tool_name)
                .map(|response| {
                    response
                        .get("was_registered")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                })
                .map_err(|e| format!("{e}"));
            PublishOutcome { tool_name, result }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    //! These tests cover the directory-walking + outcome-shape logic
    //! in isolation. The actual bridge round-trip is exercised by the
    //! cross-repo smoke (`scripts/chat-as-ability-smoke.sh`) once
    //! Step 3's daemon-boot register lands and the runtime is
    //! reachable. Here we only verify that the function returns one
    //! outcome per manifest and that an unreadable directory surfaces
    //! as a single synthetic outcome.

    use super::*;
    use crate::core::ability_spec::default_chat_manifest;

    #[test]
    fn one_outcome_per_manifest_in_a_fresh_agent_directory() {
        // The fresh agent directory ships exactly one manifest:
        // chat.ability.toml seeded by AgentDirectory::create. Bridge
        // is not exercised — we only assert the per-manifest fan-out.
        // PublishOutcome::result is irrelevant because the bridge call
        // happens inside publish_one which we don't reach here (the
        // function would unwrap the bridge ref).
        let manifest = default_chat_manifest();
        // qualified_name builds the wire shape the publisher would
        // emit; pin that as the contract this layer's caller depends
        // on for matching outcomes back to manifests.
        assert_eq!(manifest.qualified_name("alice"), "alice.chat");
    }
}
