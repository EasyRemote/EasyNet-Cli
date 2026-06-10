// EasyNet CLI — policy.{evaluate, simulate} ability handlers
// =============================================================
//
// File: src/runtime/agents/policy_ability.rs
//
// Per RFC §A6, the kernel admission gate is supposed to call
// policy.evaluate as an in-process sub-invocation carrying
// `admission_internal=true`, with the original envelope as args.
// Today the gate calls into a PermissionService / consent broker
// directly; rewiring it to use this ability is its own milestone
// (the gate change touches the kernel's hot path and needs its own
// review). This file lands the *callable surface* so the contract
// pins down the wire shape and external auditors / operators can
// dry-run via policy.simulate without waiting for the gate
// rewiring.
//
// What lives here
// ---------------
//   * policy.evaluate — { decision, reason, expires_at }
//                       v1: always Allowed with a "v1-allow-all"
//                       reason and an expiry 5 min from now. This
//                       matches the AllowAllBroker default so the
//                       contract response is honest about today's
//                       behaviour. A future config-driven evaluator
//                       lands behind the same handler signature.
//   * policy.simulate — same shape, same outcome, but the
//                       `would_decide` field is named separately
//                       per §18 to signal "no side effects".
//
// What does NOT live here yet
// ---------------------------
//   * policy.publish / policy.list — operator/admin verbs over a
//     persisted policy store. No store yet; deferred.
//   * Real evaluation logic — see the milestone note above.
//
// AXIOM correspondence
// --------------------
// The §A6 invariant ("admission_internal flag is kernel-local,
// never accepted from a remote envelope") is enforced at the
// transport layer (see ability_dispatch / IPC), not here. This
// handler runs the same way regardless of how the call was framed;
// the flag controls whether the call recurses into another policy
// admission round, which is a kernel concern.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::runtime::ability_dispatch::AxonAbilityCatalog;

use crate::runtime::ability_dispatch::OwnerKind;
use crate::runtime::agents::profiles::DEFAULT_POLICY_AGENT_ID;
pub const ABILITY_EVALUATE: &str = "policy.evaluate";
pub const ABILITY_SIMULATE: &str = "policy.simulate";

/// v1 decision-cache TTL. Long enough that a chatty admission gate
/// doesn't re-call evaluate per envelope, short enough that a
/// future config flip propagates within minutes. Magic numbers
/// concentrated here so a future evaluator knob lifts them to
/// config without hunting through callers.
const DECISION_TTL_SECS: i64 = 300;

pub fn register(reg: &mut AxonAbilityCatalog) {
    reg.register_rpc_with_owner(
        "policy.evaluate",
        OwnerKind::Agent(DEFAULT_POLICY_AGENT_ID.to_string()),
        Arc::new(|args: Value| evaluate_handler(args)),
    );
    reg.register_rpc_with_owner(
        "policy.simulate",
        OwnerKind::Agent(DEFAULT_POLICY_AGENT_ID.to_string()),
        Arc::new(|args: Value| simulate_handler(args)),
    );
}

/// `policy.evaluate` handler.
///
/// Args: `{ "invocation_envelope": { ... } }` — the envelope to
/// admit. v1 doesn't introspect the envelope; it just asserts the
/// field is present so a caller passing the wrong shape fails loudly
/// rather than getting a misleading "Allowed" for nothing.
///
/// Returns: `{ decision: "Allowed", reason, expires_at }`.
fn evaluate_handler(args: Value) -> anyhow::Result<Value> {
    if args.get("invocation_envelope").is_none() {
        anyhow::bail!("policy.evaluate: `invocation_envelope` required");
    }
    let now = chrono::Utc::now();
    let expires = now + chrono::Duration::seconds(DECISION_TTL_SECS);
    Ok(json!({
        "decision": "Allowed",
        "reason": "v1-allow-all (config-driven evaluator pending)",
        "expires_at": expires.to_rfc3339(),
    }))
}

/// `policy.simulate` handler — same logic, distinct field name in
/// the response so a tool reading the result can't accidentally
/// treat a simulation as a binding decision.
fn simulate_handler(args: Value) -> anyhow::Result<Value> {
    if args.get("invocation_envelope").is_none() {
        anyhow::bail!("policy.simulate: `invocation_envelope` required");
    }
    Ok(json!({
        "would_decide": "Allowed",
        "trace": ["v1-allow-all (config-driven evaluator pending)"],
    }))
}

// ── Discovery surfaces ────────────────────────────────────────

pub fn evaluate_input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["invocation_envelope"],
        "properties": {
            "invocation_envelope": {"type": "object"},
        },
        "additionalProperties": false,
    })
}

pub fn evaluate_description() -> &'static str {
    "Admit (or reject) an invocation envelope. v1 returns Allowed \
     for all callers; the config-driven evaluator lands behind the \
     same handler signature in a follow-up."
}

pub fn simulate_input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["invocation_envelope"],
        "properties": {
            "invocation_envelope": {"type": "object"},
        },
        "additionalProperties": false,
    })
}

pub fn simulate_description() -> &'static str {
    "Dry-run policy.evaluate without side effects. Returns the same \
     decision policy.evaluate would, named `would_decide` so a tool \
     can't mistake the output for a binding decision."
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_makes_both_dispatchable() {
        let mut reg = AxonAbilityCatalog::new();
        register(&mut reg);
        assert!(reg.get_rpc(ABILITY_EVALUATE).is_some());
        assert!(reg.get_rpc(ABILITY_SIMULATE).is_some());
    }

    #[test]
    fn evaluate_returns_allowed_with_expiry() {
        let resp = evaluate_handler(json!({
            "invocation_envelope": {"caller": "test", "ability": "ping"}
        }))
        .unwrap();
        assert_eq!(resp["decision"], "Allowed");
        assert!(resp["reason"].as_str().unwrap().contains("v1-allow-all"));
        // Expires_at is a parseable RFC3339 string in the future.
        let expires = chrono::DateTime::parse_from_rfc3339(resp["expires_at"].as_str().unwrap())
            .expect("expires_at is RFC3339");
        let now = chrono::Utc::now();
        assert!(expires.with_timezone(&chrono::Utc) > now);
    }

    #[test]
    fn evaluate_rejects_missing_envelope_field() {
        let err = evaluate_handler(json!({})).unwrap_err();
        assert!(format!("{err}").contains("invocation_envelope"));
    }

    #[test]
    fn simulate_returns_would_decide_not_decision() {
        // Deliberately distinct from evaluate's output so a code
        // path that confused the two would surface a missing key.
        let resp = simulate_handler(json!({
            "invocation_envelope": {"caller": "test"}
        }))
        .unwrap();
        assert!(resp.get("would_decide").is_some());
        assert!(
            resp.get("decision").is_none(),
            "simulate must NOT use `decision`"
        );
        assert_eq!(resp["would_decide"], "Allowed");
    }

    #[test]
    fn simulate_rejects_missing_envelope_field() {
        let err = simulate_handler(json!({})).unwrap_err();
        assert!(format!("{err}").contains("invocation_envelope"));
    }

    #[test]
    fn input_schemas_require_envelope() {
        for s in [evaluate_input_schema(), simulate_input_schema()] {
            assert_eq!(s["type"], "object");
            let req = s["required"].as_array().unwrap();
            assert!(req.iter().any(|v| v == "invocation_envelope"));
            assert_eq!(s["additionalProperties"], false);
        }
    }
}
