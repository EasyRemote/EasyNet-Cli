//! Seven-axes W1 — `easynet discover` end-to-end
//! ===============================================
//!
//! File: tests/seven_axes_w1_discover_e2e.rs
//! Spec: docs/spec/seven-axes-p0-landing-v1.md §3 W1-E2E-1/2.
//! Fixture: `seven_axes_fixture` (real UDS daemon + product-file
//! seeded HOME — see that module's header for the stack).
//!
//! Covered here (W1-E2E-2 in full; W1-E2E-1 single-daemon subset):
//!   * ladder-entry resolution over the wire
//!     (`agent.list` → `discover` with `testbot` as selected callee);
//!   * typed degradation: an unjoined daemon answers
//!     `federation_not_joined` — never an error — local tiers intact;
//!   * candidate projection: URA round-trips the Axon parser,
//!     owner_kind comes typed, and the score reproduces the frozen
//!     ranking contract digit-for-digit;
//!   * the seven-tuple audit surface: the report carries the
//!     invocation envelope echo (spec 0.1-7).
//!
//! Still tracked elsewhere: cross-owner projection through the user
//! tier needs the two-daemon hub fixture
//! (`cross_hub_two_daemon_real_tls_e2e.rs` pattern).
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
use easynet_cli::daemon::ability::descriptors::{
    AbilityDescriptor, AbilityHints, AdmissionAction, Visibility,
};
use easynet_cli::daemon::persistence::config;
use seven_axes_fixture::SevenAxesHome;

const REMOTE_PUBLIC_NAME: &str = "remote-file-reader";

fn args(intent: &str) -> DiscoverArgs {
    DiscoverArgs {
        intent: intent.to_string(),
        limit: 15,
        scope: DiscoverScopeMode::Realm,
        as_agent: None,
        tree: false,
        source_window: SourceWindowMode::Bounded,
        format: OutputFormat::Table,
    }
}

fn advertise_remote_user_tier_ability(
    home: &SevenAxesHome,
    host_device_ura: &str,
    owner_ura: &str,
    projection_revision: u64,
) -> String {
    let ability_ura = easynet_cli::core::ura::owner_ability_ura(owner_ura, REMOTE_PUBLIC_NAME)
        .expect("mint remote ability URA");

    let descriptor = AbilityDescriptor::new(
        REMOTE_PUBLIC_NAME,
        owner_ura,
        Visibility::Public,
        AdmissionAction::Invoke,
    )
    .expect("build synthetic governed remote descriptor")
    .with_description("read a remote file from another owner")
    .with_source("seven-axes:remote-file-reader")
    .with_hints(AbilityHints {
        read_only: true,
        destructive: false,
        idempotent: true,
    });
    let ability_summary =
        easynet_cli::daemon::federation::read_model::owner_projection::
            canonical_summary_values_from_descriptors(owner_ura, &[descriptor])
                .expect("synthetic remote descriptor must project canonically")
                .into_iter()
                .next()
                .expect("one descriptor must produce one projection summary");
    home.advertise_hosted_agent_projection(
        host_device_ura,
        owner_ura,
        projection_revision,
        vec![ability_summary],
    );

    ability_ura
}

#[test]
fn discover_e2e_local_scope_and_typed_federation_degradation() {
    let home = SevenAxesHome::seed();
    let daemon = home.start_daemon();

    // ── W1-E2E-2: unjoined federation degrades typed ─────────────────
    let joined_credentials =
        config::load_credentials().expect("fixture should seed joined credentials");
    config::delete_credentials().expect("temporarily unjoin the fixture");
    let report = discover::execute(&args("chat")).expect("discover executes against live daemon");
    assert_eq!(
        report.tiers_searched,
        vec!["device"],
        "user tier must not be listed when federation is degraded"
    );
    let fed = report
        .federation
        .as_ref()
        .expect("degradation must surface as a typed status object");
    assert_eq!(
        fed.status, "federation_not_joined",
        "unjoined daemon must degrade as federation_not_joined; got {fed:?}"
    );
    assert!(
        report.invocations.len() >= 2,
        "report must carry both local and realm-tier invocation echoes; got {:?}",
        report.invocations
    );
    assert!(
        report.invocations.iter().all(|meta| meta.is_object()),
        "every envelope echo must be a structured object; got {:?}",
        report.invocations
    );
    config::save_credentials(&joined_credentials).expect("restore joined credentials");

    // ── W1-E2E-1 (single-daemon subset): candidate projection ────────
    let mut self_args = args("weather forecast");
    self_args.as_agent = Some("testbot".into());
    let weather = discover::execute(&self_args).expect("discover executes for the seeded ability");
    let candidate = weather
        .candidates
        .iter()
        .find(|c| c.name.ends_with("weather-probe"))
        .unwrap_or_else(|| panic!("seeded ability must rank: {:?}", weather.candidates));

    let selector = easynet_cli::core::ura::AbilitySelector::parse(&candidate.ura)
        .expect("candidate URA must round-trip the Axon parser");
    assert_eq!(candidate.owner_kind, "agent");
    assert_eq!(selector.owner_kind(), "agent");
    assert_eq!(
        candidate.scope, "self",
        "the ladder is testbot's own; its ability sits in the self tier"
    );

    // Frozen ranking contract, recomputed by hand for this fixture
    // (spec W1-E2E-1 ③ — a user can predict every row's score):
    //   "weather": name hit 3 + segment-prefix 2 + description 1
    //              + owner(URA) 1                          = 7
    //   "forecast": description 1                          = 1
    //   every token hit somewhere, 2 tokens → bonus        = 2
    assert_eq!(
        candidate.score, 10,
        "score must follow the frozen name×3(+2)/desc×1/owner×1/+2 contract"
    );
    assert!(
        weather.diagnostics.iter().all(|diagnostic| !matches!(
            diagnostic.code,
            "candidate_parse_skipped"
        )),
        "candidate projection defects must fail closed or surface as typed diagnostics, not skipped counters: {:?}",
        weather.diagnostics
    );

    // ── W1-E2E-1 user tier: same daemon acting as local hub ──────────
    //
    // This is the cross-owner discover path without the expensive
    // two-binary TLS harness: the test writes the hub read models
    // through the public federation advertise abilities, then the
    // normal `<agent>.discover(scope=user)` path calls
    // `federation.resolve` over the daemon Invocation surface.
    let remote_ura =
        advertise_remote_user_tier_ability(&home, &home.loopback_caller, &home.testbot_ura, 2);
    let user_scope =
        discover::execute(&args("remote file")).expect("discover user tier through local hub");
    assert_eq!(
        user_scope.tiers_searched,
        vec!["device", "user"],
        "joined daemon must list the user tier"
    );
    assert!(
        user_scope.federation.is_none(),
        "joined daemon must not surface federation degradation: {:?}",
        user_scope.federation
    );
    let remote = user_scope
        .candidates
        .iter()
        .find(|c| c.ura == remote_ura)
        .unwrap_or_else(|| {
            panic!(
                "remote user-tier ability must rank: {:?}",
                user_scope.candidates
            )
        });
    assert_eq!(remote.scope, "user");
    assert_eq!(remote.owner_kind, "agent");

    drop(daemon);
}
