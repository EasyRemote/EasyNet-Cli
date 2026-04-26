// EasyNet CLI — `easynet mcp_server`
// ===================================
//
// File: src/facade/cli/mcp_server.rs
//
// User-facing entrypoint for the stdio MCP server. Per RFC-001 §A3,
// MCP is permitted only at edge adapters; this command IS such an
// edge adapter. Every tool call it accepts MUST translate into an
// in-process Invoke against the right Agent's ability — no direct
// bridge calls, no duplicate tool catalog. P4.8d quarantine.
//
// Wire shape vs. pre-RFC behaviour
// --------------------------------
// Pre-RFC the server advertised 16 hard-coded tools (hub_status,
// list_devices, deploy_ability, …) backed by direct DendriteBridge
// calls. Every one of those tools dispatched through SDK methods
// that P1.5 stubbed to `AxonError::Bridge("...removed by P1.5...")`
// — which means the entire pre-RFC tool surface had been silently
// broken since the SDK collapse.
//
// P4.8d replaces the catalog with the host's AbilityDescriptors,
// projected to MCP shape by `runtime::agents::profiles::mcp`. Each
// tool name is the canonical ability_name (`observe.health`,
// `fleet.list_agents`, `consent.subscribe`, …); each `tools/call`
// goes through the in-process AbilityProxy. The dispatch path is
// the same one the IPC server uses, so a tool that works through
// `easynetd ipc` works identically through `easynet mcp_server`.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use anyhow::Context;
use clap::Args;

#[derive(Debug, Args)]
pub struct McpServerArgs {
    /// Tenant ID. Surfaces in audit logs only; the actual dispatch
    /// honours whatever tenant the loaded credentials carry.
    #[arg(long, default_value = "default")]
    pub tenant: String,
    /// Agent label (informational; included in server name).
    #[arg(long)]
    pub agent: Option<String>,
}

pub fn run(args: McpServerArgs) -> anyhow::Result<()> {
    use crate::runtime::agents::profiles::mcp as mcp_profile;
    use crate::services::control::ability_proxy::AbilityProxy;

    // Build the AbilityProxy with the live registry. We construct a
    // kernel-shaped KernelApi handle around a NoopGateway because
    // `easynet mcp_server` is a one-shot edge adapter: ability
    // dispatch goes through the in-process registry, not through
    // any remote node.
    let gateway: std::sync::Arc<dyn crate::runtime::gateway_api::GatewayApi> =
        std::sync::Arc::new(crate::runtime::gateway::NoopGateway::new());
    let kernel: std::sync::Arc<dyn crate::runtime::kernel_api::KernelApi> =
        std::sync::Arc::new(crate::runtime::kernel::Kernel::new(gateway));
    let proxy = std::sync::Arc::new(AbilityProxy::new(kernel));

    // Build the descriptor list the way `runtime::publish` does for
    // federation.advertise_*. The MCP catalog and the federation
    // catalog share one source of truth — RFC §1.6.
    //
    // Pre-join state: descriptors are anchored on whatever
    // host_device_uri local-agents.json carries, defaulting to "self"
    // when unset. The descriptor names themselves are stable; only
    // the owner_agent_uri varies.
    let local_agents = crate::persistence::local_agents::load().unwrap_or_default();
    let host_uri = if local_agents.host_device_agent_uri.is_empty() {
        "self".to_string()
    } else {
        local_agents.host_device_agent_uri.clone()
    };
    let consent_uri = crate::persistence::local_agents::lookup_hosted_uri(
        &local_agents,
        "consent",
        "default",
    );
    let policy_uri = crate::persistence::local_agents::lookup_hosted_uri(
        &local_agents,
        "policy",
        "default",
    );
    let mcp_uri = crate::persistence::local_agents::lookup_hosted_uri(
        &local_agents,
        "mcp",
        "default",
    );
    let llm_uris: Vec<(String, String)> = local_agents
        .hosted_agents
        .iter()
        .filter(|e| e.profile == "llm")
        .map(|e| (e.name.clone(), e.agent_uri.clone()))
        .collect();
    let descriptors = crate::runtime::agents::profiles::all_descriptors_for_host(
        &host_uri,
        consent_uri.as_deref(),
        policy_uri.as_deref(),
        mcp_uri.as_deref(),
        &llm_uris,
    );

    let invoker = mcp_profile::ProxyLocalInvoker::new(proxy);
    let provider = mcp_profile::InvokeMcpProvider::new(invoker, descriptors);

    eprintln!(
        "[easynet mcp] tenant={} agent={} advertising {} tools (RFC-001 §A3 edge adapter)",
        args.tenant,
        args.agent.as_deref().unwrap_or("?"),
        provider.descriptor_count(),
    );

    let server_name = format!(
        "easynet-mcp-{}",
        args.agent.as_deref().unwrap_or("device")
    );
    let server = easynet_axon::mcp::StdioMcpServer::new(provider)
        .with_server_name(server_name)
        .with_server_version(env!("CARGO_PKG_VERSION"));
    server
        .run(std::io::stdin().lock(), &mut std::io::stdout())
        .context("mcp server")
}
