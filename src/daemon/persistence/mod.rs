// EasyNet CLI — Persistence Layer
// ================================
//
// File: src/daemon/persistence/mod.rs
// Description: On-disk state for the local EasyNet installation.
//
// Scope
// -----
// This module owns every path under `~/.easynet/` that the binary
// reads or writes. It is the single authority for:
//
// - `state.json`       — runtime state (PID, endpoint, tenant label)
// - `credentials.json` — pairing secret produced by `device join`
// - `device_settings.json` — per-device feature flags
// - `heartbeat.pid`    — daemon PID file (written by start, read by stop)
// - mission run dirs   — path accessors only; write logic lives in
//                         `cli::mission_runs` because the lifecycle
//                         is driven from there
//
// What does NOT live here
// -----------------------
// - The **agent registry** (`agents.json`) — it lives in
//   `agent_registry` because it is a daemon-local read/write model for
//   hosted agents, not a core identity type.
// - Network plumbing (`support::bridge_pool`, `support::net`) — those
//   are transport concerns, not persistence.
//
// Why `persistence` is a top-level module, not `support::config`
// ------------------------------------------------------------
// Before this split, `shared/` was a dumping ground containing
// everything that didn't fit elsewhere. Three distinct concerns
// had grown inside it: persistence (config.rs), registry
// (agents.rs, agent_id.rs), and infrastructure (bridge_pool,
// net, output, sysinfo, shutdown, timeouts). The dumping-ground
// shape meant a reader asking "where does the CLI load the Hub
// endpoint from?" had to grep across a module named after a
// non-concept ("shared"). Naming the module after what it
// actually owns (`persistence`) answers that question from the
// use-site alone.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

// Public so integration tests + future external embedders can
// construct `Credentials`. Inner field visibility on the struct
// itself is already `pub`.
pub mod agent_registry;
pub mod chat_sessions;
pub mod config;
pub mod context_store;
/// Daemon-side configuration for the gRPC InvocationServer
/// (`~/.easynet/daemon-config.toml`). Authored by RFC-003 PR-1; see
/// `pr-drafts/PR-0-spec-daemon-invocation-server.md §1` for the
/// listener invariants this module enforces at load time.
pub mod daemon_config;
pub(crate) mod file_lock;
pub(crate) mod local_agents;
pub(crate) mod owner_projections;
/// Local resources registry — `~/.easynet/resources.json`. Maps a
/// stable hardware identifier (CoreAudio/PulseAudio device UID, USB
/// serial, EDID, camera device-path, …) to the canonical resource
/// URA used as the `subject` of RFC-005 v3.2 media invocations
/// (`mic.subscribe`, `camera.snapshot`, …). Lives here rather than
/// `runtime/` because it is on-disk state owned by the persistence
/// layer; the runtime side reads it through this module's public
/// API.
pub mod resources;
pub(crate) mod teach_grants;
pub(crate) mod tenant_paths;
