//! facade::mcp — quarantined RFC-001 §A3 edge adapter (P4.8d)
//!
//! Per the binding contract from plan §21 and the user's P4.8d
//! decision (Option 2: Quarantine):
//!
//!   1. `easynet mcp_server` and `easynet start --mcp` may remain
//!      as user-facing compatibility entrypoints.
//!   2. They become thin shims into the mcp-profile Agent.
//!   3. All tool calls translate into in-process Invoke against the
//!      real owner Agent's ability.
//!   4. No direct CallMCPTool / runtime_local_tools / system.* path
//!      may remain.
//!   5. facade/mcp must not own separate routing, catalog, or
//!      dispatch semantics.
//!   6. Conformance allows facade/mcp ONLY when it imports/uses
//!      mcp-profile and contains no independent tool registry.
//!
//! What lived here pre-quarantine
//! ------------------------------
//! Seven submodules totaling ~3.9k LOC:
//!   * handlers.rs       — 16 handler functions calling deleted
//!                         DendriteBridge methods (every single one
//!                         returned `AxonError::Bridge("...removed
//!                         by P1.5...")` since the SDK collapse)
//!   * specs.rs          — duplicate tool catalog (16 hard-coded
//!                         JSON Schemas; replicated info that
//!                         AbilityDescriptor now carries
//!                         authoritatively)
//!   * provider.rs       — HubMcpProvider; the McpToolProvider
//!                         impl that hand-routed to handlers.rs
//!   * agent_dispatch.rs — duplicate `<agent>.chat` dispatch
//!                         (now lives in `runtime::ability_dispatch`
//!                         and AbilityProxy)
//!   * bound_node.rs     — node-scoped argument patching
//!   * error.rs          — wrapper error type
//!   * server.rs         — placeholder
//!
//! Where the surviving behaviour lives now
//! ---------------------------------------
//!   * Tool catalog: `runtime::agents::profiles::all_descriptors_for_host`
//!   * MCP-shape projection: `runtime::agents::profiles::mcp::tool_specs_from_descriptors`
//!   * Tool dispatch: `runtime::agents::profiles::mcp::InvokeMcpProvider`
//!     (drives the in-process AbilityProxy via ProxyLocalInvoker)
//!   * Stdio server scaffolding: `easynet_axon::mcp::StdioMcpServer`
//!     (unchanged — it's the SDK trait surface)
//!
//! What's left in this directory
//! -----------------------------
//! Nothing — this `mod.rs` is empty by design and is the conformance
//! anchor. The CLI command `easynet mcp_server` (in
//! `facade/cli/mcp_server.rs`) constructs `InvokeMcpProvider`
//! directly. The `--mcp` branch in `facade/cli/start.rs` does the
//! same. There is no facade-layer code path that owns MCP semantics.
//!
//! Author: Silan Hu <silan.hu@u.nus.edu>
//! Copyright (c) 2026 EasyNet. All rights reserved.
