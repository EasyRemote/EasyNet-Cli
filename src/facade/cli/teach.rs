// EasyNet CLI — `easynet ability teach|learn|forget`
// ====================================================
//
// File: src/facade/cli/teach.rs
// Description: GET route B verbs (seven-axes T3.3) over the
//              `meta.teach` / `meta.acquire` / `meta.forget`
//              abilities. Thin wrappers by design: each verb is one
//              daemon invocation, so every transfer is admitted,
//              ledgered, and receipted — the receipt chain IS the
//              audit trail of who conferred what to whom.
//
//              `learn` rides `invoke_local_ability_with_invocation_meta`
//              with subject = the taught ability's URA: the
//              seven-tuple names the thing being transferred
//              (spec 0.1-7).
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use anyhow::Context;
use clap::Args;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::runtime::agents::teach_ability::{ACQUIRE, FORGET, TEACH};
use crate::support::local_invoke::invoke_local_ability_with_hosted_agent_delegation;
use crate::support::output;

/// Typed projection of the `meta.teach` daemon response. Parsing the
/// response (rather than reaching into an untyped `Value` with an
/// `unwrap_or` fallback) means a missing field fails the command
/// loudly instead of rendering a placeholder success — the CLI never
/// reports a transfer it cannot describe.
#[derive(Debug, Deserialize)]
struct TaughtResponse {
    taught: String,
    execution_mode: String,
}

/// Typed projection of the `meta.acquire` daemon response.
#[derive(Debug, Deserialize)]
struct LearnedResponse {
    new_ura: String,
    execution_mode: String,
}

/// Typed projection of the `meta.forget` daemon response.
#[derive(Debug, Deserialize)]
struct ForgottenResponse {
    forgotten: String,
    had_learned_from: String,
}

#[derive(Debug, Args)]
pub struct TeachArgs {
    /// Owner-local ability name (`<agent>.<name>`, e.g. `mentor.quote`).
    pub ability: String,
    /// Learner's canonical Agent URA.
    #[arg(long = "to", value_name = "AGENT_URA")]
    pub to: String,
    /// Skip the interactive confirmation.
    #[arg(long, short = 'y')]
    pub yes: bool,
}

#[derive(Debug, Args)]
pub struct LearnArgs {
    /// Canonical Ability URA of the taught ability.
    pub ability_ura: String,
    /// Local agent that learns (becomes the new copy's owner).
    #[arg(long = "as", value_name = "AGENT")]
    pub learner: String,
    /// Skip the interactive confirmation.
    #[arg(long, short = 'y')]
    pub yes: bool,
}

#[derive(Debug, Args)]
pub struct ForgetArgs {
    /// Public ability name to forget (e.g. `quote`).
    pub ability: String,
    /// Local agent unlearning it.
    #[arg(long, value_name = "AGENT")]
    pub agent: String,
    /// Skip the interactive confirmation.
    #[arg(long, short = 'y')]
    pub yes: bool,
}

/// Compute half of `teach` (e2e surface).
pub fn execute_teach(args: &TeachArgs) -> anyhow::Result<Value> {
    let owner_ura = resolve_owner_ura_from_ability(&args.ability)?;
    invoke_local_ability_with_hosted_agent_delegation(
        TEACH,
        json!({ "ability": args.ability, "learner_ura": args.to }),
        None,
        &[],
        None,
        None,
        &owner_ura,
    )
    .map(|(resp, _meta)| resp)
    .context("confer the teach grant")
}

/// Compute half of `forget` (e2e surface).
pub fn execute_forget(args: &ForgetArgs) -> anyhow::Result<Value> {
    let agent_ura = resolve_learner_ura(&args.agent)?;
    invoke_local_ability_with_hosted_agent_delegation(
        FORGET,
        json!({ "ability": args.ability, "agent": args.agent }),
        None,
        &[],
        None,
        None,
        &agent_ura,
    )
    .map(|(resp, _meta)| resp)
    .context("forget the learned ability")
}

pub fn run_teach(args: TeachArgs) -> anyhow::Result<()> {
    if !args.yes {
        anyhow::bail!(
            "teaching makes {} learnable by {}; re-run with -y to confirm",
            args.ability,
            args.to
        );
    }
    let resp = execute_teach(&args)?;
    let taught: TaughtResponse =
        serde_json::from_value(resp).context("parse meta.teach response")?;
    output::success(&format!(
        "taught {} → {} (execution_mode: {})",
        taught.taught, args.to, taught.execution_mode,
    ));
    Ok(())
}

/// Compute half of `learn` (e2e surface): the acquisition response
/// plus the invocation envelope echo.
pub fn execute_learn(args: &LearnArgs) -> anyhow::Result<(Value, Value)> {
    let learner_ura = resolve_learner_ura(&args.learner)?;
    invoke_local_ability_with_hosted_agent_delegation(
        ACQUIRE,
        json!({ "ability_ura": args.ability_ura, "learner": args.learner }),
        Some(args.ability_ura.clone()),
        &[],
        None,
        None,
        &learner_ura,
    )
    .context("acquire the taught ability")
}

fn resolve_learner_ura(learner: &str) -> anyhow::Result<String> {
    let local = crate::persistence::local_agents::load()
        .with_context(|| format!("resolve learner {learner:?} from local-agents.json"))?;
    let entry = crate::persistence::local_agents::lookup_hosted_agent_by_name(&local, learner)?
        .ok_or_else(|| {
            anyhow::anyhow!(
                "learner {learner:?} has no persisted local Agent URA; run agent publish/join \
                 before acquiring taught abilities"
            )
        })?;
    Ok(entry.agent_ura.clone())
}

fn resolve_owner_ura_from_ability(ability: &str) -> anyhow::Result<String> {
    let (owner, _ability_name) = ability
        .split_once('.')
        .ok_or_else(|| anyhow::anyhow!("teach ability must be in <agent>.<ability> form"))?;
    resolve_learner_ura(owner)
}

pub fn run_learn(args: LearnArgs) -> anyhow::Result<()> {
    if !args.yes {
        anyhow::bail!(
            "learning installs {} into agent {:?}; re-run with -y to confirm",
            args.ability_ura,
            args.learner
        );
    }
    let (resp, _meta) = execute_learn(&args)?;
    let learned: LearnedResponse =
        serde_json::from_value(resp).context("parse meta.acquire response")?;
    output::success(&format!("learned · new ura: {}", learned.new_ura));
    output::detail("execution_mode", &learned.execution_mode);
    Ok(())
}

pub fn run_forget(args: ForgetArgs) -> anyhow::Result<()> {
    if !args.yes {
        anyhow::bail!(
            "forgetting removes the learned {} from agent {:?}; re-run with -y to confirm",
            args.ability,
            args.agent
        );
    }
    let resp = execute_forget(&args)?;
    let forgotten: ForgottenResponse =
        serde_json::from_value(resp).context("parse meta.forget response")?;
    output::success(&format!(
        "forgot {} (was learned from {})",
        forgotten.forgotten, forgotten.had_learned_from,
    ));
    Ok(())
}
