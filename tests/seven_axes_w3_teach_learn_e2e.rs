//! Seven-axes W3 — `easynet ability teach/learn/forget` end-to-end
//! =================================================================
//!
//! File: tests/seven_axes_w3_teach_learn_e2e.rs
//! Spec: docs/spec/seven-axes-p0-landing-v1.md §3 W3-E2E-3
//! (single-device subset per the D6 v1 ruling: same-device,
//! manifest-only).
//! Fixture: `seven_axes_fixture` — `testbot` owns `weather-probe`;
//! `zlearner` owns nothing and exists to learn.
//!
//! Covered, over the real wire — owner initiative three ways:
//!   ①  the default refusal comes FIRST: without a grant, learn
//!      refuses and names the gate (`allow_transferred_code=false`);
//!   ②  after teach → learn: the learner owns a NEW ability under
//!      its own URA (round-trips the Axon parser, owner = learner),
//!      the original owner keeps its copy, and BOTH are
//!      independently discoverable — `exactly one owner` holds for
//!      each;
//!   ③  the learn response declares `execution_mode: sandbox_first`
//!      (the InstallPolicy default — declared, with executor
//!      enforcement tracked as its own milestone);
//!   plus forget: the learned copy disappears from discovery, the
//!   original survives.
//!
//! One `#[test]` on purpose — fixture owns process env (see fixture
//! header).
//!
//! Author: Silan Hu <silan.hu@u.nus.edu>
//! Copyright (c) 2026 EasyNet. All rights reserved.

#![cfg(all(feature = "axon-pb", unix))]

mod seven_axes_fixture;

use easynet_cli::facade::cli::discover::{self, DiscoverArgs, OutputFormat};
use easynet_cli::facade::cli::teach::{self, ForgetArgs, LearnArgs, TeachArgs};
use seven_axes_fixture::SevenAxesHome;

const LOCAL_SYSTEM_AGENT_URA: &str = "easynet:///r/_system/agent/_system.local";

fn discover_weather() -> DiscoverArgs {
    DiscoverArgs {
        intent: "weather forecast".into(),
        limit: 15,
        local_only: true,
        as_agent: None,
        tree: false,
        format: OutputFormat::Table,
    }
}

#[test]
fn teach_learn_e2e_owner_initiative_and_independent_ownership() {
    let home = SevenAxesHome::seed();
    let daemon = home.start_daemon();
    let taught_ura = home.taught_ability_ura();

    // ① Default refusal FIRST — the most important assertion in the
    // file: a capability is conferred, never pulled.
    let err = teach::execute_learn(&LearnArgs {
        ability_ura: taught_ura.clone(),
        learner: "zlearner".into(),
        yes: true,
    })
    .expect_err("learn without a grant must refuse");
    assert!(
        format!("{err:#}").contains("allow_transferred_code=false"),
        "the refusal must name the gate: {err:#}"
    );

    // Owner teaches — to ONE learner, through the wire.
    teach::execute_teach(&TeachArgs {
        ability: "testbot.weather-probe".into(),
        to: home.zlearner_ura.clone(),
        yes: true,
    })
    .expect("owner confers the grant");

    // ② Learn: a NEW ability under the learner's own URA.
    let (resp, meta) = teach::execute_learn(&LearnArgs {
        ability_ura: taught_ura.clone(),
        learner: "zlearner".into(),
        yes: true,
    })
    .expect("learner acquires");
    let new_ura = resp["new_ura"].as_str().expect("new_ura present");
    let selector = easynet_cli::ura::AbilitySelector::parse(new_ura)
        .expect("learner's URA round-trips the Axon parser");
    assert_eq!(selector.owner_kind(), "agent");
    assert_eq!(
        selector.owner_ura(),
        home.zlearner_ura,
        "the copy's owner is the learner"
    );
    assert_ne!(new_ura, taught_ura, "two abilities, two identities");
    assert_eq!(
        meta["caller_ura"], LOCAL_SYSTEM_AGENT_URA,
        "local daemon IPC calls use the process-local system caller"
    );
    assert_eq!(
        meta["callee_ura"], home.loopback_caller,
        "meta.acquire is a device-owned mutation surface"
    );
    assert_eq!(
        meta["delegation"]["kind"], "hosted_agent",
        "hosted-agent delegation must be explicit, not encoded by rewriting caller identity"
    );
    assert_eq!(
        meta["delegation"]["agent_ura"], home.zlearner_ura,
        "the learner rides as the delegated hosted agent"
    );
    assert_eq!(
        meta["subject_ura"], taught_ura,
        "the taught ability is the subject of the transfer"
    );
    assert_eq!(
        resp["mutated_by"], home.loopback_caller,
        "hosted_by authority should be explicit rather than hidden"
    );
    assert!(
        meta.pointer("/receipt/anchor/receipt_ura")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|value| !value.is_empty()),
        "learn must return a receipt anchor for downstream causal references"
    );

    // ③ Declared execution posture (InstallPolicy default).
    assert_eq!(resp["execution_mode"], "sandbox_first");

    // ② cont.: BOTH copies are independently discoverable — the
    // original owner kept its ability.
    let report = discover::execute(&discover_weather()).expect("discover after learn");
    let uras: Vec<&str> = report.candidates.iter().map(|c| c.ura.as_str()).collect();
    assert!(
        uras.contains(&taught_ura.as_str()),
        "original survives: {uras:?}"
    );
    assert!(
        uras.contains(&new_ura),
        "learned copy is discoverable: {uras:?}"
    );

    // Forget: the learned copy leaves discovery; the original stays.
    teach::execute_forget(&ForgetArgs {
        ability: "weather-probe".into(),
        agent: "zlearner".into(),
        yes: true,
    })
    .expect("learner forgets");
    let report = discover::execute(&discover_weather()).expect("discover after forget");
    let uras: Vec<&str> = report.candidates.iter().map(|c| c.ura.as_str()).collect();
    assert!(uras.contains(&taught_ura.as_str()), "original still here");
    assert!(
        !uras.contains(&new_ura),
        "forgotten copy must leave discovery: {uras:?}"
    );

    drop(daemon);
}
