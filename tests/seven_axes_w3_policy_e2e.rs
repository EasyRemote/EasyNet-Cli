//! Seven-axes W3 — `easynet policy simulate` end-to-end
//! ======================================================
//!
//! File: tests/seven_axes_w3_policy_e2e.rs
//! Spec: docs/spec/seven-axes-p0-landing-v1.md §3 W3-E2E-2 (the
//! daemon-side subset; `policy why` and the §A6 gate rewiring are
//! the Axon-batch items per the v1.4 rescoping).
//! Fixture: `seven_axes_fixture` — same real UDS daemon stack as
//! W1/W2.
//!
//! Covered, over the real wire:
//!   ①  a deny is never a black box: the refusal carries the rule id
//!      and the matched condition;
//!   ②  dry-run equals the binding path by construction (`simulate`
//!      and `evaluate` share one `decide` — asserted at unit level;
//!      here we assert the dry-run *shape*: `would_decide`, never
//!      `decision`);
//!   ③  rule removal restores the spoken baseline;
//!   plus the PROTECT interlock end-to-end: a `trust level set`
//!   through the daemon lifts the same caller over a
//!   `deny-below-ELEVATED` rule — the trust directory (T2.1) feeds
//!   the policy matcher (T3.2) across the wire, not just in unit
//!   tests.
//!
//! Rules are seeded by writing `policy-rules.json` — the documented
//! operator-inspectable store (`persistence/policy_rules.rs` header);
//! the daemon reloads it per decision, so edits apply immediately.
//!
//! One `#[test]` on purpose — fixture owns process env (see fixture
//! header).
//!
//! Author: Silan Hu <silan.hu@u.nus.edu>
//! Copyright (c) 2026 EasyNet. All rights reserved.

#![cfg(all(feature = "axon-pb", unix))]

mod seven_axes_fixture;

use easynet_cli::facade::cli::policy_cli::{self, OutputFormat, SimulateArgs};
use easynet_cli::facade::cli::trust_level::{self, SetArgs};
use seven_axes_fixture::SevenAxesHome;

fn simulate(caller: &str, ability: &str) -> SimulateArgs {
    SimulateArgs {
        caller: caller.to_string(),
        ability: ability.to_string(),
        format: OutputFormat::Table,
    }
}

fn write_rules(home: &SevenAxesHome, rules: serde_json::Value) {
    let path = home.home.path().join(".easynet/policy-rules.json");
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&serde_json::json!({ "rules": rules })).unwrap(),
    )
    .expect("write policy-rules.json");
}

#[test]
fn policy_simulate_e2e_explainable_deny_and_trust_interlock() {
    let home = SevenAxesHome::seed();
    let daemon = home.start_daemon();
    let caller = home.testbot_ura.clone();

    // Baseline: empty store answers allow, and says it's a baseline.
    let baseline = policy_cli::execute_simulate(&simulate(&caller, "fs.read"))
        .expect("simulate against live daemon");
    assert_eq!(baseline["would_decide"], "Allowed");
    assert!(baseline["rule"].is_null(), "baseline names no rule");
    assert!(
        baseline.get("decision").is_none(),
        "dry-run shape must use would_decide, never decision (②)"
    );

    // ① An explainable deny: rule id + matched condition, never a
    // black box.
    write_rules(
        &home,
        serde_json::json!([{
            "id": "pr-1", "effect": "deny", "action": "invoke",
            "trust_below": "ELEVATED", "created_at": "2026-06-13T00:00:00+00:00",
        }]),
    );
    let denied =
        policy_cli::execute_simulate(&simulate(&caller, "fs.read")).expect("simulate deny");
    assert_eq!(denied["would_decide"], "Denied");
    assert_eq!(denied["rule"], "pr-1", "a refusal must name its rule (①)");
    let trace = denied["trace"][0].as_str().unwrap_or_default();
    assert!(
        trace.contains("caller_trust < ELEVATED"),
        "the matched condition must reach the operator: {trace}"
    );

    // PROTECT interlock over the wire: lift the caller's trust via
    // `identity.set_trust` (T2.1) and the same rule stops matching.
    trust_level::execute_set(&SetArgs {
        agent_ura: caller.clone(),
        level: "privileged".into(),
        yes: true,
        format: trust_level::OutputFormat::Table,
    })
    .expect("lift trust through the daemon");
    let lifted = policy_cli::execute_simulate(&simulate(&caller, "fs.read"))
        .expect("simulate after trust lift");
    assert_eq!(
        lifted["would_decide"], "Allowed",
        "PRIVILEGED outranks the ELEVATED threshold — trust directory feeds the matcher"
    );

    // ③ Removing the rule restores the spoken baseline.
    write_rules(&home, serde_json::json!([]));
    let restored = policy_cli::execute_simulate(&simulate(&caller, "fs.read"))
        .expect("simulate after rule removal");
    assert_eq!(restored["would_decide"], "Allowed");
    assert!(restored["rule"].is_null());

    drop(daemon);
}
