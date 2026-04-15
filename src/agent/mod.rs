// EasyNet CLI — External Agent Dispatch
// =====================================
//
// File: src/agent/mod.rs
// Description: Reverse-dispatch layer that lets EasyNet invoke external agent CLIs
//              (Claude Code / Codex) as programmable "edge agents".

pub(crate) mod claude_code;
pub(crate) mod codex;
pub(crate) mod context;
pub(crate) mod conversation;
pub(crate) mod dispatch;
pub(crate) mod process_runner;
pub(crate) mod run_store;
pub(crate) mod stream_ui;
pub(crate) mod toml_escape;
pub(crate) mod workspace;
