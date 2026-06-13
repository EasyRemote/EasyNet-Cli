// EasyNet CLI — `easynet trust level show|set`
// ==============================================
//
// File: src/facade/cli/trust_level.rs
// Description: The trust-LEVEL plane of the trust surface (seven-axes
//              W2 T2.2; D10 default ruling — extend the existing
//              `trust` noun rather than minting a new one).
//
//              Two planes, one noun:
//                trust show         — the ANCHOR: whose keys does
//                                     admission accept (read-only).
//                trust level show   — the LEVEL: once accepted, how
//                trust level set      far do we trust them. Backed by
//                                     `identity.get_trust`/`set_trust`
//                                     (RFC-001 restatement), invoked
//                                     like any ability — every `set`
//                                     is a ledgered invocation.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use anyhow::Context;
use clap::{Args, Subcommand};
use serde_json::{json, Value};

use crate::support::local_invoke::{
    invoke_local_ability, invoke_local_ability_with_invocation_meta,
};
use crate::support::output;

/// Narrow re-export for `pub` consumers of this module (same pattern
/// as `discover::OutputFormat`).
pub use crate::support::output::OutputFormat;

#[derive(Debug, Args)]
pub struct LevelArgs {
    #[command(subcommand)]
    pub action: LevelAction,
}

#[derive(Debug, Subcommand)]
pub enum LevelAction {
    /// Show a subject's trust level — explicit ruling or baseline.
    Show(ShowArgs),
    /// Record a trust-level ruling for a subject (a ledgered write).
    Set(SetArgs),
}

#[derive(Debug, Args)]
pub struct ShowArgs {
    /// Canonical Agent URA of the subject.
    pub agent_ura: String,
    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,
}

#[derive(Debug, Args)]
pub struct SetArgs {
    /// Canonical Agent URA of the subject.
    pub agent_ura: String,
    /// New level: untrusted | probation | standard | elevated | privileged.
    #[arg(long)]
    pub level: String,
    /// Skip the interactive confirmation.
    #[arg(long, short = 'y')]
    pub yes: bool,
    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,
}

pub fn run(args: LevelArgs) -> anyhow::Result<()> {
    match args.action {
        LevelAction::Show(a) => run_show(a),
        LevelAction::Set(a) => run_set(a),
    }
}

/// Compute half of `show` — the typed surface e2e tests consume
/// (same compute/render split as `discover::execute`).
pub fn execute_show(args: &ShowArgs) -> anyhow::Result<Value> {
    invoke_local_ability(
        crate::runtime::agents::trust_ability::GET_TRUST,
        json!({ "agent_ura": args.agent_ura }),
    )
    .context("query trust level")
}

/// Compute half of `set`: the ability response plus the invocation
/// envelope echo. The `-y` confirmation is a render-layer concern
/// and lives in `run_set`.
pub fn execute_set(args: &SetArgs) -> anyhow::Result<(Value, Value)> {
    // The set is a complete seven-tuple invocation: subject = the
    // entity whose trust changes (spec 0.1-7), and the envelope echo
    // gives us the invocation id the operator can audit.
    invoke_local_ability_with_invocation_meta(
        crate::runtime::agents::trust_ability::SET_TRUST,
        json!({ "agent_ura": args.agent_ura, "trust_level": args.level }),
        Some(args.agent_ura.clone()),
        &[],
        None,
        None,
        None,
    )
    .context("record trust-level ruling")
}

fn run_show(args: ShowArgs) -> anyhow::Result<()> {
    let resp = execute_show(&args)?;
    if matches!(args.format, OutputFormat::Json) {
        println!("{}", serde_json::to_string_pretty(&resp)?);
        return Ok(());
    }
    render_ruling(&resp);
    Ok(())
}

fn run_set(args: SetArgs) -> anyhow::Result<()> {
    if !args.yes {
        anyhow::bail!(
            "setting a trust level changes what {} may do on this device; \
             re-run with -y to confirm",
            args.agent_ura
        );
    }
    let (resp, meta) = execute_set(&args)?;

    if matches!(args.format, OutputFormat::Json) {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "result": resp,
                "invocation": meta,
            }))?
        );
        return Ok(());
    }

    output::success(&format!(
        "trust level updated · {} → {}{}",
        resp.get("previous")
            .and_then(Value::as_str)
            .unwrap_or("(baseline)"),
        resp.get("trust_level")
            .and_then(Value::as_str)
            .unwrap_or("?"),
        meta.get("invocation_ura")
            .and_then(Value::as_str)
            .map(|id| format!(" · invocation {id}"))
            .unwrap_or_default(),
    ));
    Ok(())
}

fn render_ruling(resp: &Value) {
    let s = |k: &str| {
        resp.get(k)
            .and_then(Value::as_str)
            .unwrap_or("–")
            .to_string()
    };
    let mut rows = vec![
        ("subject", s("subject")),
        ("trust_level", s("trust_level")),
        ("source", s("source")),
    ];
    if resp.get("updated_at").and_then(Value::as_str).is_some() {
        rows.push(("updated_at", s("updated_at")));
    }
    let borrowed: Vec<(&str, &str)> = rows.iter().map(|(k, v)| (*k, v.as_str())).collect();
    output::kv_section_stdout(&borrowed);
}
