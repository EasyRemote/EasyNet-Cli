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
use clap::Args;
use serde::Deserialize;
use serde_json::json;

use crate::runtime::agents::invocation_history_ability::ABILITY_TRACE_GET;
use crate::support::local_invoke::invoke_local_ability;
use crate::support::output;

/// Narrow re-export for `pub` consumers (house pattern).
pub use crate::support::output::OutputFormat;

type Record = easynet_axon::invocation::InvocationLedgerRecord;
use easynet_axon::invocation::axiom::InvocationUsage;
use easynet_axon::invocation::InvocationState;

/// Poll cadence for `--follow`. A constant, not a flag: the ledger
/// read is daemon-local and cheap, and a knob would only invite
/// configuration where none is needed.
const FOLLOW_INTERVAL: Duration = Duration::from_millis(500);

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
    /// 'table' renders a snapshot; 'json' emits NDJSON events
    /// (streams have no table form — spec §0.2-3 exception).
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,
}

/// One NDJSON event. The `event` tag plus field names are frozen by
/// the W2-E2E-2 contract (spec §0.2-9).
#[derive(Debug, PartialEq, serde::Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum WatchEvent {
    /// A new invocation appeared in the trace, or its state moved.
    State {
        invocation: String,
        ability: String,
        state: String,
    },
    /// Every invocation in the trace reached a terminal state.
    /// `usage` is the trace-level consumption sum, copied from the
    /// SIGNED terminal receipts via the ledger rows (DEC-010 card ①)
    /// — the cost line the TUI renders without an `unsigned` tag.
    Terminal {
        trace: String,
        status: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        usage: Option<InvocationUsage>,
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
}

impl<'a> From<&'a Record> for RecordView<'a> {
    fn from(r: &'a Record) -> Self {
        RecordView {
            invocation_ura: &r.invocation_ura,
            ability_name: &r.ability_name,
            state: &r.state,
            usage: r.usage,
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

    /// Trace-level consumption: the sum of every record's signed
    /// usage. Summation is the only aggregation that means anything
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
    Ok(CausalSet {
        trace_id: snap.trace_id,
        nodes: snap.nodes,
    })
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
    let ura = args
        .invocation
        .as_deref()
        .expect("clap requires invocation unless --trace");
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
fn mission_liveness(trace_id: &str) -> Option<WatchEvent> {
    let summary = crate::facade::cli::mission_runs::find_run(trace_id).ok()?;
    let interrupted = summary.meta.status
        == crate::facade::cli::mission_runs::MissionRunStatus::Running
        && !summary.running;
    interrupted.then(|| WatchEvent::Liveness {
        status: "interrupted".to_string(),
        source: "local".to_string(),
    })
}

/// One-shot snapshot of a trace's causal set — the typed surface the
/// snapshot mode renders and integration tests assert (same
/// compute/render split as `discover::execute`).
#[derive(Debug, serde::Serialize)]
pub struct WatchSnapshot {
    pub trace_id: String,
    pub events: Vec<WatchEvent>,
    /// Present when every record is terminal.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminal: Option<WatchEvent>,
}

/// Fetch and project the trace once, without following.
pub fn execute_once(args: &WatchArgs) -> anyhow::Result<WatchSnapshot> {
    let snap = fetch_causal_set(args)?;
    let mut engine = WatchEngine::new(snap.trace_id);
    let events = engine.diff(snap.nodes.iter().map(RecordView::from));
    let terminal = WatchEngine::all_terminal(snap.nodes.iter().map(RecordView::from)).then(|| {
        WatchEvent::Terminal {
            trace: engine.trace_id().to_string(),
            status: WatchEngine::terminal_status(snap.nodes.iter().map(RecordView::from))
                .to_string(),
            usage: Some(WatchEngine::total_usage(
                snap.nodes.iter().map(RecordView::from),
            )),
        }
    });
    Ok(WatchSnapshot {
        trace_id: engine.trace_id().to_string(),
        events,
        terminal,
    })
}

pub fn run(args: WatchArgs) -> anyhow::Result<()> {
    let ndjson = matches!(args.format, OutputFormat::Json);
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

    let first = fetch_causal_set(&args)?;
    let mut engine = WatchEngine::new(first.trace_id.clone());
    emit(&engine.diff(first.nodes.iter().map(RecordView::from)))?;
    let mut nodes = first.nodes;

    loop {
        if WatchEngine::all_terminal(nodes.iter().map(RecordView::from)) {
            break;
        }
        if let Some(liveness) = mission_liveness(engine.trace_id()) {
            emit(std::slice::from_ref(&liveness))?;
            return Ok(());
        }
        std::thread::sleep(FOLLOW_INTERVAL);
        nodes = fetch_causal_set(&args)?.nodes;
        emit(&engine.diff(nodes.iter().map(RecordView::from)))?;
    }

    emit(&[WatchEvent::Terminal {
        trace: engine.trace_id().to_string(),
        status: WatchEngine::terminal_status(nodes.iter().map(RecordView::from)).to_string(),
        usage: Some(WatchEngine::total_usage(nodes.iter().map(RecordView::from))),
    }])
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
            output::success(&format!("trace {trace} → {status}{cost}"))
        }
        WatchEvent::Liveness { status, source } => {
            output::warn(&format!("liveness: {status} ({source})"))
        }
    }
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
            usage: None,
        };
        assert_eq!(
            serde_json::to_string(&t).unwrap(),
            r#"{"event":"terminal","trace":"t-1","status":"ok"}"#,
            "absent usage keeps the frozen W2 shape — adding, never renaming"
        );
        let t = WatchEvent::Terminal {
            trace: "t-1".into(),
            status: "ok".into(),
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
            "signed usage rides the terminal event"
        );
        let l = WatchEvent::Liveness {
            status: "interrupted".into(),
            source: "local".into(),
        };
        assert_eq!(
            serde_json::to_string(&l).unwrap(),
            r#"{"event":"liveness","status":"interrupted","source":"local"}"#
        );
    }
}
