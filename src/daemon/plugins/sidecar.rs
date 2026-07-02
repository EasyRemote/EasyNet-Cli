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

use crate::daemon::plugins::errors::Result;
use crate::runtime::ability_dispatch::EnvelopeContext;

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
    let _ = ability;
    Ok(SidecarInvocationEnvelope {
        caller: env.caller().to_string(),
        callee: env.callee().to_string(),
        ability: env.ability().to_string(),
        subject: env.subject().to_string(),
        invocation_nonce: env.invocation_nonce().to_vec(),
        causal_context: env.causal_context().clone(),
        args,
    })
}
