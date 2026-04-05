// EasyNet CLI — Terminal Output
// =============================
//
// File: src/shared/output.rs
// Description: Formatted CLI output — tables, status indicators, colored messages.
//
// Conventions:
//   success()  → green ✓ prefix (to stderr)
//   info()     → plain text (to stderr)
//   step()     → indented step detail (to stderr)
//   table()    → UTF8 bordered table (to stdout via caller println!)
//
// All status output goes to stderr so stdout remains clean for JSON/pipe.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use clap::ValueEnum;
use comfy_table::{presets::UTF8_FULL_CONDENSED, ContentArrangement, Table};
use console::style;

/// Output format for list commands (devices, abilities).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    Table,
    Json,
}

pub fn success(msg: &str) {
    eprintln!("{} {msg}", style("✓").green().bold());
}

pub fn warn(msg: &str) {
    eprintln!("{} {msg}", style("⚠").yellow());
}

pub fn info(msg: &str) {
    eprintln!("{msg}");
}

pub fn step(msg: &str) {
    eprintln!("  {msg}");
}

pub fn detail(key: &str, value: &str) {
    eprintln!("  {key}: {value}");
}

pub fn table(headers: &[&str]) -> Table {
    let mut t = Table::new();
    t.load_preset(UTF8_FULL_CONDENSED);
    t.set_content_arrangement(ContentArrangement::Dynamic);
    t.set_header(headers.iter().map(|h| h.to_uppercase()));
    t
}

/// Prompt the user for a yes/no confirmation on stderr.
/// Returns `Ok(true)` if the user answered yes, `Ok(false)` if no.
/// Errors if stdin is not a terminal and `--yes` was not passed.
pub fn confirm(prompt: &str) -> anyhow::Result<bool> {
    if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        anyhow::bail!(
            "confirmation required but stdin is not a terminal. \
             Use --yes (-y) to skip the prompt."
        );
    }
    eprint!("{prompt} [y/N] ");
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    Ok(matches!(answer.trim(), "y" | "Y" | "yes" | "YES"))
}

/// Format a Unix-millisecond timestamp as a human-friendly relative string (e.g., "3m ago").
pub fn relative_time(unix_ms: i64) -> String {
    let now_ms = chrono::Utc::now().timestamp_millis();
    let diff_secs = (now_ms - unix_ms) / 1000;

    match diff_secs {
        neg if neg < 0 => "in the future".into(),
        0..60 => "just now".into(),
        60..3600 => format!("{}m ago", diff_secs / 60),
        3600..86400 => format!("{}h ago", diff_secs / 3600),
        _ => {
            let days = diff_secs / 86400;
            match days {
                1 => "yesterday".into(),
                2..30 => format!("{days}d ago"),
                _ => format!("{}mo ago", days / 30),
            }
        }
    }
}

