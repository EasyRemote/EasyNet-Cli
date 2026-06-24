//! Seven-axes W2 — `easynet invocation watch` end-to-end
//! =======================================================
//!
//! File: tests/seven_axes_w2_watch_e2e.rs
//! Spec: docs/spec/seven-axes-p0-landing-v1.md §3 W2-E2E-2 (the
//! ledger projection plus the multi-step mission stream and the
//! heartbeat-liveness tail).
//! Fixture: `seven_axes_fixture` — now wiring a real
//! `InvocationLedger` at the production-resolved path, so the unary
//! invokes the other e2e files exercise are RECORDED, not just
//! executed.
//!
//! Covered, over the real wire:
//!   * a real invocation (`testbot.echo`) lands in the ledger,
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
use easynet_cli::facade::cli::mission_runs::{
    self, MissionRunDir, MissionRunMeta, MissionRunOpts, MissionRunStatus,
};
use easynet_cli::facade::cli::receipt_verification::CliReceiptChainVerification;
use easynet_cli::runtime::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION;
use seven_axes_fixture::SevenAxesHome;

#[test]
fn watch_e2e_projects_a_ledgered_invocation_to_terminal() {
    let home = SevenAxesHome::seed();
    let daemon = home.start_daemon();

    // Drive one real, ledgered invocation through the wire.
    let meta = home.invoke_testbot_echo_with_meta("watch me");
    let expected_echo_ability_ref = format!(
        "{}@{}",
        easynet_cli::ura::owner_ability_ura(&home.testbot_ura, "echo")
            .expect("mint testbot echo ability URA"),
        DEFAULT_ABILITY_DESCRIPTOR_VERSION
    );
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
        state.0.as_str(),
        expected_echo_ability_ref.as_str(),
        "the projection names the ability the ledger recorded"
    );

    // The set completed, so the snapshot is terminal-ok — by the Axon
    // state vocabulary, not a local table.
    match snapshot.terminal {
        Some(WatchEvent::Terminal {
            ref trace,
            ref status,
            ledger_reported_receipt_chain_verified,
            cli_receipt_chain_verification,
            ..
        }) => {
            assert_eq!(trace, &snapshot.trace_id);
            assert_eq!(status, "ok");
            assert!(ledger_reported_receipt_chain_verified);
            assert_eq!(
                cli_receipt_chain_verification,
                CliReceiptChainVerification::not_performed()
            );
        }
        other => panic!("a completed invocation must be terminal; got {other:?}"),
    }

    let mission = r#"
mission "watch-stream" {
  let first = testbot.echo(message: "one")
  let second = testbot.echo(message: first.output)
  let third = testbot.echo(message: second.output)
}
"#;
    let run = mission_runs::run_mission_inproc(
        mission,
        MissionRunOpts {
            source_label: Some("seven-axes-watch-stream.eal".into()),
            trace_path: None,
            invocation_context: None,
        },
    )
    .expect("three-step mission runs through the daemon");
    assert_eq!(
        run.meta.trace_id, run.run_id,
        "T2.0 contract: mission run id is the child invocation trace id"
    );

    let watch = invocation_watch::execute_once(&WatchArgs {
        invocation: None,
        trace: Some(run.meta.trace_id.clone()),
        follow: false,
        format: invocation_watch::OutputFormat::Json,
    })
    .expect("watch the mission trace");
    assert_eq!(watch.trace_id, run.meta.trace_id);
    assert_eq!(
        watch.rows.len(),
        3,
        "three dependent EAL calls must land as three child invocations"
    );
    assert!(
        watch
            .rows
            .iter()
            .all(|row| row.ability == expected_echo_ability_ref),
        "watch must project child invocation facts, not EAL step names: {:?}",
        watch.rows
    );
    assert!(
        watch.events.iter().all(|event| match event {
            WatchEvent::State { invocation, .. } => !invocation.is_empty(),
            _ => true,
        }),
        "every child event carries its invocation id"
    );
    match watch.terminal {
        Some(WatchEvent::Terminal {
            ref trace,
            ref status,
            ledger_reported_receipt_chain_verified,
            cli_receipt_chain_verification,
            ref usage,
        }) => {
            assert_eq!(trace, &run.meta.trace_id);
            assert_eq!(status, "ok");
            assert!(ledger_reported_receipt_chain_verified);
            assert_eq!(
                cli_receipt_chain_verification,
                CliReceiptChainVerification::not_performed()
            );
            assert!(
                usage.is_some(),
                "terminal event must carry the trace usage aggregate"
            );
        }
        other => panic!("completed mission trace must be terminal; got {other:?}"),
    }
    let watch_json = serde_json::to_string(&watch).expect("watch snapshot serializes");
    assert!(
        !watch_json.contains("step"),
        "watch surface must not expose step-level addressing: {watch_json}"
    );

    let stale_run = MissionRunDir::create("watch-stale-heartbeat")
        .expect("create a stale-heartbeat mission run fixture");
    let stale_trace = stale_run
        .path
        .file_name()
        .expect("run dir has a final component")
        .to_string_lossy()
        .to_string();
    stale_run
        .write_meta(&MissionRunMeta {
            name: "watch-stale-heartbeat".into(),
            trace_id: stale_trace.clone(),
            started_at: "2026-06-13T00:00:00+00:00".into(),
            status: MissionRunStatus::Running,
            ..Default::default()
        })
        .expect("write running meta");
    stale_run.finish();

    let liveness_events = invocation_watch::execute_follow_until_terminal(&WatchArgs {
        invocation: None,
        trace: Some(stale_trace.clone()),
        follow: true,
        format: invocation_watch::OutputFormat::Json,
    })
    .expect("follow projects stale heartbeat to local liveness");
    assert_eq!(
        liveness_events,
        vec![WatchEvent::Liveness {
            status: "interrupted".into(),
            source: "local".into(),
        }],
        "a running meta with a dead heartbeat must not watch forever"
    );

    drop(daemon);
}
