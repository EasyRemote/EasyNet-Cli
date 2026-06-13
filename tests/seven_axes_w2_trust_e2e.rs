//! Seven-axes W2 — `easynet trust level` end-to-end
//! ==================================================
//!
//! File: tests/seven_axes_w2_trust_e2e.rs
//! Spec: docs/spec/seven-axes-p0-landing-v1.md §3 W2-E2E-1 (the
//! device-side subset; enforcement ② and agent→node projection ④
//! are cross-repo per the T2.3 reclassification).
//! Fixture: `seven_axes_fixture` — same real UDS daemon stack as W1.
//!
//! Covered:
//!   ①  `set` is a complete invocation: response carries the ruling
//!      transition, envelope echo rides back, and the daemon-side
//!      subject is the trusted entity's URA;
//!   ③  rulings survive a daemon restart (state lives in
//!      `trust-levels.json`, not in the process);
//!   plus the refusal surface over the real wire: an unknown level
//!   gets the full menu back, and a non-agent subject is refused per
//!   the D8 default ruling.
//!
//! One `#[test]` on purpose — fixture owns process env (see fixture
//! header).
//!
//! Author: Silan Hu <silan.hu@u.nus.edu>
//! Copyright (c) 2026 EasyNet. All rights reserved.

#![cfg(all(feature = "axon-pb", unix))]

mod seven_axes_fixture;

use easynet_cli::facade::cli::trust_level::{self, OutputFormat, SetArgs, ShowArgs};
use seven_axes_fixture::SevenAxesHome;

fn show(ura: &str) -> ShowArgs {
    ShowArgs {
        agent_ura: ura.to_string(),
        format: OutputFormat::Table,
    }
}

fn set(ura: &str, level: &str) -> SetArgs {
    SetArgs {
        agent_ura: ura.to_string(),
        level: level.to_string(),
        yes: true,
        format: OutputFormat::Table,
    }
}

#[test]
fn trust_level_e2e_ruling_round_trip_and_restart_persistence() {
    let home = SevenAxesHome::seed();
    let daemon = home.start_daemon();
    let subject = home.testbot_ura.clone();

    // Baseline: no ruling yet → default, said out loud.
    let baseline = trust_level::execute_show(&show(&subject)).expect("show baseline");
    assert_eq!(baseline["trust_level"], "STANDARD");
    assert_eq!(baseline["source"], "default");

    // ① Record a ruling — a complete, audited invocation.
    let (resp, meta) = trust_level::execute_set(&set(&subject, "elevated")).expect("set ruling");
    assert_eq!(resp["trust_level"], "ELEVATED");
    assert!(resp["previous"].is_null(), "first ruling has no previous");
    let writer = resp["updated_by_invocation"]
        .as_str()
        .expect("trust write must bind to the executing invocation id");
    assert!(!writer.is_empty());
    assert!(meta.is_object(), "envelope echo must ride back: {meta}");

    let shown = trust_level::execute_show(&show(&subject)).expect("show ruling");
    assert_eq!(shown["trust_level"], "ELEVATED");
    assert_eq!(shown["source"], "device-directory");
    assert_eq!(shown["updated_by_invocation"], writer);

    // ③ Rulings live in trust-levels.json, not in the process.
    drop(daemon);
    let daemon = home.start_daemon();
    let after_restart =
        trust_level::execute_show(&show(&subject)).expect("show after daemon restart");
    assert_eq!(
        after_restart["trust_level"], "ELEVATED",
        "ruling must survive a daemon restart"
    );
    assert_eq!(after_restart["source"], "device-directory");

    // Refusals over the real wire: full menu on a bad level…
    let err =
        trust_level::execute_set(&set(&subject, "max")).expect_err("unknown level must refuse");
    assert!(
        format!("{err:#}").contains("privileged"),
        "menu must reach the operator through the daemon error: {err:#}"
    );
    // …and the D8 subject shape is enforced daemon-side.
    let err = trust_level::execute_show(&show(&home.loopback_caller))
        .expect_err("device URA subject must refuse under D8");
    assert!(
        format!("{err:#}").contains("Agent URA"),
        "D8 refusal must name the expected shape: {err:#}"
    );

    drop(daemon);
}
