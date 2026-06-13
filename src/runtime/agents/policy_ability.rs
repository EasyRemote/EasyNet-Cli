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
// What lives here (seven-axes T3.2, CLI half)
// --------------------------------------------
//   * policy.evaluate — { decision, rule, reason, expires_at }.
//                       Backed by the TINY MATCHER over the
//                       persisted rule store
//                       (`persistence/policy_rules.rs`): first
//                       matching rule wins; an empty store or no
//                       match keeps the historical allow, said out
//                       loud as `baseline-allow`. Trust predicates
//                       rank by the Axon pb `TrustLevel` enum via
//                       `trust_ability::level_rank` — one source.
//   * policy.simulate — the SAME `decide` function (dry-run cannot
//                       drift from the binding path), but the
//                       `would_decide` field is named separately
//                       per §18 to signal "no side effects".
//
// What does NOT live here yet
// ---------------------------
//   * policy.publish / policy.list as abilities — the operator verbs
//     ship first as CLI surface over the store (T3.2 follow-up);
//     ability registration follows when a remote admin story needs it.
//   * `policy why <invocation-id>` — needs gate decisions ledgered;
//     lands with the §A6 gate rewiring milestone.
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

/// One policy verdict — produced by [`decide`], rendered by both
/// handlers so dry-run and binding decisions can never drift
/// (spec W3-E2E-2 ②: simulate is the same function, not a second
/// implementation).
struct Decision {
    allowed: bool,
    rule: Option<String>,
    reason: String,
}

impl Decision {
    fn baseline(reason: &str) -> Self {
        Decision {
            allowed: true,
            rule: None,
            reason: reason.to_string(),
        }
    }
}

/// The tiny matcher (seven-axes T3.2, spec D7 ruling: guided
/// predicates, no expression language). First matching rule wins, in
/// file order; an empty store or no match keeps the daemon's
/// historical allow — said out loud in the reason.
///
/// Predicate semantics over missing facts are conservative: a rule
/// whose predicate needs a fact the envelope doesn't carry simply
/// does not match — admission never guesses.
fn decide(envelope: &Value) -> anyhow::Result<Decision> {
    let rules = crate::persistence::policy_rules::load()?;
    if rules.rules.is_empty() {
        return Ok(Decision::baseline("baseline-allow (no rules configured)"));
    }

    let caller = envelope.get("caller").and_then(Value::as_str);
    let ability = envelope.get("ability").and_then(Value::as_str);
    // Every envelope this surface admits is an invoke; rules carry
    // `action` so richer admitted actions slot in without a schema
    // change.
    const ACTION: &str = "invoke";
    let caller_rank = caller
        .map(crate::runtime::agents::trust_ability::effective_level)
        .as_deref()
        .and_then(crate::runtime::agents::trust_ability::level_rank);

    for rule in &rules.rules {
        if rule.action != ACTION {
            continue;
        }
        if let Some(prefix) = rule.family_prefix.as_deref() {
            let Some(ability) = ability else { continue };
            if !ability.starts_with(prefix) {
                continue;
            }
        }
        if let Some(threshold) = rule.trust_below.as_deref() {
            let Some(threshold_rank) = crate::runtime::agents::trust_ability::level_rank(threshold)
            else {
                continue;
            };
            let Some(caller_rank) = caller_rank else {
                continue;
            };
            if caller_rank >= threshold_rank {
                continue;
            }
        }
        let mut matched = format!("action={ACTION}");
        if let Some(p) = rule.family_prefix.as_deref() {
            matched.push_str(&format!(" && ability.family startswith {p:?}"));
        }
        if let Some(t) = rule.trust_below.as_deref() {
            matched.push_str(&format!(" && caller_trust < {t}"));
        }
        return Ok(Decision {
            allowed: rule.effect == crate::persistence::policy_rules::RuleEffect::Allow,
            rule: Some(rule.id.clone()),
            reason: matched,
        });
    }
    Ok(Decision::baseline("baseline-allow (no rule matched)"))
}

/// `policy.evaluate` handler.
///
/// Args: `{ "invocation_envelope": { caller?, ability?, ... } }` —
/// the envelope to admit; absence of the field fails loudly so a
/// caller passing the wrong shape can't read a misleading "Allowed".
///
/// Returns: `{ decision, rule, reason, expires_at }`.
fn evaluate_handler(args: Value) -> anyhow::Result<Value> {
    let Some(envelope) = args.get("invocation_envelope") else {
        anyhow::bail!("policy.evaluate: `invocation_envelope` required");
    };
    let verdict = decide(envelope)?;
    let now = chrono::Utc::now();
    let expires = now + chrono::Duration::seconds(DECISION_TTL_SECS);
    Ok(json!({
        "decision": if verdict.allowed { "Allowed" } else { "Denied" },
        "rule": verdict.rule,
        "reason": verdict.reason,
        "expires_at": expires.to_rfc3339(),
    }))
}

/// `policy.simulate` handler — the SAME `decide` function, distinct
/// field name in the response so a tool reading the result can't
/// accidentally treat a simulation as a binding decision.
fn simulate_handler(args: Value) -> anyhow::Result<Value> {
    let Some(envelope) = args.get("invocation_envelope") else {
        anyhow::bail!("policy.simulate: `invocation_envelope` required");
    };
    let verdict = decide(envelope)?;
    Ok(json!({
        "would_decide": if verdict.allowed { "Allowed" } else { "Denied" },
        "rule": verdict.rule,
        "trace": [verdict.reason],
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
    "Admit (or reject) an invocation envelope against the persisted \
     policy rules (tiny matcher: action / ability-family / trust \
     threshold; first match wins). No rules or no match means the \
     baseline allow, reported as such."
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
    use crate::facade::cli::test_support::HomeGuard;
    use crate::persistence::policy_rules::{self, PolicyRule, PolicyRulesFile, RuleEffect};

    fn rule(id: &str, effect: RuleEffect, family: Option<&str>, below: Option<&str>) -> PolicyRule {
        PolicyRule {
            id: id.into(),
            effect,
            action: "invoke".into(),
            family_prefix: family.map(str::to_string),
            trust_below: below.map(str::to_string),
            created_at: "t0".into(),
        }
    }

    fn save_rules(rules: Vec<PolicyRule>) {
        policy_rules::save(&PolicyRulesFile { rules }).expect("save rules");
    }

    fn envelope(caller: &str, ability: &str) -> Value {
        json!({ "invocation_envelope": { "caller": caller, "ability": ability } })
    }

    #[test]
    fn registration_makes_both_dispatchable() {
        let mut reg = AxonAbilityCatalog::new();
        register(&mut reg);
        assert!(reg.get_rpc(ABILITY_EVALUATE).is_some());
        assert!(reg.get_rpc(ABILITY_SIMULATE).is_some());
    }

    #[test]
    fn empty_store_is_the_spoken_baseline_with_expiry() {
        let _g = HomeGuard::new();
        let resp = evaluate_handler(envelope("test", "ping")).unwrap();
        assert_eq!(resp["decision"], "Allowed");
        assert!(resp["rule"].is_null(), "baseline carries no rule id");
        assert!(resp["reason"].as_str().unwrap().contains("baseline-allow"));
        let expires = chrono::DateTime::parse_from_rfc3339(resp["expires_at"].as_str().unwrap())
            .expect("expires_at is RFC3339");
        assert!(expires.with_timezone(&chrono::Utc) > chrono::Utc::now());
    }

    #[test]
    fn deny_below_trust_blocks_default_caller_and_names_the_rule() {
        let _g = HomeGuard::new();
        // `deny action=invoke unless trust>=ELEVATED`: the caller's
        // baseline is STANDARD, which ranks below — deny, with the
        // rule id and the matched condition in the answer (a black-box
        // refusal is an acceptance failure, spec W3-E2E-2 ①).
        save_rules(vec![rule("pr-1", RuleEffect::Deny, None, Some("ELEVATED"))]);
        let caller = crate::ura::agent_ura("localhost", "dev", "stranger");
        let resp = evaluate_handler(envelope(&caller, "fs.read")).unwrap();
        assert_eq!(resp["decision"], "Denied");
        assert_eq!(resp["rule"], "pr-1");
        assert!(resp["reason"]
            .as_str()
            .unwrap()
            .contains("caller_trust < ELEVATED"));
    }

    #[test]
    fn trust_ruling_lifts_the_caller_over_the_threshold() {
        let _g = HomeGuard::new();
        save_rules(vec![rule("pr-1", RuleEffect::Deny, None, Some("ELEVATED"))]);
        let caller = crate::ura::agent_ura("localhost", "dev", "vip");
        let mut directory = crate::persistence::trust_levels::TrustLevelsFile::default();
        directory.upsert(&caller, "PRIVILEGED", "t0", None);
        crate::persistence::trust_levels::save(&directory).expect("seed trust");

        let resp = evaluate_handler(envelope(&caller, "fs.read")).unwrap();
        assert_eq!(
            resp["decision"], "Allowed",
            "PRIVILEGED outranks the ELEVATED threshold — PROTECT axes interlock"
        );
        assert!(resp["reason"].as_str().unwrap().contains("no rule matched"));
    }

    #[test]
    fn family_prefix_scopes_the_rule_and_never_routes() {
        let _g = HomeGuard::new();
        save_rules(vec![rule("pr-1", RuleEffect::Deny, Some("aris."), None)]);
        let caller = crate::ura::agent_ura("localhost", "dev", "x");
        let in_family = evaluate_handler(envelope(&caller, "aris.review")).unwrap();
        assert_eq!(in_family["decision"], "Denied");
        let outside = evaluate_handler(envelope(&caller, "fs.read")).unwrap();
        assert_eq!(
            outside["decision"], "Allowed",
            "family is scope, not routing"
        );
    }

    #[test]
    fn first_match_wins_in_file_order() {
        let _g = HomeGuard::new();
        save_rules(vec![
            rule("pr-1", RuleEffect::Allow, Some("aris."), None),
            rule("pr-2", RuleEffect::Deny, None, None),
        ]);
        let caller = crate::ura::agent_ura("localhost", "dev", "x");
        let aris = evaluate_handler(envelope(&caller, "aris.review")).unwrap();
        assert_eq!(aris["decision"], "Allowed");
        assert_eq!(aris["rule"], "pr-1");
        let other = evaluate_handler(envelope(&caller, "fs.read")).unwrap();
        assert_eq!(other["decision"], "Denied");
        assert_eq!(other["rule"], "pr-2");
    }

    #[test]
    fn simulate_is_the_same_decision_with_the_dry_run_shape() {
        let _g = HomeGuard::new();
        save_rules(vec![rule("pr-1", RuleEffect::Deny, None, Some("ELEVATED"))]);
        let caller = crate::ura::agent_ura("localhost", "dev", "stranger");
        let sim = simulate_handler(envelope(&caller, "fs.read")).unwrap();
        assert_eq!(sim["would_decide"], "Denied");
        assert_eq!(sim["rule"], "pr-1");
        assert!(
            sim.get("decision").is_none(),
            "simulate must NOT use `decision`"
        );
        let real = evaluate_handler(envelope(&caller, "fs.read")).unwrap();
        assert_eq!(
            real["decision"], "Denied",
            "dry-run equals the binding path"
        );
    }

    #[test]
    fn evaluate_rejects_missing_envelope_field() {
        let err = evaluate_handler(json!({})).unwrap_err();
        assert!(format!("{err}").contains("invocation_envelope"));
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
