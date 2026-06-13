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
use serde_json::{json, Value};

use crate::runtime::agents::teach_ability::{ACQUIRE, FORGET, TEACH};
use crate::support::local_invoke::{
    invoke_local_ability, invoke_local_ability_with_invocation_meta,
};
use crate::support::output;

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
    invoke_local_ability(
        TEACH,
        json!({ "ability": args.ability, "learner_ura": args.to }),
    )
    .context("confer the teach grant")
}

/// Compute half of `forget` (e2e surface).
pub fn execute_forget(args: &ForgetArgs) -> anyhow::Result<Value> {
    invoke_local_ability(
        FORGET,
        json!({ "ability": args.ability, "agent": args.agent }),
    )
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
    output::success(&format!(
        "taught {} → {} (execution_mode: {})",
        resp["taught"].as_str().unwrap_or("?"),
        args.to,
        resp["execution_mode"].as_str().unwrap_or("?"),
    ));
    Ok(())
}

/// Compute half of `learn` (e2e surface): the acquisition response
/// plus the invocation envelope echo.
pub fn execute_learn(args: &LearnArgs) -> anyhow::Result<(Value, Value)> {
    invoke_local_ability_with_invocation_meta(
        ACQUIRE,
        json!({ "ability_ura": args.ability_ura, "learner": args.learner }),
        Some(args.ability_ura.clone()),
        &[],
        None,
        None,
        None,
    )
    .context("acquire the taught ability")
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
    output::success(&format!(
        "learned · new ura: {}",
        resp["new_ura"].as_str().unwrap_or("?"),
    ));
    output::detail(
        "execution_mode",
        resp["execution_mode"].as_str().unwrap_or("?"),
    );
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
    output::success(&format!(
        "forgot {} (was learned from {})",
        resp["forgotten"].as_str().unwrap_or("?"),
        resp["had_learned_from"].as_str().unwrap_or("?"),
    ));
    Ok(())
}
