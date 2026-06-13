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
//!     (`agent.list` → `testbot.discover`);
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

use easynet_cli::facade::cli::discover::{self, DiscoverArgs, OutputFormat};
use seven_axes_fixture::SevenAxesHome;

fn args(intent: &str) -> DiscoverArgs {
    DiscoverArgs {
        intent: intent.to_string(),
        limit: 15,
        local_only: false,
        tree: false,
        format: OutputFormat::Table,
    }
}

#[test]
fn discover_e2e_local_tiers_and_typed_federation_degradation() {
    let home = SevenAxesHome::seed();
    let daemon = home.start_daemon();

    // ── W1-E2E-2: unjoined federation degrades typed ─────────────────
    let report = discover::execute(&args("chat")).expect("discover executes against live daemon");
    assert_eq!(
        report.tiers_searched,
        vec!["self", "device"],
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
    let invocation = report
        .invocation
        .as_ref()
        .expect("report must carry the invocation envelope echo");
    assert!(
        invocation.is_object(),
        "envelope echo must be a structured object; got {invocation}"
    );

    // ── W1-E2E-1 (single-daemon subset): candidate projection ────────
    let weather = discover::execute(&args("weather forecast"))
        .expect("discover executes for the seeded ability");
    let candidate = weather
        .candidates
        .iter()
        .find(|c| c.name.ends_with("weather-probe"))
        .unwrap_or_else(|| panic!("seeded ability must rank: {:?}", weather.candidates));

    let selector = easynet_cli::ura::AbilitySelector::parse(&candidate.ura)
        .expect("candidate URA must round-trip the Axon parser");
    assert_eq!(candidate.owner_kind, "agent");
    assert_eq!(selector.owner_kind(), "agent");
    assert_eq!(
        candidate.scope, "self",
        "the ladder is testbot's own; its ability sits in the self tier"
    );
    assert!(
        candidate.trust_level.is_none(),
        "trust column stays null until W2 wires the level into discover"
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
    assert_eq!(weather.skipped_unparseable, 0, "nothing may drop silently");

    drop(daemon);
}
