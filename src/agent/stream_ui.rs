// EasyNet CLI — Shared Agent Stream Timeline Printer
// ====================================================
//
// File: src/agent/stream_ui.rs
// Description: Formats agent stream events (tool calls, results, text,
//              thinking, usage) into a compact, colourised timeline that is
//              identical across Claude Code and Codex wrappers.
//
// Visual format:
//   139s → Read /Users/.../README.md
//   139s tokens in=85341 out=4 cache_r=82587 cache_w=2750
//   140s ← 1     # EasyNet Axon Ecosystem Plan (Draft)
//   140s → Grep mission|dispatch|orchestrate
//   140s ← Found 10 files
//   199s · 基于对两个仓库的深度分析，我的判断如下：
//
// All lines are right-padded on the leading elapsed seconds (`{:>4}s`) so
// they stay vertically aligned as the counter grows from 1s to 999s+.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::time::Instant;

use console::style;

/// Cumulative token usage, mirrored across both agent types.
#[derive(Debug, Clone, Copy, Default)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
}

/// Print a "tool call" line: `123s → Write /tmp/foo.html`.
pub fn print_tool_use(run_start: Instant, tool: &str, arg_summary: &str) {
    let elapsed = run_start.elapsed().as_secs();
    eprintln!(
        "  {} {} {} {}",
        style(format!("{:>4}s", elapsed)).dim(),
        style("→").cyan(),
        style(tool).cyan().bold(),
        style(arg_summary).dim(),
    );
}

/// Print a "tool result" line: `123s ← ok` or `123s ✗ error ...`.
pub fn print_tool_result(run_start: Instant, text: &str, is_error: bool) {
    let elapsed = run_start.elapsed().as_secs();
    let snippet = first_line(text, 120);
    if is_error {
        eprintln!(
            "  {} {} {}",
            style(format!("{:>4}s", elapsed)).dim(),
            style("✗").red(),
            style(snippet).red().dim(),
        );
    } else if !snippet.is_empty() {
        eprintln!(
            "  {} {} {}",
            style(format!("{:>4}s", elapsed)).dim(),
            style("←").green().dim(),
            style(snippet).dim(),
        );
    }
}

/// Print a free-form "assistant thinking" text line: `123s · some text`.
pub fn print_assistant_text(run_start: Instant, text: &str) {
    let elapsed = run_start.elapsed().as_secs();
    let snippet = first_line(text, 120);
    if !snippet.is_empty() {
        eprintln!(
            "  {} {} {}",
            style(format!("{:>4}s", elapsed)).dim(),
            style("·").dim(),
            style(snippet).dim(),
        );
    }
}

/// Print a cumulative-usage line: `123s tokens in=12504 out=892 cache_r=11200 cache_w=2300`.
pub fn print_usage(run_start: Instant, usage: &Usage) {
    let elapsed = run_start.elapsed().as_secs();
    let total_in =
        usage.input_tokens + usage.cache_read_tokens + usage.cache_creation_tokens;
    eprintln!(
        "  {} {} {}",
        style(format!("{:>4}s", elapsed)).dim(),
        style("tokens").dim(),
        style(format!(
            "in={} out={} cache_r={} cache_w={}",
            total_in, usage.output_tokens, usage.cache_read_tokens, usage.cache_creation_tokens
        ))
        .dim(),
    );
}

/// Print the one-line banner that appears at the very start of a run:
/// `timeout 900s` / `model claude-sonnet-4-6`.
pub fn print_header(timeout_secs: u64, model: Option<&str>) {
    eprintln!(
        "  {} {}",
        style("timeout").dim(),
        style(format!("{timeout_secs}s")).cyan(),
    );
    if let Some(m) = model {
        eprintln!("  {} {}", style("model").dim(), style(m).cyan());
    }
}

// ── helpers ────────────────────────────────────────────────────────────────

fn first_line(s: &str, max: usize) -> String {
    let line = s.lines().next().unwrap_or("").trim();
    truncate(line, max)
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let head: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{head}…")
    }
}

/// Shared tool-input summariser. Returns a compact single-line description
/// of the tool's input payload, used to populate the dim text after the
/// tool name on a `→` line.
pub fn summarise_tool_input(tool: &str, input: &serde_json::Value) -> String {
    use serde_json::Value;
    match tool {
        "Bash" => input
            .get("command")
            .and_then(Value::as_str)
            .map(|s| truncate(s, 100))
            .unwrap_or_default(),
        "Write" | "Edit" | "Read" | "NotebookEdit" => input
            .get("file_path")
            .and_then(Value::as_str)
            .map(|s| truncate(s, 100))
            .unwrap_or_default(),
        "Glob" => input
            .get("pattern")
            .and_then(Value::as_str)
            .map(|s| truncate(s, 100))
            .unwrap_or_default(),
        "Grep" => input
            .get("pattern")
            .and_then(Value::as_str)
            .map(|s| truncate(s, 100))
            .unwrap_or_default(),
        _ => {
            // Generic: show up to 2 scalar fields.
            if let Some(obj) = input.as_object() {
                obj.iter()
                    .take(2)
                    .filter_map(|(k, v)| match v {
                        Value::String(s) => Some(format!("{k}={}", truncate(s, 40))),
                        Value::Number(n) => Some(format!("{k}={n}")),
                        Value::Bool(b) => Some(format!("{k}={b}")),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            } else {
                String::new()
            }
        }
    }
}
