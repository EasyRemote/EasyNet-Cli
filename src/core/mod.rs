// EasyNet CLI — Core Layer
// ========================
//
// File: src/core/mod.rs
// Description: Zero-dependency ontology types shared across every
//              other subsystem (runtime, eal, registry, mcp, …).
//
// Dependency rule:
//   Nothing in `core/` may `use` any other crate-internal module.
//   The check is enforced by convention (and by the layering diagram
//   in the implementation plan): core sits at the bottom of the
//   dependency DAG, so every other module is free to depend on it
//   without risking a cycle.
//
// What lives here:
//   - `agent_id` — identity types used by every cross-agent call
//     surface: `AgentId`, `AbilityName`, `NodeId`, plus their typed
//     error enums (`AgentIdError`, `NodeIdError`) and validators.
//     This is deliberately a single file — the types are tightly
//     coupled by a shared validation grammar, and splitting them
//     along "one type per file" lines would scatter the grammar
//     across files without eliminating the coupling.
//
// What does NOT live here:
//   - Resolution / lookup (`registry/`).
//   - Wire envelopes / MCP schemas (`mcp/`).
//   - Execution state (`runtime/session`).
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

pub mod ability_spec;
pub mod agent_id;
pub mod agent_spec;
/// Pure runtime-domain identifier types (TenantId/NodeId/ScheduleId).
/// Moved from `runtime::domain` (T4.1 pre-move a): they have zero
/// internal dependencies, and their old home made `persistence`
/// reach upward into `runtime` — the only production edge keeping
/// the future `easynet-domain` leaf crate from being a leaf.
pub mod domain;

// We intentionally do NOT re-export `agent_id::*` at the crate
// root. Every call site that needs an identity type reaches for it
// via `crate::core::agent_id::AgentId` (etc.) — the explicit path
// makes the layering visible at the import line. Re-exporting a
// large, heterogenous surface (`AgentId` / `AbilityName` / `NodeId`
// + their error enums + module-level constants) would hide that
// layering and would fight the `unused_imports` lint.
