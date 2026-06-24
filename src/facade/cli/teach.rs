// EasyNet CLI - descriptor grant/import/forget commands
// =====================================================
//
// File: src/facade/cli/teach.rs
// Description: GET route B verbs (seven-axes T3.3) over the
//              `meta.teach` / `meta.acquire` / `meta.forget`
//              abilities. Thin wrappers by design: each verb is one
//              daemon invocation, so every descriptor grant/import is
//              admitted, ledgered, and receipted.
//
//              `learn` rides `invoke_local_ability_with_invocation_meta`
//              with subject = the granted descriptor URA: the
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
struct DescriptorGrantResponse {
    granted_descriptor: String,
    execution_mode: String,
    transfer_kind: String,
    invokable_after_acquire: bool,
}

/// Typed projection of the `meta.acquire` daemon response.
#[derive(Debug, Deserialize)]
struct DescriptorImportResponse {
    new_descriptor_ura: String,
    execution_mode: String,
    transfer_kind: String,
    invokable: bool,
}

/// Typed projection of the `meta.forget` daemon response.
#[derive(Debug, Deserialize)]
struct DescriptorRemovalResponse {
    removed_descriptor: String,
    source_descriptor_ura: String,
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
    /// Canonical Ability URA of the granted source descriptor.
    pub ability_ura: String,
    /// Local agent that imports the descriptor copy.
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
    /// Local agent that owns the imported descriptor.
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
    .context("forget the imported descriptor")
}

pub fn run_teach(args: TeachArgs) -> anyhow::Result<()> {
    if !args.yes {
        anyhow::bail!(
            "granting descriptor {} to {}; re-run with -y to confirm",
            args.ability,
            args.to
        );
    }
    let resp = execute_teach(&args)?;
    let grant: DescriptorGrantResponse =
        serde_json::from_value(resp).context("parse meta.teach response")?;
    output::success(&format!(
        "descriptor grant {} -> {} (execution_mode: {}, transfer_kind: {}, invokable_after_acquire: {})",
        grant.granted_descriptor,
        args.to,
        grant.execution_mode,
        grant.transfer_kind,
        grant.invokable_after_acquire,
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
    .context("import the granted descriptor")
}

fn resolve_learner_ura(learner: &str) -> anyhow::Result<String> {
    let local = crate::persistence::local_agents::load()
        .with_context(|| format!("resolve learner {learner:?} from local-agents.json"))?;
    let entry = crate::persistence::local_agents::lookup_hosted_agent_by_name(&local, learner)?
        .ok_or_else(|| {
            anyhow::anyhow!(
                "learner {learner:?} has no persisted local Agent URA; run agent publish/join \
                 before importing granted descriptors"
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
            "importing descriptor {} into agent {:?}; re-run with -y to confirm",
            args.ability_ura,
            args.learner
        );
    }
    let (resp, _meta) = execute_learn(&args)?;
    let imported: DescriptorImportResponse =
        serde_json::from_value(resp).context("parse meta.acquire response")?;
    output::success(&format!(
        "descriptor imported · new descriptor ura: {}",
        imported.new_descriptor_ura
    ));
    output::detail("execution_mode", &imported.execution_mode);
    output::detail("transfer_kind", &imported.transfer_kind);
    output::detail("invokable", &imported.invokable.to_string());
    Ok(())
}

pub fn run_forget(args: ForgetArgs) -> anyhow::Result<()> {
    if !args.yes {
        anyhow::bail!(
            "forgetting removes imported descriptor {} from agent {:?}; re-run with -y to confirm",
            args.ability,
            args.agent
        );
    }
    let resp = execute_forget(&args)?;
    let removed: DescriptorRemovalResponse =
        serde_json::from_value(resp).context("parse meta.forget response")?;
    output::success(&format!(
        "removed imported descriptor {} (source descriptor {})",
        removed.removed_descriptor, removed.source_descriptor_ura,
    ));
    Ok(())
}
