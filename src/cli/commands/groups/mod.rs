// EasyNet CLI — Layered Subcommand Groups
// =======================================
//
// File: src/cli/commands/groups/mod.rs
// Description: Aggregated, "noun-first" subcommand groups that replace the
//              retired flat top-level command surface. Each module defines a
//              parent `<Group>Args` (clap derive), a nested action enum, and a
//              typed product entrypoint for one noun-owned command family.
//
// Why a layered CLI?
// - 20+ flat top-level verbs forced users to memorise an unstructured list.
// - Operations on the same noun (device, ability, runtime, mcp, mission,
//   agent) were scattered across the help output instead of grouped.
// - Several lifecycle stages were missing entirely (ability uninstall, device
//   show, mission history, agent sessions). Building the groups also gave us
//   a place to slot those gaps in cleanly.
//
// Layout: one file per top-level noun. Each file owns its own `Args` /
//         `Action` enum and a `run()` entry point invoked from `cli::Command`
//         in `../mod.rs`.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

pub mod ability;
pub mod agent;
pub mod auth;
pub mod call;
pub mod device;
pub mod federation;
pub mod invocation;
pub mod mcp;
pub mod mission;
pub mod plugin;
mod plugin_template;
pub mod principal;
mod principal_routes_gen;
pub mod runtime;
pub mod selfcmd;
pub mod trust;
