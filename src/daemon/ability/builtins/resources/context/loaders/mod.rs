// EasyNet CLI — Context loaders for the chat ability
// ====================================================
//
// File: src/runtime/system/context_loaders/mod.rs
// Description: Concrete implementations of `chat_ability::ContextLoader`
//              that the daemon registers at boot. The trait seam
//              lives in `chat_ability`; the loaders that contribute
//              real content (upcoming schedules, recent memory, user
//              profile) live here.
//
// Why a sub-module
// ----------------
// Each loader is < 100 lines on its own but pulls in distinct
// dependencies (ScheduleService, filesystem walks, TOML parsing).
// Grouping them under `resources::context::loaders` keeps `system::mod`
// short and gives a future "list all loaders" introspection a
// single import to walk.
//
// What each loader contributes (text format)
// ------------------------------------------
// All loaders emit markdown-shaped fragments that the chat handler
// concatenates into the LLM's context block (`compose_prompt`'s
// `context` argument). Conventions:
//
//   * Each fragment opens with a `## <title>` heading so the LLM can
//     see the section boundary even when multiple loaders run.
//   * Empty payloads return `Ok(None)` rather than `Ok(Some(""))` —
//     the chat handler skips the loader entirely in that case so
//     `context_used` reflects only loaders that contributed.
//   * Errors return `Err(...)`. The chat handler logs them and
//     records a `bytes: 0, error: "..."` entry in `context_used`
//     rather than failing the whole call.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

pub mod memory;
pub mod schedule;
pub mod user_profile;

use std::sync::Arc;

use crate::daemon::ability::builtins::agents::chat::ContextLoader;

/// Build the v1 default context-loader chain in canonical order.
/// The order matters because each loader's output is concatenated
/// into the prompt — putting user profile first gives the LLM the
/// "who is talking" frame before schedule / memory dumps that
/// reference it.
///
/// Order:
///   1. user_profile  — global, lightweight, sets the frame
///   2. schedule      — agent-scoped upcoming tasks
///   3. memory        — agent-scoped recent memory files
///
/// Daemon callers pass this Vec into `system::build_registry_for_daemon`.
/// Tests and the standalone MCP server can pass an empty Vec to
/// disable all loaders or build a curated subset.
pub fn default_loaders(
    schedule_service: Arc<crate::runtime::execution::schedule::ScheduleService>,
) -> Vec<Arc<dyn ContextLoader>> {
    vec![
        Arc::new(user_profile::UserProfileLoader::new()),
        Arc::new(schedule::ScheduleLoader::new(schedule_service)),
        Arc::new(memory::MemoryLoader::new()),
    ]
}
