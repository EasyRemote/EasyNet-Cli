// EasyNet CLI — MCP Server
// ========================
//
// File: src/mcp/mod.rs
// Description: Hub-level MCP server implementation for Claude Code / Codex integration.
//
// Architecture:
//   specs.rs     — tool schema definitions (11 tools, JSON Schema input specs)
//   handlers.rs  — per-tool handler implementations (DendriteBridge + EAL calls)
//   hub_kit.rs   — McpToolProvider implementation with cached bridge connection
//   server.rs    — (placeholder; actual server runner is cli/mcp_server.rs)
//
// The MCP server exposes Hub-level operations (device listing, ability management,
// remote execution, EAL mission orchestration) as standard MCP tools over stdio.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

pub mod handlers;
pub mod hub_kit;
pub mod server;
pub mod specs;

