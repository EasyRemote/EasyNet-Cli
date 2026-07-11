// EasyNet CLI — Invocation audit abilities
// ========================================
//
// File: src/daemon/ability/builtins/governance/invocation_history.rs
// Description: Device-owned read-only surfaces over the Axon
//              invocation ledger. The daemon writes the ledger at
//              invoke time; these abilities expose that durable
//              history without requiring frontend/backend code to
//              reconstruct URAs.
//
// Contract
// --------
// - Every returned identity field is copied from the Axon ledger
//   record: invocation_ura, caller_ura, callee_ura, subject_ura,
//   ability_ura. This module does not build URAs.
// - Payload bodies remain digest-or-sealed records. The history
//   surface never unwraps encrypted event content.
// - Missing ledger file is a valid empty history state.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde_json::{json, Value};

use crate::daemon::ability::dispatch::{AxonAbilityCatalog, OwnerKind};
use crate::daemon::persistence::daemon_config::{
    default_config_path, default_ledger_dir, DaemonConfig,
};
use easynet_axon::invocation::{
    InvocationLedger, InvocationLedgerFetchKey, InvocationLedgerQuery, InvocationLedgerRecord,
};

pub const ABILITY_HISTORY_LIST: &str =
    crate::daemon::ability::names::governance::INVOCATION_HISTORY_LIST;
pub const ABILITY_HISTORY_GET: &str =
    crate::daemon::ability::names::governance::INVOCATION_HISTORY_GET;
pub const ABILITY_TRACE_GET: &str = crate::daemon::ability::names::governance::INVOCATION_TRACE_GET;
pub const ABILITY_HISTORY_PATH: &str =
    crate::daemon::ability::names::governance::INVOCATION_HISTORY_PATH;

/// Daemon-internal, side-effect-free read RPC: fetch one ledger record by
/// `request_id` or canonical invocation URA.
///
/// Distinct from [`ABILITY_HISTORY_GET`]: this is NOT dispatched through the
/// Axon LocalRuntime (which would append a second ledger row and corrupt the
/// audit trail the caller is observing). The daemon `invoke` handler services
/// it directly off the in-process `InvocationLedger` and writes no ledger row.
/// It is the channel an out-of-process CLI uses to observe the receipt
/// projection of a request it just issued, instead of opening the daemon-owned
/// redb file (which redb forbids from a second process via its exclusive lock).
pub const ABILITY_INVOCATION_RECORD_GET: &str =
    crate::daemon::ability::names::governance::INVOCATION_RECORD_GET;

/// Fetch one ledger record by `request_id` directly off an in-process ledger
/// handle, with no dispatch and no ledger write. Returns `None` when no record
/// exists yet (the sink persists asynchronously after the unary response).
///
/// Side-effect-free by construction: it only issues a redb read transaction on
/// a handle the daemon already holds open, so it never takes a second
/// cross-process lock and never appends to the ledger.
pub fn record_by_request_id(
    ledger: &InvocationLedger,
    request_id: &str,
) -> anyhow::Result<Option<Value>> {
    let request_id = request_id.trim();
    if request_id.is_empty() {
        anyhow::bail!("invocation.record.get: request_id must not be empty");
    }
    record_by_query(
        ledger,
        InvocationLedgerQuery::new()
            .key(InvocationLedgerFetchKey::RequestId(request_id.to_string()))
            .limit(1),
    )
}

/// Fetch one ledger record from the daemon-held ledger handle using the same
/// key shape as `invocation.history.get`, without dispatching an Axon ability
/// and without appending an observation row.
pub fn record_by_args(ledger: &InvocationLedger, args: &Value) -> anyhow::Result<Option<Value>> {
    let query = record_query_from_args(args)?;
    record_by_query(ledger, query)
}

/// Fetch one trace graph from the daemon-held ledger handle using the same key
/// shape as `invocation.trace.get`, without dispatching an Axon ability and
/// without appending an observation row.
pub fn trace_by_args(ledger: &InvocationLedger, args: &Value) -> anyhow::Result<Value> {
    let query = query_from_args(args)?;
    let trace_id = match query.key.as_ref() {
        Some(InvocationLedgerFetchKey::TraceId(trace_id)) => Some(trace_id.clone()),
        _ => ledger
            .fetch_one(query)?
            .and_then(|record| (!record.trace_id.is_empty()).then_some(record.trace_id)),
    };
    let Some(trace_id) = trace_id else {
        anyhow::bail!("expected key.trace_id, key.ura, or key.request_id");
    };
    let graph = ledger.trace_graph(&trace_id)?;
    Ok(json!({
        "ledger_ura": ledger_resource_ura(),
        "ledger_path": ledger_path_from_config().display().to_string(),
        "trace_id": graph.trace_id,
        "nodes": graph.records,
        "edges": graph.edges,
        "edge_semantics": "Axon causal links: source receipt URA/hash -> target invocation URA",
    }))
}

fn record_query_from_args(args: &Value) -> anyhow::Result<InvocationLedgerQuery> {
    if let Some(request_id) = args.get("request_id").and_then(non_empty_str) {
        return Ok(InvocationLedgerQuery::new()
            .key(InvocationLedgerFetchKey::RequestId(request_id.to_string()))
            .limit(1));
    }
    let query = query_from_args(args)?.limit(1);
    if query.key.is_none() {
        anyhow::bail!("expected request_id or key.ura, key.request_id, or key.trace_id");
    }
    Ok(query)
}

fn record_by_query(
    ledger: &InvocationLedger,
    query: InvocationLedgerQuery,
) -> anyhow::Result<Option<Value>> {
    let Some(record) = ledger.fetch_one(query)? else {
        return Ok(None);
    };
    serde_json::to_value(record)
        .map(Some)
        .map_err(|err| anyhow::anyhow!("serialize invocation ledger record: {err}"))
}

const DEFAULT_LIMIT: usize = 50;
const MAX_LIMIT: usize = 500;
const HISTORY_CURSOR_PREFIX: &str = "receipt-history:v1:";
const MAX_HISTORY_CURSOR_LEN: usize = 4096;

pub fn register(reg: &mut AxonAbilityCatalog, ledger: Option<Arc<InvocationLedger>>) {
    let reader = Arc::new(InvocationLedgerReader::new(ledger));

    let list_reader = Arc::clone(&reader);
    reg.register_rpc_with_spec(
        ABILITY_HISTORY_LIST,
        OwnerKind::Device,
        crate::daemon::ability::catalog::system_manifest::registry_manifest(
            ABILITY_HISTORY_LIST,
            list_history_description(),
            list_history_input_schema(),
        ),
        Arc::new(move |args| list_reader.list_history(args)),
    );
    let get_reader = Arc::clone(&reader);
    reg.register_rpc_with_spec(
        ABILITY_HISTORY_GET,
        OwnerKind::Device,
        crate::daemon::ability::catalog::system_manifest::registry_manifest(
            ABILITY_HISTORY_GET,
            get_history_description(),
            get_history_input_schema(),
        ),
        Arc::new(move |args| get_reader.get_history(args)),
    );
    let trace_reader = Arc::clone(&reader);
    reg.register_rpc_with_spec(
        ABILITY_TRACE_GET,
        OwnerKind::Device,
        crate::daemon::ability::catalog::system_manifest::registry_manifest(
            ABILITY_TRACE_GET,
            get_trace_description(),
            get_trace_input_schema(),
        ),
        Arc::new(move |args| trace_reader.get_trace(args)),
    );
    let path_reader = Arc::clone(&reader);
    reg.register_rpc_with_spec(
        ABILITY_HISTORY_PATH,
        OwnerKind::Device,
        crate::daemon::ability::catalog::system_manifest::registry_manifest(
            ABILITY_HISTORY_PATH,
            get_path_description(),
            get_path_input_schema(),
        ),
        Arc::new(move |args| path_reader.get_path(args)),
    );
}

struct InvocationLedgerReader {
    shared: Option<Arc<InvocationLedger>>,
}

impl InvocationLedgerReader {
    fn new(shared: Option<Arc<InvocationLedger>>) -> Self {
        Self { shared }
    }

    fn list_history(&self, args: Value) -> anyhow::Result<Value> {
        let requested_limit = bounded_limit(args.get("limit").and_then(Value::as_u64));
        let cursor_anchor = history_cursor_anchor(args.get("cursor").and_then(non_empty_str))?;
        let exclude_ability_uras = string_set_arg(&args, "exclude_ability_uras");
        let include_ability_uras = filter_string_set(&args, "ability_uras");
        // Cursor semantics are defined over the daemon-owned, already sorted
        // Axon ledger projection after all supported predicates have been
        // applied. The SDK treats the cursor as opaque; only this provider
        // decodes the anchor and decides the next page boundary.
        let query = query_from_args(&args)?.limit(0);
        let compact = args
            .get("compact")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let path = ledger_path_from_config();
        let mut records = self.fetch_records(&path, query)?;
        retain_by_ability_ura_sets(&mut records, &include_ability_uras, &exclude_ability_uras);
        apply_history_cursor(&mut records, cursor_anchor.as_deref())?;
        let next_cursor = next_history_cursor(&mut records, requested_limit);
        let records = if compact {
            compact_records(&records)?
        } else {
            json!(records)
        };
        let mut response = json!({
            "ledger_ura": ledger_resource_ura(),
            "ledger_path": path.display().to_string(),
            "records": records,
        });
        if let Some(cursor) = next_cursor {
            response["next_cursor"] = Value::String(cursor);
        }
        Ok(response)
    }

    fn get_history(&self, args: Value) -> anyhow::Result<Value> {
        let query = query_from_args(&args)?.limit(1);
        if query.key.is_none() {
            anyhow::bail!("expected key.ura, key.request_id, or key.trace_id");
        }

        let path = ledger_path_from_config();
        let record = self.fetch_one(&path, query)?;
        Ok(json!({
            "ledger_ura": ledger_resource_ura(),
            "ledger_path": path.display().to_string(),
            "record": record,
        }))
    }

    fn get_trace(&self, args: Value) -> anyhow::Result<Value> {
        let query = query_from_args(&args)?;
        let trace_id = match query.key.as_ref() {
            Some(InvocationLedgerFetchKey::TraceId(trace_id)) => Some(trace_id.clone()),
            _ => {
                let path = ledger_path_from_config();
                self.fetch_one(&path, query)?
                    .and_then(|record| (!record.trace_id.is_empty()).then_some(record.trace_id))
            }
        };
        let Some(trace_id) = trace_id else {
            anyhow::bail!("expected key.trace_id, key.ura, or key.request_id");
        };

        let path = ledger_path_from_config();
        let graph = self.trace_graph(&path, &trace_id)?;

        Ok(json!({
            "ledger_ura": ledger_resource_ura(),
            "ledger_path": path.display().to_string(),
            "trace_id": graph.trace_id,
            "nodes": graph.records,
            "edges": graph.edges,
            "edge_semantics": "Axon causal links: source receipt URA/hash -> target invocation URA",
        }))
    }

    fn get_path(&self, _args: Value) -> anyhow::Result<Value> {
        let path = ledger_path_from_config();
        Ok(json!({
            "ledger_ura": ledger_resource_ura(),
            "ledger_path": path.display().to_string(),
        }))
    }

    fn fetch_records(
        &self,
        path: &Path,
        query: InvocationLedgerQuery,
    ) -> anyhow::Result<Vec<InvocationLedgerRecord>> {
        if let Some(ledger) = self.shared.as_ref() {
            return Ok(ledger.fetch(query)?);
        }
        fetch_records_from_path(path, query)
    }

    fn fetch_one(
        &self,
        path: &Path,
        query: InvocationLedgerQuery,
    ) -> anyhow::Result<Option<InvocationLedgerRecord>> {
        if let Some(ledger) = self.shared.as_ref() {
            return Ok(ledger.fetch_one(query)?);
        }
        fetch_one_from_path(path, query)
    }

    fn trace_graph(
        &self,
        path: &Path,
        trace_id: &str,
    ) -> anyhow::Result<easynet_axon::invocation::InvocationTraceGraph> {
        if let Some(ledger) = self.shared.as_ref() {
            return Ok(ledger.trace_graph(trace_id)?);
        }
        trace_graph_from_path(path, trace_id)
    }
}

fn bounded_limit(raw: Option<u64>) -> usize {
    raw.map(|n| n as usize)
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_LIMIT)
        .min(MAX_LIMIT)
}

fn history_cursor_anchor(raw: Option<&str>) -> anyhow::Result<Option<String>> {
    let Some(cursor) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    if cursor.len() > MAX_HISTORY_CURSOR_LEN {
        anyhow::bail!("invocation.history.list cursor exceeds the maximum bound");
    }
    let encoded = cursor.strip_prefix(HISTORY_CURSOR_PREFIX).ok_or_else(|| {
        anyhow::anyhow!("invocation.history.list cursor is not a recognized receipt history cursor")
    })?;
    let bytes = URL_SAFE_NO_PAD
        .decode(encoded.as_bytes())
        .map_err(|err| anyhow::anyhow!("invocation.history.list cursor is not valid: {err}"))?;
    let invocation_ura = String::from_utf8(bytes)
        .map_err(|err| anyhow::anyhow!("invocation.history.list cursor is not UTF-8: {err}"))?;
    if invocation_ura.trim().is_empty() {
        anyhow::bail!("invocation.history.list cursor anchor must not be empty");
    }
    Ok(Some(invocation_ura))
}

fn history_cursor_for(record: &InvocationLedgerRecord) -> String {
    format!(
        "{HISTORY_CURSOR_PREFIX}{}",
        URL_SAFE_NO_PAD.encode(record.invocation_ura.as_bytes())
    )
}

fn apply_history_cursor(
    records: &mut Vec<InvocationLedgerRecord>,
    cursor_anchor: Option<&str>,
) -> anyhow::Result<()> {
    let Some(anchor) = cursor_anchor else {
        return Ok(());
    };
    let Some(position) = records
        .iter()
        .position(|record| record.invocation_ura == anchor)
    else {
        anyhow::bail!("invocation.history.list cursor does not match the current query");
    };
    records.drain(..=position);
    Ok(())
}

fn next_history_cursor(
    records: &mut Vec<InvocationLedgerRecord>,
    requested_limit: usize,
) -> Option<String> {
    if records.len() <= requested_limit {
        return None;
    }
    let next = records
        .get(requested_limit.saturating_sub(1))
        .map(history_cursor_for);
    records.truncate(requested_limit);
    next
}

fn compact_records(records: &[InvocationLedgerRecord]) -> anyhow::Result<Value> {
    let mut out = Vec::with_capacity(records.len());
    for record in records {
        let value = serde_json::to_value(record)?;
        out.push(compact_record_value(value));
    }
    Ok(Value::Array(out))
}

fn compact_record_value(mut value: Value) -> Value {
    if let Some(object) = value.as_object_mut() {
        // List views only need routing, timing, state, and error
        // summaries. Full payload envelopes, diagnostics, causal links,
        // receipt-chain, and privacy metadata remain available through
        // invocation.history.get for a single record, avoiding
        // multi-megabyte list payloads on active nodes.
        object.remove("args");
        object.remove("result");
        object.remove("diagnostics");
        object.remove("causal_links");
        object.remove("receipt_chain");
        object.remove("visibility");
    }
    value
}

fn string_set_arg(args: &Value, key: &str) -> HashSet<String> {
    value_string_set(args.get(key))
}

/// Read a string set from `args.filter.<key>` (e.g. `filter.ability_uras`).
fn filter_string_set(args: &Value, key: &str) -> HashSet<String> {
    value_string_set(args.get("filter").and_then(|f| f.get(key)))
}

fn value_string_set(value: Option<&Value>) -> HashSet<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

/// Apply include/exclude Ability URA sets in place. Empty sets are
/// no-ops, so callers can pass either or both.
fn retain_by_ability_ura_sets(
    records: &mut Vec<InvocationLedgerRecord>,
    include: &HashSet<String>,
    exclude: &HashSet<String>,
) {
    if !include.is_empty() {
        records.retain(|record| include.contains(record.ability_ura.as_str()));
    }
    if !exclude.is_empty() {
        records.retain(|record| !exclude.contains(record.ability_ura.as_str()));
    }
}

fn query_from_args(args: &Value) -> anyhow::Result<InvocationLedgerQuery> {
    let mut query = InvocationLedgerQuery::new();
    if let Some(key) = args.get("key") {
        query = query.key(fetch_key_from_value(key)?);
    }
    if let Some(filter) = args.get("filter") {
        query = apply_filter_object(query, filter)?;
    }
    Ok(query)
}

fn fetch_key_from_value(value: &Value) -> anyhow::Result<InvocationLedgerFetchKey> {
    let object = value.as_object().ok_or_else(|| {
        anyhow::anyhow!("key must be an object with ura, request_id, or trace_id")
    })?;
    if let Some(ura) = object.get("ura").and_then(non_empty_str) {
        return Ok(InvocationLedgerFetchKey::InvocationUra(ura.to_string()));
    }
    if let Some(request_id) = object.get("request_id").and_then(non_empty_str) {
        return Ok(InvocationLedgerFetchKey::RequestId(request_id.to_string()));
    }
    if let Some(trace_id) = object.get("trace_id").and_then(non_empty_str) {
        return Ok(InvocationLedgerFetchKey::TraceId(trace_id.to_string()));
    }
    anyhow::bail!("key must include one of ura, request_id, or trace_id")
}

fn apply_filter_object(
    mut query: InvocationLedgerQuery,
    value: &Value,
) -> anyhow::Result<InvocationLedgerQuery> {
    let object = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("filter must be an object"))?;
    validate_filter_keys(object)?;
    if let Some(caller) = object.get("caller_ura").and_then(non_empty_str) {
        query = query.caller_ura(caller);
    }
    let callee_ura = scoped_callee_ura(object)?;
    if let Some(callee) = callee_ura.as_deref() {
        query = query.callee_ura(callee);
    }
    if let Some(subjects) = subject_filter_values(object)? {
        query = query.subject_uras(subjects);
    }
    if let Some(ability_ura) = object.get("ability_ura").and_then(non_empty_str) {
        query = query.ability_ura(ability_ura);
    }
    if let Some(state) = object.get("state").and_then(non_empty_str) {
        query = query.state(state);
    }
    if let Some(trace_id) = object.get("trace_id").and_then(non_empty_str) {
        query = query.trace_id(trace_id);
    }
    Ok(query)
}

fn validate_filter_keys(object: &serde_json::Map<String, Value>) -> anyhow::Result<()> {
    for key in object.keys() {
        match key.as_str() {
            "caller_ura" | "callee_ura" | "agent_ura" | "subject_ura" | "subject_uras"
            | "ability_ura" | "ability_uras" | "state" | "trace_id" => {}
            other => anyhow::bail!("unsupported filter field `{other}`"),
        }
    }
    Ok(())
}

fn scoped_callee_ura(object: &serde_json::Map<String, Value>) -> anyhow::Result<Option<String>> {
    let callee = object.get("callee_ura").and_then(non_empty_str);
    let agent = object.get("agent_ura").and_then(non_empty_str);
    match (callee, agent) {
        (Some(callee), Some(agent)) if callee != agent => {
            anyhow::bail!("filter.callee_ura and filter.agent_ura must match when both are set")
        }
        (Some(value), _) | (_, Some(value)) => Ok(Some(value.to_string())),
        (None, None) => Ok(None),
    }
}

fn non_empty_str(value: &Value) -> Option<&str> {
    value.as_str().map(str::trim).filter(|s| !s.is_empty())
}

fn subject_filter_values(
    object: &serde_json::Map<String, Value>,
) -> anyhow::Result<Option<Vec<String>>> {
    let single = object.get("subject_ura");
    let many = object.get("subject_uras");
    if single.is_some() && many.is_some() {
        anyhow::bail!("filter.subject_ura and filter.subject_uras are mutually exclusive");
    }
    if let Some(value) = single {
        let subject = non_empty_str(value)
            .ok_or_else(|| anyhow::anyhow!("filter.subject_ura must be a non-empty string"))?;
        return Ok(Some(vec![subject.to_string()]));
    }
    let Some(value) = many else {
        return Ok(None);
    };
    let array = value
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("filter.subject_uras must be an array of strings"))?;
    let subjects = array
        .iter()
        .filter_map(non_empty_str)
        .map(str::to_string)
        .collect::<Vec<_>>();
    if subjects.is_empty() {
        anyhow::bail!("filter.subject_uras must include at least one non-empty string");
    }
    Ok(Some(subjects))
}

fn ledger_path_from_config() -> PathBuf {
    DaemonConfig::load(&default_config_path())
        .map(|config| config.ledger_dir().join("invocations.redb"))
        .unwrap_or_else(|_| default_ledger_dir().join("invocations.redb"))
}

fn ledger_resource_ura() -> Option<String> {
    let local = crate::daemon::persistence::local_agents::load().ok()?;
    let parsed = crate::core::ura::parse_ura(&local.host_device_agent_ura).ok()?;
    let owner = match parsed.kind {
        crate::core::ura::URAKind::Device => format!("device.{}", parsed.device_id()?),
        crate::core::ura::URAKind::User => format!("{}.invocations", parsed.user_id()?),
        crate::core::ura::URAKind::Agent => {
            // DEC-F048 / RFC gap: `agent_ids()` is None for
            // device-sponsored System Agents, and that None is the
            // declared outcome — no resource_dot owner shape exists
            // for them yet, and we do not invent one (RFC-007/008
            // agenda; F-047 verdict).
            let (user_id, _) = parsed.agent_ids()?;
            format!("{user_id}.invocations")
        }
        _ => return None,
    };
    Some(crate::core::ura::resource_dot_ura(
        &parsed.realm,
        &owner,
        "billing/invocations",
    ))
}

fn fetch_records_from_path(
    path: &Path,
    query: InvocationLedgerQuery,
) -> anyhow::Result<Vec<InvocationLedgerRecord>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    Ok(InvocationLedger::open(path)?.fetch(query)?)
}

fn fetch_one_from_path(
    path: &Path,
    query: InvocationLedgerQuery,
) -> anyhow::Result<Option<InvocationLedgerRecord>> {
    if !path.exists() {
        return Ok(None);
    }
    Ok(InvocationLedger::open(path)?.fetch_one(query)?)
}

fn trace_graph_from_path(
    path: &Path,
    trace_id: &str,
) -> anyhow::Result<easynet_axon::invocation::InvocationTraceGraph> {
    if !path.exists() {
        return Ok(easynet_axon::invocation::InvocationTraceGraph {
            trace_id: trace_id.to_string(),
            ..Default::default()
        });
    }
    Ok(InvocationLedger::open(path)?.trace_graph(trace_id)?)
}

pub fn list_history_description() -> &'static str {
    "List recent invocation ledger records for this device. Returns \
     complete URAs from the persisted envelope plus digest/sealed \
     payload metadata for audit and billing."
}

pub fn get_history_description() -> &'static str {
    "Fetch one invocation ledger record by key.ura, key.request_id, or key.trace_id."
}

pub fn get_trace_description() -> &'static str {
    "Fetch all invocation ledger records that share a trace_id and \
     project them through Axon's causal DAG graph API."
}

pub fn get_path_description() -> &'static str {
    "Return the daemon-owned invocation ledger's resource URA and \
     on-disk path. Singular: one daemon owns one ledger; this \
     surface does not enumerate per-tenant or per-realm ledgers."
}

pub fn list_history_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "limit": { "type": "integer", "minimum": 1, "maximum": MAX_LIMIT },
            "cursor": {
                "type": "string",
                "minLength": 1,
                "maxLength": MAX_HISTORY_CURSOR_LEN,
                "description": "Opaque receipt history cursor returned by the previous invocation.history.list page."
            },
            "compact": { "type": "boolean" },
            "exclude_ability_uras": {
                "type": "array",
                "items": { "type": "string", "minLength": 1 }
            },
            "key": key_schema(),
            "filter": filter_schema()
        },
        "additionalProperties": false
    })
}

pub fn get_history_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "key": key_schema(),
            "filter": filter_schema()
        },
        "required": ["key"],
        "additionalProperties": false
    })
}

pub fn get_trace_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "key": key_schema(),
            "filter": filter_schema()
        },
        "required": ["key"],
        "additionalProperties": false
    })
}

pub fn get_path_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false
    })
}

fn key_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "ura": { "type": "string", "description": "Invocation URA." },
            "request_id": { "type": "string" },
            "trace_id": { "type": "string" }
        },
        "additionalProperties": false,
        "minProperties": 1,
        "maxProperties": 1
    })
}

fn filter_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "caller_ura": { "type": "string", "description": "Caller Agent URA." },
            "callee_ura": { "type": "string", "description": "Callee/owner Agent URA." },
            "agent_ura": {
                "type": "string",
                "description": "Ability owner/callee Agent URA. Equivalent to callee_ura for history scope."
            },
            "subject_ura": { "type": "string", "description": "Single subject URA." },
            "subject_uras": {
                "type": "array",
                "description": "OR-scope over subject URAs.",
                "items": { "type": "string", "minLength": 1 },
                "minItems": 1,
                "uniqueItems": true
            },
            "ability_ura": { "type": "string", "description": "Canonical Ability URA." },
            "ability_uras": {
                "type": "array",
                "description": "Canonical Ability URAs. Records match when ability_ura equals any value.",
                "items": { "type": "string", "minLength": 1 },
                "minItems": 1,
                "uniqueItems": true
            },
            "state": { "type": "string" },
            "trace_id": { "type": "string" }
        },
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_makes_invocation_history_dispatchable() {
        let mut reg = AxonAbilityCatalog::new();
        register(&mut reg, None);
        assert!(reg.get_rpc(ABILITY_HISTORY_LIST).is_some());
        assert!(reg.get_rpc(ABILITY_HISTORY_GET).is_some());
        assert!(reg.get_rpc(ABILITY_TRACE_GET).is_some());
        assert!(reg.get_rpc(ABILITY_HISTORY_PATH).is_some());
    }

    #[test]
    fn registration_publishes_invocation_history_manifests() {
        let mut reg = AxonAbilityCatalog::new();
        register(&mut reg, None);

        for ability in [
            ABILITY_HISTORY_LIST,
            ABILITY_HISTORY_GET,
            ABILITY_TRACE_GET,
            ABILITY_HISTORY_PATH,
        ] {
            let record = reg
                .control_plane_record_for_mode(ability, crate::daemon::ability::CallMode::Rpc)
                .expect("control-plane lookup must be unambiguous")
                .unwrap_or_else(|| panic!("{ability} must publish a canonical descriptor"));
            assert_eq!(
                record
                    .descriptor()
                    .input_schema()
                    .get("type")
                    .and_then(Value::as_str),
                Some("object"),
                "{ability} must publish an object input schema"
            );
        }
    }

    #[test]
    fn list_history_schema_exposes_opaque_cursor() {
        let schema = list_history_input_schema();
        let cursor = schema
            .get("properties")
            .and_then(|properties| properties.get("cursor"))
            .expect("cursor schema");
        assert_eq!(cursor.get("type").and_then(Value::as_str), Some("string"));
        assert_eq!(
            cursor.get("maxLength").and_then(Value::as_u64),
            Some(MAX_HISTORY_CURSOR_LEN as u64)
        );
    }

    #[test]
    fn missing_ledger_returns_empty_history() {
        let dir = tempfile::tempdir().expect("tempdir");
        let records = fetch_records_from_path(
            &dir.path().join("missing.redb"),
            InvocationLedgerQuery::new().limit(10),
        )
        .unwrap();
        assert!(records.is_empty());
    }

    #[test]
    fn get_record_reads_by_request_id() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("invocations.redb");
        let ledger = easynet_axon::invocation::InvocationLedger::open(&path).unwrap();
        let record = sample_record("req-test");
        ledger.put(&record).unwrap();
        drop(ledger);

        let fetched = fetch_one_from_path(
            &path,
            InvocationLedgerQuery::new()
                .key(InvocationLedgerFetchKey::RequestId("req-test".to_string())),
        )
        .unwrap()
        .unwrap();
        assert_eq!(fetched.invocation_ura, record.invocation_ura);
    }

    #[test]
    fn shared_ledger_reader_does_not_reopen_database() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("invocations.redb");
        let ledger = Arc::new(easynet_axon::invocation::InvocationLedger::open(&path).unwrap());
        let record = sample_record("req-shared");
        ledger.put(&record).unwrap();

        let reader = InvocationLedgerReader::new(Some(Arc::clone(&ledger)));
        let fetched = reader
            .fetch_one(
                &path,
                InvocationLedgerQuery::new().key(InvocationLedgerFetchKey::RequestId(
                    "req-shared".to_string(),
                )),
            )
            .unwrap()
            .unwrap();
        assert_eq!(fetched.invocation_ura, record.invocation_ura);
    }

    #[test]
    fn list_history_response_uses_ura_field_names() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("invocations.redb");
        let ledger = Arc::new(easynet_axon::invocation::InvocationLedger::open(&path).unwrap());
        ledger.put(&sample_record("req-list-json")).unwrap();

        let reader = InvocationLedgerReader::new(Some(Arc::clone(&ledger)));
        let value = reader.list_history(json!({ "limit": 5 })).unwrap();
        assert!(value.get("ledger_ura").is_some());
        let records = value
            .get("records")
            .and_then(Value::as_array)
            .expect("records");
        assert_eq!(records.len(), 1);
        assert!(records[0].get("invocation_ura").is_some());
        assert!(records[0].get("caller_ura").is_some());
        assert!(records[0].get("callee_ura").is_some());
        assert!(records[0].get("subject_ura").is_some());
        assert!(records[0].get("ability_ura").is_some());
    }

    #[test]
    fn compact_record_value_removes_heavy_list_only_fields() {
        let value = compact_record_value(json!({
            "invocation_ura": "easynet:///r/test/resource/alice.invocations/req",
            "request_id": "req",
            "args": { "kind": "digest" },
            "result": { "kind": "digest" },
            "receipt_chain": { "anchors": [] },
            "visibility": { "args": { "policy": "digest_only" } }
        }));

        assert!(value.get("invocation_ura").is_some());
        assert!(value.get("request_id").is_some());
        assert!(value.get("args").is_none());
        assert!(value.get("result").is_none());
        assert!(value.get("receipt_chain").is_none());
        assert!(value.get("visibility").is_none());
    }

    #[test]
    fn list_history_excludes_noisy_abilities_before_truncating() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("invocations.redb");
        let ledger = Arc::new(easynet_axon::invocation::InvocationLedger::open(&path).unwrap());
        let mut noisy = sample_record("req-noisy");
        noisy.ability_name = "terminal.read".to_string();
        noisy.ability_ura = "easynet:///r/test/ability/terminal.read".to_string();
        noisy.started_unix_ms = 3;
        let mut wanted = sample_record("req-wanted");
        wanted.started_unix_ms = 2;
        ledger.put(&noisy).unwrap();
        ledger.put(&wanted).unwrap();

        let reader = InvocationLedgerReader::new(Some(Arc::clone(&ledger)));
        let value = reader
            .list_history(json!({
                "limit": 1,
                "compact": true,
                "exclude_ability_uras": ["easynet:///r/test/ability/terminal.read"]
            }))
            .unwrap();
        let records = value
            .get("records")
            .and_then(Value::as_array)
            .expect("records");

        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0].get("request_id").and_then(Value::as_str),
            Some("req-wanted")
        );
    }

    #[test]
    fn list_history_returns_stable_cursor_and_resumes_after_anchor() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("invocations.redb");
        let ledger = Arc::new(easynet_axon::invocation::InvocationLedger::open(&path).unwrap());
        let mut newest = sample_record("req-newest");
        newest.started_unix_ms = 30;
        let mut middle = sample_record("req-middle");
        middle.started_unix_ms = 20;
        let mut oldest = sample_record("req-oldest");
        oldest.started_unix_ms = 10;
        ledger.put(&oldest).unwrap();
        ledger.put(&newest).unwrap();
        ledger.put(&middle).unwrap();

        let reader = InvocationLedgerReader::new(Some(Arc::clone(&ledger)));
        let first = reader.list_history(json!({ "limit": 1 })).unwrap();
        let first_records = first["records"].as_array().expect("first records");
        assert_eq!(first_records.len(), 1);
        assert_eq!(
            first_records[0].get("request_id").and_then(Value::as_str),
            Some("req-newest")
        );
        let first_cursor = first["next_cursor"].as_str().expect("first cursor");
        assert_eq!(first_cursor, history_cursor_for(&newest));

        let second = reader
            .list_history(json!({ "limit": 1, "cursor": first_cursor }))
            .unwrap();
        let second_records = second["records"].as_array().expect("second records");
        assert_eq!(second_records.len(), 1);
        assert_eq!(
            second_records[0].get("request_id").and_then(Value::as_str),
            Some("req-middle")
        );
        let second_cursor = second["next_cursor"].as_str().expect("second cursor");
        assert_eq!(second_cursor, history_cursor_for(&middle));

        let third = reader
            .list_history(json!({ "limit": 5, "cursor": second_cursor }))
            .unwrap();
        let third_records = third["records"].as_array().expect("third records");
        assert_eq!(third_records.len(), 1);
        assert_eq!(
            third_records[0].get("request_id").and_then(Value::as_str),
            Some("req-oldest")
        );
        assert!(third.get("next_cursor").is_none());
    }

    #[test]
    fn list_history_rejects_cursor_outside_current_query() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("invocations.redb");
        let ledger = Arc::new(easynet_axon::invocation::InvocationLedger::open(&path).unwrap());
        let mut kept = sample_record("req-kept");
        kept.ability_ura = "easynet:///r/test/ability/kept".to_string();
        let mut excluded = sample_record("req-excluded");
        excluded.ability_ura = "easynet:///r/test/ability/excluded".to_string();
        ledger.put(&kept).unwrap();
        ledger.put(&excluded).unwrap();

        let reader = InvocationLedgerReader::new(Some(Arc::clone(&ledger)));
        let err = reader
            .list_history(json!({
                "limit": 1,
                "cursor": history_cursor_for(&excluded),
                "filter": {
                    "ability_uras": ["easynet:///r/test/ability/kept"]
                }
            }))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("cursor does not match the current query"),
            "got {err}"
        );
    }

    #[test]
    fn list_history_rejects_malformed_cursor() {
        let reader = InvocationLedgerReader::new(None);
        let err = reader
            .list_history(json!({
                "cursor": "not-a-receipt-history-cursor"
            }))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("recognized receipt history cursor"),
            "got {err}"
        );
    }

    #[test]
    fn query_from_args_uses_key_and_filter_objects() {
        let query = query_from_args(&json!({
            "key": { "ura": "easynet:///r/test/resource/alice.invocations/req-test" },
            "filter": {
                "caller_ura": "easynet:///r/test/device/caller",
                "agent_ura": "easynet:///r/test/device/callee",
                "subject_ura": "easynet:///r/test/user/alice",
                "ability_ura": "easynet:///r/test/ability/hub.observe.health",
                "state": "completed"
            }
        }))
        .unwrap();
        assert!(matches!(
            query.key,
            Some(InvocationLedgerFetchKey::InvocationUra(_))
        ));
        assert_eq!(
            query.caller_ura.as_deref(),
            Some("easynet:///r/test/device/caller")
        );
        assert_eq!(
            query.callee_ura.as_deref(),
            Some("easynet:///r/test/device/callee")
        );
        assert!(query.subject_uras.contains("easynet:///r/test/user/alice"));
        assert_eq!(
            query.ability_ura.as_deref(),
            Some("easynet:///r/test/ability/hub.observe.health")
        );
    }

    #[test]
    fn query_from_args_accepts_subject_array() {
        let query = query_from_args(&json!({
            "filter": {
                "subject_uras": [
                    "easynet:///r/test/device/mac-1",
                    "easynet:///r/test/agent/alice.frontend"
                ]
            }
        }))
        .unwrap();
        assert_eq!(query.subject_uras.len(), 2);
        assert!(query
            .subject_uras
            .contains("easynet:///r/test/device/mac-1"));
        assert!(query
            .subject_uras
            .contains("easynet:///r/test/agent/alice.frontend"));
    }

    #[test]
    fn query_from_args_rejects_ambiguous_subject_scope() {
        let err = query_from_args(&json!({
            "filter": {
                "subject_ura": "easynet:///r/test/user/alice",
                "subject_uras": ["easynet:///r/test/device/mac-1"]
            }
        }))
        .unwrap_err()
        .to_string();
        assert!(err.contains("mutually exclusive"), "got {err}");
    }

    #[test]
    fn query_from_args_rejects_unknown_filter_fields() {
        let err = query_from_args(&json!({
            "filter": {
                "subject": "easynet:///r/test/user/alice"
            }
        }))
        .unwrap_err()
        .to_string();
        assert!(err.contains("unsupported filter field"), "got {err}");
    }

    #[test]
    fn query_from_args_rejects_conflicting_agent_and_callee_scope() {
        let err = query_from_args(&json!({
            "filter": {
                "agent_ura": "easynet:///r/test/device/owner-a",
                "callee_ura": "easynet:///r/test/device/owner-b"
            }
        }))
        .unwrap_err()
        .to_string();
        assert!(err.contains("must match"), "got {err}");
    }

    #[test]
    fn trace_graph_reads_causal_edges() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("invocations.redb");
        let ledger = easynet_axon::invocation::InvocationLedger::open(&path).unwrap();
        let parent = sample_record("req-parent");
        let mut child = sample_record("req-child");
        child.causal_links = vec![easynet_axon::invocation::InvocationCausalLink {
            source_invocation_ura: Some(parent.invocation_ura.clone()),
            source_receipt_ura: format!("{}/receipt/1", parent.invocation_ura),
            source_receipt_hash: "01".repeat(32),
            relation: "child_spawned".to_string(),
        }];
        ledger.put(&parent).unwrap();
        ledger.put(&child).unwrap();
        drop(ledger);

        let graph = trace_graph_from_path(&path, "trace-test").unwrap();
        assert_eq!(graph.records.len(), 2);
        assert_eq!(graph.edges.len(), 1);
        assert_eq!(
            graph.edges[0].from_invocation_ura.as_deref(),
            Some(parent.invocation_ura.as_str())
        );
        assert_eq!(
            graph.edges[0].from_receipt_ura,
            format!("{}/receipt/1", parent.invocation_ura)
        );
        assert_eq!(graph.edges[0].relation, "child_spawned");
    }

    fn sample_record(request_id: &str) -> easynet_axon::invocation::InvocationLedgerRecord {
        easynet_axon::invocation::InvocationLedgerRecordBuilder::new()
            .invocation_ura(crate::core::ura::resource_dot_ura(
                "test",
                "alice.invocations",
                request_id,
            ))
            .request_id(request_id.to_string())
            .trace_id("trace-test".to_string())
            .span_id("span-test".to_string())
            .caller_ura("easynet:///r/test/device/caller".to_string())
            .callee_ura("easynet:///r/test/device/callee".to_string())
            .subject_ura("easynet:///r/test/user/alice".to_string())
            .ability_ura("easynet:///r/test/ability/hub.observe.health".to_string())
            .ability_name("observe.health".to_string())
            .state("completed".to_string())
            .started_unix_ms(1)
            .completed_unix_ms(2)
            .elapsed_ms(1_u64)
            .args(easynet_axon::invocation::LedgerEventPayload::digest(
                "application/json",
                b"{}",
            ))
            .result(easynet_axon::invocation::LedgerEventPayload::digest(
                "application/json",
                b"{\"ok\":true}",
            ))
            .build()
            .unwrap()
    }

    fn record_with_ability(
        request_id: &str,
        ability_name: &str,
        ability_ura: &str,
    ) -> easynet_axon::invocation::InvocationLedgerRecord {
        easynet_axon::invocation::InvocationLedgerRecordBuilder::new()
            .invocation_ura(crate::core::ura::resource_dot_ura(
                "test",
                "alice.invocations",
                request_id,
            ))
            .request_id(request_id.to_string())
            .trace_id(format!("trace-{request_id}"))
            .span_id(format!("span-{request_id}"))
            .caller_ura("easynet:///r/test/device/caller".to_string())
            .callee_ura("easynet:///r/test/device/callee".to_string())
            .subject_ura("easynet:///r/test/user/alice".to_string())
            .ability_ura(ability_ura.to_string())
            .ability_name(ability_name.to_string())
            .state("completed".to_string())
            .started_unix_ms(1)
            .args(easynet_axon::invocation::LedgerEventPayload::digest(
                "application/json",
                b"{}",
            ))
            .build()
            .unwrap()
    }

    #[test]
    fn list_history_or_matches_multiple_ability_uras() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("invocations.redb");
        let ledger = easynet_axon::invocation::InvocationLedger::open(&path).unwrap();
        ledger
            .put(&record_with_ability(
                "req-by-ura",
                "liangbing.chat",
                "easynet:///r/test/ability/dev.liangbing.chat",
            ))
            .unwrap();
        ledger
            .put(&record_with_ability(
                "req-second-ura",
                "liangbing.chat",
                "easynet:///r/_system/ability/liangbing.chat",
            ))
            .unwrap();
        ledger
            .put(&record_with_ability(
                "req-unrelated",
                "observe.health",
                "easynet:///r/test/ability/device.dev.observe.health",
            ))
            .unwrap();
        let reader = InvocationLedgerReader::new(Some(Arc::new(ledger)));

        // ONE request carrying canonical Ability URAs; local registry names
        // are not query authority.
        let resp = reader
            .list_history(json!({
                "limit": 50,
                "filter": {
                    "ability_uras": [
                        "easynet:///r/test/ability/dev.liangbing.chat",
                        "easynet:///r/_system/ability/liangbing.chat",
                    ]
                },
            }))
            .unwrap();
        let ids: Vec<&str> = resp["records"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|r| r["request_id"].as_str())
            .collect();
        assert!(ids.contains(&"req-by-ura"), "should match by ability_ura");
        assert!(
            ids.contains(&"req-second-ura"),
            "should match only when its ability_ura is explicitly present"
        );
        assert!(
            !ids.contains(&"req-unrelated"),
            "non-candidate ability must be excluded, got {ids:?}"
        );
    }

    #[test]
    fn retain_by_ability_ura_sets_applies_include_then_exclude() {
        let mut records = vec![
            record_with_ability("a", "chat", "easynet:///r/test/ability/dev.a.chat"),
            record_with_ability("b", "terminal.read", "easynet:///r/test/ability/dev.b.read"),
            record_with_ability(
                "c",
                "observe.health",
                "easynet:///r/test/ability/dev.c.health",
            ),
        ];
        let include: HashSet<String> = [
            "easynet:///r/test/ability/dev.a.chat".to_string(),
            "easynet:///r/test/ability/dev.b.read".to_string(),
        ]
        .into_iter()
        .collect();
        let exclude: HashSet<String> = ["easynet:///r/test/ability/dev.b.read".to_string()]
            .into_iter()
            .collect();
        retain_by_ability_ura_sets(&mut records, &include, &exclude);
        let ids: Vec<&str> = records.iter().map(|r| r.request_id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["a"],
            "include keeps chat+read, exclude drops read"
        );
    }
}
