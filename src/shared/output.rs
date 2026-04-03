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
//   node_indicator() → ● green (healthy), ● yellow (suspect), ○ dim (offline)
//
// All status output goes to stderr so stdout remains clean for JSON/pipe.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use comfy_table::{presets::UTF8_FULL_CONDENSED, ContentArrangement, Table};
use console::style;

pub fn success(msg: &str) {
    eprintln!("{} {msg}", style("✓").green().bold());
}

pub fn info(msg: &str) {
    eprintln!("{msg}");
}

pub fn step(msg: &str) {
    eprintln!("  {msg}");
}

pub fn table(headers: &[&str]) -> Table {
    let mut t = Table::new();
    t.load_preset(UTF8_FULL_CONDENSED);
    t.set_content_arrangement(ContentArrangement::Dynamic);
    t.set_header(headers.iter().map(|h| h.to_uppercase()));
    t
}

pub fn node_indicator(state: &str) -> String {
    match state.to_uppercase().as_str() {
        "HEALTHY" | "ONLINE" => format!("{}", style("●").green()),
        "SUSPECT" | "DRAINING" => format!("{}", style("●").yellow()),
        _ => format!("{}", style("○").dim()),
    }
}
