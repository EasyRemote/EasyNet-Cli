// EasyNet CLI — Core Layer
// ========================
//
// File: src/core/mod.rs
// Description: Zero-dependency ontology types shared across every
//              other subsystem (daemon execution, eal, cli, ffi, …).
//
// Dependency rule:
//   Nothing in `core/` may `use` any other crate-internal module.
//   The check is enforced by convention (and by the layering diagram
//   in the implementation plan): core sits at the bottom of the
//   dependency DAG, so every other module is free to depend on it
//   without risking a cycle.
//
// What lives here:
//   - `agent::id` — identity types used by every cross-agent call
//     surface: `AgentId`, `AbilityName`, `NodeId`, plus their typed
//     error enums (`AgentIdError`, `NodeIdError`) and validators.
//     This is deliberately a single file — the types are tightly
//     coupled by a shared validation grammar, and splitting them
//     along "one type per file" lines would scatter the grammar
//     across files without eliminating the coupling.
//
// What does NOT live here:
//   - Resolution / lookup (`daemon::persistence::agent_registry`).
//   - Wire envelopes / MCP schemas (`mcp/`).
//   - Execution state (`daemon::execution`).
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

pub mod ability;
pub mod agent;
/// Pure domain identifier types (TenantId/NodeId/ScheduleId).
/// These have zero internal dependencies; keeping them in `core`
/// prevents persistence and execution modules from depending upward
/// on a higher-level owner.
pub mod domain;
pub mod identity;
pub mod ura;

pub use ability::spec as ability_spec;
pub use agent::id as agent_id;
pub use agent::spec as agent_spec;

// Public module aliases preserve the pre-structure Rust API paths
// (`core::agent_id`, `core::agent_spec`, `core::ability_spec`) while
// the source owner remains the semantic directory underneath
// `core/{agent,ability}`. Do not glob-reexport the contents here:
// callers should still import concrete types from the semantic owner
// when writing new code.
