// EasyNet CLI — Trust Surface
// ============================
//
// File: src/cli/groups/trust.rs
// Description: `easynet trust …` — read-only view of the realm trust
//              anchor: whose keys this daemon's admission gate accepts
//              and in what role (commit-plan-2 D3 / Gate D). The anchor
//              has write paths through pairing and protocol key
//              registration; this CLI noun does not define a separate
//              ability-permission system.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use anyhow::Context;
use clap::{Args, Subcommand};
use console::style;
use serde_json::json;

use crate::daemon::trust::anchor::trust_anchor_path_from_env_or_default;
use crate::daemon::trust::anchor::{
    RealmTrustAnchor, RealmTrustAnchorLoadState, TrustedAgent, TrustedAgentRole,
};
use crate::support::platform::output::OutputFormat;

#[derive(Debug, Args)]
pub struct TrustArgs {
    #[command(subcommand)]
    pub action: TrustAction,
}

#[derive(Debug, Subcommand)]
pub enum TrustAction {
    /// Show the realm trust anchor: every key the admission gate
    /// accepts, or one subject's entries when a URA is given.
    Show(ShowArgs),
}

#[derive(Debug, Args)]
pub struct ShowArgs {
    /// Subject URA to inspect (device / user / hub). Omit for the
    /// full anchor overview.
    pub target_ura: Option<String>,
    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,
}

pub fn run(args: TrustArgs) -> anyhow::Result<()> {
    match args.action {
        TrustAction::Show(a) => run_show(a),
    }
}

fn run_show(args: ShowArgs) -> anyhow::Result<()> {
    let path = trust_anchor_path_from_env_or_default();
    let anchor = match RealmTrustAnchor::load_with_state(&path)
        .with_context(|| format!("load trust anchor at {}", path.display()))?
    {
        RealmTrustAnchorLoadState::Loaded(anchor) => anchor,
        RealmTrustAnchorLoadState::Missing { .. } => RealmTrustAnchor::default(),
    };

    let entries: Vec<&TrustedAgent> = match args.target_ura.as_deref() {
        Some(target) => {
            // A user may hold several device keys (multi-device
            // admission); collect every entry that names the subject.
            let mut hits: Vec<&TrustedAgent> = anchor.lookup_user_all(target).iter().collect();
            if hits.is_empty() {
                if let Some(single) = anchor.lookup(target) {
                    hits.push(single);
                }
            }
            hits
        }
        None => {
            // entries_sorted clones; borrow through a leaked-free local
            // by collecting owned values below instead.
            Vec::new()
        }
    };

    if matches!(args.format, OutputFormat::Json) {
        let rows: Vec<_> = if args.target_ura.is_some() {
            entries.iter().map(|e| entry_json(e)).collect()
        } else {
            anchor.entries_sorted().iter().map(entry_json).collect()
        };
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "anchor_path": path.display().to_string(),
                "entries": rows,
            }))?
        );
        return Ok(());
    }

    match args.target_ura.as_deref() {
        Some(target) if entries.is_empty() => {
            println!(
                "{} {}",
                style("not trusted:").red().bold(),
                style(format!(
                    "no anchor entry for `{target}` in {} — admission will reject its \
                     signatures",
                    path.display()
                ))
            );
        }
        Some(target) => {
            println!(
                "{} ({} entr{})",
                style(target).bold(),
                entries.len(),
                if entries.len() == 1 { "y" } else { "ies" }
            );
            print_table(&entries);
            print_trust_is_not_permission();
        }
        None => {
            let owned = anchor.entries_sorted();
            println!(
                "trust anchor: {}  ({} entr{})",
                style(path.display()).bold(),
                owned.len(),
                if owned.len() == 1 { "y" } else { "ies" }
            );
            if owned.is_empty() {
                println!(
                    "{}",
                    style("anchor is empty — pair a device or register a pubkey to admit callers")
                        .dim()
                );
                return Ok(());
            }
            let refs: Vec<&TrustedAgent> = owned.iter().collect();
            print_table(&refs);
            print_trust_is_not_permission();
        }
    }
    Ok(())
}

fn print_table(entries: &[&TrustedAgent]) {
    println!(
        "{:<52} {:<8} {:<14} {:<20} {}",
        style("SUBJECT URA").bold(),
        style("ROLE").bold(),
        style("KEY").bold(),
        style("ADDED").bold(),
        style("ORIGIN REALM").bold()
    );
    for e in entries {
        println!(
            "{:<52} {:<8} {:<14} {:<20} {}",
            e.agent_ura,
            role_label(&e.role),
            key_fingerprint(&e.public_key_b64),
            format_added(e.added_at_unix_ms),
            e.origin_realm.as_deref().unwrap_or("—"),
        );
    }
}

fn print_trust_is_not_permission() {
    println!(
        "\n{}",
        style(
            "trust = whose signatures admission accepts; it does not grant any ability \
             permission (that belongs to ability access/permission)"
        )
        .dim()
    );
}

fn entry_json(e: &TrustedAgent) -> serde_json::Value {
    json!({
        "agent_ura": e.agent_ura,
        "role": role_label(&e.role),
        "public_key_b64": e.public_key_b64,
        "added_at_unix_ms": e.added_at_unix_ms,
        "origin_realm": e.origin_realm,
    })
}

fn role_label(role: &TrustedAgentRole) -> &'static str {
    match role {
        TrustedAgentRole::Backend => "backend",
        TrustedAgentRole::Device => "device",
        TrustedAgentRole::Hub => "hub",
        TrustedAgentRole::User => "user",
    }
}

/// First 12 chars of the base64 key — enough to eyeball-match against
/// a peer's `federation.resolve_key` answer without dumping whole keys
/// into terminal scrollback.
fn key_fingerprint(b64: &str) -> String {
    let head: String = b64.chars().take(12).collect();
    format!("{head}…")
}

fn format_added(unix_ms: u64) -> String {
    use chrono::TimeZone;
    chrono::Utc
        .timestamp_millis_opt(unix_ms as i64)
        .single()
        .map(|t| t.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_else(|| format!("{unix_ms}ms"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_fingerprint_truncates_to_twelve_chars() {
        let fp = key_fingerprint("AAAAC3NzaC1lZDI1NTE5AAAA");
        assert_eq!(fp, "AAAAC3NzaC1l…");
    }

    #[test]
    fn format_added_renders_utc_date() {
        assert!(format_added(1_750_000_000_000).starts_with("2025-"));
    }
}
