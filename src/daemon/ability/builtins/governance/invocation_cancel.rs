//! Descriptor-bound invocation cancellation command.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::daemon::ability::catalog::system_manifest::registry_manifest;
use crate::daemon::ability::dispatch::{AxonAbilityCatalog, EnvelopeContext, OwnerKind};
use crate::daemon::invocation::dispatch::cancellation::{
    InvocationCancelCommand, InvocationCancellationRegistry, ABILITY_INVOCATION_CANCEL,
};

pub fn register(registry: &mut AxonAbilityCatalog, cancellations: InvocationCancellationRegistry) {
    for owner in [OwnerKind::Device, OwnerKind::RealmAuthority] {
        let cancellations = cancellations.clone();
        registry.register_rpc_with_envelope_and_spec(
            ABILITY_INVOCATION_CANCEL,
            owner,
            registry_manifest(ABILITY_INVOCATION_CANCEL, description(), input_schema()),
            Arc::new(move |envelope, args| execute(&cancellations, envelope, args)),
        );
    }
}

fn execute(
    cancellations: &InvocationCancellationRegistry,
    envelope: EnvelopeContext,
    args: Value,
) -> anyhow::Result<Value> {
    let command: InvocationCancelCommand = serde_json::from_value(args)
        .map_err(|error| anyhow::anyhow!("invocation.cancel: invalid command: {error}"))?;
    let command = InvocationCancelCommand::new(
        command.target_lifecycle_hash,
        command.target_invocation_id,
        command.reason,
    )?;
    let runtime = tokio::runtime::Handle::try_current()
        .map_err(|error| anyhow::anyhow!("invocation.cancel: runtime unavailable: {error}"))?;
    let result = runtime.block_on(cancellations.request_cancel(
        command,
        envelope.caller(),
        envelope.callee(),
    ))?;
    serde_json::to_value(result)
        .map_err(|error| anyhow::anyhow!("invocation.cancel: encode result: {error}"))
}

pub fn description() -> &'static str {
    "Request cancellation of one registered Invocation lifecycle owned by this authority."
}

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["target_lifecycle_hash", "reason"],
        "additionalProperties": false,
        "properties": {
            "target_lifecycle_hash": {
                "type": "string",
                "pattern": "^[0-9a-fA-F]{64}$"
            },
            "target_invocation_id": {
                "type": "string",
                "minLength": 1
            },
            "reason": {
                "type": "string",
                "maxLength": 1024
            }
        }
    })
}
