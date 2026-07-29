// EasyNet CLI - descriptor grant/import/forget commands
// =====================================================
//
// File: src/cli/teach.rs
// Description: GET route B verbs (seven-axes T3.3) over the
//              `meta.teach` / `meta.acquire` / `meta.forget`
//              abilities. Thin wrappers by design: each verb is one
//              daemon invocation, so every descriptor grant/import is
//              admitted, ledgered, and receipted.
//
//              `learn` rides `invoke_local_target_with_invocation_meta`
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

use crate::daemon::ability::builtins::governance::teach::{ACQUIRE, FORGET, TEACH};
use crate::daemon::persistence::agent_aggregate::{
    AgentAggregateRepository, HostedAgentNameLookupError,
};
use crate::support::platform::local_invoke::{
    invoke_local_target_with_hosted_agent_delegation, LocalAbilityTarget,
    LocalSystemInvocationIssuer,
};
use crate::support::platform::output;

/// Typed projection of the `meta.teach` daemon response. Parsing the
/// response (rather than reaching into an untyped `Value` with an
/// `unwrap_or` fallback) means a missing field fails the command
/// loudly instead of rendering a placeholder success — the CLI never
/// reports a transfer it cannot describe.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DescriptorGrantResponse {
    granted_descriptor: String,
    execution_mode: String,
    transfer_kind: String,
    invokable_after_acquire: bool,
}

/// Typed projection of the `meta.acquire` daemon response.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DescriptorImportResponse {
    new_descriptor_ura: String,
    execution_mode: String,
    transfer_kind: String,
    invokable: bool,
    descriptor_transaction_status: String,
}

/// Typed projection of the `meta.forget` daemon response.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DescriptorRemovalResponse {
    removed_descriptor: String,
    source_descriptor_ura: String,
    descriptor_transaction_status: String,
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct ConfirmationGate {
    prompt: String,
    bypassed: bool,
}

impl ConfirmationGate {
    fn new(prompt: impl Into<String>, bypassed: bool) -> Self {
        Self {
            prompt: prompt.into(),
            bypassed,
        }
    }

    fn confirm(&self) -> anyhow::Result<bool> {
        if self.bypassed {
            return Ok(true);
        }
        output::confirm(&self.prompt)
    }
}

fn teach_confirmation(args: &TeachArgs) -> ConfirmationGate {
    ConfirmationGate::new(
        format!(
            "Grant descriptor {} to learner {}. Continue?",
            args.ability, args.to
        ),
        args.yes,
    )
}

fn learn_confirmation(args: &LearnArgs) -> ConfirmationGate {
    ConfirmationGate::new(
        format!(
            "Import descriptor {} into agent {:?}. Continue?",
            args.ability_ura, args.learner
        ),
        args.yes,
    )
}

fn forget_confirmation(args: &ForgetArgs) -> ConfirmationGate {
    ConfirmationGate::new(
        format!(
            "Remove imported descriptor {} from agent {:?}. Continue?",
            args.ability, args.agent
        ),
        args.yes,
    )
}

fn confirm_or_cancel(gate: ConfirmationGate) -> anyhow::Result<bool> {
    if gate.confirm()? {
        return Ok(true);
    }
    output::info("Cancelled.");
    Ok(false)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DescriptorTransactionStatus {
    Committed,
    CommittedRuntimeDegraded,
}

impl DescriptorTransactionStatus {
    fn parse(status: &str, operation: &str) -> anyhow::Result<Self> {
        match status {
            "committed" => Ok(Self::Committed),
            "committed_runtime_degraded" => Ok(Self::CommittedRuntimeDegraded),
            other => anyhow::bail!(
                "descriptor {operation} returned unsupported transaction status `{other}`"
            ),
        }
    }

    fn warn_if_runtime_degraded(self, operation: &str) {
        if self == Self::CommittedRuntimeDegraded {
            output::warn(&format!(
                "descriptor {operation} reached durable storage, but live runtime sync degraded; \
                 retry once the agent runtime is wired so the control-plane projection converges"
            ));
        }
    }
}

fn parse_descriptor_transaction_status(
    status: &str,
    operation: &str,
) -> anyhow::Result<DescriptorTransactionStatus> {
    DescriptorTransactionStatus::parse(status, operation)
}

/// Report an acquire/import transaction status.
///
/// Acquire has best-effort runtime convergence, so `committed_runtime_degraded`
/// is a legitimate outcome the operator is warned about (retry once the runtime
/// is wired).
fn report_descriptor_transaction_status(status: &str, operation: &str) -> anyhow::Result<()> {
    parse_descriptor_transaction_status(status, operation)?.warn_if_runtime_degraded(operation);
    Ok(())
}

/// Report a forget transaction status.
///
/// Forget has require-committed convergence semantics (the daemon only returns
/// success once runtime sync committed), so `committed` is the ONLY valid
/// status — a degraded forget surfaces as a daemon error, not a status. Sharing
/// the acquire warn-on-degraded handler would leave a dead branch and invite a
/// maintainer to weaken forget to warn-and-continue, violating require-committed.
fn require_committed_forget_status(status: &str) -> anyhow::Result<()> {
    match parse_descriptor_transaction_status(status, "removal")? {
        DescriptorTransactionStatus::Committed => Ok(()),
        DescriptorTransactionStatus::CommittedRuntimeDegraded => anyhow::bail!(
            "descriptor removal returned `committed_runtime_degraded`, but forget requires \
             committed runtime convergence; this is a daemon contract violation"
        ),
    }
}

/// Compute half of `teach` (e2e surface).
pub fn execute_teach(args: &TeachArgs) -> anyhow::Result<Value> {
    let subject = resolve_owner_ability_subject(&args.ability)?;
    invoke_descriptor_mutation(
        TEACH,
        json!({ "ability": args.ability, "learner_ura": args.to }),
        &subject.ability_ura,
        &subject.owner_ura,
    )
    .map(|(resp, _meta)| resp)
    .context("confer the teach grant")
}

/// Compute half of `forget` (e2e surface).
pub fn execute_forget(args: &ForgetArgs) -> anyhow::Result<Value> {
    let agent_ura = resolve_learner_ura(&args.agent)?;
    let subject_ura = crate::core::ura::owner_ability_ura(&agent_ura, &args.ability)
        .ok_or_else(|| anyhow::anyhow!("could not mint descriptor URA for forgotten ability"))?;
    invoke_descriptor_mutation(
        FORGET,
        json!({ "ability": args.ability, "agent": args.agent }),
        &subject_ura,
        &agent_ura,
    )
    .map(|(resp, _meta)| resp)
    .context("forget the imported descriptor")
}

pub fn run_teach(args: TeachArgs) -> anyhow::Result<()> {
    if !confirm_or_cancel(teach_confirmation(&args))? {
        return Ok(());
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
    invoke_descriptor_mutation(
        ACQUIRE,
        json!({ "ability_ura": args.ability_ura, "learner": args.learner }),
        &args.ability_ura,
        &learner_ura,
    )
    .context("import the granted descriptor")
}

fn invoke_descriptor_mutation(
    ability: &str,
    args: Value,
    subject_ura: &str,
    hosted_agent_ura: &str,
) -> anyhow::Result<(Value, Value)> {
    let local_daemon_ura = crate::daemon::identity::local_invocation::local_daemon_ura()?;
    let target = LocalAbilityTarget::new(ability, &local_daemon_ura)?;
    let context = LocalSystemInvocationIssuer::root_context(
        subject_ura,
        &[],
        std::time::Duration::from_secs(30),
        None,
    )?;
    invoke_local_target_with_hosted_agent_delegation(&target, args, context, hosted_agent_ura)
}

fn resolve_learner_ura(learner: &str) -> anyhow::Result<String> {
    let snapshot = AgentAggregateRepository::try_load_snapshot()
        .with_context(|| format!("resolve learner {learner:?} from Agent aggregate"))?;
    let learner_ura = snapshot
        .hosted_agent_ura_by_name(learner)
        .map_err(|error| resolve_learner_lookup_error(learner, error))?
        .ok_or_else(|| {
            anyhow::anyhow!(
                "learner {learner:?} has no persisted local Agent URA; run agent publish/join \
                 before importing granted descriptors"
            )
        })?;
    Ok(learner_ura.to_string())
}

fn resolve_learner_lookup_error(learner: &str, error: HostedAgentNameLookupError) -> anyhow::Error {
    match error {
        HostedAgentNameLookupError::Ambiguous {
            first_profile,
            second_profile,
            ..
        } => anyhow::anyhow!(
            "learner {learner:?} is ambiguous across profiles {first_profile:?} and {second_profile:?}"
        ),
        HostedAgentNameLookupError::InvalidUra {
            agent_ura, reason, ..
        } => anyhow::anyhow!("learner {learner:?} has invalid Agent URA {agent_ura:?}: {reason}"),
        HostedAgentNameLookupError::NonAgentUra { agent_ura, .. } => {
            anyhow::anyhow!("learner {learner:?} resolved to non-Agent URA {agent_ura:?}")
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OwnerAbilitySubject {
    owner_ura: String,
    ability_ura: String,
}

fn resolve_owner_ability_subject(ability: &str) -> anyhow::Result<OwnerAbilitySubject> {
    let owner_local = crate::core::ura::OwnerLocalAbilityName::parse(ability)
        .map_err(|err| anyhow::anyhow!("teach ability must be in <agent>.<ability> form: {err}"))?;
    let owner_ura = resolve_learner_ura(owner_local.owner())?;
    let ability_ura = crate::core::ura::owner_ability_ura(&owner_ura, owner_local.public_name())
        .ok_or_else(|| anyhow::anyhow!("could not mint teach descriptor subject URA"))?;
    Ok(OwnerAbilitySubject {
        owner_ura,
        ability_ura,
    })
}

pub fn run_learn(args: LearnArgs) -> anyhow::Result<()> {
    if !confirm_or_cancel(learn_confirmation(&args))? {
        return Ok(());
    }
    let (resp, _meta) = execute_learn(&args)?;
    let imported: DescriptorImportResponse =
        serde_json::from_value(resp).context("parse meta.acquire response")?;
    report_descriptor_transaction_status(&imported.descriptor_transaction_status, "import")?;
    output::success(&format!(
        "descriptor imported · new descriptor ura: {}",
        imported.new_descriptor_ura
    ));
    output::detail("execution_mode", &imported.execution_mode);
    output::detail("transfer_kind", &imported.transfer_kind);
    output::detail("invokable", &imported.invokable.to_string());
    output::detail(
        "descriptor_transaction_status",
        &imported.descriptor_transaction_status,
    );
    Ok(())
}

pub fn run_forget(args: ForgetArgs) -> anyhow::Result<()> {
    if !confirm_or_cancel(forget_confirmation(&args))? {
        return Ok(());
    }
    let resp = execute_forget(&args)?;
    let removed: DescriptorRemovalResponse =
        serde_json::from_value(resp).context("parse meta.forget response")?;
    require_committed_forget_status(&removed.descriptor_transaction_status)?;
    output::success(&format!(
        "removed imported descriptor {} (source descriptor {})",
        removed.removed_descriptor, removed.source_descriptor_ura,
    ));
    output::detail(
        "descriptor_transaction_status",
        &removed.descriptor_transaction_status,
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn teach_confirmation_prompt_names_transfer_edges() {
        let args = TeachArgs {
            ability: "mentor.quote".to_string(),
            to: "easynet:///r/default/agent/user.student".to_string(),
            yes: false,
        };
        let gate = teach_confirmation(&args);

        assert!(!gate.bypassed);
        assert!(gate.prompt.contains("mentor.quote"));
        assert!(gate
            .prompt
            .contains("easynet:///r/default/agent/user.student"));
    }

    #[test]
    fn yes_flag_bypasses_interactive_prompt_only() {
        let args = LearnArgs {
            ability_ura: "easynet:///r/default/ability/user.mentor/quote".to_string(),
            learner: "student".to_string(),
            yes: true,
        };

        assert!(learn_confirmation(&args).confirm().unwrap());
    }

    #[test]
    fn acquire_response_requires_descriptor_transaction_status() {
        let missing_status = serde_json::json!({
            "new_descriptor_ura": "easynet:///r/default/agent/user.student/ability/quote",
            "execution_mode": "sandbox_first",
            "transfer_kind": "discovery_only_manifest",
            "invokable": false,
        });

        assert!(
            serde_json::from_value::<DescriptorImportResponse>(missing_status).is_err(),
            "learn must not silently ignore missing descriptor transaction status"
        );
    }

    #[test]
    fn descriptor_transfer_responses_reject_unknown_fields() {
        let grant = serde_json::from_value::<DescriptorGrantResponse>(serde_json::json!({
            "granted_descriptor": "easynet:///r/default/agent/user.mentor/ability/quote",
            "execution_mode": "sandbox_first",
            "transfer_kind": "discovery_only_manifest",
            "invokable_after_acquire": false,
            "state_code": "J200"
        }))
        .expect_err("teach response must reject read-model drift");
        assert!(
            grant.to_string().contains("state_code"),
            "schema error should name the noncanonical field: {grant}"
        );

        let import = serde_json::from_value::<DescriptorImportResponse>(serde_json::json!({
            "new_descriptor_ura": "easynet:///r/default/agent/user.student/ability/quote",
            "execution_mode": "sandbox_first",
            "transfer_kind": "discovery_only_manifest",
            "invokable": false,
            "descriptor_transaction_status": "committed",
            "descriptor_ref": "legacy"
        }))
        .expect_err("acquire response must reject descriptor projection drift");
        assert!(
            import.to_string().contains("descriptor_ref"),
            "schema error should name the noncanonical field: {import}"
        );

        let removal = serde_json::from_value::<DescriptorRemovalResponse>(serde_json::json!({
            "removed_descriptor": "easynet:///r/default/agent/user.student/ability/quote",
            "source_descriptor_ura": "easynet:///r/default/agent/user.mentor/ability/quote",
            "descriptor_transaction_status": "committed",
            "legacy_subject": "quote"
        }))
        .expect_err("forget response must reject retired aliases");
        assert!(
            removal.to_string().contains("legacy_subject"),
            "schema error should name the noncanonical field: {removal}"
        );
    }

    #[test]
    fn degraded_descriptor_transaction_status_is_success_with_warning_semantics() {
        assert_eq!(
            parse_descriptor_transaction_status("committed", "import").unwrap(),
            DescriptorTransactionStatus::Committed
        );
        assert_eq!(
            parse_descriptor_transaction_status("committed_runtime_degraded", "import").unwrap(),
            DescriptorTransactionStatus::CommittedRuntimeDegraded
        );
        assert!(parse_descriptor_transaction_status("unknown", "import")
            .unwrap_err()
            .to_string()
            .contains("unsupported transaction status"));
    }
}
