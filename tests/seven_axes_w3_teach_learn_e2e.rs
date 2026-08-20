//! Seven-axes W3 - descriptor grant/import/forget end-to-end
//! =========================================================
//!
//! File: tests/seven_axes_w3_teach_learn_e2e.rs
//! Spec: docs/spec/seven-axes-p0-landing-v1.md §3 W3-E2E-3
//! (single-device subset per the D6 v1 ruling: same-device,
//! manifest-only).
//! Fixture: `seven_axes_fixture` — `testbot` owns `weather-probe`;
//! `zlearner` owns nothing and exists to import the granted descriptor.
//!
//! Covered, over the real wire — owner initiative three ways:
//!   ①  the default refusal comes FIRST: without a grant, import
//!      refuses and names the gate (`allow_transferred_code=false`);
//!   ②  after grant -> import: the learner owns a NEW descriptor URA,
//!      the original owner keeps its descriptor, and BOTH are independently
//!      discoverable;
//!   ③  the import response declares `execution_mode: sandbox_first` and
//!      `invokable: false`;
//!   plus forget: the imported descriptor disappears from discovery, the
//!   original survives.
//!
//! One `#[test]` on purpose — fixture owns process env (see fixture
//! header).
//!
//! Author: Silan Hu <silan.hu@u.nus.edu>
//! Copyright (c) 2026 EasyNet. All rights reserved.

#![cfg(all(feature = "axon-pb", unix))]

mod seven_axes_fixture;

use easynet_cli::cli::discover::{
    self, DiscoverArgs, DiscoverScopeMode, OutputFormat, SourceWindowMode,
};
use easynet_cli::cli::teach::{self, ForgetArgs, LearnArgs, TeachArgs};
use seven_axes_fixture::SevenAxesHome;

const LOCAL_SYSTEM_AGENT_URA: &str = "easynet:///r/_system/agent/_system.local";

fn discover_weather() -> DiscoverArgs {
    DiscoverArgs {
        intent: "weather forecast".into(),
        limit: 15,
        scope: DiscoverScopeMode::Local,
        as_agent: None,
        tree: false,
        source_window: SourceWindowMode::Bounded,
        format: OutputFormat::Table,
    }
}

#[test]
fn descriptor_import_e2e_owner_initiative_and_independent_ownership() {
    let home = SevenAxesHome::seed();
    let daemon = home.start_daemon();
    let source_descriptor_ura = home.source_descriptor_ura();

    // ① Default refusal FIRST — the most important assertion in the
    // file: descriptor import is granted by the owner, never pulled.
    let err = teach::execute_learn(&LearnArgs {
        ability_ura: source_descriptor_ura.clone(),
        learner: "zlearner".into(),
        yes: true,
    })
    .expect_err("import without a grant must refuse");
    assert!(
        format!("{err:#}").contains("allow_transferred_code=false"),
        "the refusal must name the gate: {err:#}"
    );

    // Owner grants descriptor import to ONE learner through the wire.
    teach::execute_teach(&TeachArgs {
        ability: "testbot.weather-probe".into(),
        to: home.zlearner_ura.clone(),
        yes: true,
    })
    .expect("owner grants descriptor import");

    // ② Import: a NEW descriptor under the learner's own URA.
    let (resp, meta) = teach::execute_learn(&LearnArgs {
        ability_ura: source_descriptor_ura.clone(),
        learner: "zlearner".into(),
        yes: true,
    })
    .expect("learner imports descriptor");
    let new_descriptor_ura = resp["new_descriptor_ura"]
        .as_str()
        .expect("new_descriptor_ura present");
    let selector = easynet_cli::core::ura::AbilitySelector::parse(new_descriptor_ura)
        .expect("learner's URA round-trips the Axon parser");
    assert_eq!(selector.owner_kind(), "agent");
    assert_eq!(
        selector.owner_ura(),
        home.zlearner_ura,
        "the descriptor copy's owner is the learner"
    );
    assert_ne!(
        new_descriptor_ura, source_descriptor_ura,
        "two descriptors, two identities"
    );
    assert_eq!(
        meta["caller_ura"], LOCAL_SYSTEM_AGENT_URA,
        "local daemon IPC calls use the process-local system caller"
    );
    assert_eq!(
        meta["callee_ura"],
        easynet_cli::core::ura::device_agent_ura(
            "cli",
            "local",
            easynet_cli::daemon::ability::names::governance::DESCRIPTOR_TRANSFER_SYSTEM_AGENT_ID,
        ),
        "meta.acquire is owned by the descriptor-transfer SystemAgent"
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
        meta["subject_ura"], source_descriptor_ura,
        "the source descriptor is the subject of the transfer"
    );
    assert_eq!(
        resp["mutated_by"], home.loopback_caller,
        "hosted_by authority should be explicit rather than hidden"
    );
    assert!(
        meta.pointer("/receipt/anchor/receipt_ura")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|value| !value.is_empty()),
        "descriptor import must return a receipt anchor for downstream causal references"
    );

    // ③ Declared execution posture and explicit non-invokability.
    assert_eq!(resp["execution_mode"], "sandbox_first");
    assert_eq!(resp["transfer_kind"], "discovery_only_manifest");
    assert_eq!(resp["invokable"], false);

    // ② cont.: BOTH descriptors are independently discoverable; the
    // original owner kept its descriptor.
    let report = discover::execute(&discover_weather()).expect("discover after import");
    let uras: Vec<&str> = report.candidates.iter().map(|c| c.ura.as_str()).collect();
    assert!(
        uras.contains(&source_descriptor_ura.as_str()),
        "original survives: {uras:?}"
    );
    assert!(
        uras.contains(&new_descriptor_ura),
        "imported descriptor copy is discoverable: {uras:?}"
    );

    // Forget: the imported descriptor leaves discovery; the original stays.
    teach::execute_forget(&ForgetArgs {
        ability: "weather-probe".into(),
        agent: "zlearner".into(),
        yes: true,
    })
    .expect("learner removes imported descriptor");
    let report = discover::execute(&discover_weather()).expect("discover after forget");
    let uras: Vec<&str> = report.candidates.iter().map(|c| c.ura.as_str()).collect();
    assert!(
        uras.contains(&source_descriptor_ura.as_str()),
        "original still here"
    );
    assert!(
        !uras.contains(&new_descriptor_ura),
        "removed import must leave discovery: {uras:?}"
    );

    drop(daemon);
}
