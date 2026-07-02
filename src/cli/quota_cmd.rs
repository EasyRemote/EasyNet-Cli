// EasyNet CLI — `easynet quota` subcommand (#185)
// =================================================
//
// File: src/cli/quota_cmd.rs
//
// Owner-facing verb to inspect and edit the per-consumer invocation
// quota policy the daemon enforces (`[daemon.quota]` in
// `~/.easynet/daemon-config.toml`). Quota meters an already-admitted
// caller; it is a governance refinement on top of the permission gate,
// not an identity decision. See `daemon::invocation::state::usage_quota` for the
// enforcement counter and `persistence::daemon_config::QuotaConfig` for
// the policy shape.
//
// Two properties operators must know (both surfaced in `quota list`):
// - Caps apply PER ABILITY per window — a cap of N admits N calls to
//   each distinct ability per window, not N total across abilities.
// - Only the unary `invoke` RPC is metered; streaming / bidi calls are
//   not.
//
// Subcommands
// -----------
// - `quota list`                        — print the configured policy.
// - `quota set --default-cap <n>`       — set the per-window default.
// - `quota set --window-ms <ms>`        — set the tumbling-window width.
// - `quota set --consumer <ura> --cap <n>` — set a per-consumer override.
// - `quota clear --consumer <ura>`      — drop one consumer override.
//
// Daemon roundtrip
// ----------------
// Like `federation peers`, this command edits the operator's local
// config file. A running daemon picks up the change at its next boot
// or SIGHUP reload — the printed footer says so. Writes preserve the
// rest of the file via `toml_edit` (comments / formatting / unrelated
// tables are not clobbered).
//
// Author: Silan.Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};

use crate::support::output;

/// Mirrors `QuotaConfig::DEFAULT_WINDOW_MS` without importing the
/// daemon-config module into this front-door command.
#[cfg(test)]
const DEFAULT_QUOTA_WINDOW_MS: i64 = 60_000;

/// Default daemon-config.toml location. Delegates to the daemon-config
/// module so CLI edits and daemon boot cannot drift to different files.
fn daemon_config_path() -> PathBuf {
    crate::persistence::daemon_config::default_config_path()
}

#[derive(Debug, Args)]
pub struct QuotaArgs {
    #[command(subcommand)]
    pub command: QuotaCommand,
}

#[derive(Debug, Subcommand)]
pub enum QuotaCommand {
    /// Print the configured per-consumer quota policy.
    List {
        /// Emit JSON for scripts instead of a plain-text listing.
        #[arg(long)]
        json: bool,
    },
    /// Set a quota knob. At least one of `--default-cap`,
    /// `--window-ms`, or (`--consumer` + `--cap`) is required.
    ///
    /// Caps are applied PER ABILITY per window: a cap of N admits up
    /// to N calls to each distinct ability per window, not N total.
    Set {
        /// Per-ability, per-window cap for consumers without an
        /// override (admits N calls to *each* ability per window).
        /// `0` = unmetered.
        #[arg(long)]
        default_cap: Option<i32>,
        /// Tumbling-window width in milliseconds.
        #[arg(long)]
        window_ms: Option<i64>,
        /// Consumer URA to set a per-consumer override for. Requires
        /// `--cap`.
        #[arg(long, requires = "cap")]
        consumer: Option<String>,
        /// Per-ability, per-window cap for `--consumer` (admits N
        /// calls to *each* ability per window). `0` = unmetered.
        #[arg(long, requires = "consumer")]
        cap: Option<i32>,
    },
    /// Remove one consumer's per-consumer override (it falls back to
    /// the default cap).
    Clear {
        /// Consumer URA whose override to remove.
        #[arg(long)]
        consumer: String,
    },
}

pub fn run(args: QuotaArgs) -> anyhow::Result<()> {
    match args.command {
        QuotaCommand::List { json } => run_list(json),
        QuotaCommand::Set {
            default_cap,
            window_ms,
            consumer,
            cap,
        } => run_set(default_cap, window_ms, consumer, cap),
        QuotaCommand::Clear { consumer } => run_clear(&consumer),
    }
}

fn run_list(json: bool) -> anyhow::Result<()> {
    let view = read_quota()?;
    if json {
        println!("{}", serde_json::to_string_pretty(&view)?);
        return Ok(());
    }
    match view {
        None => {
            output::info("quota: not configured");
            output::detail(
                "(off)",
                "no [daemon.quota] table; every caller is unmetered",
            );
        }
        Some(view) => {
            output::info("quota policy ([daemon.quota])");
            output::detail(
                "granularity",
                "caps apply per ability per window (not per consumer total)",
            );
            output::detail(
                "default_cap_per_window",
                &view.default_cap_per_window.to_string(),
            );
            output::detail("window_ms", &view.window_ms.to_string());
            if view.per_consumer.is_empty() {
                output::detail("per_consumer", "(none)");
            } else {
                output::info("per_consumer overrides:");
                for (consumer, cap) in &view.per_consumer {
                    output::detail(consumer, &cap.to_string());
                }
            }
        }
    }
    eprintln!();
    output::info("Caps meter unary invoke only; streaming/bidi calls are not metered.");
    output::info("A running daemon applies edits at its next boot or SIGHUP reload.");
    Ok(())
}

fn run_set(
    default_cap: Option<i32>,
    window_ms: Option<i64>,
    consumer: Option<String>,
    cap: Option<i32>,
) -> anyhow::Result<()> {
    if default_cap.is_none() && window_ms.is_none() && consumer.is_none() {
        anyhow::bail!(
            "nothing to set: pass --default-cap, --window-ms, and/or (--consumer with --cap)"
        );
    }
    if let Some(v) = default_cap {
        validate_cap("--default-cap", v)?;
    }
    if let Some(v) = cap {
        validate_cap("--cap", v)?;
    }
    if let Some(v) = window_ms {
        validate_window_ms(v)?;
    }
    let path = daemon_config_path();
    let mut doc = load_doc(&path)?;
    ensure_quota_table(&mut doc);

    if let Some(v) = default_cap {
        doc["daemon"]["quota"]["default_cap_per_window"] = toml_edit::value(i64::from(v));
    }
    if let Some(v) = window_ms {
        doc["daemon"]["quota"]["window_ms"] = toml_edit::value(v);
    }
    if let (Some(consumer), Some(cap)) = (consumer, cap) {
        // `per_consumer` is a sub-table of `[daemon.quota]`.
        let quota = doc["daemon"]["quota"]
            .as_table_mut()
            .expect("quota table ensured above");
        if !quota.contains_key("per_consumer") {
            quota.insert("per_consumer", toml_edit::table());
        }
        quota["per_consumer"][&consumer] = toml_edit::value(i64::from(cap));
    }

    write_doc(&path, &doc)?;
    output::success("quota policy updated");
    output::info("A running daemon applies the change at its next boot or SIGHUP reload.");
    Ok(())
}

fn run_clear(consumer: &str) -> anyhow::Result<()> {
    let path = daemon_config_path();
    let mut doc = load_doc(&path)?;
    let removed = doc
        .get_mut("daemon")
        .and_then(|d| d.get_mut("quota"))
        .and_then(|q| q.get_mut("per_consumer"))
        .and_then(|p| p.as_table_mut())
        .map(|t| t.remove(consumer).is_some())
        .unwrap_or(false);
    if !removed {
        output::info(&format!(
            "no per-consumer override for {consumer}; nothing to clear"
        ));
        return Ok(());
    }
    write_doc(&path, &doc)?;
    output::success(&format!("cleared per-consumer override for {consumer}"));
    output::info("A running daemon applies the change at its next boot or SIGHUP reload.");
    Ok(())
}

/// Plain-data view of the `[daemon.quota]` table for `list`/`--json`.
#[derive(Debug, serde::Serialize, PartialEq, Eq)]
struct QuotaView {
    default_cap_per_window: i64,
    window_ms: i64,
    per_consumer: std::collections::BTreeMap<String, i64>,
}

fn read_quota() -> anyhow::Result<Option<QuotaView>> {
    let path = daemon_config_path();
    if !path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(&path)?;
    parse_quota(&raw)
}

/// Parse the `[daemon.quota]` table out of a daemon-config TOML string.
/// Returns `None` when the table is absent. Defaults mirror
/// `QuotaConfig::from`: missing cap → 0, missing window → 60_000.
fn parse_quota(raw: &str) -> anyhow::Result<Option<QuotaView>> {
    let doc: RawQuotaDoc = toml::from_str(raw)
        .map_err(|err| anyhow::anyhow!("daemon-config TOML quota section is invalid: {err}"))?;
    let Some(raw_quota) = doc.daemon.and_then(|daemon| daemon.quota) else {
        return Ok(None);
    };
    let quota = crate::persistence::daemon_config::QuotaConfig::from(raw_quota);
    Ok(Some(QuotaView {
        default_cap_per_window: i64::from(quota.default_cap_per_window()),
        window_ms: quota.window_ms(),
        per_consumer: quota
            .per_consumer()
            .iter()
            .map(|(consumer, cap)| (consumer.clone(), i64::from(*cap)))
            .collect(),
    }))
}

#[derive(Debug, serde::Deserialize)]
struct RawQuotaDoc {
    #[serde(default)]
    daemon: Option<RawQuotaDaemon>,
}

#[derive(Debug, serde::Deserialize)]
struct RawQuotaDaemon {
    #[serde(default)]
    quota: Option<crate::persistence::daemon_config::RawQuotaSection>,
}

fn validate_cap(flag: &str, value: i32) -> anyhow::Result<()> {
    if value < 0 {
        anyhow::bail!("{flag} must be >= 0; use 0 for unmetered")
    }
    Ok(())
}

fn validate_window_ms(value: i64) -> anyhow::Result<()> {
    if value <= 0 {
        anyhow::bail!("--window-ms must be > 0")
    }
    Ok(())
}

fn load_doc(path: &Path) -> anyhow::Result<toml_edit::DocumentMut> {
    if !path.exists() {
        anyhow::bail!(
            "daemon config not found at {} — run the daemon once or create it before setting quota",
            path.display()
        );
    }
    let raw = std::fs::read_to_string(path)?;
    Ok(raw.parse()?)
}

/// Ensure `[daemon]` and `[daemon.quota]` exist so subsequent index
/// assignments land in a real table.
fn ensure_quota_table(doc: &mut toml_edit::DocumentMut) {
    if !doc.as_table().contains_key("daemon") {
        doc["daemon"] = toml_edit::table();
    }
    let daemon = doc["daemon"]
        .as_table_mut()
        .expect("daemon is a table by construction");
    if !daemon.contains_key("quota") {
        daemon.insert("quota", toml_edit::table());
    }
}

fn write_doc(path: &Path, doc: &toml_edit::DocumentMut) -> anyhow::Result<()> {
    crate::persistence::config::atomic_write(path, doc.to_string().as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_absent_quota_table_is_none() {
        let raw = r#"
[daemon]
mode = "hub"
realm = "r1"
"#;
        assert_eq!(parse_quota(raw).expect("valid TOML"), None);
    }

    #[test]
    fn parse_invalid_toml_surfaces_error_instead_of_none() {
        let err = parse_quota("[daemon\nquota = true").expect_err("bad TOML must fail");
        assert!(
            err.to_string()
                .contains("daemon-config TOML quota section is invalid"),
            "operator must see a config parse error, got: {err}"
        );
    }

    #[test]
    fn parse_invalid_quota_field_type_surfaces_error_instead_of_defaulting() {
        let raw = r#"
[daemon.quota]
default_cap_per_window = "100"
"#;
        let err = parse_quota(raw).expect_err("typed quota field mismatch must fail");
        assert!(
            err.to_string()
                .contains("daemon-config TOML quota section is invalid"),
            "operator must see the same strict parse failure as daemon reload, got: {err}"
        );
    }

    #[test]
    fn parse_quota_reads_caps_window_and_overrides() {
        let raw = r#"
[daemon]
mode = "hub"
realm = "r1"

[daemon.quota]
default_cap_per_window = 100
window_ms = 30000

[daemon.quota.per_consumer]
"easynet:///r/r1/user/alice" = 5
"#;
        let view = parse_quota(raw)
            .expect("valid TOML")
            .expect("quota present");
        assert_eq!(view.default_cap_per_window, 100);
        assert_eq!(view.window_ms, 30_000);
        assert_eq!(
            view.per_consumer.get("easynet:///r/r1/user/alice"),
            Some(&5)
        );
    }

    #[test]
    fn parse_empty_quota_table_uses_defaults() {
        let raw = r#"
[daemon]
mode = "hub"
realm = "r1"

[daemon.quota]
"#;
        let view = parse_quota(raw)
            .expect("valid TOML")
            .expect("empty table still present");
        assert_eq!(view.default_cap_per_window, 0);
        assert_eq!(view.window_ms, DEFAULT_QUOTA_WINDOW_MS);
        assert!(view.per_consumer.is_empty());
    }

    #[test]
    fn parse_non_positive_window_uses_default() {
        let raw = r#"
[daemon]
mode = "hub"
realm = "r1"

[daemon.quota]
default_cap_per_window = 100
window_ms = -1
"#;
        let view = parse_quota(raw)
            .expect("valid TOML")
            .expect("quota present");
        assert_eq!(view.window_ms, DEFAULT_QUOTA_WINDOW_MS);
    }

    #[test]
    fn validation_rejects_negative_caps_and_non_positive_window() {
        assert!(validate_cap("--default-cap", -1).is_err());
        assert!(validate_cap("--cap", -1).is_err());
        assert!(validate_window_ms(0).is_err());
        assert!(validate_window_ms(-1).is_err());

        assert!(validate_cap("--default-cap", 0).is_ok());
        assert!(validate_window_ms(1).is_ok());
    }

    #[test]
    fn set_then_read_round_trips_through_toml_edit() {
        // Build a minimal config, apply each `set` mutation via the
        // same toml_edit path `run_set` uses, and confirm `parse_quota`
        // reads the result back. This pins the writer/reader contract
        // without touching the operator's real $HOME.
        let mut doc: toml_edit::DocumentMut = r#"
[daemon]
mode = "hub"
realm = "r1"
"#
        .parse()
        .expect("base config parses");

        ensure_quota_table(&mut doc);
        doc["daemon"]["quota"]["default_cap_per_window"] = toml_edit::value(50i64);
        doc["daemon"]["quota"]["window_ms"] = toml_edit::value(15_000i64);
        let quota = doc["daemon"]["quota"].as_table_mut().unwrap();
        quota.insert("per_consumer", toml_edit::table());
        quota["per_consumer"]["easynet:///r/r1/user/bob"] = toml_edit::value(3i64);

        let view = parse_quota(&doc.to_string())
            .expect("valid TOML")
            .expect("written quota reads back");
        assert_eq!(view.default_cap_per_window, 50);
        assert_eq!(view.window_ms, 15_000);
        assert_eq!(view.per_consumer.get("easynet:///r/r1/user/bob"), Some(&3));
    }

    #[test]
    fn ensure_quota_table_preserves_existing_daemon_keys() {
        let mut doc: toml_edit::DocumentMut = r#"
[daemon]
mode = "hub"
realm = "r1"
"#
        .parse()
        .unwrap();
        ensure_quota_table(&mut doc);
        // The pre-existing keys survive; the quota table is added.
        assert_eq!(doc["daemon"]["mode"].as_str(), Some("hub"));
        assert_eq!(doc["daemon"]["realm"].as_str(), Some("r1"));
        assert!(doc["daemon"]["quota"].is_table());
    }
}
