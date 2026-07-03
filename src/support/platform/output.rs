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

/// Print a user-facing error line. Use this for recoverable / reported
/// errors that shouldn't abort the process — the caller is still
/// responsible for returning a non-zero exit when appropriate.
pub fn error(msg: &str) {
    eprintln!("{} {msg}", style("✗").red().bold());
}

pub fn info(msg: &str) {
    eprintln!("{msg}");
}

pub fn step(msg: &str) {
    eprintln!("  {msg}");
}

/// Single key/value row at fixed 2-space indent, no width
/// alignment. Kept for callers that mix `detail()` with prose
/// `info()` lines where padding would look artificial.
///
/// For tabular output (multiple `detail()`s in a block) prefer
/// `kv_section()` — it pads every key to the column width derived
/// from the longest key in the block, so the values line up
/// vertically. Banner and `runtime status` use that one.
pub fn detail(key: &str, value: &str) {
    eprintln!("  {key}: {value}");
}

/// Render a vertically-aligned key/value block. Every key gets
/// padded with spaces so the values start at the same column,
/// regardless of how long individual keys are. Keys render in
/// bold cyan (matching the banner's accent), values in default
/// terminal style — same palette the `easynet --help` banner
/// uses, so the visual feel of `easynet --help`,
/// `easynet runtime status`, and `easynet auth whoami` is
/// identical.
///
/// Stderr (status surface). Use [`kv_section_stdout`] for the
/// authoritative-data surfaces (`auth whoami`, where consumers
/// might pipe to grep / jq).
pub fn kv_section(rows: &[(&str, &str)]) {
    // Status surface: values render dim because they're
    // informational coordinates ("daemon is at ...sock") rather
    // than the command's answer, and dimming them lets the cyan
    // keys index the block without the values fighting for
    // attention.
    for line in format_kv_rows(rows, /*dim_values=*/ true) {
        eprintln!("{line}");
    }
}

/// Same layout + palette as [`kv_section`] but writes to stdout
/// with values in the terminal's default (not dim) color. Use this
/// when the rendered block is the *answer* to the command's
/// question (e.g. `whoami` printing identity facts), so
/// `cmd | grep email` reads the same value the user saw and the
/// value column isn't visually de-emphasised.
pub fn kv_section_stdout(rows: &[(&str, &str)]) {
    for line in format_kv_rows(rows, /*dim_values=*/ false) {
        println!("{line}");
    }
}

/// Build the styled key/value lines without writing them. The
/// stderr / stdout split lives in the two thin wrappers above so
/// the formatting is one source of truth.
fn format_kv_rows(rows: &[(&str, &str)], dim_values: bool) -> Vec<String> {
    let width = rows.iter().map(|(k, _)| k.len()).max().unwrap_or(0);
    rows.iter()
        .map(|(k, v)| {
            let padded = format!("{k:<width$}");
            let key = style(format!("{padded}:")).cyan().bold();
            if dim_values {
                format!("  {key} {}", style(v).dim())
            } else {
                format!("  {key} {v}")
            }
        })
        .collect()
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
