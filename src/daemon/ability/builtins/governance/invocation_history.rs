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
use crate::daemon::invocation::dispatch::attempt_audit::{
    attempt_ledger_path, InvocationAttemptLedger, InvocationAttemptRecord,
};
use crate::daemon::persistence::agent_aggregate::AgentAggregateRepository;
use crate::daemon::persistence::daemon_config::{
    default_config_path, default_ledger_dir, DaemonConfig,
};
use axon_sdk::invocation::{
    InvocationLedger, InvocationLedgerFetchKey, InvocationLedgerQuery, InvocationLedgerRecord,
};

pub const ABILITY_HISTORY_LIST: &str =
    crate::daemon::ability::names::governance::INVOCATION_HISTORY_LIST;
pub const ABILITY_HISTORY_GET: &str =
    crate::daemon::ability::names::governance::INVOCATION_HISTORY_GET;
pub const ABILITY_TRACE_GET: &str = crate::daemon::ability::names::governance::INVOCATION_TRACE_GET;
pub const ABILITY_HISTORY_PATH: &str =
    crate::daemon::ability::names::governance::INVOCATION_HISTORY_PATH;
pub const ABILITY_INVOCATION_RECORD_GET: &str =
    crate::daemon::ability::names::governance::INVOCATION_RECORD_GET;

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

const DEFAULT_LIMIT: usize = 50;
const MAX_LIMIT: usize = 500;
const HISTORY_CURSOR_PREFIX: &str = "receipt-history:v1:";
const MAX_HISTORY_CURSOR_LEN: usize = 4096;

pub fn register(reg: &mut AxonAbilityCatalog, ledger: Option<Arc<InvocationLedger>>) {
    let reader = Arc::new(InvocationLedgerReader::new(ledger));
    for owner in reg.local_runtime_owners() {
        register_for_owner(reg, owner, Arc::clone(&reader));
    }
}

fn register_for_owner(
    reg: &mut AxonAbilityCatalog,
    owner: OwnerKind,
    reader: Arc<InvocationLedgerReader>,
) {
    let list_reader = Arc::clone(&reader);
    reg.register_rpc_with_spec(
        ABILITY_HISTORY_LIST,
        owner.clone(),
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
        owner.clone(),
        crate::daemon::ability::catalog::system_manifest::registry_manifest(
            ABILITY_HISTORY_GET,
            get_history_description(),
            get_history_input_schema(),
        ),
        Arc::new(move |args| get_reader.get_history(args)),
    );
    let record_reader = Arc::clone(&reader);
    reg.register_rpc_with_spec(
        ABILITY_INVOCATION_RECORD_GET,
        owner.clone(),
        crate::daemon::ability::catalog::system_manifest::registry_manifest(
            ABILITY_INVOCATION_RECORD_GET,
            get_record_description(),
            get_record_input_schema(),
        ),
        Arc::new(move |args| record_reader.get_record(args)),
    );
    let trace_reader = Arc::clone(&reader);
    reg.register_rpc_with_spec(
        ABILITY_TRACE_GET,
        owner.clone(),
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
        owner,
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
        let exclude_ability_uras = string_set_arg(&args, "exclude_ability_uras")?;
        let include_ability_uras = filter_string_set(&args, "ability_uras")?;
        // Cursor semantics are defined over the daemon-owned, already sorted
        // Axon ledger projection after all supported predicates have been
        // applied. The SDK treats the cursor as opaque; only this provider
        // decodes the anchor and decides the next page boundary.
        let query = query_from_args(&args)?.limit(0);
        let compact = args
            .get("compact")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let include_attempts = args
            .get("include_attempts")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let path = ledger_path_from_config();
        let mut records = self.fetch_records(&path, query)?;
        retain_by_ability_ura_sets(&mut records, &include_ability_uras, &exclude_ability_uras);
        apply_history_cursor(&mut records, cursor_anchor.as_deref())?;
        let next_cursor = next_history_cursor(&mut records, requested_limit);
        let diagnostic_records = if include_attempts {
            let attempts = filtered_attempt_records(&args, requested_limit)?;
            Some(merged_diagnostic_records(
                &records,
                &attempts,
                requested_limit,
            )?)
        } else {
            None
        };
        let records = if compact && include_attempts {
            diagnostic_records
                .clone()
                .unwrap_or_else(|| Value::Array(Vec::new()))
        } else if compact {
            compact_records(&records)?
        } else {
            json!(records)
        };
        let mut response = json!({
            "ledger_ura": ledger_resource_ura()?,
            "ledger_path": path.display().to_string(),
            "records": records,
        });
        if let Some(diagnostics) = diagnostic_records {
            response["diagnostic_records"] = diagnostics;
            response["attempt_ledger_path"] =
                Value::String(attempt_ledger_path_from_config().display().to_string());
        }
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
            "ledger_ura": ledger_resource_ura()?,
            "ledger_path": path.display().to_string(),
            "record": record,
        }))
    }

    fn get_record(&self, args: Value) -> anyhow::Result<Value> {
        let query = record_query_from_args(&args)?;
        let path = ledger_path_from_config();
        let record = self.fetch_one(&path, query)?;
        Ok(json!({ "record": record }))
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
            "ledger_ura": ledger_resource_ura()?,
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
            "ledger_ura": ledger_resource_ura()?,
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
    ) -> anyhow::Result<axon_sdk::invocation::InvocationTraceGraph> {
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

fn canonical_diagnostic_record(record: &InvocationLedgerRecord) -> anyhow::Result<Value> {
    let mut value = compact_record_value(serde_json::to_value(record)?);
    let diagnostic = json!({
        "summary": record
            .error
            .as_ref()
            .map(|error| error.message.clone())
            .unwrap_or_else(|| format!("Invocation {}.", record.state)),
        "suggested_action": canonical_suggested_action(record),
        "route_ura": Value::Null,
        "execution_host_ura": Value::Null,
    });
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "record_kind".to_string(),
            Value::String("invocation".to_string()),
        );
        object.insert("diagnostic".to_string(), diagnostic);
    }
    Ok(value)
}

fn canonical_suggested_action(record: &InvocationLedgerRecord) -> String {
    match record.error.as_ref().map(|error| error.code.as_str()) {
        Some("INVALID_ARGUMENT") | Some("PERMISSION_DENIED") => {
            "Check ability args, caller authority, subject binding, and descriptor ref.".to_string()
        }
        Some("UNAVAILABLE") => {
            "Check selected execution host liveness and upstream session connectivity.".to_string()
        }
        Some(_) => "Inspect the canonical invocation record and receipt diagnostics.".to_string(),
        None => "Canonical invocation completed; inspect receipts for proof details.".to_string(),
    }
}

fn merged_diagnostic_records(
    records: &[InvocationLedgerRecord],
    attempts: &[InvocationAttemptRecord],
    limit: usize,
) -> anyhow::Result<Value> {
    let mut out = Vec::with_capacity(records.len() + attempts.len());
    for record in records {
        out.push(canonical_diagnostic_record(record)?);
    }
    for attempt in attempts {
        out.push(attempt.diagnostic_value());
    }
    out.sort_by(|a, b| {
        diagnostic_started_unix_ms(b)
            .cmp(&diagnostic_started_unix_ms(a))
            .then_with(|| diagnostic_id(b).cmp(&diagnostic_id(a)))
    });
    if limit > 0 && out.len() > limit {
        out.truncate(limit);
    }
    Ok(Value::Array(out))
}

fn diagnostic_started_unix_ms(value: &Value) -> i64 {
    value
        .get("started_unix_ms")
        .and_then(Value::as_i64)
        .unwrap_or_default()
}

fn diagnostic_id(value: &Value) -> String {
    value
        .get("invocation_ura")
        .or_else(|| value.get("attempt_id"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn string_set_arg(args: &Value, key: &'static str) -> anyhow::Result<HashSet<String>> {
    value_string_set(args.get(key), key, false)
}

/// Read a string set from `args.filter.<key>` (e.g. `filter.ability_uras`).
fn filter_string_set(args: &Value, key: &'static str) -> anyhow::Result<HashSet<String>> {
    let Some(filter) = args.get("filter") else {
        return Ok(HashSet::new());
    };
    let object = filter
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("filter must be an object"))?;
    value_string_set(object.get(key), &format!("filter.{key}"), true)
}

fn value_string_set(
    value: Option<&Value>,
    field: &str,
    require_non_empty: bool,
) -> anyhow::Result<HashSet<String>> {
    let Some(value) = value else {
        return Ok(HashSet::new());
    };
    let array = value
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("{field} must be an array of non-empty strings"))?;
    if require_non_empty && array.is_empty() {
        anyhow::bail!("{field} must include at least one non-empty string");
    }
    let mut out = HashSet::new();
    for (index, item) in array.iter().enumerate() {
        let value = non_empty_str(item)
            .ok_or_else(|| anyhow::anyhow!("{field}[{index}] must be a non-empty string"))?;
        out.insert(value.to_string());
    }
    Ok(out)
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
    if let Some(caller) = optional_filter_string(object, "caller_ura")? {
        query = query.caller_ura(caller);
    }
    let callee_ura = scoped_callee_ura(object)?;
    if let Some(callee) = callee_ura.as_deref() {
        query = query.callee_ura(callee);
    }
    if let Some(subjects) = subject_filter_values(object)? {
        query = query.subject_uras(subjects);
    }
    if let Some(ability_ura) = optional_filter_string(object, "ability_ura")? {
        query = query.ability_ura(ability_ura);
    }
    if let Some(state) = optional_filter_string(object, "state")? {
        query = query.state(state);
    }
    if let Some(trace_id) = optional_filter_string(object, "trace_id")? {
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
    let callee = optional_filter_string(object, "callee_ura")?;
    let agent = optional_filter_string(object, "agent_ura")?;
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

fn optional_filter_string<'a>(
    object: &'a serde_json::Map<String, Value>,
    key: &'static str,
) -> anyhow::Result<Option<&'a str>> {
    let Some(value) = object.get(key) else {
        return Ok(None);
    };
    non_empty_str(value)
        .map(Some)
        .ok_or_else(|| anyhow::anyhow!("filter.{key} must be a non-empty string"))
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
    let mut subjects = Vec::with_capacity(array.len());
    for (index, item) in array.iter().enumerate() {
        let subject = non_empty_str(item).ok_or_else(|| {
            anyhow::anyhow!("filter.subject_uras[{index}] must be a non-empty string")
        })?;
        subjects.push(subject.to_string());
    }
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

fn attempt_ledger_path_from_config() -> PathBuf {
    DaemonConfig::load(&default_config_path())
        .map(|config| attempt_ledger_path(config.ledger_dir()))
        .unwrap_or_else(|_| attempt_ledger_path(&default_ledger_dir()))
}

fn filtered_attempt_records(
    args: &Value,
    limit: usize,
) -> anyhow::Result<Vec<InvocationAttemptRecord>> {
    let path = attempt_ledger_path_from_config();
    let ledger = InvocationAttemptLedger::open(path)?;
    let mut attempts = ledger.list_recent(MAX_LIMIT)?;
    if let Some(filter) = args.get("filter") {
        let object = filter
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("filter must be an object"))?;
        let mut filtered = Vec::with_capacity(attempts.len());
        for attempt in attempts {
            if attempt_matches_filter(&attempt, object)? {
                filtered.push(attempt);
            }
        }
        attempts = filtered;
    }
    let include_ability_uras = filter_string_set(args, "ability_uras")?;
    if !include_ability_uras.is_empty() {
        attempts.retain(|attempt| {
            attempt
                .ability_ura
                .as_ref()
                .is_some_and(|ability| include_ability_uras.contains(ability))
        });
    }
    if limit > 0 && attempts.len() > limit {
        attempts.truncate(limit);
    }
    Ok(attempts)
}

fn attempt_matches_filter(
    attempt: &InvocationAttemptRecord,
    object: &serde_json::Map<String, Value>,
) -> anyhow::Result<bool> {
    Ok(
        string_filter_matches(object, "caller_ura", attempt.caller_ura.as_deref())?
            && scoped_attempt_callee_matches(attempt, object)?
            && subject_attempt_filter_matches(attempt, object)?
            && string_filter_matches(object, "ability_ura", attempt.ability_ura.as_deref())?
            && attempt_state_matches(optional_filter_string(object, "state")?, attempt)
            && string_filter_matches(object, "trace_id", attempt.trace_id.as_deref())?,
    )
}

fn string_filter_matches(
    object: &serde_json::Map<String, Value>,
    key: &'static str,
    actual: Option<&str>,
) -> anyhow::Result<bool> {
    Ok(optional_filter_string(object, key)?.is_none_or(|expected| actual == Some(expected)))
}

fn scoped_attempt_callee_matches(
    attempt: &InvocationAttemptRecord,
    object: &serde_json::Map<String, Value>,
) -> anyhow::Result<bool> {
    let expected = scoped_callee_ura(object)?;
    Ok(expected
        .as_deref()
        .is_none_or(|expected| attempt.callee_ura.as_deref() == Some(expected)))
}

fn subject_attempt_filter_matches(
    attempt: &InvocationAttemptRecord,
    object: &serde_json::Map<String, Value>,
) -> anyhow::Result<bool> {
    let Some(subjects) = subject_filter_values(object)? else {
        return Ok(true);
    };
    let actual = attempt.subject_ura.as_deref();
    Ok(subjects.iter().any(|expected| actual == Some(expected)))
}

fn attempt_state_matches(expected: Option<&str>, attempt: &InvocationAttemptRecord) -> bool {
    let Some(expected) = expected else {
        return true;
    };
    let state = serde_json::to_value(&attempt.state)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default();
    state == expected
        || (expected == "failed"
            && matches!(
                state.as_str(),
                "rejected" | "runtime_rejected" | "runtime_failed"
            ))
}

fn ledger_resource_ura() -> anyhow::Result<Option<String>> {
    let hosted_identity =
        AgentAggregateRepository::load_hosted_identity_status().map_err(|error| {
            anyhow::anyhow!("invocation.history ledger owner projection unavailable: {error:#}")
        })?;
    ledger_resource_ura_from_host_device_agent_ura(hosted_identity.host_device_agent_ura())
}

fn ledger_resource_ura_from_host_device_agent_ura(
    host_device_agent_ura: Option<&str>,
) -> anyhow::Result<Option<String>> {
    let Some(host_device_agent_ura) = host_device_agent_ura
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    let parsed = crate::core::ura::parse_ura(host_device_agent_ura).map_err(|error| {
        anyhow::anyhow!(
            "invocation.history ledger owner projection has invalid host_device_agent_ura {host_device_agent_ura:?}: {error}"
        )
    })?;
    let owner = match parsed.kind {
        crate::core::ura::URAKind::Device => {
            let device_id = parsed.device_id().ok_or_else(|| {
                anyhow::anyhow!(
                    "invocation.history ledger owner projection device URA missing device id"
                )
            })?;
            format!("device.{device_id}")
        }
        crate::core::ura::URAKind::User => {
            let user_id = parsed.user_id().ok_or_else(|| {
                anyhow::anyhow!(
                    "invocation.history ledger owner projection user URA missing user id"
                )
            })?;
            format!("{user_id}.invocations")
        }
        crate::core::ura::URAKind::Agent => {
            // DEC-F048 / RFC gap: `agent_ids()` is None for
            // device-sponsored System Agents, and that None is the
            // declared outcome — no resource_dot owner shape exists
            // for them yet, and we do not invent one (RFC-007/008
            // agenda; F-047 verdict).
            let Some((user_id, _)) = parsed.agent_ids() else {
                return Ok(None);
            };
            format!("{user_id}.invocations")
        }
        _ => return Ok(None),
    };
    Ok(Some(crate::core::ura::resource_dot_ura(
        &parsed.realm,
        &owner,
        "billing/invocations",
    )))
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
) -> anyhow::Result<axon_sdk::invocation::InvocationTraceGraph> {
    if !path.exists() {
        return Ok(axon_sdk::invocation::InvocationTraceGraph {
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

pub fn get_record_description() -> &'static str {
    "Observe one invocation ledger record by request_id or canonical ledger key."
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
            "include_attempts": {
                "type": "boolean",
                "description": "When true, include pre-runtime invocation attempts and row diagnostics for UI history views."
            },
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

pub fn get_record_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "request_id": { "type": "string", "minLength": 1 },
            "key": key_schema(),
            "filter": filter_schema()
        },
        "anyOf": [
            { "required": ["request_id"] },
            { "required": ["key"] }
        ],
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
        assert!(reg.get_rpc(ABILITY_INVOCATION_RECORD_GET).is_some());
        assert!(reg.get_rpc(ABILITY_TRACE_GET).is_some());
        assert!(reg.get_rpc(ABILITY_HISTORY_PATH).is_some());
    }

    #[test]
    fn receipt_routes_are_generated_from_manifest() {
        use sha2::Digest as _;

        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("provider_routes/easynet-receipt-routes.v1.json");
        let digest = sha2::Sha256::digest(std::fs::read(manifest).expect("read manifest"));

        assert_eq!(
            crate::daemon::ability::receipt_routes_gen::RECEIPT_ROUTE_MANIFEST_SHA256,
            hex::encode(digest)
        );
        assert_eq!(
            crate::daemon::ability::receipt_routes_gen::RECEIPT_PROFILE,
            "receipt"
        );
        assert_eq!(
            ABILITY_HISTORY_LIST,
            crate::daemon::ability::receipt_routes_gen::INVOCATION_HISTORY_LIST
        );
    }

    #[test]
    fn registration_publishes_invocation_history_manifests() {
        let mut reg = AxonAbilityCatalog::new();
        register(&mut reg, None);

        for ability in [
            ABILITY_HISTORY_LIST,
            ABILITY_HISTORY_GET,
            ABILITY_INVOCATION_RECORD_GET,
            ABILITY_TRACE_GET,
            ABILITY_HISTORY_PATH,
        ] {
            let records = reg
                .authority_ability_catalog_snapshot()
                .into_iter()
                .filter(|row| row.name == ability)
                .collect::<Vec<_>>();
            assert!(
                !records.is_empty(),
                "{ability} must publish a canonical descriptor"
            );
            for record in records {
                assert_eq!(
                    record
                        .descriptor
                        .input_schema()
                        .get("type")
                        .and_then(Value::as_str),
                    Some("object"),
                    "{ability} must publish an object input schema"
                );
            }
        }
    }

    #[test]
    fn combined_runtime_registers_observation_routes_for_device_and_hub() {
        let device_ura = crate::core::ura::device_ura("history-test", "device-a");
        let authority =
            crate::daemon::ability::dispatch::AbilityAuthorityContext::for_combined_authority_roots(
                device_ura,
            )
            .expect("combined authority context");
        let runtime = crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
            crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
            None,
        );
        let mut reg =
            AxonAbilityCatalog::new_with_runtime_and_authority_context(runtime, authority);

        register(&mut reg, None);

        for ability in [ABILITY_INVOCATION_RECORD_GET, ABILITY_TRACE_GET] {
            let rows = reg
                .authority_ability_catalog_snapshot()
                .into_iter()
                .filter(|row| row.name == ability)
                .collect::<Vec<_>>();
            assert_eq!(rows.len(), 2, "{ability} must exist on both local planes");
            assert!(rows.iter().any(|row| row.owner == OwnerKind::Device));
            assert!(rows.iter().any(|row| row.owner == OwnerKind::Hub));
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
    fn history_key_schema_excludes_attempt_id() {
        let schema = get_history_input_schema();
        let key_properties = schema
            .get("properties")
            .and_then(|properties| properties.get("key"))
            .and_then(|key| key.get("properties"))
            .and_then(Value::as_object)
            .expect("key properties");

        assert!(key_properties.contains_key("ura"));
        assert!(key_properties.contains_key("request_id"));
        assert!(key_properties.contains_key("trace_id"));
        assert!(
            !key_properties.contains_key("attempt_id"),
            "attempt diagnostics are list projections, not canonical history get keys"
        );
    }

    #[test]
    fn get_history_rejects_attempt_id_key() {
        let reader = InvocationLedgerReader::new(None);
        let err = reader
            .get_history(json!({ "key": { "attempt_id": "att-retired" } }))
            .expect_err("attempt_id must not route into the attempt ledger");
        let message = err.to_string();

        assert!(
            message.contains("key must include one of ura, request_id, or trace_id"),
            "unexpected error: {message}"
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
        let ledger = axon_sdk::invocation::InvocationLedger::open(&path).unwrap();
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
        let ledger = Arc::new(axon_sdk::invocation::InvocationLedger::open(&path).unwrap());
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
        let ledger = Arc::new(axon_sdk::invocation::InvocationLedger::open(&path).unwrap());
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
    fn ledger_resource_ura_projection_distinguishes_unjoined_from_invalid_identity() {
        assert_eq!(
            ledger_resource_ura_from_host_device_agent_ura(None).expect("unjoined state"),
            None
        );
        assert_eq!(
            ledger_resource_ura_from_host_device_agent_ura(Some("   ")).expect("blank state"),
            None
        );

        let device =
            ledger_resource_ura_from_host_device_agent_ura(Some("easynet:///r/test/device/dev-1"))
                .expect("device ledger URA")
                .expect("joined device ledger URA");
        assert_eq!(
            device,
            "easynet:///r/test/resource/device.dev-1/billing/invocations"
        );

        let user =
            ledger_resource_ura_from_host_device_agent_ura(Some("easynet:///r/test/user/alice"))
                .expect("user ledger URA")
                .expect("joined user ledger URA");
        assert_eq!(
            user,
            "easynet:///r/test/resource/alice.invocations/billing/invocations"
        );

        let agent = ledger_resource_ura_from_host_device_agent_ura(Some(
            "easynet:///r/test/agent/alice.ops",
        ))
        .expect("agent ledger URA")
        .expect("joined agent ledger URA");
        assert_eq!(
            agent,
            "easynet:///r/test/resource/alice.invocations/billing/invocations"
        );

        let error = ledger_resource_ura_from_host_device_agent_ura(Some("not-a-ura"))
            .expect_err("malformed hosted identity must fail closed");
        assert!(
            error.to_string().contains("invalid host_device_agent_ura"),
            "unexpected error: {error}"
        );
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
    fn diagnostic_records_merge_invocations_and_attempts_for_ui_rows() {
        let record = sample_record("req-diagnostic");
        let attempt = crate::daemon::invocation::dispatch::attempt_audit::InvocationAttemptRecord {
            attempt_id: "att-diagnostic".to_string(),
            call_mode: "Invoke".to_string(),
            state: crate::daemon::invocation::dispatch::attempt_audit::AttemptState::Rejected,
            stage: "runtime_admission".to_string(),
            started_unix_ms: record.started_unix_ms + 10,
            completed_unix_ms: Some(record.started_unix_ms + 11),
            elapsed_ms: Some(1),
            invocation_ura: None,
            request_id: Some("req-attempt".to_string()),
            trace_id: Some("trace-attempt".to_string()),
            span_id: None,
            caller_ura: Some("easynet:///r/test/device/caller".to_string()),
            callee_ura: Some("easynet:///r/test/device/callee".to_string()),
            subject_ura: Some("easynet:///r/test/agent/alice.pages".to_string()),
            ability: Some("project_list".to_string()),
            ability_ura: None,
            route_ura: None,
            execution_host_ura: None,
            status_code: Some("INVALID_ARGUMENT".to_string()),
            status_message: Some(
                "runtime session authority does not admit descriptor-bound subject_ura".to_string(),
            ),
            error_stage: Some("runtime_admission".to_string()),
            retryable: Some(false),
            diagnostic_summary: "runtime_admission: rejected".to_string(),
            suggested_action: "Check caller/callee/subject URA binding.".to_string(),
        };

        let merged = merged_diagnostic_records(&[record], &[attempt], 10).expect("merge");
        let rows = merged.as_array().expect("diagnostic rows");

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["record_kind"], "attempt");
        assert_eq!(rows[0]["attempt_id"], "att-diagnostic");
        assert!(rows[0]["diagnostic"]["summary"]
            .as_str()
            .unwrap()
            .contains("runtime_admission"));
        assert_eq!(rows[1]["record_kind"], "invocation");
        assert!(rows[1]["diagnostic"]["suggested_action"]
            .as_str()
            .unwrap()
            .contains("receipts"));
    }

    #[test]
    fn list_history_excludes_noisy_abilities_before_truncating() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("invocations.redb");
        let ledger = Arc::new(axon_sdk::invocation::InvocationLedger::open(&path).unwrap());
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
        let ledger = Arc::new(axon_sdk::invocation::InvocationLedger::open(&path).unwrap());
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
        let ledger = Arc::new(axon_sdk::invocation::InvocationLedger::open(&path).unwrap());
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
                "ability_ura": "easynet:///r/test/ability/authority.observe.health",
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
            Some("easynet:///r/test/ability/authority.observe.health")
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
    fn query_from_args_rejects_malformed_subject_array_items() {
        let err = query_from_args(&json!({
            "filter": {
                "subject_uras": [
                    "easynet:///r/test/device/mac-1",
                    42
                ]
            }
        }))
        .unwrap_err()
        .to_string();
        assert!(
            err.contains("filter.subject_uras[1] must be a non-empty string"),
            "got {err}"
        );
    }

    #[test]
    fn query_from_args_rejects_malformed_string_filter_fields() {
        for (field, value) in [
            ("caller_ura", json!(42)),
            ("callee_ura", json!("")),
            ("agent_ura", json!(false)),
            ("ability_ura", json!([])),
            ("state", json!("  ")),
            ("trace_id", json!({})),
        ] {
            let err = query_from_args(&json!({
                "filter": {
                    field: value
                }
            }))
            .unwrap_err()
            .to_string();
            assert!(
                err.contains(&format!("filter.{field} must be a non-empty string")),
                "field {field} got {err}"
            );
        }
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
    fn list_history_rejects_malformed_ability_set_filters() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("invocations.redb");
        let ledger = Arc::new(axon_sdk::invocation::InvocationLedger::open(&path).unwrap());
        let reader = InvocationLedgerReader::new(Some(ledger));

        let err = reader
            .list_history(json!({
                "exclude_ability_uras": ["easynet:///r/test/ability/ok", 7]
            }))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("exclude_ability_uras[1] must be a non-empty string"),
            "got {err}"
        );

        let err = reader
            .list_history(json!({
                "filter": {
                    "ability_uras": []
                }
            }))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("filter.ability_uras must include at least one non-empty string"),
            "got {err}"
        );
    }

    #[test]
    fn attempt_filter_rejects_malformed_subject_scope() {
        let attempt = crate::daemon::invocation::dispatch::attempt_audit::InvocationAttemptRecord {
            attempt_id: "att-filter".to_string(),
            call_mode: "Invoke".to_string(),
            state: crate::daemon::invocation::dispatch::attempt_audit::AttemptState::Rejected,
            stage: "runtime_admission".to_string(),
            started_unix_ms: 1,
            completed_unix_ms: Some(2),
            elapsed_ms: Some(1),
            invocation_ura: None,
            request_id: Some("req-attempt".to_string()),
            trace_id: Some("trace-attempt".to_string()),
            span_id: None,
            caller_ura: Some("easynet:///r/test/device/caller".to_string()),
            callee_ura: Some("easynet:///r/test/device/callee".to_string()),
            subject_ura: Some("easynet:///r/test/user/alice".to_string()),
            ability: Some("observe.health".to_string()),
            ability_ura: Some("easynet:///r/test/ability/authority.observe.health".to_string()),
            route_ura: None,
            execution_host_ura: None,
            status_code: Some("PERMISSION_DENIED".to_string()),
            status_message: Some("rejected".to_string()),
            error_stage: Some("runtime_admission".to_string()),
            retryable: Some(false),
            diagnostic_summary: "runtime_admission: rejected".to_string(),
            suggested_action: "Check caller/callee/subject URA binding.".to_string(),
        };
        let filter = json!({
            "subject_uras": [
                "easynet:///r/test/user/alice",
                {"bad": true}
            ]
        });
        let err = attempt_matches_filter(&attempt, filter.as_object().expect("filter object"))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("filter.subject_uras[1] must be a non-empty string"),
            "got {err}"
        );
    }

    #[test]
    fn trace_graph_reads_causal_edges() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("invocations.redb");
        let ledger = axon_sdk::invocation::InvocationLedger::open(&path).unwrap();
        let parent = sample_record("req-parent");
        let mut child = sample_record("req-child");
        child.causal_links = vec![axon_sdk::invocation::InvocationCausalLink {
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

    fn sample_record(request_id: &str) -> axon_sdk::invocation::InvocationLedgerRecord {
        axon_sdk::invocation::InvocationLedgerRecordBuilder::new()
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
            .ability_ura("easynet:///r/test/ability/authority.observe.health".to_string())
            .ability_name("observe.health".to_string())
            .authority_form("self".to_string())
            .state("completed".to_string())
            .started_unix_ms(1)
            .completed_unix_ms(2)
            .elapsed_ms(1_u64)
            .args(axon_sdk::invocation::LedgerEventPayload::digest(
                "application/json",
                b"{}",
            ))
            .result(axon_sdk::invocation::LedgerEventPayload::digest(
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
    ) -> axon_sdk::invocation::InvocationLedgerRecord {
        axon_sdk::invocation::InvocationLedgerRecordBuilder::new()
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
            .authority_form("self".to_string())
            .state("completed".to_string())
            .started_unix_ms(1)
            .args(axon_sdk::invocation::LedgerEventPayload::digest(
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
        let ledger = axon_sdk::invocation::InvocationLedger::open(&path).unwrap();
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
