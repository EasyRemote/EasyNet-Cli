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

/// Format a Unix-millisecond timestamp as a human-friendly relative string (e.g., "3m ago").
pub fn relative_time(unix_ms: i64) -> String {
    let now_ms = chrono::Utc::now().timestamp_millis();
    let diff_secs = (now_ms - unix_ms) / 1000;

    if diff_secs < 0 {
        return "just now".to_string();
    }
    if diff_secs < 60 {
        return "just now".to_string();
    }
    if diff_secs < 3600 {
        let mins = diff_secs / 60;
        return format!("{mins}m ago");
    }
    if diff_secs < 86400 {
        let hours = diff_secs / 3600;
        return format!("{hours}h ago");
    }
    let days = diff_secs / 86400;
    if days == 1 {
        return "yesterday".to_string();
    }
    if days < 30 {
        return format!("{days}d ago");
    }
    let months = days / 30;
    format!("{months}mo ago")
}

