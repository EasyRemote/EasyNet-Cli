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
use crate::daemon::plugins::errors::Result;

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
    ability_ura: &str,
    args: Value,
) -> Result<SidecarInvocationEnvelope> {
    Ok(SidecarInvocationEnvelope {
        caller_ura: env.caller().to_string(),
        callee_ura: env.callee().to_string(),
        ability_ura: ability_ura.to_string(),
        subject_ura: env.subject().to_string(),
        invocation_nonce: env.invocation_nonce().to_vec(),
        causal_context: env.causal_context().clone(),
        args,
    })
}
