// EasyNet CLI — MCP Server
// ========================
//
// File: src/mcp/mod.rs
// Description: Hub-level MCP server implementation for Claude Code / Codex integration.
//
// Architecture:
//   error.rs       — structured `McpError` with stable `error_code` strings
//   specs.rs       — tool schema definitions (pure data: the 11 base
//                    JSON Schema input specs, no transformation logic)
//   bound_node.rs  — single home for the bound-node abstraction: the
//                    NODE_SCOPED_TOOLS membership list plus the two
//                    transforms it drives (schema patching at spec time,
//                    argument patching at dispatch time)
//   handlers.rs    — per-tool handler implementations (DendriteBridge +
//                    EAL calls)
//   provider.rs    — `HubMcpProvider`: McpToolProvider implementation
//                    with cached bridge connection and a persistent
//                    mission BridgePool
//   server.rs      — (placeholder; actual server runner is
//                    cli/mcp_server.rs)
//
// The MCP server exposes Hub-level operations (device listing, ability management,
// remote execution, EAL mission orchestration) as standard MCP tools over stdio.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

pub(crate) mod bound_node;
pub(crate) mod error;
pub(crate) mod handlers;
pub(crate) mod provider;
pub(crate) mod server;
pub(crate) mod specs;
