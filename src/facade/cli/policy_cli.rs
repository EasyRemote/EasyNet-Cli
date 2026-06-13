// EasyNet CLI — `easynet policy list|create|remove|simulate`
// ============================================================
//
// File: src/facade/cli/policy_cli.rs
// Description: Operator surface over the policy rule store and the
//              tiny matcher (seven-axes T3.2). Guided flags by
//              ruling (spec D7) — `--effect deny --min-trust
//              standard --family aris.` — no expression language.
//
//              `simulate` goes through the daemon's
//              `policy.simulate` ability — the SAME `decide`
//              function the admission path will bind to (§A6), so a
//              dry-run can never drift from the real gate.
//              `list`/`create`/`remove` edit the local store
//              directly: they are operator verbs over an
//              operator-owned file, not network calls.
//
// Verbs DELIBERATELY ABSENT:
//
//   why <invocation-id> — explaining a PAST decision needs gate
//               decisions ledgered, which lands with the §A6 gate
//               rewiring milestone (Axon side). Shipping a `why`
//               that re-simulates TODAY's rules against yesterday's
//               call would lie about history.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use anyhow::Context;
use clap::{Args, Subcommand};
use serde_json::{json, Value};

use crate::persistence::policy_rules::{self, PolicyRule, RuleEffect};
use crate::runtime::agents::policy_ability::ABILITY_SIMULATE;
use crate::support::local_invoke::invoke_local_ability;
use crate::support::output;

/// Narrow re-export (house pattern).
pub use crate::support::output::OutputFormat;

#[derive(Debug, Args)]
pub struct PolicyArgs {
    #[command(subcommand)]
    pub action: PolicyAction,
}

#[derive(Debug, Subcommand)]
pub enum PolicyAction {
    /// List the persisted rules, in match order (first match wins).
    List(ListArgs),
    /// Append a rule (guided flags — no expression language).
    Create(CreateArgs),
    /// Remove a rule by id.
    Remove(RemoveArgs),
    /// Dry-run a hypothetical call through the daemon's matcher.
    Simulate(SimulateArgs),
}

#[derive(Debug, Args)]
pub struct ListArgs {
    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,
}

#[derive(Debug, Args)]
pub struct CreateArgs {
    /// allow | deny.
    #[arg(long)]
    pub effect: String,
    /// Admitted action the rule covers (today: invoke).
    #[arg(long, default_value = "invoke")]
    pub action: String,
    /// Match only abilities whose name starts with this family
    /// prefix (policy scope, never routing).
    #[arg(long)]
    pub family: Option<String>,
    /// Match only callers whose trust level ranks BELOW this level
    /// ("deny unless trust >= L" spells `--effect deny --min-trust L`).
    #[arg(long, value_name = "LEVEL")]
    pub min_trust: Option<String>,
    /// Skip the interactive confirmation.
    #[arg(long, short = 'y')]
    pub yes: bool,
}

#[derive(Debug, Args)]
pub struct RemoveArgs {
    /// Rule id (`pr-<n>`).
    pub id: String,
    /// Skip the interactive confirmation.
    #[arg(long, short = 'y')]
    pub yes: bool,
}

#[derive(Debug, Args)]
pub struct SimulateArgs {
    /// Hypothetical caller (canonical Agent URA).
    #[arg(long)]
    pub caller: String,
    /// Hypothetical ability name (e.g. `aris.review`).
    #[arg(long)]
    pub ability: String,
    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,
}

pub fn run(args: PolicyArgs) -> anyhow::Result<()> {
    match args.action {
        PolicyAction::List(a) => run_list(a),
        PolicyAction::Create(a) => run_create(a),
        PolicyAction::Remove(a) => run_remove(a),
        PolicyAction::Simulate(a) => run_simulate(a),
    }
}

fn run_list(args: ListArgs) -> anyhow::Result<()> {
    let store = policy_rules::load()?;
    if matches!(args.format, OutputFormat::Json) {
        println!("{}", serde_json::to_string_pretty(&store)?);
        return Ok(());
    }
    if store.rules.is_empty() {
        println!("no rules — the matcher answers baseline-allow");
        return Ok(());
    }
    let mut table = output::table(&["ID", "EFFECT", "ACTION", "FAMILY", "TRUST BELOW"]);
    for r in &store.rules {
        table.add_row(vec![
            r.id.clone(),
            r.effect.as_str().to_string(),
            r.action.clone(),
            r.family_prefix.clone().unwrap_or_else(|| "–".into()),
            r.trust_below.clone().unwrap_or_else(|| "–".into()),
        ]);
    }
    println!("{table}");
    Ok(())
}

fn run_create(args: CreateArgs) -> anyhow::Result<()> {
    let effect = match args.effect.to_ascii_lowercase().as_str() {
        "allow" => RuleEffect::Allow,
        "deny" => RuleEffect::Deny,
        other => anyhow::bail!("--effect must be allow or deny; got {other:?}"),
    };
    // Validate the threshold against the pb vocabulary up front so a
    // typo'd level can't sit inert in the store.
    if let Some(level) = args.min_trust.as_deref() {
        if crate::runtime::agents::trust_ability::level_rank(level).is_none() {
            anyhow::bail!(
                "unknown trust level {level:?}; expected one of \
                 untrusted | probation | standard | elevated | privileged"
            );
        }
    }
    if !args.yes {
        anyhow::bail!("policy rules gate live admission; re-run with -y to confirm");
    }

    let mut store = policy_rules::load()?;
    let rule = PolicyRule {
        id: store.next_id(),
        effect,
        action: args.action,
        family_prefix: args.family,
        trust_below: args.min_trust.map(|l| l.to_ascii_uppercase()),
        created_at: chrono::Local::now().to_rfc3339(),
    };
    let id = rule.id.clone();
    store.rules.push(rule);
    policy_rules::save(&store)?;
    output::success(&format!("policy {id} created"));
    Ok(())
}

fn run_remove(args: RemoveArgs) -> anyhow::Result<()> {
    if !args.yes {
        anyhow::bail!("removing a rule changes live admission; re-run with -y to confirm");
    }
    let mut store = policy_rules::load()?;
    let before = store.rules.len();
    store.rules.retain(|r| r.id != args.id);
    if store.rules.len() == before {
        anyhow::bail!("no rule with id {:?} (see `policy list`)", args.id);
    }
    policy_rules::save(&store)?;
    output::success(&format!("policy {} removed", args.id));
    Ok(())
}

fn run_simulate(args: SimulateArgs) -> anyhow::Result<()> {
    let resp = execute_simulate(&args)?;
    if matches!(args.format, OutputFormat::Json) {
        println!("{}", serde_json::to_string_pretty(&resp)?);
        return Ok(());
    }
    let verdict = resp
        .get("would_decide")
        .and_then(Value::as_str)
        .unwrap_or("?");
    let rule = resp
        .get("rule")
        .and_then(Value::as_str)
        .unwrap_or("baseline");
    output::info(&format!("would decide: {verdict} (rule: {rule})"));
    Ok(())
}

/// Compute half of `simulate` (e2e surface): the daemon's matcher,
/// through the wire, with the dry-run response shape.
pub fn execute_simulate(args: &SimulateArgs) -> anyhow::Result<Value> {
    invoke_local_ability(
        ABILITY_SIMULATE,
        json!({
            "invocation_envelope": { "caller": args.caller, "ability": args.ability }
        }),
    )
    .context("simulate against the daemon's policy matcher")
}
