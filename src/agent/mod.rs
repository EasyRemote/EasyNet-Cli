// EasyNet CLI — External Agent Dispatch
// =====================================
//
// File: src/agent/mod.rs
// Description: Reverse-dispatch layer that lets EasyNet invoke external agent CLIs
//              (Claude Code / Codex) as programmable "edge agents".

pub mod claude_code;
pub mod codex;
pub mod conversation;
pub mod dispatch;
pub mod process_runner;
pub mod run_store;
pub mod stream_ui;
pub mod workspace;
