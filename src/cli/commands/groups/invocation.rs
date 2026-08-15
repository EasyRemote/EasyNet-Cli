// EasyNet CLI — Invocation Audit Group
// ====================================
//
// File: src/cli/groups/invocation.rs
// Description: `easynet invocation ...` queries the local daemon's
//              governance-owned invocation.history.* abilities through the
//              runtime-state read issuer. The daemon owns the native ledger
//              handle, so the CLI never races native storage locks or
//              reimplements ability dispatch.

use anyhow::Context;
use clap::{Args, Subcommand};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::cli::commands::receipt_verification::CliReceiptChainVerification;
use crate::support::platform::local_invoke::LocalRuntimeGovernanceReadIssuer;
use crate::support::platform::output::{self, OutputFormat};

type InvocationRecord = axon_sdk::invocation::InvocationLedgerRecord;
type TraceEdge = axon_sdk::invocation::InvocationTraceEdge;
type TraceGraph = axon_sdk::invocation::InvocationTraceGraph;

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
    /// Watch an Invocation causal set live — by invocation URA, or a
    /// whole run via its trace anchor (seven-axes T2.4).
    Watch(crate::cli::commands::invocation_watch::WatchArgs),
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
                invoke_invocation_history_read(InvocationHistoryRead::Path)?;
            println!("{}", response.ledger_path);
            Ok(())
        }
        InvocationAction::Watch(a) => crate::cli::commands::invocation_watch::run(a),
    }
}

fn run_list(args: ListArgs) -> anyhow::Result<()> {
    let (ledger_source, records) = if args.limit == 0 {
        (
            "daemon ledger query skipped because --limit 0".to_string(),
            Vec::new(),
        )
    } else {
        let response = fetch_history_list(&args)?;
        let _next_cursor = response.next_cursor;
        (
            ledger_source_label(&response.ledger_ura, response.ledger_path.as_deref()),
            response.records,
        )
    };

    if args.format == OutputFormat::Json {
        println!("{}", serde_json::to_string_pretty(&records)?);
        return Ok(());
    }

    if records.is_empty() {
        output::info(&format!(
            "No invocation records at {}. Run an ability through the daemon first.",
            ledger_source
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
            public_ability_label(&record.callee_ura, &record.ability_ura),
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
    let ledger_source = ledger_source_label(&response.ledger_ura, response.ledger_path.as_deref());
    let record = response.record.ok_or_else(|| {
        anyhow::anyhow!(
            "invocation record not found for `{}` in {}",
            args.id,
            ledger_source
        )
    })?;

    if args.format == OutputFormat::Json {
        println!(
            "{}",
            serde_json::to_string_pretty(&show_record_json(&record)?)?
        );
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
    let tokens_in = record.usage.tokens_in.to_string();
    let tokens_out = record.usage.tokens_out.to_string();
    let duration_ms = record.usage.duration_ms.to_string();
    let external_calls = record.usage.external_calls.to_string();
    let ledger_reported_receipt_chain_verified =
        ledger_reported_receipt_chain_verified(&record).to_string();
    let cli_receipt_chain_verification = cli_receipt_chain_verification().to_string();
    let ability = public_ability_label(&record.callee_ura, &record.ability_ura);
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
        ("usage_tokens_in", &tokens_in),
        ("usage_tokens_out", &tokens_out),
        ("usage_duration_ms", &duration_ms),
        ("usage_external_calls", &external_calls),
        (
            "ledger_reported_receipt_chain_verified",
            &ledger_reported_receipt_chain_verified,
        ),
        (
            "cli_receipt_chain_verification",
            &cli_receipt_chain_verification,
        ),
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

fn show_record_json(record: &InvocationRecord) -> anyhow::Result<Value> {
    let mut value = serde_json::to_value(record).context("serialize invocation record")?;
    let object = value
        .as_object_mut()
        .context("serialized invocation record must be a JSON object")?;
    object.insert(
        "ledger_reported_receipt_chain_verified".to_string(),
        Value::Bool(ledger_reported_receipt_chain_verified(record)),
    );
    object.insert(
        "cli_receipt_chain_verification".to_string(),
        serde_json::to_value(cli_receipt_chain_verification())?,
    );
    Ok(value)
}

fn ledger_reported_receipt_chain_verified(record: &InvocationRecord) -> bool {
    record.receipt_chain.verified
}

fn cli_receipt_chain_verification() -> CliReceiptChainVerification {
    CliReceiptChainVerification::not_performed()
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
    let ledger_source = ledger_source_label(&response.ledger_ura, response.ledger_path.as_deref());
    let graph = response.into_graph();

    if args.format == OutputFormat::Json {
        println!("{}", serde_json::to_string_pretty(&graph)?);
        return Ok(());
    }

    if graph.records.is_empty() {
        output::info(&format!(
            "No invocation records with trace_id `{trace_id}` in {}.",
            ledger_source
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
            public_ability_label(&record.callee_ura, &record.ability_ura),
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
#[serde(deny_unknown_fields)]
struct HistoryListResponse {
    ledger_ura: String,
    ledger_path: Option<String>,
    next_cursor: Option<String>,
    #[serde(default)]
    records: Vec<InvocationHistorySummary>,
}

/// Bounded navigation DTO returned by `invocation.history.list`.
///
/// The list surface intentionally excludes ledger payloads, diagnostics,
/// causal links, visibility, and receipt chains. Those proof-bearing fields
/// belong to `invocation.history.get`; decoding a list row as the complete
/// ledger record couples the CLI to data the provider is forbidden to expose
/// on this bounded read model.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct InvocationHistorySummary {
    invocation_ura: String,
    request_id: String,
    trace_id: String,
    span_id: String,
    caller_ura: String,
    callee_ura: String,
    subject_ura: String,
    ability_ura: String,
    ability_name: String,
    state: String,
    started_unix_ms: i64,
    completed_unix_ms: Option<i64>,
    elapsed_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<InvocationHistoryErrorSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct InvocationHistoryErrorSummary {
    source: String,
    code: String,
    message: String,
    retryable: bool,
    truncated: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HistoryGetResponse {
    ledger_ura: String,
    ledger_path: Option<String>,
    record: Option<InvocationRecord>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TraceGetResponse {
    ledger_ura: String,
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
#[serde(deny_unknown_fields)]
struct HistoryPathResponse {
    ledger_ura: String,
    ledger_path: String,
}

trait InvocationHistoryResponse {
    fn validate_response(&self, operation: &str) -> anyhow::Result<()>;
}

impl InvocationHistoryResponse for HistoryListResponse {
    fn validate_response(&self, operation: &str) -> anyhow::Result<()> {
        validate_history_ledger_ura(&self.ledger_ura, operation)
    }
}

impl InvocationHistoryResponse for HistoryGetResponse {
    fn validate_response(&self, operation: &str) -> anyhow::Result<()> {
        validate_history_ledger_ura(&self.ledger_ura, operation)
    }
}

impl InvocationHistoryResponse for TraceGetResponse {
    fn validate_response(&self, operation: &str) -> anyhow::Result<()> {
        validate_history_ledger_ura(&self.ledger_ura, operation)
    }
}

impl InvocationHistoryResponse for HistoryPathResponse {
    fn validate_response(&self, operation: &str) -> anyhow::Result<()> {
        validate_history_ledger_ura(&self.ledger_ura, operation)
    }
}

fn validate_history_ledger_ura(ledger_ura: &str, operation: &str) -> anyhow::Result<()> {
    let trimmed = ledger_ura.trim();
    if trimmed.is_empty() {
        anyhow::bail!("{operation} response ledger_ura must not be empty");
    }
    let parsed = crate::core::ura::parse_ura(trimmed)
        .with_context(|| format!("{operation} response ledger_ura must be a canonical URA"))?;
    if parsed.kind != crate::core::ura::URAKind::Resource {
        anyhow::bail!(
            "{operation} response ledger_ura must be a Resource URA, got kind={:?}",
            parsed.kind
        );
    }
    Ok(())
}

fn ledger_source_label(ledger_ura: &str, ledger_path: Option<&str>) -> String {
    match ledger_path {
        Some(path) if !path.trim().is_empty() => {
            format!("{} ({})", path.trim(), ledger_ura.trim())
        }
        _ => ledger_ura.trim().to_string(),
    }
}

/// `easynet invocation stats` — the D7 operator view: one screen that
/// answers "is invoke healthy right now" from the daemon's own ledger,
/// no log spelunking. Pure projection over invocation.history.list;
/// the daemon stays the only ledger reader.
fn run_stats(args: StatsArgs) -> anyhow::Result<()> {
    let response: HistoryListResponse = invoke_invocation_history_read(
        InvocationHistoryRead::List(InvocationHistoryListQuery::for_stats(args.limit)),
    )?;
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

fn summarize(records: &[InvocationHistorySummary]) -> StatsSummary {
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
    invoke_invocation_history_read(InvocationHistoryRead::List(
        InvocationHistoryListQuery::from_list_args(args)?,
    ))
}

fn fetch_history_record(id: &str) -> anyhow::Result<HistoryGetResponse> {
    invoke_invocation_history_read(InvocationHistoryRead::Get(
        InvocationHistoryKey::for_record_lookup(id),
    ))
}

fn fetch_trace_graph_by_trace_id(trace_id: &str) -> anyhow::Result<TraceGetResponse> {
    invoke_invocation_history_read(InvocationHistoryRead::Trace(InvocationHistoryKey::TraceId(
        trace_id.to_string(),
    )))
}

fn invoke_invocation_history_read<T>(read: InvocationHistoryRead) -> anyhow::Result<T>
where
    T: DeserializeOwned + InvocationHistoryResponse,
{
    let operation = read.operation_label();
    // Route through the named runtime-state read issuer so the
    // "one CLI subcommand = one ability invoke" rule stays held while
    // the subject is selected explicitly before LocalRuntime admission.
    let value = read
        .invoke()
        .with_context(|| format!("invoke {operation} through local Axon daemon"))?;
    let response: T =
        serde_json::from_value(value).with_context(|| format!("decode {operation} response"))?;
    response.validate_response(operation)?;
    Ok(response)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum InvocationHistoryRead {
    Path,
    List(InvocationHistoryListQuery),
    Get(InvocationHistoryKey),
    Trace(InvocationHistoryKey),
}

impl InvocationHistoryRead {
    fn operation_label(&self) -> &'static str {
        match self {
            Self::Path => "invocation history path read",
            Self::List(_) => "invocation history list read",
            Self::Get(_) => "invocation history get read",
            Self::Trace(_) => "invocation trace get read",
        }
    }

    fn invoke(self) -> anyhow::Result<Value> {
        match self {
            Self::Path => {
                LocalRuntimeGovernanceReadIssuer::invocation_history_path(Value::Object(Map::new()))
            }
            Self::List(query) => {
                LocalRuntimeGovernanceReadIssuer::invocation_history_list(query.into_args())
            }
            Self::Get(key) => LocalRuntimeGovernanceReadIssuer::invocation_history_get(
                json!({ "key": key.into_args() }),
            ),
            Self::Trace(key) => LocalRuntimeGovernanceReadIssuer::invocation_trace_get(
                json!({ "key": key.into_args() }),
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InvocationHistoryListQuery {
    limit: usize,
    filter: InvocationHistoryFilter,
}

impl InvocationHistoryListQuery {
    fn for_stats(limit: usize) -> Self {
        Self {
            limit,
            filter: InvocationHistoryFilter::default(),
        }
    }

    fn from_list_args(args: &ListArgs) -> anyhow::Result<Self> {
        Ok(Self {
            limit: args.limit,
            filter: InvocationHistoryFilter {
                state: args.state.clone(),
                ability_ura: canonical_history_ability_filter(args.ability_ura.as_deref())?,
                caller_ura: canonical_history_ura_filter("--caller-ura", args.caller.as_deref())?,
                callee_ura: canonical_history_callee_filter(
                    args.callee.as_deref(),
                    args.agent_ura.as_deref(),
                )?,
                subject_ura: canonical_history_ura_filter(
                    "--subject-ura",
                    args.subject.as_deref(),
                )?,
            },
        })
    }

    fn into_args(self) -> Value {
        let mut body = Map::new();
        body.insert("limit".to_string(), json!(self.limit));
        if let Some(filter) = self.filter.into_args() {
            body.insert("filter".to_string(), filter);
        }
        Value::Object(body)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct InvocationHistoryFilter {
    state: Option<String>,
    ability_ura: Option<String>,
    caller_ura: Option<String>,
    callee_ura: Option<String>,
    subject_ura: Option<String>,
}

impl InvocationHistoryFilter {
    fn into_args(self) -> Option<Value> {
        let mut filter = Map::new();
        Self::insert_arg_value(&mut filter, "state", self.state);
        Self::insert_arg_value(&mut filter, "ability_ura", self.ability_ura);
        Self::insert_arg_value(&mut filter, "caller_ura", self.caller_ura);
        Self::insert_arg_value(&mut filter, "callee_ura", self.callee_ura);
        Self::insert_arg_value(&mut filter, "subject_ura", self.subject_ura);
        (!filter.is_empty()).then_some(Value::Object(filter))
    }

    fn insert_arg_value(filter: &mut Map<String, Value>, key: &str, value: Option<String>) {
        if let Some(value) = value.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
            filter.insert(key.to_string(), json!(value));
        }
    }
}

fn canonical_history_callee_filter(
    callee_ura: Option<&str>,
    agent_ura: Option<&str>,
) -> anyhow::Result<Option<String>> {
    let callee = canonical_history_ura_filter("--callee-ura", callee_ura)?;
    let agent = canonical_history_ura_filter("--agent-ura", agent_ura)?;
    match (callee.as_deref(), agent.as_deref()) {
        (Some(callee), Some(agent)) if callee != agent => anyhow::bail!(
            "`--agent-ura` is a CLI facade for `--callee-ura`; both values must match when supplied"
        ),
        (Some(value), _) | (_, Some(value)) => Ok(Some(value.to_string())),
        (None, None) => Ok(None),
    }
}

fn canonical_history_ability_filter(value: Option<&str>) -> anyhow::Result<Option<String>> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let selector = crate::core::ura::AbilitySelector::parse(value)
        .with_context(|| "`--ability-ura` must be a canonical Ability URA".to_string())?;
    Ok(Some(selector.ability_ura().to_string()))
}

fn canonical_history_ura_filter(flag: &str, value: Option<&str>) -> anyhow::Result<Option<String>> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    crate::core::ura::parse_ura(value)
        .with_context(|| format!("`{flag}` must be a canonical URA"))?;
    Ok(Some(value.to_string()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum InvocationHistoryKey {
    InvocationUra(String),
    RequestId(String),
    TraceId(String),
}

impl InvocationHistoryKey {
    fn for_record_lookup(id: &str) -> Self {
        if id.starts_with("easynet:///") {
            Self::InvocationUra(id.to_string())
        } else {
            Self::RequestId(id.to_string())
        }
    }

    fn into_args(self) -> Value {
        match self {
            Self::InvocationUra(ura) => json!({ "ura": ura }),
            Self::RequestId(request_id) => json!({ "request_id": request_id }),
            Self::TraceId(trace_id) => json!({ "trace_id": trace_id }),
        }
    }
}

fn public_ability_label(callee_ura: &str, ability_ura: &str) -> String {
    crate::core::ura::public_ability_name_from_ability_ura(callee_ura, ability_ura)
        .unwrap_or_else(|| ability_ura.to_string())
}

fn short_ura(ura: &str) -> String {
    crate::core::ura::parse_ura(ura)
        .ok()
        .and_then(|parsed| match parsed.kind {
            crate::core::ura::URAKind::User => {
                parsed.user_id().map(|user_id| format!("user/{user_id}"))
            }
            crate::core::ura::URAKind::Device => parsed
                .device_id()
                .map(|device_id| format!("device/{device_id}")),
            crate::core::ura::URAKind::Agent => parsed
                .agent_ids()
                .map(|(user_id, agent_id)| format!("agent/{user_id}.{agent_id}")),
            crate::core::ura::URAKind::Authority => Some(format!("hub/{}", parsed.realm)),
            _ => None,
        })
        .unwrap_or_else(|| ura.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_test_args(read: InvocationHistoryRead) -> Value {
        match read {
            InvocationHistoryRead::Path => Value::Object(Map::new()),
            InvocationHistoryRead::List(query) => query.into_args(),
            InvocationHistoryRead::Get(key) | InvocationHistoryRead::Trace(key) => {
                json!({ "key": key.into_args() })
            }
        }
    }

    #[test]
    fn invocation_history_read_list_emits_explicit_ura_scope_fields() {
        let body = read_test_args(InvocationHistoryRead::List(
            InvocationHistoryListQuery::from_list_args(&ListArgs {
                limit: 25,
                state: Some("completed".into()),
                ability_ura: Some(
                    "easynet:///r/test/ability/system-agent.callee.locomotion.fs.read".into(),
                ),
                caller: Some("easynet:///r/test/device/caller".into()),
                callee: None,
                agent_ura: Some("easynet:///r/test/agent/device.callee.locomotion".into()),
                subject: Some("easynet:///r/test/user/alice".into()),
                format: OutputFormat::Json,
            })
            .unwrap(),
        ));

        assert_eq!(body["limit"], 25);
        assert_eq!(
            body["filter"]["ability_ura"],
            "easynet:///r/test/ability/system-agent.callee.locomotion.fs.read"
        );
        assert_eq!(
            body["filter"]["caller_ura"],
            "easynet:///r/test/device/caller"
        );
        assert_eq!(
            body["filter"]["callee_ura"],
            "easynet:///r/test/agent/device.callee.locomotion"
        );
        assert_eq!(
            body["filter"]["subject_ura"],
            "easynet:///r/test/user/alice"
        );
        assert!(body["filter"].get("agent_ura").is_none());
        assert!(body["filter"].get("subject").is_none());
    }

    #[test]
    fn invocation_history_read_list_omits_blank_filter_values() {
        let body = read_test_args(InvocationHistoryRead::List(
            InvocationHistoryListQuery::from_list_args(&ListArgs {
                limit: 25,
                state: Some(" ".into()),
                ability_ura: None,
                caller: None,
                callee: None,
                agent_ura: None,
                subject: None,
                format: OutputFormat::Json,
            })
            .unwrap(),
        ));

        assert_eq!(body["limit"], 25);
        assert!(body.get("filter").is_none());
    }

    #[test]
    fn invocation_history_responses_reject_unknown_envelope_fields() {
        let paged_list = serde_json::from_value::<HistoryListResponse>(json!({
            "ledger_ura": "easynet:///r/test/resource/device.callee/billing/invocations",
            "ledger_path": "/tmp/ledger",
            "next_cursor": "receipt-history:v1:abc",
            "records": []
        }))
        .expect("history list response must admit canonical pagination cursor");
        assert_eq!(
            paged_list.next_cursor.as_deref(),
            Some("receipt-history:v1:abc")
        );

        let list = serde_json::from_value::<HistoryListResponse>(json!({
            "ledger_ura": "easynet:///r/test/resource/device.callee/billing/invocations",
            "ledger_path": "/tmp/ledger",
            "records": [],
            "state_code": "J200"
        }))
        .expect_err("history list response must reject read-model drift");
        assert!(
            list.to_string().contains("state_code"),
            "schema error should name the noncanonical field: {list}"
        );

        let get = serde_json::from_value::<HistoryGetResponse>(json!({
            "ledger_ura": "easynet:///r/test/resource/device.callee/billing/invocations",
            "ledger_path": "/tmp/ledger",
            "record": null,
            "legacy_subject": "subject"
        }))
        .expect_err("history get response must reject retired aliases");
        assert!(
            get.to_string().contains("legacy_subject"),
            "schema error should name the noncanonical field: {get}"
        );

        let trace = serde_json::from_value::<TraceGetResponse>(json!({
            "ledger_ura": "easynet:///r/test/resource/device.callee/billing/invocations",
            "ledger_path": "/tmp/ledger",
            "trace_id": "trace-1",
            "nodes": [],
            "edges": [],
            "cursor": "legacy"
        }))
        .expect_err("trace response must reject uncontracted fields");
        assert!(
            trace.to_string().contains("cursor"),
            "schema error should name the noncanonical field: {trace}"
        );

        let path = serde_json::from_value::<HistoryPathResponse>(json!({
            "ledger_ura": "easynet:///r/test/resource/device.callee/billing/invocations",
            "ledger_path": "/tmp/ledger",
            "state_code": "J200"
        }))
        .expect_err("history path response must reject read-model drift");
        assert!(
            path.to_string().contains("state_code"),
            "schema error should name the noncanonical field: {path}"
        );
    }

    #[test]
    fn invocation_history_list_decodes_bounded_summary_without_full_ledger_payloads() {
        let response = serde_json::from_value::<HistoryListResponse>(json!({
            "ledger_ura": "easynet:///r/test/resource/device.callee/billing/invocations",
            "records": [{
                "invocation_ura": "easynet:///r/test/resource/invocation.i-1",
                "request_id": "req-1",
                "trace_id": "trace-1",
                "span_id": "span-1",
                "caller_ura": "easynet:///r/test/device/caller",
                "callee_ura": "easynet:///r/test/agent/device.callee.locomotion",
                "subject_ura": "easynet:///r/test/resource/device.callee/item",
                "ability_ura": "easynet:///r/test/ability/system-agent.callee.locomotion.fs.read",
                "ability_name": "fs.read",
                "state": "cancelled",
                "started_unix_ms": 1_700_000_000_000_i64,
                "completed_unix_ms": 1_700_000_000_100_i64,
                "elapsed_ms": 100,
                "error": {
                    "source": "runtime",
                    "code": "CANCELLED",
                    "message": "consumer disconnected",
                    "retryable": false,
                    "truncated": false
                }
            }]
        }))
        .expect("bounded invocation history summary must not require full record fields");

        assert_eq!(response.records.len(), 1);
        assert_eq!(response.records[0].state, "cancelled");
        assert_eq!(
            response.records[0]
                .error
                .as_ref()
                .map(|error| error.code.as_str()),
            Some("CANCELLED")
        );
    }

    #[test]
    fn invocation_history_responses_require_canonical_ledger_ura() {
        let missing = serde_json::from_value::<HistoryListResponse>(json!({
            "ledger_path": "/tmp/ledger",
            "records": []
        }))
        .expect_err("history list response must require canonical ledger_ura");
        assert!(
            missing.to_string().contains("ledger_ura"),
            "missing ledger_ura error should name field: {missing}"
        );

        let malformed = serde_json::from_value::<HistoryGetResponse>(json!({
            "ledger_ura": "https://example.invalid/ledger",
            "ledger_path": "/tmp/ledger",
            "record": null
        }))
        .expect("serde should decode shape before semantic validation");
        let err = malformed
            .validate_response("invocation.history.get")
            .expect_err("history get response must validate ledger_ura semantics");
        assert!(
            err.to_string()
                .contains("ledger_ura must be a canonical URA"),
            "malformed ledger_ura error should name canonical URA requirement: {err}"
        );

        let wrong_kind = serde_json::from_value::<HistoryPathResponse>(json!({
            "ledger_ura": "easynet:///r/test/device/callee",
            "ledger_path": "/tmp/ledger"
        }))
        .expect("serde should decode shape before semantic validation");
        let err = wrong_kind
            .validate_response("invocation.history.path")
            .expect_err("history path response must require Resource ledger_ura");
        assert!(
            err.to_string().contains("Resource URA"),
            "wrong-kind ledger_ura error should name Resource URA requirement: {err}"
        );
    }

    #[test]
    fn invocation_history_rejects_malformed_filter_uras() {
        let cases = [
            (
                "ability",
                ListArgs {
                    limit: 25,
                    state: None,
                    ability_ura: Some("easynet:///r/test/device/dev-a".into()),
                    caller: None,
                    callee: None,
                    agent_ura: None,
                    subject: None,
                    format: OutputFormat::Json,
                },
                "`--ability-ura` must be a canonical Ability URA",
            ),
            (
                "caller",
                ListArgs {
                    limit: 25,
                    state: None,
                    ability_ura: None,
                    caller: Some("not-a-ura".into()),
                    callee: None,
                    agent_ura: None,
                    subject: None,
                    format: OutputFormat::Json,
                },
                "`--caller-ura` must be a canonical URA",
            ),
            (
                "callee",
                ListArgs {
                    limit: 25,
                    state: None,
                    ability_ura: None,
                    caller: None,
                    callee: Some("not-a-ura".into()),
                    agent_ura: None,
                    subject: None,
                    format: OutputFormat::Json,
                },
                "`--callee-ura` must be a canonical URA",
            ),
            (
                "agent facade",
                ListArgs {
                    limit: 25,
                    state: None,
                    ability_ura: None,
                    caller: None,
                    callee: None,
                    agent_ura: Some("not-a-ura".into()),
                    subject: None,
                    format: OutputFormat::Json,
                },
                "`--agent-ura` must be a canonical URA",
            ),
            (
                "subject",
                ListArgs {
                    limit: 25,
                    state: None,
                    ability_ura: None,
                    caller: None,
                    callee: None,
                    agent_ura: None,
                    subject: Some("not-a-ura".into()),
                    format: OutputFormat::Json,
                },
                "`--subject-ura` must be a canonical URA",
            ),
        ];

        for (label, args, expected) in cases {
            let error = match InvocationHistoryListQuery::from_list_args(&args) {
                Ok(_) => panic!("{label} filter accepted malformed URA"),
                Err(error) => error,
            };
            assert!(
                error.to_string().contains(expected),
                "{label} filter error = {error}"
            );
        }
    }

    #[test]
    fn invocation_history_read_projects_path_get_and_trace_arguments() {
        assert_eq!(read_test_args(InvocationHistoryRead::Path), json!({}));

        let by_ura = InvocationHistoryRead::Get(InvocationHistoryKey::for_record_lookup(
            "easynet:///r/test/resource/invocation.i-1",
        ));
        assert_eq!(
            read_test_args(by_ura),
            json!({ "key": { "ura": "easynet:///r/test/resource/invocation.i-1" } })
        );

        let by_request =
            InvocationHistoryRead::Get(InvocationHistoryKey::for_record_lookup("req-1"));
        assert_eq!(
            read_test_args(by_request),
            json!({ "key": { "request_id": "req-1" } })
        );

        let trace = InvocationHistoryRead::Trace(InvocationHistoryKey::TraceId("trace-1".into()));
        assert_eq!(
            read_test_args(trace),
            json!({ "key": { "trace_id": "trace-1" } })
        );
    }

    #[test]
    fn invocation_history_stats_uses_list_query_without_scope_filter() {
        let body = read_test_args(InvocationHistoryRead::List(
            InvocationHistoryListQuery::for_stats(500),
        ));

        assert_eq!(body, json!({ "limit": 500 }));
    }

    #[test]
    fn invocation_history_list_args_constructor_is_the_only_cli_filter_projection() {
        let body = InvocationHistoryListQuery::from_list_args(&ListArgs {
            limit: 25,
            state: Some("completed".into()),
            ability_ura: Some(
                "easynet:///r/test/ability/system-agent.callee.locomotion.fs.read".into(),
            ),
            caller: Some("easynet:///r/test/device/caller".into()),
            callee: None,
            agent_ura: Some("easynet:///r/test/agent/device.callee.locomotion".into()),
            subject: Some("easynet:///r/test/user/alice".into()),
            format: OutputFormat::Json,
        })
        .unwrap()
        .into_args();

        assert_eq!(body["limit"], 25);
        assert_eq!(
            body["filter"]["ability_ura"],
            "easynet:///r/test/ability/system-agent.callee.locomotion.fs.read"
        );
        assert_eq!(
            body["filter"]["caller_ura"],
            "easynet:///r/test/device/caller"
        );
        assert_eq!(
            body["filter"]["callee_ura"],
            "easynet:///r/test/agent/device.callee.locomotion"
        );
        assert_eq!(
            body["filter"]["subject_ura"],
            "easynet:///r/test/user/alice"
        );
        assert!(body["filter"].get("agent_ura").is_none());
        assert!(body["filter"].get("subject").is_none());
    }

    #[test]
    fn invocation_history_agent_filter_is_cli_only_callee_lowering() {
        let query = InvocationHistoryListQuery::from_list_args(&ListArgs {
            limit: 25,
            state: None,
            ability_ura: None,
            caller: None,
            callee: Some("easynet:///r/test/device/callee".into()),
            agent_ura: Some("easynet:///r/test/device/other".into()),
            subject: None,
            format: OutputFormat::Json,
        })
        .expect_err("conflicting CLI facade and canonical callee filters must fail");

        assert!(
            query.to_string().contains("facade for `--callee-ura`"),
            "got {query}"
        );

        let body = InvocationHistoryListQuery::from_list_args(&ListArgs {
            limit: 25,
            state: None,
            ability_ura: None,
            caller: None,
            callee: None,
            agent_ura: Some("easynet:///r/test/device/callee".into()),
            subject: None,
            format: OutputFormat::Json,
        })
        .unwrap()
        .into_args();

        assert_eq!(
            body["filter"]["callee_ura"],
            "easynet:///r/test/device/callee"
        );
        assert!(body["filter"].get("agent_ura").is_none());
    }

    // ── `invocation stats` aggregation (F-051) ──────────────────

    fn test_record_builder(
        ability: &str,
        state: &str,
    ) -> axon_sdk::invocation::InvocationLedgerRecordBuilder {
        axon_sdk::invocation::InvocationLedgerRecordBuilder::new()
            .invocation_ura("easynet:///r/test/resource/alice.invocations/i-1")
            .request_id("req-1")
            .caller_ura("easynet:///r/test/device/caller")
            .callee_ura("easynet:///r/test/agent/device.callee.locomotion")
            .subject_ura("easynet:///r/test/resource/user.alice/document/report")
            .ability_ura("easynet:///r/test/ability/system-agent.callee.locomotion.fs.read")
            .ability_name(ability)
            .state(state)
            .started_unix_ms(1_700_000_000_000_i64)
            .authority_form("self")
            .args(axon_sdk::invocation::LedgerEventPayload::Digest {
                content_type: "application/json".to_string(),
                sha256: "0".repeat(64),
                size_bytes: 2,
            })
    }

    fn stats_record(
        ability: &str,
        state: &str,
        error_code: Option<&str>,
        elapsed_ms: Option<u64>,
    ) -> InvocationHistorySummary {
        let mut b = test_record_builder(ability, state);
        if let Some(code) = error_code {
            b = b.error(axon_sdk::invocation::LedgerErrorRecord {
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
        let record = b.build().expect("stats test record");
        InvocationHistorySummary {
            invocation_ura: record.invocation_ura,
            request_id: record.request_id,
            trace_id: record.trace_id,
            span_id: record.span_id,
            caller_ura: record.caller_ura,
            callee_ura: record.callee_ura,
            subject_ura: record.subject_ura,
            ability_ura: record.ability_ura,
            ability_name: record.ability_name,
            state: record.state,
            started_unix_ms: record.started_unix_ms,
            completed_unix_ms: record.completed_unix_ms,
            elapsed_ms: record.elapsed_ms,
            error: record.error.map(|error| InvocationHistoryErrorSummary {
                source: error.source,
                code: error.code,
                message: error.message,
                retryable: error.retryable,
                truncated: !error.context.is_empty(),
            }),
        }
    }

    #[test]
    fn show_record_json_reports_usage_attestation() {
        let ability_ura = "easynet:///r/test/ability/system-agent.callee.locomotion.fs.read";
        let descriptor_ref = format!("{ability_ura}@1.0.0#{}!read", "a".repeat(64));
        let record = test_record_builder("fs.read", "completed")
            .descriptor_ref(descriptor_ref.clone())
            .admission_action("read")
            .safe_read(true)
            .usage(axon_sdk::invocation::axiom::InvocationUsage {
                tokens_in: 11,
                tokens_out: 7,
                duration_ms: 29,
                external_calls: 2,
            })
            .receipt_chain(axon_sdk::invocation::InvocationReceiptChainSummary {
                verified: true,
                ..Default::default()
            })
            .build()
            .expect("show test record");

        let value = show_record_json(&record).expect("json projection");
        assert_eq!(value["usage"]["tokens_in"], 11);
        assert_eq!(value["usage"]["tokens_out"], 7);
        assert_eq!(value["ability_name"], "fs.read");
        assert_eq!(value["ability_ura"], ability_ura);
        assert_eq!(value["descriptor_ref"], descriptor_ref);
        assert_eq!(value["admission_action"], "read");
        assert_eq!(value["safe_read"], true);
        assert_eq!(value["ledger_reported_receipt_chain_verified"], true);
        assert_eq!(value["cli_receipt_chain_verification"], "not_performed");
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
        let mut records: Vec<InvocationHistorySummary> = (1..=100)
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
