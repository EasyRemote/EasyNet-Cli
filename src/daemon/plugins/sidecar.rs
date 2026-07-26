// EasyNet CLI — sidecar plugin host boundary
// ==========================================
//
// File: src/daemon/plugins/sidecar.rs
// Description: Public facade for sidecar plugin process execution.

mod bidi;
mod command;
mod frame;
mod io;
mod process;
mod stream;
#[cfg(test)]
mod tests;

use serde_json::Value;

use crate::daemon::ability::dispatch::EnvelopeContext;
use crate::daemon::plugins::errors::{PluginHostError, Result};

pub use command::{SidecarCommand, SidecarExecutionModel};
pub use frame::{SidecarInvocationEnvelope, SidecarRequestFrame, SidecarResponseFrame};
pub use process::{SidecarRuntimeHost, SidecarRuntimeLimits};

/// Build a sidecar invocation envelope from the daemon's Axon envelope context.
///
/// What this is NOT: a sidecar-owned interpretation of invocation identity.
/// The daemon has already performed tuple construction before this projection;
/// missing caller, callee, subject, or nonce is therefore a host boundary error.
pub fn sidecar_invocation_from_context(
    env: EnvelopeContext,
    dispatch_ability: &str,
    args: Value,
) -> Result<SidecarInvocationEnvelope> {
    let ability_ura = canonical_sidecar_ability_ura(&env, dispatch_ability)?;
    Ok(SidecarInvocationEnvelope {
        caller_ura: env.caller().to_string(),
        callee_ura: env.callee().to_string(),
        ability_ura,
        subject_ura: env.subject().to_string(),
        invocation_nonce: env.invocation_nonce().to_vec(),
        causal_context: env.causal_context().clone(),
        args,
    })
}

fn canonical_sidecar_ability_ura(env: &EnvelopeContext, dispatch_ability: &str) -> Result<String> {
    let envelope_ability = env.ability().trim();
    if let Ok(ability_ura) =
        crate::daemon::axon_bridge::descriptor_ref::ability_ura_from_descriptor_ref(
            envelope_ability,
        )
    {
        return Ok(ability_ura);
    }
    if crate::core::ura::AbilitySelector::parse(envelope_ability).is_ok() {
        return Ok(envelope_ability.to_string());
    }

    let public_name =
        crate::core::ura::descriptor_public_ability_name(env.callee(), dispatch_ability);
    let ability_ura =
        crate::core::ura::owner_ability_ura(env.callee(), &public_name).ok_or_else(|| {
            PluginHostError::SidecarProtocolViolation {
                message: format!(
                    "cannot derive canonical sidecar ability_ura for callee_ura {:?} \
                 dispatch ability {:?}",
                    env.callee(),
                    dispatch_ability
                ),
            }
        })?;
    crate::core::ura::AbilitySelector::parse(&ability_ura).map_err(|error| {
        PluginHostError::SidecarProtocolViolation {
            message: format!("derived invalid sidecar ability_ura {ability_ura:?}: {error}"),
        }
    })?;
    Ok(ability_ura)
}
