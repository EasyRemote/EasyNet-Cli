//! Seven-axes W3 — signed usage end-to-end
//! =========================================
//!
//! File: tests/seven_axes_w3_usage_e2e.rs
//! Spec: docs/spec/seven-axes-p0-landing-v1.md §3 W3-E2E-1 (the
//! wire-ride subset; the tamper-breaks-verification property is
//! pinned at the protocol layer by Axon's
//! `usage_tail_is_signed_material` — the signature math lives there,
//! not in this consumer).
//! Fixture: `seven_axes_fixture` — real UDS daemon + one-handle ledger.
//!
//! Covered, over the real wire:
//!   * a real invocation's terminal receipt carries `usage`, the
//!     LedgerSink copies it verbatim into the queryable row, and the
//!     watch surface sums it into the Terminal event — receipt →
//!     ledger → projection, one unbroken ride;
//!   * the same terminal receipt exposes descriptor/runtime proof
//!     facts: descriptor version, descriptor-package schema and
//!     implementation hashes, input hash, output hash, and runtime
//!     environment;
//!   * `duration_ms` is runtime-owned wall time (admitted →
//!     terminal): present as a number, zero-or-more — zero is a fact
//!     for a sub-millisecond local call, absence would be a bug;
//!   * token counters are zero (no handler reports them yet — the
//!     EmitExtras seam exists; zero-not-absent is the contract).
//!
//! One `#[test]` on purpose — fixture owns process env (see fixture
//! header).
//!
//! Author: Silan Hu <silan.hu@u.nus.edu>
//! Copyright (c) 2026 EasyNet. All rights reserved.

#![cfg(all(feature = "axon-pb", unix))]

mod seven_axes_fixture;

use easynet_cli::facade::cli::invocation_watch::{self, WatchArgs, WatchEvent};
use seven_axes_fixture::SevenAxesHome;

#[test]
fn usage_e2e_rides_receipt_to_ledger_to_watch_terminal() {
    let home = SevenAxesHome::seed();
    let daemon = home.start_daemon();

    let meta = home.invoke_testbot_echo_with_meta("usage ride");
    // The echo manifest declares descriptor_version = "2.3.0" (not the
    // default), so this asserts the receipt proof fact carries the version
    // the runtime actually registered — proving the version threads
    // manifest -> control plane -> runtime proof binding -> wire envelope ->
    // receipt, rather than a fabricated default stamped at the wire boundary.
    let expected_echo_descriptor_version = "2.3.0";
    let expected_echo_ability_ref = format!(
        "{}@{}",
        easynet_cli::ura::owner_ability_ura(&home.testbot_ura, "echo")
            .expect("mint testbot echo ability URA"),
        expected_echo_descriptor_version
    );
    let invocation_ura = meta["invocation_ura"]
        .as_str()
        .expect("envelope echo carries invocation_ura")
        .to_string();

    let snapshot = invocation_watch::execute_once(&WatchArgs {
        invocation: Some(invocation_ura),
        trace: None,
        follow: false,
        max_wait_seconds: 60,
        format: invocation_watch::OutputFormat::Table,
    })
    .expect("watch the ledgered invocation");

    let usage = match snapshot.terminal {
        Some(WatchEvent::Terminal { usage, .. }) => {
            usage.expect("terminal event must carry the signed usage sum")
        }
        other => panic!("completed invocation must be terminal; got {other:?}"),
    };

    // Token counters: zero-not-absent until a handler reports through
    // the EmitExtras seam.
    assert_eq!(usage.tokens_in, 0);
    assert_eq!(usage.tokens_out, 0);
    assert_eq!(usage.external_calls, 0);
    // duration_ms is runtime-owned wall time. A local echo can
    // complete inside one millisecond — zero is then the honest
    // value; the FIELD's presence (proved by reaching this line
    // through receipt → ledger → watch) is the contract.
    let _wall_time: u64 = usage.duration_ms;

    let proof = meta["receipt_proof_facts"]
        .as_object()
        .expect("terminal receipt must expose descriptor/runtime proof facts");
    assert_eq!(
        proof["descriptor_version"],
        expected_echo_descriptor_version
    );
    assert_eq!(proof["runtime_env"], "axon-local-runtime-rs");
    assert_eq!(
        proof["ability_binding"].as_str(),
        Some(expected_echo_ability_ref.as_str())
    );
    assert_nonzero_hex32(proof["schema_hash"].as_str().expect("schema hash"));
    assert_nonzero_hex32(proof["impl_hash"].as_str().expect("impl hash"));
    assert_nonzero_hex32(proof["input_hash"].as_str().expect("input hash"));
    assert_nonzero_hex32(proof["output_hash"].as_str().expect("output hash"));

    drop(daemon);
}

fn assert_nonzero_hex32(value: &str) {
    assert_eq!(value.len(), 64, "hash must be 32 bytes of lowercase hex");
    assert!(
        value.chars().all(|ch| ch.is_ascii_hexdigit()),
        "hash must be hex: {value}"
    );
    assert_ne!(
        value, "0000000000000000000000000000000000000000000000000000000000000000",
        "descriptor-bound receipt proof hash must not be the default zero hash"
    );
}
