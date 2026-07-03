// EasyNet CLI — Runtime Drivers
// =============================
//
// File: src/daemon/execution/mission/drivers/mod.rs
// Description: Per-runtime subprocess drivers. Each file here owns
//              exactly one runtime binary (claude-code, codex, …)
//              and exports an `impl AgentAdapter` consumed by
//              `runtime::dispatch`.
//
// New-driver checklist:
//   1. Add `pub(crate) mod <name>;` below.
//   2. Implement spawn + stream parse + usage accounting inside
//      `<name>.rs`. Share subprocess helpers through
//      `runtime::process_runner`, NOT by reaching into sibling
//      drivers.
//   3. Write `impl AgentAdapter for <name>::<Name>Adapter { ... }`.
//   4. Add a row to `registry()` below mapping the matching
//      `AgentType` variant to the adapter singleton.
//
// What does NOT live here:
//   - Multi-agent orchestration (`runtime::conversation`).
//   - Shared stream rendering (`runtime::stream_ui`).
//   - Per-run persistence (`runtime::run_store`).
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

pub(crate) mod claude_code;
pub(crate) mod codex;
pub(crate) mod external;
pub(crate) mod invocation_trace;

use super::adapter::AgentAdapter;
use crate::daemon::persistence::agent_registry::AgentType;

/// Resolve the adapter for a given `AgentType`. The match here is
/// intentionally exhaustive and `&'static dyn AgentAdapter` — every
/// driver is a zero-sized singleton, so we hand out immovable
/// references cheaply and never re-allocate at dispatch time.
///
/// This is the **one and only** place the runtime layer branches on
/// `AgentType`. Callers receive a trait object and invoke through
/// it; adding a new runtime is one match arm + one adapter
/// singleton, not a sweep of the codebase.
pub(crate) fn adapter_for(agent_type: AgentType) -> &'static dyn AgentAdapter {
    match agent_type {
        AgentType::ClaudeCode => &claude_code::ClaudeCodeAdapter,
        AgentType::Codex => &codex::CodexExecAdapter,
        AgentType::CodexAppServer => &codex::CodexAppServerAdapter,
        AgentType::External => &external::ExternalAdapter,
    }
}
