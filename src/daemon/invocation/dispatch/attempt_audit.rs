// EasyNet CLI - Invocation attempt audit
// ======================================
//
// File: src/daemon/invocation/dispatch/attempt_audit.rs
// Description: Transport-boundary audit for daemon Invocation attempts that
//              may be rejected before Axon mints a canonical invocation id.
//
// Protocol Responsibility
// -----------------------
// Axon's InvocationLedger remains the canonical receipt ledger once runtime
// admission starts. This module records the earlier attempt lifecycle:
// malformed target, route miss, authority rejection, and runtime-admission
// failure. It stores no raw arguments or result bytes.
//
// Implementation Approach
// -----------------------
// A small JSONL ledger is append-only and newest-first on read. The writer is
// protected by a process-local mutex because this is an audit sidecar, not a
// high-throughput execution queue.
//
// Usage Contract
// --------------
// Start an attempt at the Invocation transport entrance, then finalize exactly
// once on every terminal path. Runtime-started attempts link to Axon's
// invocation id; pre-runtime rejected attempts carry status diagnostics.
//
// Architectural Position
// ----------------------
// Transport boundary -> attempt audit -> Axon runtime. This module does not
// own runtime admission, route selection, or receipt verification.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
#[cfg(feature = "axon-pb")]
use tonic::Status;

#[cfg(feature = "axon-pb")]
use axon_sdk::invocation::InvocationState;
#[cfg(feature = "axon-pb")]
use axon_sdk::pb::axon::v1::{
    Envelope, EnvelopeOpen, InvocationTarget, InvokeRequest, InvokeResponse,
    InvokeServerStreamRequest,
};

static ATTEMPT_COUNTER: AtomicU64 = AtomicU64::new(1);

const MAX_ATTEMPT_READ_LINES: usize = 20_000;

#[derive(Debug, Clone)]
pub(crate) struct InvocationAttemptLedger {
    path: PathBuf,
    writer: Arc<Mutex<()>>,
}

impl InvocationAttemptLedger {
    pub(crate) fn open(path: impl Into<PathBuf>) -> anyhow::Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        OpenOptions::new().create(true).append(true).open(&path)?;
        Ok(Self {
            path,
            writer: Arc::new(Mutex::new(())),
        })
    }

    #[cfg(feature = "axon-pb")]
    pub(crate) fn begin_invoke(
        &self,
        request: &InvokeRequest,
    ) -> anyhow::Result<InvocationAttemptHandle> {
        self.begin("Invoke", AttemptIdentity::from_invoke_request(request))
    }

    #[cfg(feature = "axon-pb")]
    pub(crate) fn begin_stream(
        &self,
        request: &InvokeServerStreamRequest,
    ) -> anyhow::Result<InvocationAttemptHandle> {
        self.begin(
            "InvokeStream",
            AttemptIdentity::from_stream_request(request),
        )
    }

    pub(crate) fn begin(
        &self,
        call_mode: &str,
        identity: AttemptIdentity,
    ) -> anyhow::Result<InvocationAttemptHandle> {
        let started_unix_ms = current_unix_ms();
        let attempt_id = format!(
            "att_{started_unix_ms}_{:016x}",
            ATTEMPT_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let record = InvocationAttemptRecord {
            attempt_id: attempt_id.clone(),
            call_mode: call_mode.to_string(),
            state: AttemptState::Received,
            stage: "transport".to_string(),
            started_unix_ms,
            completed_unix_ms: None,
            elapsed_ms: None,
            invocation_ura: None,
            request_id: identity.request_id.clone(),
            trace_id: identity.trace_id.clone(),
            span_id: identity.span_id.clone(),
            caller_ura: identity.caller_ura.clone(),
            callee_ura: identity.callee_ura.clone(),
            subject_ura: identity.subject_ura.clone(),
            ability: identity.ability.clone(),
            ability_ura: identity.ability_ura.clone(),
            route_ura: None,
            execution_host_ura: None,
            status_code: None,
            status_message: None,
            error_stage: None,
            retryable: None,
            diagnostic_summary: "Request reached daemon invocation transport.".to_string(),
            suggested_action: "If this row remains non-terminal, inspect daemon transport logs."
                .to_string(),
        };
        self.append(&record)?;
        Ok(InvocationAttemptHandle {
            ledger: Arc::new(self.clone()),
            attempt_id,
            started_unix_ms,
            call_mode: call_mode.to_string(),
            identity,
        })
    }

    pub(crate) fn finalize(&self, record: InvocationAttemptRecord) -> anyhow::Result<()> {
        self.append(&record)
    }

    pub(crate) fn list_recent(&self, limit: usize) -> anyhow::Result<Vec<InvocationAttemptRecord>> {
        let file = match File::open(&self.path) {
            Ok(file) => file,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(err.into()),
        };
        let mut records = Vec::new();
        for (index, line) in BufReader::new(file)
            .lines()
            .take(MAX_ATTEMPT_READ_LINES)
            .enumerate()
        {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let record = serde_json::from_str::<InvocationAttemptRecord>(&line).map_err(|err| {
                anyhow::anyhow!(
                    "decode invocation attempt ledger row {} from {}: {err}",
                    index + 1,
                    self.path.display()
                )
            })?;
            records.push(record);
        }
        let mut coalesced = BTreeMap::<String, InvocationAttemptRecord>::new();
        for record in records {
            coalesced
                .entry(record.attempt_id.clone())
                .and_modify(|existing| merge_attempt_record(existing, &record))
                .or_insert(record);
        }
        let mut records = coalesced.into_values().collect::<Vec<_>>();
        records.sort_by(|a, b| {
            b.started_unix_ms
                .cmp(&a.started_unix_ms)
                .then_with(|| b.attempt_id.cmp(&a.attempt_id))
        });
        if limit > 0 && records.len() > limit {
            records.truncate(limit);
        }
        Ok(records)
    }

    fn append(&self, record: &InvocationAttemptRecord) -> anyhow::Result<()> {
        let _guard = self
            .writer
            .lock()
            .map_err(|_| anyhow::anyhow!("invocation attempt ledger writer lock poisoned"))?;
        let line = serde_json::to_string(record)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|err| {
                anyhow::anyhow!("open invocation attempt ledger append handle: {err}")
            })?;
        writeln!(file, "{line}")
            .map_err(|err| anyhow::anyhow!("append invocation attempt ledger row: {err}"))?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub(crate) struct InvocationAttemptHandle {
    ledger: Arc<InvocationAttemptLedger>,
    attempt_id: String,
    started_unix_ms: i64,
    call_mode: String,
    identity: AttemptIdentity,
}

impl InvocationAttemptHandle {
    #[cfg(feature = "axon-pb")]
    pub(crate) fn reject_status(&self, stage: &str, status: &Status) -> anyhow::Result<()> {
        self.reject_diagnostic(
            stage,
            status.code().to_string(),
            status.message().to_string(),
            Some(is_retryable_status(status)),
        )
    }

    pub(crate) fn reject_diagnostic(
        &self,
        stage: &str,
        status_code: impl Into<String>,
        status_message: impl Into<String>,
        retryable: Option<bool>,
    ) -> anyhow::Result<()> {
        let status_code = status_code.into();
        let status_message = status_message.into();
        self.finish(FinishAttempt {
            state: AttemptState::Rejected,
            stage,
            invocation_ura: None,
            status_code: Some(status_code.clone()).filter(|value| !value.is_empty()),
            status_message: Some(status_message.clone()).filter(|value| !value.is_empty()),
            error_stage: Some(stage.to_string()),
            retryable,
            diagnostic_summary: if status_message.is_empty() {
                format!("{stage}: rejected")
            } else {
                format!("{stage}: {status_message}")
            },
            suggested_action: suggested_action(stage, &status_code),
        })
    }

    pub(crate) fn with_identity(mut self, identity: AttemptIdentity) -> Self {
        self.identity = identity;
        self
    }

    pub(crate) fn mark_started(&self, stage: &str) -> anyhow::Result<()> {
        self.finish(FinishAttempt {
            state: AttemptState::RuntimeStarted,
            stage,
            invocation_ura: None,
            status_code: None,
            status_message: None,
            error_stage: None,
            retryable: None,
            diagnostic_summary: format!("{stage}: stream opened"),
            suggested_action:
                "Stream accepted; inspect stream/session lifecycle for terminal state.".to_string(),
        })
    }

    #[cfg(feature = "axon-pb")]
    pub(crate) fn finalize_response(
        &self,
        stage: &str,
        response: &InvokeResponse,
    ) -> anyhow::Result<()> {
        let error = response.error.as_ref();
        let status_code = error
            .map(|error| error.code.clone())
            .filter(|s| !s.is_empty());
        let status_message = error
            .map(|error| error.message.clone())
            .filter(|s| !s.is_empty());
        let error_stage = error
            .map(|error| format!("{:?}", error.stage()))
            .filter(|s| !s.is_empty());
        let retryable = error.map(|error| error.retryable);
        let invocation_ura = response
            .header
            .as_ref()
            .map(|header| header.request_id.trim().to_string())
            .filter(|value| !value.is_empty());
        let state_projection = RuntimeResponseStateProjection::from_wire_state(
            response.state,
            invocation_ura.as_deref(),
        );
        let status_code = status_code.or_else(|| {
            state_projection
                .protocol_error
                .as_ref()
                .map(|_| "PROTOCOL_MISMATCH".to_string())
        });
        let status_message = status_message.or_else(|| state_projection.protocol_error.clone());
        let error_stage = error_stage.or_else(|| {
            state_projection
                .protocol_error
                .as_ref()
                .map(|_| "protocol_decode".to_string())
        });
        let retryable =
            retryable.or_else(|| state_projection.protocol_error.as_ref().map(|_| false));
        let diagnostic_summary = match status_message.as_deref() {
            Some(message) if !message.is_empty() => format!("{stage}: {message}"),
            _ => format!("{stage}: invocation {}", state_projection.state_name),
        };
        let suggested_code = status_code.as_deref().unwrap_or("IN_BAND").to_string();
        self.finish(FinishAttempt {
            state: state_projection.attempt_state,
            stage,
            invocation_ura,
            status_code,
            status_message,
            error_stage,
            retryable,
            diagnostic_summary,
            suggested_action: suggested_action(stage, &suggested_code),
        })
    }

    fn finish(&self, finish: FinishAttempt<'_>) -> anyhow::Result<()> {
        let completed_unix_ms = current_unix_ms();
        self.ledger.finalize(InvocationAttemptRecord {
            attempt_id: self.attempt_id.clone(),
            call_mode: self.call_mode.clone(),
            state: finish.state,
            stage: finish.stage.to_string(),
            started_unix_ms: self.started_unix_ms,
            completed_unix_ms: Some(completed_unix_ms),
            elapsed_ms: Some((completed_unix_ms - self.started_unix_ms).max(0) as u64),
            invocation_ura: finish.invocation_ura,
            request_id: self.identity.request_id.clone(),
            trace_id: self.identity.trace_id.clone(),
            span_id: self.identity.span_id.clone(),
            caller_ura: self.identity.caller_ura.clone(),
            callee_ura: self.identity.callee_ura.clone(),
            subject_ura: self.identity.subject_ura.clone(),
            ability: self.identity.ability.clone(),
            ability_ura: self.identity.ability_ura.clone(),
            route_ura: None,
            execution_host_ura: None,
            status_code: finish.status_code,
            status_message: finish.status_message,
            error_stage: finish.error_stage,
            retryable: finish.retryable,
            diagnostic_summary: finish.diagnostic_summary,
            suggested_action: finish.suggested_action,
        })
    }
}

struct FinishAttempt<'a> {
    state: AttemptState,
    stage: &'a str,
    invocation_ura: Option<String>,
    status_code: Option<String>,
    status_message: Option<String>,
    error_stage: Option<String>,
    retryable: Option<bool>,
    diagnostic_summary: String,
    suggested_action: String,
}

#[cfg(feature = "axon-pb")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct RuntimeResponseStateProjection {
    state_name: String,
    attempt_state: AttemptState,
    protocol_error: Option<String>,
}

#[cfg(feature = "axon-pb")]
impl RuntimeResponseStateProjection {
    fn from_wire_state(raw_state: i32, invocation_ura: Option<&str>) -> Self {
        match InvocationState::try_from(raw_state) {
            Ok(state) => {
                let state_name = state.as_str().to_string();
                let attempt_state = match state_name.as_str() {
                    "completed" => AttemptState::RuntimeCompleted,
                    "failed" | "timed_out" | "cancelled" if invocation_ura.is_none() => {
                        AttemptState::RuntimeRejected
                    }
                    "failed" | "timed_out" | "cancelled" => AttemptState::RuntimeFailed,
                    _ => AttemptState::RuntimeStarted,
                };
                Self {
                    state_name,
                    attempt_state,
                    protocol_error: None,
                }
            }
            Err(_) => Self {
                state_name: format!("invalid({raw_state})"),
                attempt_state: AttemptState::RuntimeFailed,
                protocol_error: Some(format!(
                    "runtime response carried invalid invocation state {raw_state}"
                )),
            },
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct AttemptIdentity {
    request_id: Option<String>,
    trace_id: Option<String>,
    span_id: Option<String>,
    caller_ura: Option<String>,
    callee_ura: Option<String>,
    subject_ura: Option<String>,
    ability: Option<String>,
    ability_ura: Option<String>,
}

impl AttemptIdentity {
    pub(crate) fn pending_bidi_open() -> Self {
        Self::empty()
    }

    fn empty() -> Self {
        Self {
            request_id: None,
            trace_id: None,
            span_id: None,
            caller_ura: None,
            callee_ura: None,
            subject_ura: None,
            ability: None,
            ability_ura: None,
        }
    }

    #[cfg(test)]
    fn for_test_without_tuple() -> Self {
        Self::empty()
    }

    #[cfg(feature = "axon-pb")]
    fn from_invoke_request(request: &InvokeRequest) -> Self {
        Self::from_parts("Invoke", request.envelope.as_ref(), request.target.as_ref())
    }

    #[cfg(feature = "axon-pb")]
    fn from_stream_request(request: &InvokeServerStreamRequest) -> Self {
        Self::from_parts(
            "InvokeStream",
            request.envelope.as_ref(),
            request.target.as_ref(),
        )
    }

    #[cfg(feature = "axon-pb")]
    pub(crate) fn from_bidi_open(open: &EnvelopeOpen) -> Self {
        Self::from_parts(
            "InvokeBidi frame 0",
            open.envelope.as_ref(),
            open.target.as_ref(),
        )
    }

    #[cfg(feature = "axon-pb")]
    fn from_parts(
        call_site: &str,
        envelope: Option<&Envelope>,
        target: Option<&InvocationTarget>,
    ) -> Self {
        let ability = crate::daemon::invocation::dispatch::invocation_wire::function_name_from_invocation_target(
            call_site,
            target,
        )
        .ok()
        .map(str::to_string);
        Self {
            request_id: envelope
                .map(|envelope| envelope.request_id.trim().to_string())
                .filter(|value| !value.is_empty()),
            trace_id: envelope
                .map(|envelope| envelope.trace_id.trim().to_string())
                .filter(|value| !value.is_empty()),
            span_id: envelope
                .map(|envelope| envelope.span_id.trim().to_string())
                .filter(|value| !value.is_empty()),
            caller_ura: envelope
                .and_then(|envelope| envelope.caller.as_ref())
                .map(|caller| caller.ura.trim().to_string())
                .filter(|value| !value.is_empty()),
            callee_ura: envelope
                .and_then(|envelope| envelope.callee.as_ref())
                .map(|callee| callee.ura.trim().to_string())
                .filter(|value| !value.is_empty()),
            subject_ura: envelope
                .and_then(|envelope| envelope.subject.as_ref())
                .map(|subject| subject.ura.trim().to_string())
                .filter(|value| !value.is_empty()),
            ability,
            ability_ura: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AttemptState {
    Received,
    Rejected,
    RuntimeStarted,
    RuntimeRejected,
    RuntimeCompleted,
    RuntimeFailed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InvocationAttemptRecord {
    pub(crate) attempt_id: String,
    pub(crate) call_mode: String,
    pub(crate) state: AttemptState,
    pub(crate) stage: String,
    pub(crate) started_unix_ms: i64,
    pub(crate) completed_unix_ms: Option<i64>,
    pub(crate) elapsed_ms: Option<u64>,
    pub(crate) invocation_ura: Option<String>,
    pub(crate) request_id: Option<String>,
    pub(crate) trace_id: Option<String>,
    pub(crate) span_id: Option<String>,
    pub(crate) caller_ura: Option<String>,
    pub(crate) callee_ura: Option<String>,
    pub(crate) subject_ura: Option<String>,
    pub(crate) ability: Option<String>,
    pub(crate) ability_ura: Option<String>,
    pub(crate) route_ura: Option<String>,
    pub(crate) execution_host_ura: Option<String>,
    pub(crate) status_code: Option<String>,
    pub(crate) status_message: Option<String>,
    pub(crate) error_stage: Option<String>,
    pub(crate) retryable: Option<bool>,
    pub(crate) diagnostic_summary: String,
    pub(crate) suggested_action: String,
}

impl InvocationAttemptRecord {
    pub(crate) fn diagnostic_value(&self) -> Value {
        json!({
            "record_kind": "attempt",
            "attempt_id": self.attempt_id,
            "invocation_ura": self.invocation_ura,
            "request_id": self.request_id,
            "trace_id": self.trace_id,
            "span_id": self.span_id,
            "state": self.state,
            "stage": self.stage,
            "ability": self.ability,
            "ability_ura": self.ability_ura,
            "caller_ura": self.caller_ura,
            "callee_ura": self.callee_ura,
            "subject_ura": self.subject_ura,
            "started_unix_ms": self.started_unix_ms,
            "completed_unix_ms": self.completed_unix_ms,
            "elapsed_ms": self.elapsed_ms,
            "status_code": self.status_code,
            "status_message": self.status_message,
            "error_stage": self.error_stage,
            "retryable": self.retryable,
            "diagnostic": {
                "summary": self.diagnostic_summary,
                "suggested_action": self.suggested_action,
                "route_ura": self.route_ura,
                "execution_host_ura": self.execution_host_ura,
            }
        })
    }
}

pub(crate) fn attempt_ledger_path(ledger_dir: &Path) -> PathBuf {
    ledger_dir.join("invocation-attempts.jsonl")
}

fn current_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or_default()
}

fn merge_attempt_record(
    existing: &mut InvocationAttemptRecord,
    incoming: &InvocationAttemptRecord,
) {
    let incoming_is_newer = incoming
        .completed_unix_ms
        .or(Some(incoming.started_unix_ms))
        >= existing
            .completed_unix_ms
            .or(Some(existing.started_unix_ms));
    if incoming_is_newer {
        let previous = existing.clone();
        *existing = incoming.clone();
        fill_missing_identity(existing, &previous);
    } else {
        fill_missing_identity(existing, incoming);
    }
}

fn fill_missing_identity(target: &mut InvocationAttemptRecord, source: &InvocationAttemptRecord) {
    if target.call_mode.is_empty() {
        target.call_mode.clone_from(&source.call_mode);
    }
    fill_option(&mut target.invocation_ura, &source.invocation_ura);
    fill_option(&mut target.request_id, &source.request_id);
    fill_option(&mut target.trace_id, &source.trace_id);
    fill_option(&mut target.span_id, &source.span_id);
    fill_option(&mut target.caller_ura, &source.caller_ura);
    fill_option(&mut target.callee_ura, &source.callee_ura);
    fill_option(&mut target.subject_ura, &source.subject_ura);
    fill_option(&mut target.ability, &source.ability);
    fill_option(&mut target.ability_ura, &source.ability_ura);
    fill_option(&mut target.route_ura, &source.route_ura);
    fill_option(&mut target.execution_host_ura, &source.execution_host_ura);
}

fn fill_option(target: &mut Option<String>, source: &Option<String>) {
    if target.as_deref().unwrap_or_default().is_empty() {
        *target = source.clone();
    }
}

#[cfg(feature = "axon-pb")]
fn is_retryable_status(status: &Status) -> bool {
    matches!(
        status.code(),
        tonic::Code::Unavailable
            | tonic::Code::DeadlineExceeded
            | tonic::Code::ResourceExhausted
            | tonic::Code::Aborted
    )
}

fn suggested_action(stage: &str, code: &str) -> String {
    match stage {
        _ if code.eq_ignore_ascii_case("PROTOCOL_MISMATCH") => {
            "Check Axon runtime/daemon protocol schema parity and regenerated protobuf bindings."
                .to_string()
        }
        "target" => {
            "Check the typed InvocationTarget and descriptor-bound ability reference.".to_string()
        }
        "routing" => "Check ability publication, presence, and namespace.resolve route projection."
            .to_string(),
        "admission" | "runtime_admission" | "daemon_route_ingress" => {
            "Check caller/callee/subject URA binding, session authority scope, and descriptor ref."
                .to_string()
        }
        _ if code.eq_ignore_ascii_case("unavailable") => {
            "Check selected execution host liveness and session.open dispatch connectivity."
                .to_string()
        }
        _ => {
            "Open the diagnostic details and inspect the stage-specific status message.".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invocation_attempt_audit_appends_and_reads_newest_first() {
        let temp = tempfile::tempdir().expect("tempdir");
        let ledger = InvocationAttemptLedger::open(temp.path().join("attempts.jsonl"))
            .expect("attempt ledger");
        let first = ledger
            .begin("Invoke", AttemptIdentity::for_test_without_tuple())
            .expect("begin first attempt");
        first
            .reject_diagnostic("target", "invalid_argument", "bad target", None)
            .expect("finish first attempt");
        let second = ledger
            .begin("Invoke", AttemptIdentity::for_test_without_tuple())
            .expect("begin second attempt");
        second
            .reject_diagnostic("routing", "not_found", "missing route", None)
            .expect("finish second attempt");

        let records = ledger.list_recent(10).expect("read attempts");
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].state, AttemptState::Rejected);
        assert_eq!(records[0].stage, "routing");
        assert!(
            records[0].diagnostic_value()["diagnostic"]["suggested_action"]
                .as_str()
                .unwrap()
                .contains("ability publication")
        );
    }

    #[test]
    fn invocation_attempt_audit_rejects_corrupt_rows() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("attempts.jsonl");
        std::fs::write(&path, "{not-json}\n").expect("write corrupt attempt ledger");
        let ledger = InvocationAttemptLedger::open(&path).expect("attempt ledger");

        let error = ledger
            .list_recent(10)
            .expect_err("corrupt attempt row must fail closed");
        assert!(
            error
                .to_string()
                .contains("decode invocation attempt ledger row 1"),
            "{error:#}"
        );
    }

    #[test]
    fn invocation_attempt_audit_rejects_unknown_row_fields() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("attempts.jsonl");
        let row = serde_json::json!({
            "attempt_id": "attempt-1",
            "call_mode": "Invoke",
            "state": "received",
            "stage": "ingress",
            "started_unix_ms": 1,
            "completed_unix_ms": null,
            "elapsed_ms": null,
            "invocation_ura": null,
            "request_id": null,
            "trace_id": null,
            "span_id": null,
            "caller_ura": null,
            "callee_ura": null,
            "subject_ura": null,
            "ability": null,
            "ability_ura": null,
            "route_ura": null,
            "execution_host_ura": null,
            "status_code": null,
            "status_message": null,
            "error_stage": null,
            "retryable": null,
            "diagnostic_summary": "received invocation",
            "suggested_action": "none",
            "state_code": "legacy"
        });
        std::fs::write(&path, format!("{row}\n")).expect("write drifted attempt ledger");
        let ledger = InvocationAttemptLedger::open(&path).expect("attempt ledger");

        let error = ledger
            .list_recent(10)
            .expect_err("attempt ledger row must reject read-model drift");

        assert!(
            error.to_string().contains("state_code"),
            "decode error should name the noncanonical field: {error:#}"
        );
    }

    #[cfg(feature = "axon-pb")]
    #[test]
    fn invocation_attempt_audit_projects_invalid_runtime_state_as_protocol_mismatch() {
        let temp = tempfile::tempdir().expect("tempdir");
        let ledger = InvocationAttemptLedger::open(temp.path().join("attempts.jsonl"))
            .expect("attempt ledger");
        let attempt = ledger
            .begin("Invoke", AttemptIdentity::for_test_without_tuple())
            .expect("begin attempt");

        attempt
            .finalize_response(
                "runtime_admission",
                &InvokeResponse {
                    state: i32::MAX,
                    ..Default::default()
                },
            )
            .expect("finalize invalid runtime state");

        let records = ledger.list_recent(1).expect("read attempts");
        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(record.state, AttemptState::RuntimeFailed);
        assert_eq!(record.status_code.as_deref(), Some("PROTOCOL_MISMATCH"));
        assert_eq!(record.error_stage.as_deref(), Some("protocol_decode"));
        assert_eq!(record.retryable, Some(false));
        assert!(
            record
                .status_message
                .as_deref()
                .unwrap_or_default()
                .contains("invalid invocation state"),
            "{record:#?}"
        );
        assert!(
            record
                .diagnostic_summary
                .contains("runtime response carried invalid invocation state"),
            "{record:#?}"
        );
        assert!(
            record.suggested_action.contains("protocol schema parity"),
            "{record:#?}"
        );
    }
}
