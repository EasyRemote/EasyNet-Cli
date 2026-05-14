// EasyNet CLI — Invocation Audit Group
// ====================================
//
// File: src/facade/cli/groups/invocation.rs
// Description: `easynet invocation ...` reads the local native
//              invocation ledger. It does not call an ability, so
//              inspecting audit history does not recursively create
//              another audit record.

use std::path::PathBuf;

use anyhow::Context;
use clap::{Args, Subcommand};

use crate::persistence::daemon_config::{default_billing_dir, default_config_path, DaemonConfig};
use crate::support::output::{self, OutputFormat};

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
    /// Filter by ability name or ability URA.
    #[arg(long)]
    pub ability: Option<String>,
    /// Filter by caller URA.
    #[arg(long)]
    pub caller: Option<String>,
    /// Filter by callee URA.
    #[arg(long)]
    pub callee: Option<String>,
    /// Filter by subject URA.
    #[arg(long)]
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
            println!("{}", ledger_path().display());
            Ok(())
        }
    }
}

fn run_list(args: ListArgs) -> anyhow::Result<()> {
    let path = ledger_path();
    let records = read_records(&path, list_query(&args))?;

    if args.format == OutputFormat::Json {
        println!("{}", serde_json::to_string_pretty(&records)?);
        return Ok(());
    }

    if records.is_empty() {
        output::info(&format!(
            "No invocation records at {}. Run an ability through the daemon first.",
            path.display()
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
            record.ability_name.clone(),
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
    let path = ledger_path();
    let record = find_record(&path, &args.id)?.ok_or_else(|| {
        anyhow::anyhow!(
            "invocation record not found for `{}` in {}",
            args.id,
            path.display()
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
    output::kv_section_stdout(&[
        ("invocation_ura", record.invocation_ura.as_str()),
        ("request_id", record.request_id.as_str()),
        ("trace_id", record.trace_id.as_str()),
        ("span_id", record.span_id.as_str()),
        ("state", record.state.as_str()),
        ("ability", record.ability_name.as_str()),
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
    let path = ledger_path();
    let trace_id = match find_record(&path, &args.id)? {
        Some(record) if !record.trace_id.is_empty() => record.trace_id,
        Some(record) => {
            if args.format == OutputFormat::Json {
                let graph = easynet_axon::invocation::InvocationTraceGraph {
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

    let graph = read_trace_graph(&path, &trace_id)?;

    if args.format == OutputFormat::Json {
        println!("{}", serde_json::to_string_pretty(&graph)?);
        return Ok(());
    }

    if graph.records.is_empty() {
        output::info(&format!(
            "No invocation records with trace_id `{trace_id}` in {}.",
            path.display()
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

fn print_trace_table(records: &[easynet_axon::invocation::InvocationLedgerRecord]) {
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
            record.ability_name.clone(),
            started,
            elapsed,
        ]);
    }
    println!("{table}");
}

fn print_trace_edges(edges: &[easynet_axon::invocation::InvocationTraceEdge]) {
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

fn find_record(
    path: &PathBuf,
    id: &str,
) -> anyhow::Result<Option<easynet_axon::invocation::InvocationLedgerRecord>> {
    if !path.exists() {
        return Ok(None);
    }
    let ledger = easynet_axon::invocation::InvocationLedger::open(path)
        .with_context(|| format!("open invocation ledger {}", path.display()))?;
    ledger
        .fetch_one(query_for_id(id))
        .map_err(|e| anyhow::anyhow!(e))
}

fn read_records(
    path: &PathBuf,
    query: easynet_axon::invocation::InvocationLedgerQuery,
) -> anyhow::Result<Vec<easynet_axon::invocation::InvocationLedgerRecord>> {
    if !path.exists() || query.limit == 0 {
        return Ok(Vec::new());
    }
    Ok(easynet_axon::invocation::InvocationLedger::open(path)
        .with_context(|| format!("open invocation ledger {}", path.display()))?
        .fetch(query)?)
}

fn read_trace_graph(
    path: &PathBuf,
    trace_id: &str,
) -> anyhow::Result<easynet_axon::invocation::InvocationTraceGraph> {
    if !path.exists() {
        return Ok(easynet_axon::invocation::InvocationTraceGraph {
            trace_id: trace_id.to_string(),
            ..Default::default()
        });
    }
    Ok(easynet_axon::invocation::InvocationLedger::open(path)
        .with_context(|| format!("open invocation ledger {}", path.display()))?
        .trace_graph(trace_id)?)
}

fn ledger_path() -> PathBuf {
    DaemonConfig::load(&default_config_path())
        .map(|config| config.billing_dir().join("invocations.redb"))
        .unwrap_or_else(|_| default_billing_dir().join("invocations.redb"))
}

fn list_query(args: &ListArgs) -> easynet_axon::invocation::InvocationLedgerQuery {
    let mut query = easynet_axon::invocation::InvocationLedgerQuery::new().limit(args.limit);
    if let Some(state) = args.state.as_deref() {
        query = query.state(state);
    }
    if let Some(caller) = args.caller.as_deref() {
        query = query.caller_ura(caller);
    }
    if let Some(callee) = args.callee.as_deref() {
        query = query.callee_ura(callee);
    }
    if let Some(subject) = args.subject.as_deref() {
        query = query.subject_ura(subject);
    }
    if let Some(ability) = args.ability.as_deref() {
        if ability.starts_with("easynet:///") {
            query = query.ability_ura(ability);
        } else {
            query = query.ability_name(ability);
        }
    }
    query
}

fn query_for_id(id: &str) -> easynet_axon::invocation::InvocationLedgerQuery {
    let key = if id.starts_with("easynet:///") {
        easynet_axon::invocation::InvocationLedgerFetchKey::InvocationUra(id.to_string())
    } else {
        easynet_axon::invocation::InvocationLedgerFetchKey::RequestId(id.to_string())
    };
    easynet_axon::invocation::InvocationLedgerQuery::new().key(key)
}

fn short_ura(ura: &str) -> String {
    crate::ura::parse_ura(ura)
        .ok()
        .and_then(|parsed| match parsed.kind {
            crate::ura::URAKind::User => Some(format!("user/{}", parsed.user_id)),
            crate::ura::URAKind::Device => Some(format!("device/{}", parsed.device_id)),
            crate::ura::URAKind::Agent => {
                Some(format!("agent/{}.{}", parsed.user_id, parsed.agent_id))
            }
            crate::ura::URAKind::Hub => Some(format!("hub/{}", parsed.realm)),
            _ => None,
        })
        .unwrap_or_else(|| ura.to_string())
}
