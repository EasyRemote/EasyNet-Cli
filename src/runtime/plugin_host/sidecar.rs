// EasyNet CLI — sidecar plugin host boundary
// ==========================================
//
// File: src/runtime/plugin_host/sidecar.rs
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

use crate::runtime::ability_dispatch::EnvelopeContext;
use crate::runtime::plugin_host::errors::{PluginHostError, Result};

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
    ability: &str,
    args: Value,
) -> Result<SidecarInvocationEnvelope> {
    let caller = env
        .caller
        .ok_or_else(|| PluginHostError::SidecarProtocolViolation {
            message: format!("missing caller in Axon envelope for sidecar ability {ability}"),
        })?;
    let callee = env
        .callee
        .ok_or_else(|| PluginHostError::SidecarProtocolViolation {
            message: format!("missing callee in Axon envelope for sidecar ability {ability}"),
        })?;
    let subject = env
        .subject
        .ok_or_else(|| PluginHostError::SidecarProtocolViolation {
            message: format!("missing subject in Axon envelope for sidecar ability {ability}"),
        })?;
    let invocation_nonce =
        env.invocation_nonce
            .ok_or_else(|| PluginHostError::SidecarProtocolViolation {
                message: format!("missing nonce in Axon envelope for sidecar ability {ability}"),
            })?;
    let causal_context =
        env.causal_context
            .ok_or_else(|| PluginHostError::SidecarProtocolViolation {
                message: format!(
                    "missing causal context in Axon envelope for sidecar ability {ability}"
                ),
            })?;
    Ok(SidecarInvocationEnvelope {
        caller,
        callee,
        ability: env.ability.unwrap_or_else(|| ability.to_string()),
        subject,
        invocation_nonce,
        causal_context,
        args,
    })
}
