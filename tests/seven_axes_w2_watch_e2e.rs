//! Seven-axes W2 — `easynet invocation watch` end-to-end
//! =======================================================
//!
//! File: tests/seven_axes_w2_watch_e2e.rs
//! Spec: docs/spec/seven-axes-p0-landing-v1.md §3 W2-E2E-2 (the
//! ledger-projection subset; the multi-step mission stream and the
//! heartbeat-liveness tail need a mission run in the fixture and are
//! tracked with the TUI follow-up).
//! Fixture: `seven_axes_fixture` — now wiring a real
//! `InvocationLedger` at the production-resolved path, so the unary
//! invokes the other e2e files exercise are RECORDED, not just
//! executed.
//!
//! Covered, over the real wire:
//!   * a real invocation (a `trust level set`) lands in the ledger,
//!     and `invocation watch <invocation-ura>` projects it — the
//!     positional entry derives the trace from the record (T2.0's
//!     two entry forms, one engine);
//!   * the event stream is a pure projection: state events carry the
//!     ledger's wire-state vocabulary, and the terminal event's
//!     status follows `InvocationState::is_terminal` — no second
//!     truth source (spec ②⑥);
//!   * the snapshot's invocation identity round-trips what the
//!     envelope echo reported — the watch surface and the seven-tuple
//!     audit surface agree on what happened.
//!
//! One `#[test]` on purpose — fixture owns process env (see fixture
//! header).
//!
//! Author: Silan Hu <silan.hu@u.nus.edu>
//! Copyright (c) 2026 EasyNet. All rights reserved.

#![cfg(all(feature = "axon-pb", unix))]

mod seven_axes_fixture;

use easynet_cli::facade::cli::invocation_watch::{self, WatchArgs, WatchEvent};
use easynet_cli::facade::cli::trust_level::{self, SetArgs};
use seven_axes_fixture::SevenAxesHome;

#[test]
fn watch_e2e_projects_a_ledgered_invocation_to_terminal() {
    let home = SevenAxesHome::seed();
    let daemon = home.start_daemon();

    // Drive one real, ledgered invocation through the wire.
    let (_resp, meta) = trust_level::execute_set(&SetArgs {
        agent_ura: home.testbot_ura.clone(),
        level: "elevated".into(),
        yes: true,
        format: trust_level::OutputFormat::Table,
    })
    .expect("a ledgered invocation to watch");
    let invocation_ura = meta["invocation_ura"]
        .as_str()
        .expect("envelope echo carries invocation_ura")
        .to_string();

    // Watch it by the positional entry: the record derives the trace.
    let snapshot = invocation_watch::execute_once(&WatchArgs {
        invocation: Some(invocation_ura.clone()),
        trace: None,
        follow: false,
        format: invocation_watch::OutputFormat::Table,
    })
    .expect("watch the ledgered invocation");

    assert!(!snapshot.trace_id.is_empty(), "record must carry its trace");
    let state = snapshot
        .events
        .iter()
        .find_map(|e| match e {
            WatchEvent::State {
                invocation,
                ability,
                state,
            } if invocation == &invocation_ura => Some((ability.clone(), state.clone())),
            _ => None,
        })
        .expect("the watched invocation must project a state event");
    assert_eq!(
        state.0, "identity.set_trust",
        "the projection names the ability the ledger recorded"
    );

    // The set completed, so the snapshot is terminal-ok — by the Axon
    // state vocabulary, not a local table.
    match snapshot.terminal {
        Some(WatchEvent::Terminal {
            ref trace,
            ref status,
            ..
        }) => {
            assert_eq!(trace, &snapshot.trace_id);
            assert_eq!(status, "ok");
        }
        other => panic!("a completed invocation must be terminal; got {other:?}"),
    }

    drop(daemon);
}
