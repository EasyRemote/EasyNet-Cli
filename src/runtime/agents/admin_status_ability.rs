// EasyNet CLI — admin.status ability handler
// =============================================
//
// File: src/runtime/agents/admin_status_ability.rs
//
// Per §18: input `{}`, body `{status, components[]}`. Sibling of
// observe.health (one-line liveness probe) and observe.network_health
// (network-side posture). admin.status is the operator-facing
// component view: each subsystem's name + a coarse state, plus the
// daemon's build version. The intent is "what would I show on a
// `easynet status` dashboard" — cheap to gather, structured enough
// to render.
//
// What v1 returns
// ---------------
//   {
//     "status":  "ok" | "degraded",
//     "version": "<crate version>",
//     "components": [
//       { "name": "membership",        "state": "joined" | "unjoined" },
//       { "name": "ability_registry",  "state": "ok", "count": <n> },
//       { "name": "hosted_agents",     "state": "ok", "count": <n> }
//     ]
//   }
//
// Aggregate `status` is "ok" when membership is joined and the
// registry has at least one ability; otherwise "degraded". This is
// not a full SLO — just a quick rollup so an operator hitting
// admin.status from a remote shell sees one line that says "fine"
// or "look closer."
//
// What v1 does NOT do
// -------------------
// Probe live gRPC reachability, query the kernel for in-flight
// invocations, or report per-component error counters. Same rationale
// as network_health.rs: the daemon doesn't currently retain the
// handles the agents/ layer would read those off of.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::runtime::ability_dispatch::LocalAbilityRegistry;

use crate::runtime::ability_dispatch::OwnerKind;
pub const ABILITY_ADMIN_STATUS: &str = "device.admin.status";

/// Register `admin.status` on the registry.
///
/// `ability_count_provider` runs at handler-call time so the count
/// reflects whatever the registry holds when the call lands (not
/// what it held at boot — relevant once hot-add of abilities exists).
pub fn register<F>(reg: &mut LocalAbilityRegistry, ability_count_provider: F)
where
    F: Fn() -> usize + Send + Sync + 'static,
{
    let provider: Arc<dyn Fn() -> usize + Send + Sync> = Arc::new(ability_count_provider);
    reg.register_rpc_with_owner(
        "device.admin.status",
        OwnerKind::Device,
        Arc::new(move |_args: Value| handler(&provider)),
    );
}

fn handler(ability_count_provider: &Arc<dyn Fn() -> usize + Send + Sync>) -> anyhow::Result<Value> {
    let local = crate::persistence::local_agents::load().unwrap_or_default();
    let joined = !local.host_device_agent_uri.is_empty();
    let ability_count = ability_count_provider();
    let hosted_count = local.hosted_agents.len();

    let aggregate = if joined && ability_count > 0 {
        "ok"
    } else {
        "degraded"
    };

    Ok(json!({
        "status":  aggregate,
        "version": env!("CARGO_PKG_VERSION"),
        "components": [
            { "name": "membership",
              "state": if joined { "joined" } else { "unjoined" } },
            { "name": "ability_registry",
              "state": "ok",
              "count": ability_count },
            { "name": "hosted_agents",
              "state": "ok",
              "count": hosted_count },
        ],
    }))
}

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {},
        "additionalProperties": false,
    })
}

pub fn description() -> &'static str {
    "Operator-facing component snapshot: build version, membership \
     state, ability count, hosted-Agent count. v1 is a cheap rollup \
     for `easynet status` dashboards; live probes land later."
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_makes_admin_status_dispatchable() {
        let mut reg = LocalAbilityRegistry::new();
        register(&mut reg, || 0);
        assert!(reg.get_rpc(ABILITY_ADMIN_STATUS).is_some());
    }

    #[test]
    fn handler_emits_required_fields() {
        // Whatever the host's join state, every contract field MUST
        // be present so a downstream caller doesn't hit a missing-key
        // error. The aggregate `status` is one of the two pinned
        // string values.
        let provider: Arc<dyn Fn() -> usize + Send + Sync> = Arc::new(|| 7);
        let resp = handler(&provider).unwrap();
        for field in ["status", "version", "components"] {
            assert!(
                resp.get(field).is_some(),
                "response missing required field {field}"
            );
        }
        assert!(matches!(
            resp["status"].as_str(),
            Some("ok") | Some("degraded")
        ));
        let comps = resp["components"].as_array().unwrap();
        assert_eq!(comps.len(), 3, "components must list all three subsystems");
        let names: Vec<&str> = comps.iter().map(|c| c["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"membership"));
        assert!(names.contains(&"ability_registry"));
        assert!(names.contains(&"hosted_agents"));
    }

    #[test]
    fn ability_count_is_pulled_from_provider_at_call_time() {
        // The provider closure is evaluated per handler call so a
        // future hot-add of abilities is reflected without
        // re-registration. Pin that property by mutating a shared
        // counter and asserting the second call sees the new value.
        use std::sync::atomic::{AtomicUsize, Ordering};
        let counter = Arc::new(AtomicUsize::new(3));
        let counter_handle = counter.clone();
        let provider: Arc<dyn Fn() -> usize + Send + Sync> =
            Arc::new(move || counter_handle.load(Ordering::SeqCst));

        let r1 = handler(&provider).unwrap();
        assert_eq!(r1["components"][1]["count"], 3);

        counter.store(11, Ordering::SeqCst);
        let r2 = handler(&provider).unwrap();
        assert_eq!(r2["components"][1]["count"], 11);
    }

    #[test]
    fn version_is_the_crate_version() {
        let provider: Arc<dyn Fn() -> usize + Send + Sync> = Arc::new(|| 1);
        let resp = handler(&provider).unwrap();
        assert_eq!(resp["version"], env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn input_schema_is_empty_object() {
        let s = input_schema();
        assert_eq!(s["type"], "object");
        assert!(s["properties"].as_object().unwrap().is_empty());
        assert_eq!(s["additionalProperties"], false);
    }
}
