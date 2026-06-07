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
}
