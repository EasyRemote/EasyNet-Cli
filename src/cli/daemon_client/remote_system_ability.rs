//! CLI facade for target-owned remote system ability dispatch.
//!
//! Command modules should map argv into ability arguments and call this facade.
//! The lower daemon invocation routing primitive remains owned by
//! `daemon::invocation`; this module is the CLI boundary that knows how simple
//! device/hub system ability sugar projects onto that primitive.

use serde_json::Value;

#[cfg(feature = "axon-pb")]
use anyhow::Context;

#[cfg(feature = "axon-pb")]
use crate::daemon::invocation::routing::remote_invoke::{
    self, RemoteAbilityInvocationTarget, RemoteInvocationRequest,
};

#[cfg(feature = "axon-pb")]
pub(crate) fn invoke_remote_device_system_ability(
    node: &str,
    selector: &str,
    args: Value,
    action_label: &str,
) -> anyhow::Result<Value> {
    let _ = action_label;
    let target_ura = crate::support::platform::remote_device::resolve_target_device_ura(node)?;
    let caller_ura =
        crate::support::platform::remote_device::require_caller_device_ura_from_credentials()?;
    invoke_target_owned_system_ability(&target_ura, selector, args, &caller_ura)
        .with_context(|| format!("forward {selector} to remote device target={target_ura}"))
}

#[cfg(not(feature = "axon-pb"))]
pub(crate) fn invoke_remote_device_system_ability(
    node: &str,
    _selector: &str,
    _args: Value,
    action_label: &str,
) -> anyhow::Result<Value> {
    let label = if action_label.trim().is_empty() {
        format!("invoking a remote system ability on node {node:?}")
    } else {
        action_label.to_string()
    };
    Err(crate::support::platform::local_invoke::federation_not_wired_error(&label))
}

#[cfg(feature = "axon-pb")]
pub(crate) fn invoke_current_realm_hub_system_ability(
    selector: &str,
    args: Value,
) -> anyhow::Result<Option<Value>> {
    let Ok(creds) = crate::daemon::persistence::config::load_credentials() else {
        return Ok(None);
    };
    let realm = creds.realm_str().trim();
    let node_id = creds.node_id.trim();
    if realm.is_empty() || node_id.is_empty() {
        return Ok(None);
    }

    let hub_ura = crate::core::ura::hub_ura(realm);
    let caller_ura = crate::core::ura::device_ura(realm, node_id);
    let value = invoke_target_owned_system_ability(&hub_ura, selector, args, &caller_ura)
        .with_context(|| format!("invoke {selector} against realm hub"))?;
    Ok(Some(value))
}

#[cfg(not(feature = "axon-pb"))]
pub(crate) fn invoke_current_realm_hub_system_ability(
    _selector: &str,
    _args: Value,
) -> anyhow::Result<Option<Value>> {
    Ok(None)
}

#[cfg(feature = "axon-pb")]
fn invoke_target_owned_system_ability(
    execution_target_ura: &str,
    selector: &str,
    args: Value,
    caller_ura: &str,
) -> anyhow::Result<Value> {
    let target_call =
        RemoteAbilityInvocationTarget::for_target_owned_selector(execution_target_ura, selector)?;
    let subject_ura = target_owned_system_subject_ura(&target_call)?;
    let request = RemoteInvocationRequest::new(
        &target_call,
        caller_ura,
        subject_ura,
        axon_sdk::invocation::fresh_nonce(),
        axon_sdk::invocation::CausalContext::None,
        args,
        std::time::Duration::from_secs(30),
    )?;
    remote_invoke::invoke_remote_target(request)
}

#[cfg(feature = "axon-pb")]
fn target_owned_system_subject_ura(
    target: &RemoteAbilityInvocationTarget,
) -> anyhow::Result<String> {
    let callee = crate::core::ura::parse_ura(target.callee_ura())
        .map_err(|error| anyhow::anyhow!("remote system callee URA is invalid: {error}"))?;
    match callee.kind {
        crate::core::ura::URAKind::Device => Ok(target.callee_ura().to_string()),
        crate::core::ura::URAKind::Hub => Ok(target.as_str().to_string()),
        other => anyhow::bail!(
            "target-owned remote system ability requires Device or Hub callee, got {other}"
        ),
    }
}
