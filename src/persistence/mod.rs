// EasyNet CLI — Persistence Layer
// ================================
//
// File: src/persistence/mod.rs
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
//   `crate::registry` because its contents are logical identity,
//   not plumbing state. It happens to persist via this layer's
//   `atomic_write`, but it is a consumer, not a cohabitant.
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

pub(crate) mod config;
pub(crate) mod tenant_paths;
