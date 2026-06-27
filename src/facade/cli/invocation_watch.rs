// EasyNet CLI — `easynet invocation watch`
// ==========================================
//
// File: src/facade/cli/invocation_watch.rs
// Description: Live view of an Invocation causal set (seven-axes W2
//              T2.4, data layer). Renders an **Invocation causal
//              tree, never a workflow graph**: there is no mission
//              object at runtime — only a trace shared by every step
//              Invocation (T2.0 anchor), and the ledger records the
//              receipts project into.
//
// Two entry forms, one engine:
//   invocation watch <invocation-ura>   — the causal set of the
//                                         trace that invocation
//                                         belongs to;
//   invocation watch --trace <id>       — a whole run by its trace
//                                         anchor (`mission run
//                                         --format json | jq -r
//                                         .trace_id`).
//
// The stream is a PROJECTION of the ledger (`invocation.trace.get`,
// polled): orchestration emits Invocations, it never redefines them —
// this surface adds no second truth source. Terminality is decided by
// the Axon `InvocationState` wire vocabulary
// (`from_wire_str(..).is_terminal()`), never a hand-copied state
// table. Mission liveness (heartbeat) is daemon-local data and is
// labelled `source: "local"` when emitted (display-truthfulness
// contract, spec T2.4).
//
// `--format json` emits NDJSON events — the one sanctioned exception
// to the table|json pair (spec §0.2-3): a stream has no table form.
// The TUI (pending D3) will consume the same engine.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::BTreeMap;
use std::time::Duration;

use anyhow::Context;
use clap::{Args, ValueEnum};
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Widget};
use serde::Deserialize;
use serde_json::json;

use crate::facade::cli::receipt_verification::CliReceiptChainVerification;
use crate::runtime::agents::invocation_history_ability::ABILITY_TRACE_GET;
use crate::support::local_invoke::invoke_local_ability;
use crate::support::output;

type Record = easynet_axon::invocation::InvocationLedgerRecord;
use easynet_axon::invocation::axiom::InvocationUsage;
use easynet_axon::invocation::InvocationState;

/// Poll cadence for `--follow`. A constant, not a flag: the ledger
/// read is daemon-local and cheap, and a knob would only invite
/// configuration where none is needed.
const FOLLOW_INTERVAL: Duration = Duration::from_millis(500);
/// Default observer-side follow budget in seconds. Bounds both an
/// all-empty trace and a non-empty trace that stops producing diffs,
/// so neither follow path can stream forever on a stuck run. It is an
/// observer timeout, not an invocation timeout — exhausting it emits a
/// liveness verdict, never a terminal failure.
const DEFAULT_FOLLOW_MAX_WAIT_SECONDS: u64 = 60;

/// Output format for `invocation watch`. It intentionally lives in
/// this module instead of `support::output::OutputFormat` because
/// `panel` is watch-specific; list/show commands stay table|json.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    Table,
    Json,
    Panel,
}

#[derive(Debug, Args)]
pub struct WatchArgs {
    /// Canonical Invocation URA — watches the causal set of the
    /// trace it belongs to.
    #[arg(required_unless_present = "trace")]
    pub invocation: Option<String>,
    /// Watch a whole run by its trace anchor
    /// (`mission run --format json` reports it as `trace_id`).
    #[arg(long, conflicts_with = "invocation")]
    pub trace: Option<String>,
    /// Keep streaming until every invocation in the trace is
    /// terminal (or the run's heartbeat goes stale).
    #[arg(long)]
    pub follow: bool,
    /// Maximum seconds `--follow` waits for a trace that exists as a
    /// local mission run but has not produced any invocation ledger rows.
    /// Once a ledger row exists, follow mode tracks it until terminality
    /// and only emits no-progress liveness notices.
    #[arg(
        long = "max-wait-seconds",
        default_value_t = DEFAULT_FOLLOW_MAX_WAIT_SECONDS,
        value_parser = clap::value_parser!(u64).range(1..)
    )]
    pub max_wait_seconds: u64,
    /// 'table' renders state rows; 'json' emits NDJSON events; 'panel'
    /// renders a deterministic three-column trace snapshot.
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,
}

/// One NDJSON event. The `event` tag plus field names are frozen by
/// the W2-E2E-2 contract (spec §0.2-9).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum WatchEvent {
    /// A new invocation appeared in the trace, or its state moved.
    State {
        invocation: String,
        ability: String,
        state: String,
    },
    /// Every invocation in the trace reached a terminal state.
    /// `usage` is the trace-level consumption sum copied from ledger rows.
    /// Axon signs each terminal receipt's usage tail; this CLI projection
    /// labels that protocol coverage without pretending to perform local
    /// offline signature verification.
    Terminal {
        trace: String,
        status: String,
        ledger_reported_receipt_chain_verified: bool,
        cli_receipt_chain_verification: CliReceiptChainVerification,
        #[serde(skip_serializing_if = "Option::is_none")]
        usage: Option<InvocationUsage>,
    },
    /// The trace exists as a mission run, but no invocation ledger row
    /// has landed yet. This is bounded by `--max-wait-seconds` in
    /// follow mode so a scheduler bug cannot render as an infinite
    /// stream.
    Pending {
        trace: String,
        status: String,
        source: String,
    },
    /// Daemon-local liveness verdict (mission heartbeat) — labelled
    /// `local` because it is not signed ledger fact.
    Liveness { status: String, source: String },
}

/// The three record facts the engine consumes — a borrowed view, so
/// unit tests and future sources don't have to assemble full ledger
/// rows (the sdk builder rightly demands complete protocol identity).
#[derive(Clone, Copy)]
pub struct RecordView<'a> {
    pub invocation_ura: &'a str,
    pub ability_name: &'a str,
    pub state: &'a str,
    pub usage: InvocationUsage,
    pub ledger_reported_receipt_chain_verified: bool,
}

impl<'a> From<&'a Record> for RecordView<'a> {
    fn from(r: &'a Record) -> Self {
        RecordView {
            invocation_ura: &r.invocation_ura,
            ability_name: &r.ability_name,
            state: &r.state,
            usage: r.usage,
            ledger_reported_receipt_chain_verified: r.receipt_chain.verified,
        }
    }
}

/// Stateful differ over successive ledger snapshots — the engine the
/// NDJSON stream and the future TUI both consume.
pub struct WatchEngine {
    trace_id: String,
    seen: BTreeMap<String, String>,
}

impl WatchEngine {
    pub fn new(trace_id: String) -> Self {
        Self {
            trace_id,
            seen: BTreeMap::new(),
        }
    }

    pub fn trace_id(&self) -> &str {
        &self.trace_id
    }

    /// Events for every invocation that appeared or changed state
    /// since the last snapshot. Ordering is stable (ledger order).
    pub fn diff<'a>(
        &mut self,
        records: impl IntoIterator<Item = RecordView<'a>>,
    ) -> Vec<WatchEvent> {
        let mut events = Vec::new();
        for r in records {
            let changed = self
                .seen
                .insert(r.invocation_ura.to_string(), r.state.to_string())
                .map(|prev| prev != r.state)
                .unwrap_or(true);
            if changed {
                events.push(WatchEvent::State {
                    invocation: r.invocation_ura.to_string(),
                    ability: r.ability_name.to_string(),
                    state: r.state.to_string(),
                });
            }
        }
        events
    }

    /// True when every record is in an Axon-terminal state — decided
    /// by the protocol vocabulary, not a local table. An empty trace
    /// is pending, not terminal.
    pub fn all_terminal<'a>(records: impl IntoIterator<Item = RecordView<'a>>) -> bool {
        let mut any = false;
        for r in records {
            any = true;
            if !InvocationState::from_wire_str(r.state).is_terminal() {
                return false;
            }
        }
        any
    }

    /// Trace-level consumption: the sum of every record's usage.
    /// Summation is the only aggregation that means anything
    /// for "what did this run cost".
    pub fn total_usage<'a>(records: impl IntoIterator<Item = RecordView<'a>>) -> InvocationUsage {
        let mut total = InvocationUsage::default();
        for r in records {
            total.tokens_in += r.usage.tokens_in;
            total.tokens_out += r.usage.tokens_out;
            total.duration_ms += r.usage.duration_ms;
            total.external_calls += r.usage.external_calls;
        }
        total
    }

    pub fn ledger_reported_receipt_chain_verified<'a>(
        records: impl IntoIterator<Item = RecordView<'a>>,
    ) -> bool {
        let mut any = false;
        for r in records {
            any = true;
            if !r.ledger_reported_receipt_chain_verified {
                return false;
            }
        }
        any
    }

    /// Aggregate terminal status: any failure-class state poisons the
    /// run; cancellation is its own outcome; otherwise ok.
    pub fn terminal_status<'a>(records: impl IntoIterator<Item = RecordView<'a>>) -> &'static str {
        let mut cancelled = false;
        for r in records {
            match InvocationState::from_wire_str(r.state) {
                InvocationState::Failed | InvocationState::TimedOut => return "failed",
                InvocationState::Cancelled => cancelled = true,
                _ => {}
            }
        }
        if cancelled {
            "cancelled"
        } else {
            "ok"
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MissionFollowStatus {
    NotMission,
    Running,
    Interrupted,
    Terminal(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FollowDecision {
    Continue,
    Stop,
}

#[derive(Debug, Clone, PartialEq)]
struct FollowStep {
    events: Vec<WatchEvent>,
    decision: FollowDecision,
}

impl FollowStep {
    fn done(&self) -> bool {
        self.decision == FollowDecision::Stop
    }
}

#[derive(Debug, Clone, Copy)]
struct FollowPolicy {
    max_empty_polls: u64,
    stale_notice_polls: u64,
}

impl FollowPolicy {
    fn from_args(args: &WatchArgs) -> Self {
        let max_empty_polls = follow_poll_budget(args.max_wait_seconds);
        Self {
            max_empty_polls,
            stale_notice_polls: max_empty_polls,
        }
    }

    #[cfg(test)]
    fn test_budget(max_empty_polls: u64) -> Self {
        Self {
            max_empty_polls,
            stale_notice_polls: max_empty_polls,
        }
    }
}

fn follow_poll_budget(max_wait_seconds: u64) -> u64 {
    let interval_ms = FOLLOW_INTERVAL.as_millis() as u64;
    let wait_ms = max_wait_seconds.saturating_mul(1_000);
    wait_ms.div_ceil(interval_ms).max(1)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FollowPhase {
    AwaitingLedgerRows {
        empty_polls: u64,
        pending_emitted: bool,
    },
    TrackingLedgerRows {
        stale_polls: u64,
        stale_notice_emitted: bool,
    },
}

impl FollowPhase {
    fn awaiting() -> Self {
        Self::AwaitingLedgerRows {
            empty_polls: 0,
            pending_emitted: false,
        }
    }

    fn observe_progress(&mut self) {
        *self = Self::TrackingLedgerRows {
            stale_polls: 0,
            stale_notice_emitted: false,
        };
    }

    fn observe_stale_ledger_rows(&mut self, policy: FollowPolicy) -> Option<WatchEvent> {
        match self {
            Self::TrackingLedgerRows {
                stale_polls,
                stale_notice_emitted,
            } => {
                *stale_polls = stale_polls.saturating_add(1);
                if !*stale_notice_emitted && *stale_polls >= policy.stale_notice_polls {
                    *stale_notice_emitted = true;
                    return Some(no_progress_liveness_event());
                }
                None
            }
            Self::AwaitingLedgerRows { .. } => {
                *self = Self::TrackingLedgerRows {
                    stale_polls: 1,
                    stale_notice_emitted: false,
                };
                None
            }
        }
    }

    fn observe_empty_running_mission(
        &mut self,
        policy: FollowPolicy,
        trace_id: &str,
    ) -> FollowStep {
        let Self::AwaitingLedgerRows {
            empty_polls,
            pending_emitted,
        } = self
        else {
            *self = Self::awaiting();
            return self.observe_empty_running_mission(policy, trace_id);
        };

        *empty_polls = empty_polls.saturating_add(1);
        let mut events = Vec::new();
        if !*pending_emitted {
            *pending_emitted = true;
            events.push(WatchEvent::Pending {
                trace: trace_id.to_string(),
                status: "awaiting_invocation_records".to_string(),
                source: "local".to_string(),
            });
        }
        if *empty_polls >= policy.max_empty_polls {
            events.push(empty_trace_timeout_liveness_event());
            return FollowStep {
                events,
                decision: FollowDecision::Stop,
            };
        }
        FollowStep {
            events,
            decision: FollowDecision::Continue,
        }
    }
}

/// Explicit follow-mode state machine. It owns the diff engine and
/// the current follow phase so streaming JSON and the TUI cannot drift
/// into separate terminality/liveness behavior.
struct FollowEngine {
    watch: WatchEngine,
    policy: FollowPolicy,
    phase: FollowPhase,
}

impl FollowEngine {
    #[cfg(test)]
    fn with_test_budget(trace_id: String, max_empty_polls: u64) -> Self {
        Self::with_policy(trace_id, FollowPolicy::test_budget(max_empty_polls))
    }

    fn with_policy(trace_id: String, policy: FollowPolicy) -> Self {
        Self {
            watch: WatchEngine::new(trace_id),
            policy,
            phase: FollowPhase::awaiting(),
        }
    }

    fn trace_id(&self) -> &str {
        self.watch.trace_id()
    }

    fn observe(&mut self, nodes: &[Record]) -> anyhow::Result<FollowStep> {
        self.observe_with_mission_status(nodes, mission_follow_status(self.trace_id()))
    }

    fn observe_with_mission_status(
        &mut self,
        nodes: &[Record],
        mission_status: MissionFollowStatus,
    ) -> anyhow::Result<FollowStep> {
        let mut events = self.watch.diff(nodes.iter().map(RecordView::from));

        if !nodes.is_empty() {
            if WatchEngine::all_terminal(nodes.iter().map(RecordView::from)) {
                events.push(terminal_event_for_nodes(&self.watch, nodes));
                return Ok(FollowStep {
                    events,
                    decision: FollowDecision::Stop,
                });
            }
            if mission_status == MissionFollowStatus::Interrupted {
                events.push(interrupted_liveness_event());
                return Ok(FollowStep {
                    events,
                    decision: FollowDecision::Stop,
                });
            }
            // Non-terminal records present. A diff (`events` non-empty)
            // is observable progress. A string of polls with no new
            // event is a liveness observation, not a terminal condition:
            // once a ledger row exists, `--follow` means "track until
            // terminality" and the operator can interrupt a genuinely
            // stuck run.
            if events.is_empty() {
                if let Some(event) = self.phase.observe_stale_ledger_rows(self.policy) {
                    events.push(event);
                }
            } else {
                self.phase.observe_progress();
            }
            return Ok(FollowStep {
                events,
                decision: FollowDecision::Continue,
            });
        }

        match mission_status {
            MissionFollowStatus::Terminal(status) => {
                events.push(WatchEvent::Terminal {
                    trace: self.trace_id().to_string(),
                    status,
                    ledger_reported_receipt_chain_verified: false,
                    cli_receipt_chain_verification: cli_receipt_chain_verification(),
                    usage: None,
                });
                Ok(FollowStep {
                    events,
                    decision: FollowDecision::Stop,
                })
            }
            MissionFollowStatus::Interrupted => {
                events.push(interrupted_liveness_event());
                Ok(FollowStep {
                    events,
                    decision: FollowDecision::Stop,
                })
            }
            MissionFollowStatus::Running => {
                let trace_id = self.trace_id().to_string();
                Ok(self
                    .phase
                    .observe_empty_running_mission(self.policy, &trace_id))
            }
            MissionFollowStatus::NotMission => {
                anyhow::bail!(
                    "trace {:?} has no invocation records and is not a local mission run",
                    self.trace_id()
                )
            }
        }
    }
}

/// `{trace_id, nodes}` subset of the `invocation.trace.get` response.
#[derive(Debug, Deserialize)]
struct TraceSnapshot {
    trace_id: String,
    #[serde(default)]
    nodes: Vec<Record>,
}

/// What one watch entry resolves to: a trace id and the records of
/// its causal set.
struct CausalSet {
    trace_id: String,
    nodes: Vec<Record>,
}

fn fetch_trace(trace_id: &str) -> anyhow::Result<CausalSet> {
    let resp = invoke_local_ability(
        ABILITY_TRACE_GET,
        json!({ "key": { "trace_id": trace_id } }),
    )
    .context("read trace from the invocation ledger")?;
    let snap: TraceSnapshot =
        serde_json::from_value(resp).context("decode invocation.trace.get response")?;
    reject_empty_unknown_trace(&snap.trace_id, &snap.nodes)?;
    Ok(CausalSet {
        trace_id: snap.trace_id,
        nodes: snap.nodes,
    })
}

fn reject_empty_unknown_trace(trace_id: &str, nodes: &[Record]) -> anyhow::Result<()> {
    if nodes.is_empty() && !mission_run_exists(trace_id) {
        anyhow::bail!(
            "trace {trace_id:?} has no invocation records in the ledger; \
             check the trace id or start with `easynet invocation history list`"
        );
    }
    Ok(())
}

/// Resolve the watch entry to its causal set.
///
/// `--trace` reads the trace directly. The positional invocation URA
/// first reads its ledger record: a recorded trace widens the watch
/// to the whole set; a record WITHOUT a trace (a bare unary call) is
/// its own singleton causal set — the trace anchor degenerates to
/// the invocation itself, honestly, instead of refusing.
fn fetch_causal_set(args: &WatchArgs) -> anyhow::Result<CausalSet> {
    if let Some(trace) = args.trace.as_deref() {
        return fetch_trace(trace);
    }
    let ura = args.invocation.as_deref().ok_or_else(|| {
        anyhow::anyhow!("invocation watch requires either an invocation URA or --trace <trace_id>")
    })?;
    let resp = invoke_local_ability(
        crate::runtime::agents::invocation_history_ability::ABILITY_HISTORY_GET,
        json!({ "key": { "ura": ura } }),
    )
    .context("read the invocation's ledger record")?;
    let record: Record = serde_json::from_value(
        resp.get("record")
            .cloned()
            .filter(|r| !r.is_null())
            .ok_or_else(|| anyhow::anyhow!("no ledger record for invocation {ura:?}"))?,
    )
    .context("decode invocation record")?;

    if record.trace_id.is_empty() {
        return Ok(CausalSet {
            trace_id: record.invocation_ura.clone(),
            nodes: vec![record],
        });
    }
    fetch_trace(&record.trace_id)
}

/// Mission-run liveness, when the trace anchor names a mission run:
/// stored `running` plus a dead heartbeat is the exact crash state
/// F-022's pid file used to misrender as forever-running. Best
/// effort — a trace that is not a mission run simply has no local
/// liveness source.
fn mission_follow_status(trace_id: &str) -> MissionFollowStatus {
    let Ok(summary) = crate::facade::cli::mission_runs::find_run(trace_id) else {
        return MissionFollowStatus::NotMission;
    };
    if summary.meta.status.is_terminal() {
        return MissionFollowStatus::Terminal(mission_terminal_status(summary.meta.status).into());
    }
    let interrupted = summary.meta.status
        == crate::facade::cli::mission_runs::MissionRunStatus::Running
        && !summary.running;
    if interrupted {
        MissionFollowStatus::Interrupted
    } else {
        MissionFollowStatus::Running
    }
}

fn interrupted_liveness_event() -> WatchEvent {
    WatchEvent::Liveness {
        status: "interrupted".to_string(),
        source: "local".to_string(),
    }
}

fn empty_trace_timeout_liveness_event() -> WatchEvent {
    WatchEvent::Liveness {
        status: "ledger_rows_timeout".to_string(),
        source: "watch_follow_policy".to_string(),
    }
}

fn no_progress_liveness_event() -> WatchEvent {
    WatchEvent::Liveness {
        status: "no_ledger_progress_observed".to_string(),
        source: "watch_follow_policy".to_string(),
    }
}

fn mission_terminal_for_empty_trace(trace_id: &str, nodes: &[Record]) -> Option<WatchEvent> {
    if !nodes.is_empty() {
        return None;
    }
    let summary = crate::facade::cli::mission_runs::find_run(trace_id).ok()?;
    if !summary.meta.status.is_terminal() {
        return None;
    }
    Some(WatchEvent::Terminal {
        trace: trace_id.to_string(),
        status: mission_terminal_status(summary.meta.status).to_string(),
        ledger_reported_receipt_chain_verified: false,
        cli_receipt_chain_verification: cli_receipt_chain_verification(),
        usage: None,
    })
}

fn mission_terminal_status(
    status: crate::facade::cli::mission_runs::MissionRunStatus,
) -> &'static str {
    match status {
        crate::facade::cli::mission_runs::MissionRunStatus::Ok => "ok",
        crate::facade::cli::mission_runs::MissionRunStatus::Partial
        | crate::facade::cli::mission_runs::MissionRunStatus::Error => "failed",
        crate::facade::cli::mission_runs::MissionRunStatus::Cancelled => "cancelled",
        crate::facade::cli::mission_runs::MissionRunStatus::Running => "running",
    }
}

fn mission_run_exists(trace_id: &str) -> bool {
    crate::facade::cli::mission_runs::find_run(trace_id).is_ok()
}

/// One-shot snapshot of a trace's causal set — the typed surface the
/// snapshot mode renders and integration tests assert (same
/// compute/render split as `discover::execute`).
#[derive(Debug, serde::Serialize)]
pub struct WatchSnapshot {
    pub trace_id: String,
    pub events: Vec<WatchEvent>,
    pub rows: Vec<WatchRow>,
    /// Present when every record is terminal.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminal: Option<WatchEvent>,
}

/// Stable row model for table/TUI renderers. It is intentionally
/// derived from ledger records, not EAL steps, so no step address
/// leaks into the product surface.
#[derive(Debug, Clone, serde::Serialize)]
pub struct WatchRow {
    pub invocation: String,
    pub ability: String,
    pub state: String,
    pub caller: String,
    pub callee: String,
    pub subject: String,
    pub elapsed_ms: Option<u64>,
    pub usage: InvocationUsage,
    pub ledger_reported_receipt_chain_verified: bool,
    pub cli_receipt_chain_verification: CliReceiptChainVerification,
    pub authority_form: String,
}

impl From<&Record> for WatchRow {
    fn from(record: &Record) -> Self {
        Self {
            invocation: record.invocation_ura.clone(),
            ability: record.ability_name.clone(),
            state: record.state.clone(),
            caller: record.caller_ura.clone(),
            callee: record.callee_ura.clone(),
            subject: record.subject_ura.clone(),
            elapsed_ms: record.elapsed_ms,
            usage: record.usage,
            ledger_reported_receipt_chain_verified: record.receipt_chain.verified,
            cli_receipt_chain_verification: cli_receipt_chain_verification(),
            authority_form: record.authority_form.clone(),
        }
    }
}

fn snapshot_from_nodes(trace_id: String, nodes: Vec<Record>) -> WatchSnapshot {
    let mut engine = WatchEngine::new(trace_id);
    let events = engine.diff(nodes.iter().map(RecordView::from));
    let terminal = mission_terminal_for_empty_trace(engine.trace_id(), &nodes).or_else(|| {
        WatchEngine::all_terminal(nodes.iter().map(RecordView::from))
            .then(|| terminal_event_for_nodes(&engine, &nodes))
    });
    WatchSnapshot {
        trace_id: engine.trace_id().to_string(),
        events,
        rows: nodes.iter().map(WatchRow::from).collect(),
        terminal,
    }
}

fn terminal_event_for_nodes(engine: &WatchEngine, nodes: &[Record]) -> WatchEvent {
    WatchEvent::Terminal {
        trace: engine.trace_id().to_string(),
        status: WatchEngine::terminal_status(nodes.iter().map(RecordView::from)).to_string(),
        ledger_reported_receipt_chain_verified: WatchEngine::ledger_reported_receipt_chain_verified(
            nodes.iter().map(RecordView::from),
        ),
        cli_receipt_chain_verification: cli_receipt_chain_verification(),
        usage: Some(WatchEngine::total_usage(nodes.iter().map(RecordView::from))),
    }
}

/// Fetch and project the trace once, without following.
pub fn execute_once(args: &WatchArgs) -> anyhow::Result<WatchSnapshot> {
    let snap = fetch_causal_set(args)?;
    Ok(snapshot_from_nodes(snap.trace_id, snap.nodes))
}

/// Follow one watch target until it reaches either a ledger terminal
/// ledger state or a daemon-local interrupted liveness state.
///
/// This is the testable core behind `invocation watch --follow`.
/// It still reads only the invocation ledger plus the mission
/// heartbeat projection; it does not introduce a mission-level
/// execution graph or a second terminality table.
pub fn execute_follow_until_terminal(args: &WatchArgs) -> anyhow::Result<Vec<WatchEvent>> {
    let mut events = Vec::new();
    stream_follow_events(args, |batch| {
        events.extend(batch.iter().cloned());
        Ok(())
    })?;
    Ok(events)
}

pub fn run(args: WatchArgs) -> anyhow::Result<()> {
    if args.format == OutputFormat::Panel {
        return run_panel(args);
    }
    let ndjson = args.format == OutputFormat::Json;
    let emit = |events: &[WatchEvent]| -> anyhow::Result<()> {
        for e in events {
            if ndjson {
                println!("{}", serde_json::to_string(e)?);
            } else {
                render_human(e);
            }
        }
        Ok(())
    };

    if !args.follow {
        let snapshot = execute_once(&args)?;
        emit(&snapshot.events)?;
        match snapshot.terminal {
            Some(terminal) => emit(std::slice::from_ref(&terminal))?,
            None => output::info("trace is still running; re-run with --follow to stream it"),
        }
        return Ok(());
    }

    stream_follow_events(&args, emit)
}

fn stream_follow_events(
    args: &WatchArgs,
    mut emit: impl FnMut(&[WatchEvent]) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let first = fetch_causal_set(args)?;
    let mut engine = FollowEngine::with_policy(first.trace_id, FollowPolicy::from_args(args));
    let mut nodes = first.nodes;

    loop {
        let step = engine.observe(&nodes)?;
        emit(&step.events)?;
        if step.done() {
            return Ok(());
        }
        std::thread::sleep(FOLLOW_INTERVAL);
        nodes = fetch_causal_set(args)?.nodes;
    }
}

fn run_panel(args: WatchArgs) -> anyhow::Result<()> {
    if !args.follow {
        let snapshot = execute_once(&args)?;
        println!("{}", render_panel_snapshot(&snapshot));
        return Ok(());
    }

    let first = fetch_causal_set(&args)?;
    let mut engine = FollowEngine::with_policy(first.trace_id, FollowPolicy::from_args(&args));
    let mut nodes = first.nodes;
    loop {
        let step = engine.observe(&nodes)?;
        let snapshot = snapshot_from_nodes(engine.trace_id().to_string(), nodes.clone());
        print!("\x1B[2J\x1B[H{}", render_panel_snapshot(&snapshot));
        for event in step
            .events
            .iter()
            .filter(|event| !matches!(event, WatchEvent::State { .. }))
        {
            println!();
            render_human(event);
        }
        if step.done() {
            return Ok(());
        }
        std::thread::sleep(FOLLOW_INTERVAL);
        nodes = fetch_causal_set(&args)?.nodes;
    }
}

fn render_human(event: &WatchEvent) {
    match event {
        WatchEvent::State {
            invocation,
            ability,
            state,
        } => println!("{state:<12} {ability:<28} {invocation}"),
        WatchEvent::Terminal {
            trace,
            status,
            ledger_reported_receipt_chain_verified,
            cli_receipt_chain_verification,
            usage,
        } => {
            let cost = usage
                .map(|u| {
                    format!(
                        " · {} tok in / {} tok out / {}ms",
                        u.tokens_in, u.tokens_out, u.duration_ms
                    )
                })
                .unwrap_or_default();
            output::success(&format!(
                "trace {trace} → {status}{cost} · ledger_reported_receipt_chain_verified={ledger_reported_receipt_chain_verified} · cli_receipt_chain_verification={cli_receipt_chain_verification}"
            ))
        }
        WatchEvent::Pending {
            trace,
            status,
            source,
        } => output::info(&format!("trace {trace}: {status} ({source})")),
        WatchEvent::Liveness { status, source } => {
            output::warn(&format!("liveness: {status} ({source})"))
        }
    }
}

/// Render the watch snapshot as a deterministic, ratatui-backed
/// three-column frame. This is deliberately a pure renderer so tests
/// pin the product semantics without depending on terminal I/O.
pub fn render_panel_snapshot(snapshot: &WatchSnapshot) -> String {
    render_panel_snapshot_with_size(snapshot, 120, 22)
}

pub fn render_panel_snapshot_with_size(
    snapshot: &WatchSnapshot,
    width: u16,
    height: u16,
) -> String {
    let width = width.max(1);
    let height = height.max(1);

    let area = Rect::new(0, 0, width, height);
    let mut buffer = Buffer::empty(area);
    render_panel_frame(snapshot, area, &mut buffer);
    buffer_to_string(&buffer, width, height)
}

fn render_panel_frame(snapshot: &WatchSnapshot, area: Rect, buffer: &mut Buffer) {
    let constraints = if area.width < 100 {
        [
            Constraint::Percentage(25),
            Constraint::Percentage(40),
            Constraint::Percentage(35),
        ]
    } else {
        [
            Constraint::Length(28),
            Constraint::Length(48),
            Constraint::Min(36),
        ]
    };
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(area);

    render_trace_panel(snapshot, columns[0], buffer);
    render_invocation_panel(snapshot, columns[1], buffer);
    render_receipt_panel(snapshot, columns[2], buffer);
}

fn render_trace_panel(snapshot: &WatchSnapshot, area: Rect, buffer: &mut Buffer) {
    let terminal = terminal_label(snapshot.terminal.as_ref());
    let rows = [
        format!("trace: {}", compact(&snapshot.trace_id, 18)),
        format!("status: {terminal}"),
        String::new(),
        "phase projection".to_string(),
        "ledger facts only".to_string(),
        "local liveness marked local".to_string(),
    ];
    Paragraph::new(rows.join("\n"))
        .block(Block::default().title("Phases").borders(Borders::ALL))
        .render(area, buffer);
}

fn render_invocation_panel(snapshot: &WatchSnapshot, area: Rect, buffer: &mut Buffer) {
    let items: Vec<ListItem> = if snapshot.rows.is_empty() {
        vec![ListItem::new("no invocations")]
    } else {
        snapshot
            .rows
            .iter()
            .map(|row| {
                ListItem::new(format!(
                    "{:<10} {:<20} {}",
                    row.state,
                    compact(&row.ability, 20),
                    compact(&row.invocation, 12)
                ))
            })
            .collect()
    };
    List::new(items)
        .block(Block::default().title("Invocations").borders(Borders::ALL))
        .render(area, buffer);
}

fn render_receipt_panel(snapshot: &WatchSnapshot, area: Rect, buffer: &mut Buffer) {
    let row = snapshot.rows.first();
    let usage = terminal_usage(snapshot.terminal.as_ref())
        .or_else(|| row.map(|r| r.usage))
        .unwrap_or_default();
    let (ledger_reported_receipt_chain_verified, cli_receipt_chain_verification) =
        terminal_attestation(snapshot.terminal.as_ref()).unwrap_or_else(|| {
            row.map(|r| {
                (
                    r.ledger_reported_receipt_chain_verified,
                    r.cli_receipt_chain_verification,
                )
            })
            .unwrap_or((false, cli_receipt_chain_verification()))
        });
    let lines = vec![
        format!(
            "invocation: {}",
            row.map(|r| compact(&r.invocation, 28))
                .unwrap_or_else(|| "-".to_string())
        ),
        format!(
            "caller: {}",
            row.map(|r| compact(&r.caller, 32))
                .unwrap_or_else(|| "-".to_string())
        ),
        format!(
            "callee: {}",
            row.map(|r| compact(&r.callee, 32))
                .unwrap_or_else(|| "-".to_string())
        ),
        format!(
            "subject: {}",
            row.map(|r| compact(&r.subject, 30))
                .unwrap_or_else(|| "-".to_string())
        ),
        String::new(),
        format!(
            "usage: in={} out={} calls={}",
            usage.tokens_in, usage.tokens_out, usage.external_calls
        ),
        format!("duration_ms: {}", usage.duration_ms),
        format!("ledger_reported_chain_verified: {ledger_reported_receipt_chain_verified}"),
        format!("cli_chain_verification: {cli_receipt_chain_verification}"),
        format!(
            "elapsed_ms: {}",
            row.and_then(|r| r.elapsed_ms)
                .map(|ms| ms.to_string())
                .unwrap_or_else(|| "-".to_string())
        ),
        format!(
            "permission: {}",
            row.map(|r| r.authority_form.as_str()).unwrap_or("-")
        ),
    ];
    Paragraph::new(lines.join("\n"))
        .block(Block::default().title("Receipt").borders(Borders::ALL))
        .render(area, buffer);
}

fn terminal_label(event: Option<&WatchEvent>) -> &'static str {
    match event {
        Some(WatchEvent::Terminal { status, .. }) if status == "ok" => "ok",
        Some(WatchEvent::Terminal { status, .. }) if status == "failed" => "failed",
        Some(WatchEvent::Terminal { status, .. }) if status == "cancelled" => "cancelled",
        Some(WatchEvent::Terminal { .. }) => "terminal",
        _ => "running",
    }
}

fn terminal_usage(event: Option<&WatchEvent>) -> Option<InvocationUsage> {
    match event {
        Some(WatchEvent::Terminal { usage, .. }) => *usage,
        _ => None,
    }
}

fn terminal_attestation(event: Option<&WatchEvent>) -> Option<(bool, CliReceiptChainVerification)> {
    match event {
        Some(WatchEvent::Terminal {
            ledger_reported_receipt_chain_verified,
            cli_receipt_chain_verification,
            ..
        }) => Some((
            *ledger_reported_receipt_chain_verified,
            *cli_receipt_chain_verification,
        )),
        _ => None,
    }
}

fn cli_receipt_chain_verification() -> CliReceiptChainVerification {
    CliReceiptChainVerification::not_performed()
}

fn compact(value: &str, width: usize) -> String {
    if value.chars().count() <= width {
        return value.to_string();
    }
    if width <= 1 {
        return "…".to_string();
    }
    let prefix: String = value.chars().take(width - 1).collect();
    format!("{prefix}…")
}

fn buffer_to_string(buffer: &Buffer, width: u16, height: u16) -> String {
    let mut lines = Vec::with_capacity(height as usize);
    for y in 0..height {
        let mut line = String::new();
        for x in 0..width {
            line.push_str(buffer[(x, y)].symbol());
        }
        lines.push(line.trim_end().to_string());
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view<'a>(ura: &'a str, ability: &'a str, state: &'a str) -> RecordView<'a> {
        RecordView {
            invocation_ura: ura,
            ability_name: ability,
            state,
            usage: Default::default(),
            ledger_reported_receipt_chain_verified: true,
        }
    }

    #[test]
    fn diff_emits_new_then_only_changes() {
        let mut engine = WatchEngine::new("t-1".into());
        let first = engine.diff([view("inv-a", "fs.read", "RUNNING")]);
        assert_eq!(first.len(), 1);

        let unchanged = engine.diff([view("inv-a", "fs.read", "RUNNING")]);
        assert!(unchanged.is_empty(), "no event without a state change");

        let moved = engine.diff([view("inv-a", "fs.read", "COMPLETED")]);
        assert_eq!(
            moved,
            vec![WatchEvent::State {
                invocation: "inv-a".into(),
                ability: "fs.read".into(),
                state: "COMPLETED".into(),
            }]
        );
    }

    #[test]
    fn terminality_follows_the_axon_vocabulary() {
        let done = [view("a", "x", "COMPLETED"), view("b", "y", "CANCELLED")];
        assert!(WatchEngine::all_terminal(done));
        assert_eq!(WatchEngine::terminal_status(done), "cancelled");

        let failed = [view("a", "x", "COMPLETED"), view("b", "y", "TIMED_OUT")];
        assert_eq!(WatchEngine::terminal_status(failed), "failed");

        let live = [view("a", "x", "COMPLETED"), view("b", "y", "RUNNING")];
        assert!(!WatchEngine::all_terminal(live));

        assert!(
            !WatchEngine::all_terminal(std::iter::empty()),
            "an empty trace is pending, not terminal"
        );
    }

    #[test]
    fn follow_engine_emits_pending_once_then_stops_empty_running_trace_after_timeout() {
        let mut engine = FollowEngine::with_test_budget("trace-empty".into(), 3);
        let first = engine
            .observe_with_mission_status(&[], MissionFollowStatus::Running)
            .expect("first empty running poll is pending");
        assert_eq!(
            first.events,
            vec![WatchEvent::Pending {
                trace: "trace-empty".into(),
                status: "awaiting_invocation_records".into(),
                source: "local".into(),
            }]
        );
        assert_eq!(first.decision, FollowDecision::Continue);

        let second = engine
            .observe_with_mission_status(&[], MissionFollowStatus::Running)
            .expect("second empty running poll stays quiet");
        assert!(second.events.is_empty(), "{second:?}");
        assert_eq!(second.decision, FollowDecision::Continue);

        let third = engine
            .observe_with_mission_status(&[], MissionFollowStatus::Running)
            .expect("bounded empty trace returns a liveness outcome");
        assert!(third.done());
        assert_eq!(
            third.events,
            vec![WatchEvent::Liveness {
                status: "ledger_rows_timeout".into(),
                source: "watch_follow_policy".into(),
            }]
        );
    }

    #[test]
    fn follow_engine_times_out_immediately_when_empty_budget_is_one_poll() {
        let mut engine = FollowEngine::with_test_budget("trace-empty".into(), 1);
        let step = engine
            .observe_with_mission_status(&[], MissionFollowStatus::Running)
            .expect("first empty poll reaches a one-poll timeout");
        assert!(step.done());
        assert_eq!(
            step.events,
            vec![
                WatchEvent::Pending {
                    trace: "trace-empty".into(),
                    status: "awaiting_invocation_records".into(),
                    source: "local".into(),
                },
                WatchEvent::Liveness {
                    status: "ledger_rows_timeout".into(),
                    source: "watch_follow_policy".into(),
                },
            ]
        );
    }

    /// One non-terminal ledger record, decoded the same way the
    /// production `fetch_trace` path decodes a trace snapshot.
    fn running_record(ura: &str, state: &str) -> Record {
        serde_json::from_value(json!({
            "invocation_ura": ura,
            "request_id": "req-1",
            "trace_id": "trace-stuck",
            "span_id": "span-1",
            "caller_ura": "easynet:///r/acme/agent/user.caller",
            "callee_ura": "easynet:///r/acme/agent/user.callee",
            "subject_ura": "easynet:///r/acme/agent/user.callee",
            "ability_ura": "easynet:///r/acme/ability/user.callee.fs.read",
            "ability_name": "fs.read",
            "state": state,
            "started_unix_ms": 0,
            "args": { "kind": "digest", "content_type": "application/json", "sha256": "00", "size_bytes": 0 },
        }))
        .expect("minimal running ledger record decodes")
    }

    #[test]
    fn follow_engine_warns_but_keeps_tracking_stale_nonempty_running_trace() {
        // Once a ledger row exists, follow mode is a terminality tracker,
        // not an observer-side timeout. A stale RUNNING row emits one
        // liveness notice after the no-progress budget and then keeps
        // following until the ledger reaches a terminal state.
        let mut engine = FollowEngine::with_test_budget("trace-stuck".into(), 2);
        let nodes = [running_record(
            "easynet:///r/acme/invocation/01STUCK",
            "RUNNING",
        )];

        let first = engine
            .observe_with_mission_status(&nodes, MissionFollowStatus::NotMission)
            .expect("first poll reports the new record");
        assert_eq!(first.events.len(), 1, "first poll diffs the new record");
        assert_eq!(first.decision, FollowDecision::Continue);

        // No state change → no diff → stale budget accrues, no stop yet.
        let second = engine
            .observe_with_mission_status(&nodes, MissionFollowStatus::NotMission)
            .expect("unchanged poll stays quiet");
        assert!(second.events.is_empty(), "{second:?}");
        assert_eq!(second.decision, FollowDecision::Continue);

        // Budget (2) reached on the second stale poll → liveness notice,
        // but no terminal stop.
        let notice = engine
            .observe_with_mission_status(&nodes, MissionFollowStatus::NotMission)
            .expect("stale trace emits one liveness notice");
        assert_eq!(notice.decision, FollowDecision::Continue);
        assert_eq!(
            notice.events,
            vec![WatchEvent::Liveness {
                status: "no_ledger_progress_observed".into(),
                source: "watch_follow_policy".into(),
            }]
        );

        let quiet = engine
            .observe_with_mission_status(&nodes, MissionFollowStatus::NotMission)
            .expect("subsequent stale polls stay quiet");
        assert!(quiet.events.is_empty(), "{quiet:?}");
        assert_eq!(quiet.decision, FollowDecision::Continue);
    }

    #[test]
    fn follow_engine_does_not_bound_a_progressing_trace() {
        // A trace that keeps producing diffs is making observable
        // progress; the stale budget must reset every progress poll so
        // a healthy long-running stream is never cut off.
        let mut engine = FollowEngine::with_test_budget("trace-stuck".into(), 1);
        for i in 0..5 {
            let ura = format!("easynet:///r/acme/invocation/01PROG{i}");
            let nodes = [running_record(&ura, "RUNNING")];
            let step = engine
                .observe_with_mission_status(&nodes, MissionFollowStatus::NotMission)
                .expect("progress poll");
            assert_eq!(step.decision, FollowDecision::Continue, "poll {i}");
            assert_eq!(step.events.len(), 1, "poll {i} diffs a new record");
        }
    }

    #[test]
    fn follow_engine_stops_empty_terminal_mission_without_fake_usage() {
        let mut engine = FollowEngine::with_test_budget("trace-zero".into(), 2);
        let step = engine
            .observe_with_mission_status(&[], MissionFollowStatus::Terminal("ok".into()))
            .expect("terminal mission");
        assert!(step.done());
        assert_eq!(
            step.events,
            vec![WatchEvent::Terminal {
                trace: "trace-zero".into(),
                status: "ok".into(),
                ledger_reported_receipt_chain_verified: false,
                cli_receipt_chain_verification: CliReceiptChainVerification::not_performed(),
                usage: None,
            }]
        );
    }

    #[test]
    fn empty_unknown_trace_is_not_pending_forever() {
        let err = reject_empty_unknown_trace("__missing_trace_for_watch_test__", &[]).unwrap_err();
        assert!(
            err.to_string().contains("has no invocation records"),
            "{err}"
        );
    }

    #[test]
    fn empty_terminal_mission_trace_emits_terminal_event() {
        let _home = crate::facade::cli::test_support::HomeGuard::new();
        let trace_id = "2026-06-23_120000";
        let run_dir = crate::facade::cli::mission_runs::root_dir().join(trace_id);
        std::fs::create_dir_all(&run_dir).expect("run dir");
        let meta = crate::facade::cli::mission_runs::MissionRunMeta {
            name: "zero-node".to_string(),
            trace_id: trace_id.to_string(),
            started_at: "2026-06-23T12:00:00+00:00".to_string(),
            status: crate::facade::cli::mission_runs::MissionRunStatus::Ok,
            ..Default::default()
        };
        std::fs::write(
            run_dir.join("meta.json"),
            serde_json::to_string_pretty(&meta).expect("meta json"),
        )
        .expect("write meta");

        let snapshot = snapshot_from_nodes(trace_id.to_string(), Vec::new());
        assert_eq!(
            snapshot.terminal,
            Some(WatchEvent::Terminal {
                trace: trace_id.to_string(),
                status: "ok".to_string(),
                ledger_reported_receipt_chain_verified: false,
                cli_receipt_chain_verification: CliReceiptChainVerification::not_performed(),
                usage: None,
            })
        );
    }

    #[test]
    fn ndjson_event_shape_is_frozen() {
        let e = WatchEvent::State {
            invocation: "inv-a".into(),
            ability: "fs.read".into(),
            state: "RUNNING".into(),
        };
        assert_eq!(
            serde_json::to_string(&e).unwrap(),
            r#"{"event":"state","invocation":"inv-a","ability":"fs.read","state":"RUNNING"}"#
        );
        let t = WatchEvent::Terminal {
            trace: "t-1".into(),
            status: "ok".into(),
            ledger_reported_receipt_chain_verified: true,
            cli_receipt_chain_verification: CliReceiptChainVerification::not_performed(),
            usage: None,
        };
        assert_eq!(
            serde_json::to_string(&t).unwrap(),
            r#"{"event":"terminal","trace":"t-1","status":"ok","ledger_reported_receipt_chain_verified":true,"cli_receipt_chain_verification":"not_performed"}"#,
            "absent usage keeps the terminal event explicit about attestation"
        );
        let t = WatchEvent::Terminal {
            trace: "t-1".into(),
            status: "ok".into(),
            ledger_reported_receipt_chain_verified: true,
            cli_receipt_chain_verification: CliReceiptChainVerification::not_performed(),
            usage: Some(InvocationUsage {
                tokens_in: 7,
                tokens_out: 3,
                duration_ms: 42,
                external_calls: 0,
            }),
        };
        assert!(
            serde_json::to_string(&t)
                .unwrap()
                .contains(r#""duration_ms":42"#),
            "usage aggregate rides the terminal event"
        );
        let l = WatchEvent::Liveness {
            status: "interrupted".into(),
            source: "local".into(),
        };
        assert_eq!(
            serde_json::to_string(&l).unwrap(),
            r#"{"event":"liveness","status":"interrupted","source":"local"}"#
        );
        let p = WatchEvent::Pending {
            trace: "t-1".into(),
            status: "awaiting_invocation_records".into(),
            source: "local".into(),
        };
        assert_eq!(
            serde_json::to_string(&p).unwrap(),
            r#"{"event":"pending","trace":"t-1","status":"awaiting_invocation_records","source":"local"}"#
        );
    }

    #[test]
    fn panel_snapshot_renders_the_three_fact_columns() {
        let snapshot = WatchSnapshot {
            trace_id: "trace-1".into(),
            events: vec![WatchEvent::State {
                invocation: "easynet:///r/cli/invocation/abc".into(),
                ability: "testbot.echo".into(),
                state: "COMPLETED".into(),
            }],
            rows: vec![WatchRow {
                invocation: "easynet:///r/cli/invocation/abc".into(),
                ability: "testbot.echo".into(),
                state: "COMPLETED".into(),
                caller: "easynet:///r/cli/agent/user.owner".into(),
                callee: "easynet:///r/cli/device/local".into(),
                subject: "easynet:///r/cli/agent/user.target".into(),
                elapsed_ms: Some(12),
                usage: InvocationUsage {
                    tokens_in: 0,
                    tokens_out: 0,
                    duration_ms: 12,
                    external_calls: 0,
                },
                ledger_reported_receipt_chain_verified: true,
                cli_receipt_chain_verification: CliReceiptChainVerification::not_performed(),
                authority_form: "self".into(),
            }],
            terminal: Some(WatchEvent::Terminal {
                trace: "trace-1".into(),
                status: "ok".into(),
                ledger_reported_receipt_chain_verified: true,
                cli_receipt_chain_verification: CliReceiptChainVerification::not_performed(),
                usage: Some(InvocationUsage {
                    tokens_in: 0,
                    tokens_out: 0,
                    duration_ms: 12,
                    external_calls: 0,
                }),
            }),
        };

        let frame = render_panel_snapshot(&snapshot);
        assert!(frame.contains("Phases"), "{frame}");
        assert!(frame.contains("Invocations"), "{frame}");
        assert!(frame.contains("Receipt"), "{frame}");
        assert!(frame.contains("testbot.echo"), "{frame}");
        assert!(
            frame.contains("ledger_reported_chain_verified: true"),
            "{frame}"
        );
        assert!(
            frame.contains("cli_chain_verification: not_performed"),
            "{frame}"
        );
        assert!(
            !frame.contains("step"),
            "watch panel must not expose step-level addressing: {frame}"
        );

        let narrow = render_panel_snapshot_with_size(&snapshot, 72, 12);
        assert_eq!(narrow.lines().count(), 12, "{narrow}");
        assert!(
            narrow.lines().all(|line| line.chars().count() <= 72),
            "narrow renderer must not leak the fixed 120-col buffer: {narrow}"
        );
    }
}
