// EasyNet CLI — Invocation Audit Group
// ====================================
//
// File: src/facade/cli/groups/invocation.rs
// Description: `easynet invocation ...` queries the local daemon's
//              device-owned invocation.history.* abilities through Axon's
//              Invocation gRPC surface. The daemon owns the native
//              ledger handle, so the CLI never races native storage
//              locks or reimplements ability dispatch.

use anyhow::Context;
use clap::{Args, Subcommand};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::runtime::agents::invocation_history_ability::{
    ABILITY_HISTORY_GET, ABILITY_HISTORY_LIST, ABILITY_HISTORY_PATH, ABILITY_TRACE_GET,
};
use crate::support::output::{self, OutputFormat};

type InvocationRecord = easynet_axon::invocation::InvocationLedgerRecord;
type TraceEdge = easynet_axon::invocation::InvocationTraceEdge;
type TraceGraph = easynet_axon::invocation::InvocationTraceGraph;

#[derive(Debug, Args)]
pub struct InvocationArgs {
    #[command(subcommand)]
    pub action: InvocationAction,
}

#[derive(Debug, Subcommand)]
pub enum InvocationAction {
    /// List recent invocation records from the local billing ledger.
    List(ListArgs),
    /// Show one invocation record by invocation URA or request id.
    Show(ShowArgs),
    /// Show all records sharing one trace id, invocation URA, or request id.
    Trace(TraceArgs),
    /// Print the native ledger database path.
    Path,
    /// Aggregate operational stats over recent records — state mix,
    /// latency percentiles, top abilities and error codes.
    Stats(StatsArgs),
}

#[derive(Debug, Args)]
pub struct StatsArgs {
    /// How many most-recent records to aggregate.
    #[arg(long, default_value_t = 500)]
    pub limit: usize,
    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,
}

#[derive(Debug, Args)]
pub struct ListArgs {
    /// Maximum records to print.
    #[arg(long, default_value_t = 50)]
    pub limit: usize,
    /// Filter by state, for example completed or failed.
    #[arg(long)]
    pub state: Option<String>,
    /// Filter by canonical Ability URA.
    #[arg(long = "ability-ura")]
    pub ability_ura: Option<String>,
    /// Filter by caller URA.
    #[arg(long = "caller-ura")]
    pub caller: Option<String>,
    /// Filter by callee URA.
    #[arg(long = "callee-ura")]
    pub callee: Option<String>,
    /// Filter by ability owner/callee Agent URA.
    #[arg(long = "agent-ura")]
    pub agent_ura: Option<String>,
    /// Filter by subject URA.
    #[arg(long = "subject-ura")]
    pub subject: Option<String>,
    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,
}

#[derive(Debug, Args)]
pub struct ShowArgs {
    /// Invocation URA or request id.
    pub id: String,
    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,
}

#[derive(Debug, Args)]
pub struct TraceArgs {
    /// Trace id, invocation URA, or request id.
    pub id: String,
    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,
}

pub fn run(args: InvocationArgs) -> anyhow::Result<()> {
    match args.action {
        InvocationAction::List(a) => run_list(a),
        InvocationAction::Show(a) => run_show(a),
        InvocationAction::Trace(a) => run_trace(a),
        InvocationAction::Stats(a) => run_stats(a),
        InvocationAction::Path => {
            let response: HistoryPathResponse =
                invoke_invocation_ability(ABILITY_HISTORY_PATH, json!({}))?;
            println!("{}", response.ledger_path);
            Ok(())
        }
    }
}

fn run_list(args: ListArgs) -> anyhow::Result<()> {
    let response = if args.limit == 0 {
        HistoryListResponse {
            ledger_path: None,
            records: Vec::new(),
        }
    } else {
        fetch_history_list(&args)?
    };
    let ledger_path = ledger_path_label(response.ledger_path);
    let records = response.records;

    if args.format == OutputFormat::Json {
        println!("{}", serde_json::to_string_pretty(&records)?);
        return Ok(());
    }

    if records.is_empty() {
        output::info(&format!(
            "No invocation records at {}. Run an ability through the daemon first.",
            ledger_path
        ));
        return Ok(());
    }

    let mut table = output::table(&[
        "Request", "State", "Ability", "Caller", "Callee", "Age", "MS",
    ]);
    for record in &records {
        let elapsed = record
            .elapsed_ms
            .map(|ms| ms.to_string())
            .unwrap_or_else(|| "-".to_string());
        let caller = short_ura(&record.caller_ura);
        let callee = short_ura(&record.callee_ura);
        let age = output::relative_time(record.started_unix_ms);
        table.add_row(vec![
            record.request_id.clone(),
            record.state.clone(),
            public_ability_label(record),
            caller,
            callee,
            age,
            elapsed,
        ]);
    }
    println!("{table}");
    Ok(())
}

fn run_show(args: ShowArgs) -> anyhow::Result<()> {
    let response = fetch_history_record(&args.id)?;
    let ledger_path = ledger_path_label(response.ledger_path);
    let record = response.record.ok_or_else(|| {
        anyhow::anyhow!(
            "invocation record not found for `{}` in {}",
            args.id,
            ledger_path
        )
    })?;

    if args.format == OutputFormat::Json {
        println!("{}", serde_json::to_string_pretty(&record)?);
        return Ok(());
    }

    let elapsed = record
        .elapsed_ms
        .map(|ms| ms.to_string())
        .unwrap_or_else(|| "-".to_string());
    let completed = record
        .completed_unix_ms
        .map(|ms| ms.to_string())
        .unwrap_or_else(|| "-".to_string());
    let started = record.started_unix_ms.to_string();
    let ability = public_ability_label(&record);
    output::kv_section_stdout(&[
        ("invocation_ura", record.invocation_ura.as_str()),
        ("request_id", record.request_id.as_str()),
        ("trace_id", record.trace_id.as_str()),
        ("span_id", record.span_id.as_str()),
        ("state", record.state.as_str()),
        ("ability", ability.as_str()),
        ("ability_ura", record.ability_ura.as_str()),
        ("caller", record.caller_ura.as_str()),
        ("callee", record.callee_ura.as_str()),
        ("subject", record.subject_ura.as_str()),
        ("started", &started),
        ("completed", &completed),
        ("elapsed_ms", &elapsed),
    ]);
    if let Some(error) = record.error.as_ref() {
        eprintln!();
        output::kv_section_stdout(&[
            ("error_source", error.source.as_str()),
            ("error_code", error.code.as_str()),
            ("error_message", error.message.as_str()),
        ]);
    }
    Ok(())
}

fn run_trace(args: TraceArgs) -> anyhow::Result<()> {
    let trace_id = match fetch_history_record(&args.id)?.record {
        Some(record) if !record.trace_id.is_empty() => record.trace_id,
        Some(record) => {
            if args.format == OutputFormat::Json {
                let graph = TraceGraph {
                    records: vec![record],
                    ..Default::default()
                };
                println!("{}", serde_json::to_string_pretty(&graph)?);
            } else {
                output::info("Record has no trace_id; showing the single invocation node.");
                print_trace_table(&[record]);
            }
            return Ok(());
        }
        None => args.id.clone(),
    };

    let response = fetch_trace_graph_by_trace_id(&trace_id)?;
    let ledger_path = ledger_path_label(response.ledger_path.clone());
    let graph = response.into_graph();

    if args.format == OutputFormat::Json {
        println!("{}", serde_json::to_string_pretty(&graph)?);
        return Ok(());
    }

    if graph.records.is_empty() {
        output::info(&format!(
            "No invocation records with trace_id `{trace_id}` in {}.",
            ledger_path
        ));
        return Ok(());
    }
    print_trace_table(&graph.records);
    if !graph.edges.is_empty() {
        println!();
        print_trace_edges(&graph.edges);
    }
    Ok(())
}

fn print_trace_table(records: &[InvocationRecord]) {
    let mut table = output::table(&["Request", "Span", "State", "Ability", "Started", "MS"]);
    for record in records {
        let elapsed = record
            .elapsed_ms
            .map(|ms| ms.to_string())
            .unwrap_or_else(|| "-".to_string());
        let started = output::relative_time(record.started_unix_ms);
        table.add_row(vec![
            record.request_id.clone(),
            record.span_id.clone(),
            record.state.clone(),
            public_ability_label(record),
            started,
            elapsed,
        ]);
    }
    println!("{table}");
}

fn print_trace_edges(edges: &[TraceEdge]) {
    let mut table = output::table(&["From Receipt", "Relation", "To Invocation"]);
    for edge in edges {
        table.add_row(vec![
            edge.from_receipt_ura.clone(),
            edge.relation.clone(),
            edge.to_invocation_ura.clone(),
        ]);
    }
    println!("{table}");
}

#[derive(Debug, Deserialize)]
struct HistoryListResponse {
    ledger_path: Option<String>,
    #[serde(default)]
    records: Vec<InvocationRecord>,
}

#[derive(Debug, Deserialize)]
struct HistoryGetResponse {
    ledger_path: Option<String>,
    record: Option<InvocationRecord>,
}

#[derive(Debug, Deserialize)]
struct TraceGetResponse {
    ledger_path: Option<String>,
    trace_id: String,
    #[serde(default)]
    nodes: Vec<InvocationRecord>,
    #[serde(default)]
    edges: Vec<TraceEdge>,
}

impl TraceGetResponse {
    fn into_graph(self) -> TraceGraph {
        TraceGraph {
            trace_id: self.trace_id,
            records: self.nodes,
            edges: self.edges,
        }
    }
}

#[derive(Debug, Deserialize)]
struct HistoryPathResponse {
    ledger_path: String,
}

fn ledger_path_label(path: Option<String>) -> String {
    path.unwrap_or_else(|| "daemon ledger path unavailable".to_string())
}

/// `easynet invocation stats` — the D7 operator view: one screen that
/// answers "is invoke healthy right now" from the daemon's own ledger,
/// no log spelunking. Pure projection over invocation.history.list;
/// the daemon stays the only ledger reader.
fn run_stats(args: StatsArgs) -> anyhow::Result<()> {
    let response: HistoryListResponse =
        invoke_invocation_ability(ABILITY_HISTORY_LIST, json!({ "limit": args.limit }))?;
    let summary = summarize(&response.records);

    if args.format == OutputFormat::Json {
        println!("{}", serde_json::to_string_pretty(&summary)?);
        return Ok(());
    }
    if summary.total == 0 {
        output::info("No invocation records yet. Run an ability through the daemon first.");
        return Ok(());
    }

    println!("records analysed: {}", summary.total);
    println!("\nstates:");
    for (state, count) in &summary.states {
        println!(
            "  {:<12} {:>6}  ({:>5.1}%)",
            state,
            count,
            *count as f64 * 100.0 / summary.total as f64
        );
    }
    if let Some(lat) = &summary.latency_ms {
        println!(
            "\nlatency (completed, ms):  p50={}  p95={}  p99={}  max={}",
            lat.p50, lat.p95, lat.p99, lat.max
        );
    }
    println!("\ntop abilities:");
    for row in &summary.top_abilities {
        println!(
            "  {:<40} {:>6} calls  {:>4} failed",
            row.ability, row.calls, row.failed
        );
    }
    if !summary.top_errors.is_empty() {
        println!("\ntop error codes:");
        for (code, count) in &summary.top_errors {
            println!("  {:<32} {:>6}", code, count);
        }
    }
    Ok(())
}

#[derive(Debug, serde::Serialize)]
struct StatsSummary {
    total: usize,
    /// (state, count), descending by count.
    states: Vec<(String, usize)>,
    latency_ms: Option<LatencySummary>,
    top_abilities: Vec<AbilityStat>,
    /// (error code, count), descending.
    top_errors: Vec<(String, usize)>,
}

#[derive(Debug, serde::Serialize)]
struct LatencySummary {
    p50: u64,
    p95: u64,
    p99: u64,
    max: u64,
}

#[derive(Debug, serde::Serialize)]
struct AbilityStat {
    ability: String,
    calls: usize,
    failed: usize,
}

fn summarize(records: &[InvocationRecord]) -> StatsSummary {
    use std::collections::BTreeMap;

    let mut states: BTreeMap<String, usize> = BTreeMap::new();
    let mut abilities: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut errors: BTreeMap<String, usize> = BTreeMap::new();
    let mut latencies: Vec<u64> = Vec::new();

    for r in records {
        *states.entry(r.state.clone()).or_default() += 1;
        let failed = r.error.is_some();
        let slot = abilities.entry(r.ability_name.clone()).or_default();
        slot.0 += 1;
        if failed {
            slot.1 += 1;
        }
        if let Some(err) = &r.error {
            *errors.entry(err.code.clone()).or_default() += 1;
        }
        if let Some(ms) = r.elapsed_ms {
            if !failed {
                latencies.push(ms);
            }
        }
    }

    let mut states: Vec<_> = states.into_iter().collect();
    states.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

    let mut top_abilities: Vec<AbilityStat> = abilities
        .into_iter()
        .map(|(ability, (calls, failed))| AbilityStat {
            ability,
            calls,
            failed,
        })
        .collect();
    top_abilities.sort_by(|a, b| b.calls.cmp(&a.calls).then(a.ability.cmp(&b.ability)));
    top_abilities.truncate(5);

    let mut top_errors: Vec<_> = errors.into_iter().collect();
    top_errors.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    top_errors.truncate(3);

    latencies.sort_unstable();
    let latency_ms = (!latencies.is_empty()).then(|| LatencySummary {
        p50: percentile(&latencies, 50),
        p95: percentile(&latencies, 95),
        p99: percentile(&latencies, 99),
        max: *latencies.last().expect("non-empty"),
    });

    StatsSummary {
        total: records.len(),
        states,
        latency_ms,
        top_abilities,
        top_errors,
    }
}

/// Nearest-rank percentile over a sorted slice. p in [1, 100].
fn percentile(sorted: &[u64], p: u64) -> u64 {
    debug_assert!(!sorted.is_empty());
    let rank = (p as usize * sorted.len()).div_ceil(100);
    sorted[rank.saturating_sub(1).min(sorted.len() - 1)]
}

fn fetch_history_list(args: &ListArgs) -> anyhow::Result<HistoryListResponse> {
    invoke_invocation_ability(ABILITY_HISTORY_LIST, history_list_args(args))
}

fn fetch_history_record(id: &str) -> anyhow::Result<HistoryGetResponse> {
    invoke_invocation_ability(
        ABILITY_HISTORY_GET,
        json!({ "key": history_key_for_id(id) }),
    )
}

fn fetch_trace_graph_by_trace_id(trace_id: &str) -> anyhow::Result<TraceGetResponse> {
    invoke_invocation_ability(
        ABILITY_TRACE_GET,
        json!({ "key": { "trace_id": trace_id } }),
    )
}

fn invoke_invocation_ability<T>(ability: &str, args: Value) -> anyhow::Result<T>
where
    T: DeserializeOwned,
{
    // Route through the canonical `local_invoke` surface so the
    // "one CLI subcommand = one ability invoke" rule
    // (`src/support/local_invoke.rs` doc) stays held — i.e. CLI
    // surfaces never bypass into the transport plumbing directly.
    let value = crate::support::local_invoke::invoke_local_ability(ability, args)
        .with_context(|| format!("invoke {ability} through local Axon daemon"))?;
    serde_json::from_value(value).with_context(|| format!("decode {ability} response"))
}

fn history_list_args(args: &ListArgs) -> Value {
    let mut filter = Map::new();
    insert_filter_value(&mut filter, "state", args.state.as_deref());
    insert_filter_value(&mut filter, "ability_ura", args.ability_ura.as_deref());
    insert_filter_value(&mut filter, "caller_ura", args.caller.as_deref());
    insert_filter_value(&mut filter, "callee_ura", args.callee.as_deref());
    insert_filter_value(&mut filter, "agent_ura", args.agent_ura.as_deref());
    insert_filter_value(&mut filter, "subject_ura", args.subject.as_deref());

    let mut body = Map::new();
    body.insert("limit".to_string(), json!(args.limit));
    if !filter.is_empty() {
        body.insert("filter".to_string(), Value::Object(filter));
    }
    Value::Object(body)
}

fn insert_filter_value(filter: &mut Map<String, Value>, key: &str, value: Option<&str>) {
    if let Some(value) = value.map(str::trim).filter(|v| !v.is_empty()) {
        filter.insert(key.to_string(), json!(value));
    }
}

fn history_key_for_id(id: &str) -> Value {
    if id.starts_with("easynet:///") {
        json!({ "ura": id })
    } else {
        json!({ "request_id": id })
    }
}

fn public_ability_label(record: &InvocationRecord) -> String {
    crate::ura::public_ability_name_from_ability_ura(&record.callee_ura, &record.ability_ura)
        .unwrap_or_else(|| record.ability_ura.clone())
}

fn short_ura(ura: &str) -> String {
    crate::ura::parse_ura(ura)
        .ok()
        .and_then(|parsed| match parsed.kind {
            crate::ura::URAKind::User => parsed.user_id().map(|user_id| format!("user/{user_id}")),
            crate::ura::URAKind::Device => parsed
                .device_id()
                .map(|device_id| format!("device/{device_id}")),
            crate::ura::URAKind::Agent => parsed
                .agent_ids()
                .map(|(user_id, agent_id)| format!("agent/{user_id}.{agent_id}")),
            crate::ura::URAKind::Hub => Some(format!("hub/{}", parsed.realm)),
            _ => None,
        })
        .unwrap_or_else(|| ura.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_list_args_emits_explicit_ura_scope_fields() {
        let body = history_list_args(&ListArgs {
            limit: 25,
            state: Some("completed".into()),
            ability_ura: Some("easynet:///r/test/ability/device.callee.fs.read".into()),
            caller: Some("easynet:///r/test/device/caller".into()),
            callee: None,
            agent_ura: Some("easynet:///r/test/device/callee".into()),
            subject: Some("easynet:///r/test/user/alice".into()),
            format: OutputFormat::Json,
        });

        assert_eq!(body["limit"], 25);
        assert_eq!(
            body["filter"]["ability_ura"],
            "easynet:///r/test/ability/device.callee.fs.read"
        );
        assert_eq!(
            body["filter"]["caller_ura"],
            "easynet:///r/test/device/caller"
        );
        assert_eq!(
            body["filter"]["agent_ura"],
            "easynet:///r/test/device/callee"
        );
        assert_eq!(
            body["filter"]["subject_ura"],
            "easynet:///r/test/user/alice"
        );
        assert!(body["filter"].get("subject").is_none());
    }

    // ── `invocation stats` aggregation (F-051) ──────────────────

    fn stats_record(
        ability: &str,
        state: &str,
        error_code: Option<&str>,
        elapsed_ms: Option<u64>,
    ) -> InvocationRecord {
        let mut b = easynet_axon::invocation::InvocationLedgerRecordBuilder::new()
            .invocation_ura("easynet:///r/test/resource/alice.invocations/i-1")
            .request_id("req-1")
            .caller_ura("easynet:///r/test/device/caller")
            .callee_ura("easynet:///r/test/device/callee")
            .subject_ura("easynet:///r/test/device/callee")
            .ability_ura("easynet:///r/test/ability/device.callee.fs.read")
            .ability_name(ability)
            .state(state)
            .started_unix_ms(1_700_000_000_000_i64)
            .args(easynet_axon::invocation::LedgerEventPayload::Digest {
                content_type: "application/json".to_string(),
                sha256: "0".repeat(64),
                size_bytes: 2,
            });
        if let Some(code) = error_code {
            b = b.error(easynet_axon::invocation::LedgerErrorRecord {
                source: "test".to_string(),
                code: code.to_string(),
                message: "boom".to_string(),
                retryable: false,
                context: Default::default(),
            });
        }
        if let Some(ms) = elapsed_ms {
            b = b.elapsed_ms(ms);
        }
        b.build().expect("stats test record")
    }

    #[test]
    fn summarize_empty_ledger_is_all_zero() {
        let s = summarize(&[]);
        assert_eq!(s.total, 0);
        assert!(s.states.is_empty());
        assert!(s.latency_ms.is_none());
        assert!(s.top_abilities.is_empty());
        assert!(s.top_errors.is_empty());
    }

    #[test]
    fn summarize_groups_and_orders_deterministically() {
        // Counts descend; ties break by name so output is stable run
        // to run (the explainability promise in the module doc).
        let records = vec![
            stats_record("b.read", "completed", None, Some(10)),
            stats_record("b.read", "completed", None, Some(20)),
            stats_record("a.write", "failed", Some("E_IO"), None),
            stats_record("c.list", "completed", None, Some(30)),
        ];
        let s = summarize(&records);
        assert_eq!(s.total, 4);
        assert_eq!(
            s.states,
            vec![("completed".to_string(), 3), ("failed".to_string(), 1)]
        );
        let names: Vec<(&str, usize, usize)> = s
            .top_abilities
            .iter()
            .map(|a| (a.ability.as_str(), a.calls, a.failed))
            .collect();
        assert_eq!(
            names,
            vec![("b.read", 2, 0), ("a.write", 1, 1), ("c.list", 1, 0)]
        );
        assert_eq!(s.top_errors, vec![("E_IO".to_string(), 1)]);
    }

    #[test]
    fn summarize_latency_excludes_failures_and_uses_nearest_rank() {
        // A failed call's elapsed time is recovery noise, not service
        // latency — it must not pollute the percentiles.
        let mut records: Vec<InvocationRecord> = (1..=100)
            .map(|i| stats_record("x.call", "completed", None, Some(i)))
            .collect();
        records.push(stats_record(
            "x.call",
            "failed",
            Some("E_TIMEOUT"),
            Some(9_999),
        ));
        let s = summarize(&records);
        let lat = s.latency_ms.expect("latencies present");
        assert_eq!(lat.p50, 50);
        assert_eq!(lat.p95, 95);
        assert_eq!(lat.p99, 99);
        assert_eq!(lat.max, 100, "failed 9999ms must be excluded");
    }
}
