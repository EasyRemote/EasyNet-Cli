// EasyNet CLI — Runtime Layer
// ===========================
//
// File: src/runtime/mod.rs
// Description: Reverse-dispatch layer that lets EasyNet invoke
//              external agent CLIs (Claude Code, Codex, …) as
//              programmable "edge agents".
//
// Submodules:
//   drivers/        — Per-runtime subprocess drivers (one file per
//                     runtime binary). Each driver owns its own
//                     process-spawn plumbing, JSONL/SSE stream
//                     parsing, and usage accounting.
//   dispatch        — Unified routing + per-run persistence +
//                     recursion guard. The single entry point for
//                     EAL member-call dispatch and the MCP
//                     `send_to_agent` tool.
//   context         — Thread-local dispatch context (mission id,
//                     depth, origin) with env-var fallback for
//                     subprocess children.
//   conversation    — Multi-agent round-robin `discuss` pattern
//                     (read by `facade::cli::discuss`). Lives at
//                     runtime-top rather than under drivers/
//                     because it is runtime-agnostic — it composes
//                     any two adapter-registered agents.
//   process_runner  — Shared subprocess helpers (spawn, line
//                     callbacks, byte caps).
//   run_store       — Per-run on-disk layout
//                     (<agent-root>/runs/<ts>/…).
//   stream_ui       — Terminal rendering of live JSONL events.
//   toml_escape     — Shell-safe TOML string escaping used when
//                     generating on-disk runtime config.
//   workspace       — Projects an agent's state onto its on-disk
//                     runtime-native layout (.mcp.json / .codex/ /
//                     CLAUDE.md / AGENTS.md / .git/). Today it
//                     writes the legacy `workspaces/<name>/`
//                     layout unchanged; full AgentDirectory-driven
//                     projection lands in a subsequent PR.

pub(crate) mod abilities;
pub(crate) mod adapter;
pub(crate) mod context;
pub(crate) mod conversation;
pub(crate) mod directory;
pub(crate) mod dispatch;
pub(crate) mod drivers;
pub(crate) mod process_runner;
pub(crate) mod run_store;
pub(crate) mod stream_ui;
pub(crate) mod toml_escape;
pub(crate) mod workspace;
