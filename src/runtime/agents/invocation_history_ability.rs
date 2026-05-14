// EasyNet CLI — device.invocation.* audit abilities
// =================================================
//
// File: src/runtime/agents/invocation_history_ability.rs
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

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{json, Value};

use crate::persistence::daemon_config::{default_billing_dir, default_config_path, DaemonConfig};
use crate::runtime::ability_dispatch::{LocalAbilityRegistry, OwnerKind};
use easynet_axon::invocation::{InvocationLedger, InvocationLedgerFetchKey, InvocationLedgerQuery};

pub const ABILITY_HISTORY_LIST: &str = "device.invocation.history.list";
pub const ABILITY_HISTORY_GET: &str = "device.invocation.history.get";
pub const ABILITY_TRACE_GET: &str = "device.invocation.trace.get";

const DEFAULT_LIMIT: usize = 50;
const MAX_LIMIT: usize = 500;

pub fn register(reg: &mut LocalAbilityRegistry, ledger: Option<Arc<InvocationLedger>>) {
    let reader = Arc::new(InvocationLedgerReader::new(ledger));

    let list_reader = Arc::clone(&reader);
    reg.register_rpc_with_owner(
        ABILITY_HISTORY_LIST,
        OwnerKind::Device,
        Arc::new(move |args| list_reader.list_history(args)),
    );
    let get_reader = Arc::clone(&reader);
    reg.register_rpc_with_owner(
        ABILITY_HISTORY_GET,
        OwnerKind::Device,
        Arc::new(move |args| get_reader.get_history(args)),
    );
    let trace_reader = Arc::clone(&reader);
    reg.register_rpc_with_owner(
        ABILITY_TRACE_GET,
        OwnerKind::Device,
        Arc::new(move |args| trace_reader.get_trace(args)),
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
        let query =
            query_from_args(&args)?.limit(bounded_limit(args.get("limit").and_then(Value::as_u64)));
        let path = ledger_path_from_config();
        let records = self.fetch_records(&path, query)?;
        Ok(json!({
            "ledger_ura": ledger_resource_ura(),
            "ledger_path": path.display().to_string(),
            "records": records,
        }))
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

    fn fetch_records(
        &self,
        path: &Path,
        query: InvocationLedgerQuery,
    ) -> anyhow::Result<Vec<easynet_axon::invocation::InvocationLedgerRecord>> {
        if let Some(ledger) = self.shared.as_ref() {
            return Ok(ledger.fetch(query)?);
        }
        fetch_records_from_path(path, query)
    }

    fn fetch_one(
        &self,
        path: &Path,
        query: InvocationLedgerQuery,
    ) -> anyhow::Result<Option<easynet_axon::invocation::InvocationLedgerRecord>> {
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
    if let Some(caller) = object.get("caller").and_then(non_empty_str) {
        query = query.caller_ura(caller);
    }
    if let Some(callee) = object.get("callee").and_then(non_empty_str) {
        query = query.callee_ura(callee);
    }
    if let Some(subjects) = object
        .get("subject")
        .map(subject_filter_values)
        .transpose()?
    {
        query = query.subject_uras(subjects);
    }
    if let Some(ability) = object.get("ability").and_then(non_empty_str) {
        if ability.starts_with("easynet:///") {
            query = query.ability_ura(ability);
        } else {
            query = query.ability_name(ability);
        }
    }
    if let Some(state) = object.get("state").and_then(non_empty_str) {
        query = query.state(state);
    }
    if let Some(trace_id) = object.get("trace_id").and_then(non_empty_str) {
        query = query.trace_id(trace_id);
    }
    Ok(query)
}

fn non_empty_str(value: &Value) -> Option<&str> {
    value.as_str().map(str::trim).filter(|s| !s.is_empty())
}

fn subject_filter_values(value: &Value) -> anyhow::Result<Vec<String>> {
    if let Some(subject) = non_empty_str(value) {
        return Ok(vec![subject.to_string()]);
    }
    let array = value
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("filter.subject must be a string or an array of strings"))?;
    let subjects = array
        .iter()
        .filter_map(non_empty_str)
        .map(str::to_string)
        .collect::<Vec<_>>();
    if subjects.is_empty() {
        anyhow::bail!("filter.subject array must include at least one non-empty string");
    }
    Ok(subjects)
}

fn ledger_path_from_config() -> PathBuf {
    DaemonConfig::load(&default_config_path())
        .map(|config| config.billing_dir().join("invocations.redb"))
        .unwrap_or_else(|_| default_billing_dir().join("invocations.redb"))
}

fn ledger_resource_ura() -> Option<String> {
    let local = crate::persistence::local_agents::load().ok()?;
    let parsed = crate::ura::parse_ura(&local.host_device_agent_ura).ok()?;
    let owner = match parsed.kind {
        crate::ura::URAKind::Device => format!("device.{}", parsed.device_id),
        crate::ura::URAKind::User => format!("{}.invocations", parsed.user_id),
        crate::ura::URAKind::Agent => format!("{}.invocations", parsed.user_id),
        _ => return None,
    };
    Some(crate::ura::resource_dot_ura(
        &parsed.realm,
        &owner,
        "billing/invocations",
    ))
}

fn fetch_records_from_path(
    path: &Path,
    query: InvocationLedgerQuery,
) -> anyhow::Result<Vec<easynet_axon::invocation::InvocationLedgerRecord>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    Ok(InvocationLedger::open(path)?.fetch(query)?)
}

fn fetch_one_from_path(
    path: &Path,
    query: InvocationLedgerQuery,
) -> anyhow::Result<Option<easynet_axon::invocation::InvocationLedgerRecord>> {
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

pub fn list_history_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "limit": { "type": "integer", "minimum": 1, "maximum": MAX_LIMIT },
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
            "caller": { "type": "string", "description": "Caller URA." },
            "callee": { "type": "string", "description": "Callee URA." },
            "subject": {
                "description": "One subject URA or an array of subject URAs.",
                "oneOf": [
                    { "type": "string" },
                    {
                        "type": "array",
                        "items": { "type": "string" },
                        "minItems": 1,
                        "uniqueItems": true
                    }
                ]
            },
            "ability": { "type": "string", "description": "Ability name or ability URA." },
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
        let mut reg = LocalAbilityRegistry::new();
        register(&mut reg, None);
        assert!(reg.get_rpc(ABILITY_HISTORY_LIST).is_some());
        assert!(reg.get_rpc(ABILITY_HISTORY_GET).is_some());
        assert!(reg.get_rpc(ABILITY_TRACE_GET).is_some());
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
    fn query_from_args_uses_key_and_filter_objects() {
        let query = query_from_args(&json!({
            "key": { "ura": "easynet:///r/test/resource/alice.invocations/req-test" },
            "filter": {
                "caller": "easynet:///r/test/device/caller",
                "callee": "easynet:///r/test/device/callee",
                "subject": "easynet:///r/test/user/alice",
                "ability": "device.observe.health",
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
        assert!(query.subject_uras.contains("easynet:///r/test/user/alice"));
        assert_eq!(query.ability_name.as_deref(), Some("device.observe.health"));
    }

    #[test]
    fn query_from_args_accepts_subject_array() {
        let query = query_from_args(&json!({
            "filter": {
                "subject": [
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
            .invocation_ura(crate::ura::resource_dot_ura(
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
            .ability_ura("easynet:///r/test/ability/hub.device.observe.health".to_string())
            .ability_name("device.observe.health".to_string())
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
}
