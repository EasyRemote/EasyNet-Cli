// EasyNet CLI — `device.describe` ability handler
// =================================================
//
// File: src/runtime/agents/device_describe_ability.rs
//
// Why this exists
// ---------------
// `device.describe` is the unified-path replacement for the
// self-arm of `fleet.describe_node`. Under the joint plan
// (海峰 + 凉冰, 2026-05-03) every cross-device dispatch flows
// through `federation.forward_invoke`; an ability that wants to
// "describe the device that is hosting me" no longer takes a
// `node_id` argument because the routing decision is upstream
// (the caller decides which `target_uri` to forward to). Each
// daemon then describes ITSELF and only itself; the cross-realm
// fan-out — federation.discover — emits its own DirectoryEntry
// shape and is the right surface for "what devices exist in the
// federation".
//
// Naming
// ------
// `device.describe` (not `fleet.describe_node`):
//   * matches the URA `device` role we already canonicalised on,
//   * drops the `fleet.*` prefix that AXON-RFC-001 P1.5 left as
//     a placeholder for the `bridge.*` cull,
//   * is the noun-verb shape every other ability uses
//     (`process.exec`, `shell.run`, `observe.health`).
//
// Wire shape
// ----------
// Input  : `{}` (no arguments — describes "this device").
// Output : the same JSON envelope `fleet.describe_node` returns
//          for its self-arm, so the existing CLI renderer
//          (`facade::cli::groups::device::run_show`) keeps
//          working. Specifically:
//          * `node_id`, `tenant_id`, `agent_uri`, `is_self: true`,
//            `paired`, `hub_endpoint`, `state`, `online`,
//            `probe_status`, `probe_error`, `latency_ms`
//          * `abilities`: array projected from the realm
//            directory's per-device ability list when the device
//            is paired and `federation.resolve` succeeds; absent
//            otherwise (the renderer falls back to local
//            `easynet.discover`).
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::runtime::ability_dispatch::LocalAbilityRegistry;
use crate::runtime::ability_dispatch::OwnerKind;
use crate::runtime::agents::federation_probe;

/// Wire name. Stable contract; rename = wire break.
pub const ABILITY_NAME: &str = "device.describe";

/// Register the handler. Stateless; no per-call setup.
pub fn register(reg: &mut LocalAbilityRegistry) {
    reg.register_rpc_with_owner(ABILITY_NAME, OwnerKind::Device, Arc::new(handler));
}

fn handler(_args: Value) -> anyhow::Result<Value> {
    let local = federation_probe::local_identity();
    if local.paired {
        // Try the directory-resolve path first — paired devices
        // get the rich hub-side view (last_seen, federated peers,
        // attested abilities). Failure is non-fatal: a docker /
        // CLI-only deployment may not embed an axon-runtime, in
        // which case `resolve_device_record` returns an Err
        // ("local runtime bridge is unavailable: no running
        // runtime"). We log and fall through to the probe view —
        // surfacing 200 with the local-only fields the operator
        // expects from `device show` rather than aborting the
        // call.
        match federation_probe::resolve_device_record(&local.node_id) {
            Ok(Some(record)) => {
                let mut value = federation_probe::node_to_json(&record.node);
                if let Value::Object(map) = &mut value {
                    map.insert("abilities".to_string(), Value::Array(record.abilities));
                }
                return Ok(value);
            }
            Ok(None) => {
                // No directory record yet (fresh pair); fall through.
            }
            Err(e) => {
                eprintln!(
                    "[device.describe] directory resolve failed (falling back to probe view): {e}"
                );
            }
        }
    }
    // Unpaired or directory-resolve failed: fall back to the
    // probe view so the caller still sees node_id / tenant_id /
    // online state. `device show` already renders the missing
    // hardware-detail fields as `-`, so this best-effort shape
    // matches the existing UI contract.
    let view = federation_probe::collect_fleet_view();
    let node = view
        .nodes
        .iter()
        .find(|n| n.is_self)
        .ok_or_else(|| anyhow::anyhow!("device.describe: local node is unavailable"))?;
    Ok(federation_probe::node_to_json(node))
}

pub fn description() -> &'static str {
    "Describe the device hosting THIS daemon. No arguments — \
     cross-device addressing is the caller's job (route through \
     federation.forward_invoke against the target device URA). \
     Returns the same envelope shape `fleet.describe_node` \
     produced for its self-arm so existing CLI renderers keep \
     working unchanged."
}

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {}
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_schema_accepts_empty_object() {
        let schema = input_schema();
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["additionalProperties"], false);
    }
}
