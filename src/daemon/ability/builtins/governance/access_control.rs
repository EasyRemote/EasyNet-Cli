// EasyNet CLI — RFC-014 access-control governance abilities
// ===========================================================
//
// File: src/daemon/ability/builtins/governance/access_control.rs
// Description: Daemon-owned system abilities for PermissionGrant,
//              PermissionRequest, and admission explain projections.
//
// Protocol Responsibility:
// Expose RFC-014 policy management through governed abilities without
// creating a standalone policy CLI or leaking keyring material into policy.
//
// Implementation Approach:
// Handlers are thin adapters over `persistence::access_control` and
// `PolicyEngine`. They parse typed JSON, call the lower-layer domain service,
// and return typed DTOs.
//
// Usage Contract:
// SDKs should call these names through typed clients. Product code must not
// hand-build governance payloads.
//
// Architectural Position:
// Built-in governance ability surface on top of daemon admission policy state.

use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::daemon::ability::catalog::system_manifest::registry_manifest;
use crate::daemon::ability::dispatch::{AxonAbilityCatalog, OwnerKind};
use crate::daemon::invocation::admission::decision::{
    AbilityCallTrace, AdmissionExplainResult, OwnerResolution, OwnerSource,
    PermissionRequestStatus, RedactionReason, TraceStage,
};
use crate::daemon::invocation::admission::policy_engine::{PolicyEngine, PolicyInput};
use crate::daemon::persistence::access_control::AccessControlStore;
use easynet_axon::invocation::{InvocationLedger, InvocationLedgerFetchKey, InvocationLedgerQuery};

pub const AUTHORITY_BINDING_GRANT: &str =
    crate::daemon::ability::names::governance::AUTHORITY_BINDING_GRANT;
pub const AUTHORITY_BINDING_REVOKE: &str =
    crate::daemon::ability::names::governance::AUTHORITY_BINDING_REVOKE;
pub const AUTHORITY_BINDING_LIST: &str =
    crate::daemon::ability::names::governance::AUTHORITY_BINDING_LIST;
pub const AUTHORITY_BINDING_CHECK: &str =
    crate::daemon::ability::names::governance::AUTHORITY_BINDING_CHECK;
pub const POLICY_REQUEST_CREATE: &str =
    crate::daemon::ability::names::governance::POLICY_REQUEST_CREATE;
pub const POLICY_REQUEST_RESOLVE: &str =
    crate::daemon::ability::names::governance::POLICY_REQUEST_RESOLVE;
pub const POLICY_REQUEST_LIST: &str =
    crate::daemon::ability::names::governance::POLICY_REQUEST_LIST;
pub const ADMISSION_EXPLAIN: &str = crate::daemon::ability::names::governance::ADMISSION_EXPLAIN;

pub fn register(reg: &mut AxonAbilityCatalog) {
    register_with_ledger(reg, None);
}

pub fn register_with_ledger(reg: &mut AxonAbilityCatalog, ledger: Option<Arc<InvocationLedger>>) {
    for ability in [
        AUTHORITY_BINDING_GRANT,
        AUTHORITY_BINDING_REVOKE,
        AUTHORITY_BINDING_LIST,
        AUTHORITY_BINDING_CHECK,
        POLICY_REQUEST_CREATE,
        POLICY_REQUEST_RESOLVE,
        POLICY_REQUEST_LIST,
    ] {
        let handler = match ability {
            AUTHORITY_BINDING_GRANT => grant_handler,
            AUTHORITY_BINDING_REVOKE => revoke_handler,
            AUTHORITY_BINDING_LIST => list_grants_handler,
            AUTHORITY_BINDING_CHECK => check_handler,
            POLICY_REQUEST_CREATE => request_create_handler,
            POLICY_REQUEST_RESOLVE => request_resolve_handler,
            POLICY_REQUEST_LIST => request_list_handler,
            _ => unreachable!("static RFC-014 ability list"),
        };
        reg.register_rpc_with_spec(
            ability,
            OwnerKind::Device,
            registry_manifest(ability, description_for(ability), input_schema_for(ability)),
            std::sync::Arc::new(handler),
        );
    }

    let reader = Arc::new(AdmissionExplainReader { ledger });
    reg.register_rpc_with_spec(
        ADMISSION_EXPLAIN,
        OwnerKind::Device,
        registry_manifest(
            ADMISSION_EXPLAIN,
            description_for(ADMISSION_EXPLAIN),
            input_schema_for(ADMISSION_EXPLAIN),
        ),
        Arc::new(move |args| reader.explain(args)),
    );
}

fn grant_handler(args: Value) -> anyhow::Result<Value> {
    let request: GrantRequest = serde_json::from_value(args)?;
    let actor_ura = request
        .actor_ura
        .clone()
        .unwrap_or_else(|| request.grant.created_by.clone());
    let mut store = AccessControlStore::open_or_create(request.grant.owner_user_id.clone())?;
    let result = store.create_grant(request.grant, &actor_ura)?;
    Ok(serde_json::to_value(result)?)
}

fn revoke_handler(args: Value) -> anyhow::Result<Value> {
    let request: RevokeRequest = serde_json::from_value(args)?;
    let actor_ura = request
        .actor_ura
        .as_deref()
        .unwrap_or(request.owner_user_id.as_str());
    let mut store = AccessControlStore::open_or_create(request.owner_user_id.clone())?;
    let grant = store.revoke_grant(
        &request.grant_id,
        &request.owner_user_id,
        actor_ura,
        request.reason,
    )?;
    Ok(json!({ "grant": grant }))
}

fn list_grants_handler(args: Value) -> anyhow::Result<Value> {
    let request: ListGrantsRequest = serde_json::from_value(args)?;
    let store = AccessControlStore::open_or_create(request.owner_user_id)?;
    let mut grants = store.grants();
    if let Some(principal_id) = request.principal_id {
        grants.retain(|grant| grant.principal_id == principal_id);
    }
    if let Some(token_id) = request.token_id {
        grants.retain(|grant| grant.token_id.as_deref() == Some(token_id.as_str()));
    }
    if let Some(callee_ura) = request.callee_ura {
        grants.retain(|grant| grant.callee_ura.as_deref() == Some(callee_ura.as_str()));
    }
    if let Some(ability_ura_pattern) = request.ability_ura_pattern {
        grants.retain(|grant| {
            grant.ability_ura_pattern.as_deref() == Some(ability_ura_pattern.as_str())
        });
    }
    if let Some(subject_ura_pattern) = request.subject_ura_pattern {
        grants.retain(|grant| {
            grant.subject_ura_pattern.as_deref() == Some(subject_ura_pattern.as_str())
        });
    }
    if let Some(action) = request.action {
        grants.retain(|grant| grant.actions.contains(&action));
    }
    if let Some(effect) = request.effect {
        grants.retain(|grant| grant.effect == effect);
    }
    if let Some(state) = request.state {
        grants.retain(|grant| grant.state == state);
    }
    Ok(json!({ "grants": grants }))
}

fn check_handler(args: Value) -> anyhow::Result<Value> {
    let request: CheckRequest = serde_json::from_value(args)?;
    let store = AccessControlStore::open_or_create(request.owner_user_id.clone())?;
    let owner = OwnerResolution {
        owner_user_id: Some(request.owner_user_id.clone()),
        owner_ura: request.owner_ura,
        owner_source: request.owner_source.unwrap_or(OwnerSource::Subject),
        audit_warnings: vec![],
    };
    let decision = PolicyEngine::check(PolicyInput {
        owner,
        caller_user_id: request.caller_user_id,
        caller_ura: request.caller_ura,
        principal_kind: request.principal_kind,
        principal_id: request.principal_id,
        token_id: request.token_id,
        token_class: request.token_class,
        callee_ura: request.callee_ura,
        subject_ura: request.subject_ura,
        ability_ura: request.ability_ura,
        action: request.action,
        safe_read: request.safe_read,
        interactive_context_available: request.interactive_context_available,
        canonical_hash: request.canonical_hash,
        signature_key_id: request.signature_key_id,
        verified_authority_id: request.authority_proof_id,
        rejector_ura: request.rejector_ura,
        now: chrono::Utc::now(),
        grants: store.grants(),
    });
    Ok(json!({ "policy_decision": decision }))
}

fn request_create_handler(args: Value) -> anyhow::Result<Value> {
    let request: PermissionRequestEnvelope = serde_json::from_value(args)?;
    let actor_ura = request
        .actor_ura
        .clone()
        .unwrap_or_else(|| request.request.caller_ura.clone());
    let mut store = AccessControlStore::open_or_create(request.request.owner_user_id.clone())?;
    let request = store.upsert_permission_request(request.request, &actor_ura)?;
    Ok(json!({ "request": request }))
}

fn request_resolve_handler(args: Value) -> anyhow::Result<Value> {
    let request: PermissionRequestResolutionEnvelope = serde_json::from_value(args)?;
    let actor_ura = request
        .actor_ura
        .clone()
        .unwrap_or_else(|| request.request.resolver_ura.clone().unwrap_or_default());
    let mut store = AccessControlStore::open_or_create(request.request.owner_user_id.clone())?;
    if request.request.status == PermissionRequestStatus::Approved {
        if let Some(grant) = request.created_grant {
            let result =
                store.resolve_permission_request_with_grant(request.request, grant, &actor_ura)?;
            return Ok(serde_json::to_value(result)?);
        }
        if let Some(proof) = request.authority_proof {
            let result = store.resolve_permission_request_with_authority_proof(
                request.request,
                proof,
                &actor_ura,
            )?;
            return Ok(serde_json::to_value(result)?);
        }
    }
    let request = store.resolve_permission_request(request.request, &actor_ura)?;
    Ok(json!({ "request": request }))
}

fn request_list_handler(args: Value) -> anyhow::Result<Value> {
    let request: ListRequestsRequest = serde_json::from_value(args)?;
    let created_at = parse_creation_time_filter(request.created_at.as_deref(), "created_at")?;
    let created_at_or_after = parse_creation_time_filter(
        request.created_at_or_after.as_deref(),
        "created_at_or_after",
    )?;
    let created_at_or_before = parse_creation_time_filter(
        request.created_at_or_before.as_deref(),
        "created_at_or_before",
    )?;
    if let (Some(after), Some(before)) = (created_at_or_after, created_at_or_before) {
        if after > before {
            anyhow::bail!("created_at_or_after must not be after created_at_or_before");
        }
    }
    let store = AccessControlStore::open_or_create(request.owner_user_id)?;
    let mut requests = store.requests();
    if let Some(principal_id) = request.principal_id {
        requests.retain(|item| item.principal_id == principal_id);
    }
    if let Some(token_id) = request.token_id {
        requests.retain(|item| item.token_id.as_deref() == Some(token_id.as_str()));
    }
    if let Some(status) = request.status {
        requests.retain(|item| item.status == status);
    }
    if let Some(callee_ura) = request.callee_ura {
        requests.retain(|item| item.callee_ura == callee_ura);
    }
    if let Some(ability_ura) = request.ability_ura {
        requests.retain(|item| item.ability_ura == ability_ura);
    }
    if let Some(subject_ura) = request.subject_ura {
        requests.retain(|item| item.subject_ura == subject_ura);
    }
    if created_at.is_some() || created_at_or_after.is_some() || created_at_or_before.is_some() {
        requests.retain(|item| {
            creation_time_matches(
                &item.created_at,
                created_at,
                created_at_or_after,
                created_at_or_before,
            )
        });
    }
    Ok(json!({ "requests": requests }))
}

fn parse_creation_time_filter(
    value: Option<&str>,
    field_name: &str,
) -> anyhow::Result<Option<DateTime<Utc>>> {
    value
        .map(|raw| {
            DateTime::parse_from_rfc3339(raw)
                .map(|timestamp| timestamp.with_timezone(&Utc))
                .map_err(|_| anyhow::anyhow!("{field_name} must be RFC3339"))
        })
        .transpose()
}

fn creation_time_matches(
    created_at: &str,
    exact: Option<DateTime<Utc>>,
    at_or_after: Option<DateTime<Utc>>,
    at_or_before: Option<DateTime<Utc>>,
) -> bool {
    let Ok(created_at) =
        DateTime::parse_from_rfc3339(created_at).map(|timestamp| timestamp.with_timezone(&Utc))
    else {
        return false;
    };
    if exact.is_some_and(|exact| created_at != exact) {
        return false;
    }
    if at_or_after.is_some_and(|lower_bound| created_at < lower_bound) {
        return false;
    }
    if at_or_before.is_some_and(|upper_bound| created_at > upper_bound) {
        return false;
    }
    true
}

struct AdmissionExplainReader {
    ledger: Option<Arc<InvocationLedger>>,
}

impl AdmissionExplainReader {
    fn explain(&self, args: Value) -> anyhow::Result<Value> {
        let request: ExplainRequest = serde_json::from_value(args)?;
        let key = request.selector()?;
        let ledger = self
            .ledger
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("admission.explain: invocation ledger unavailable"))?;
        let record = ledger
            .fetch_one(InvocationLedgerQuery::new().key(key).limit(1))?
            .ok_or_else(|| anyhow::anyhow!("admission.explain: invocation record not found"))?;

        let observer = request.observer_ura.trim();
        let observer_can_read = observer == record.caller_ura
            || observer == record.subject_ura
            || observer == record.callee_ura;
        let result = if observer_can_read {
            let authority_reason = record.error.as_ref().map(|error| {
                if error.code.trim().is_empty() {
                    error.message.clone()
                } else {
                    error.code.clone()
                }
            });
            let trace = AbilityCallTrace {
                invocation_id: record.request_id.clone(),
                parent_invocation_id: None,
                root_invocation_id: record.trace_id.clone(),
                caller_ura: record.caller_ura.clone(),
                callee_ura: record.callee_ura.clone(),
                subject_ura: record.subject_ura.clone(),
                ability_ura: record.ability_ura.clone(),
                action: access_action_for(record.ability_name.as_str()),
                route_ref: None,
                execution_host_ura: None,
                rejector_ura: None,
                stage: trace_stage_for(record.state.as_str()),
                signature_decision: None,
                policy_decision: None,
                authority_proof_id: None,
                redacted: false,
                child_failure_class: None,
                redaction_reason: None,
                children: Vec::new(),
            };
            AdmissionExplainResult {
                observer_ura: observer.to_string(),
                redacted: false,
                root_trace: Some(trace),
                signature_decision: None,
                policy_decision: None,
                authority_reason,
                route_ref: None,
                rejector_ura: None,
                redaction_reason: None,
            }
        } else {
            AdmissionExplainResult {
                observer_ura: observer.to_string(),
                redacted: true,
                root_trace: None,
                signature_decision: None,
                policy_decision: None,
                authority_reason: None,
                route_ref: None,
                rejector_ura: None,
                redaction_reason: Some(RedactionReason::SubjectPrivate),
            }
        };
        Ok(serde_json::to_value(result)?)
    }
}

fn access_action_for(
    ability: &str,
) -> crate::daemon::invocation::admission::decision::AccessAction {
    use crate::daemon::invocation::admission::decision::AccessAction;
    if ability.starts_with("terminal.")
        || ability.starts_with("remote_desktop.")
        || ability.starts_with("camera.")
        || ability.starts_with("mic.")
        || ability.starts_with("screen.")
        || ability.starts_with("voice.")
    {
        AccessAction::Stream
    } else if ability.ends_with(".list")
        || ability.ends_with(".get")
        || ability.ends_with(".read")
        || ability.starts_with("meta.")
    {
        AccessAction::Read
    } else {
        AccessAction::Invoke
    }
}

fn trace_stage_for(state: &str) -> TraceStage {
    if state.eq_ignore_ascii_case("completed") {
        TraceStage::Receipted
    } else if state.eq_ignore_ascii_case("failed") {
        TraceStage::ExecutionFailed
    } else {
        TraceStage::Executed
    }
}

#[derive(Debug, Deserialize)]
struct GrantRequest {
    grant: crate::daemon::invocation::admission::grant_matcher::PermissionGrant,
    #[serde(default)]
    actor_ura: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RevokeRequest {
    grant_id: String,
    owner_user_id: String,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    actor_ura: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ListGrantsRequest {
    owner_user_id: String,
    #[serde(default)]
    principal_id: Option<String>,
    #[serde(default)]
    token_id: Option<String>,
    #[serde(default)]
    callee_ura: Option<String>,
    #[serde(default)]
    ability_ura_pattern: Option<String>,
    #[serde(default)]
    subject_ura_pattern: Option<String>,
    #[serde(default)]
    action: Option<crate::daemon::invocation::admission::decision::AccessAction>,
    #[serde(default)]
    effect: Option<crate::daemon::invocation::admission::grant_matcher::PermissionEffect>,
    #[serde(default)]
    state: Option<crate::daemon::invocation::admission::grant_matcher::PermissionGrantState>,
}

#[derive(Debug, Deserialize)]
struct CheckRequest {
    owner_user_id: String,
    #[serde(default)]
    owner_ura: Option<String>,
    #[serde(default)]
    owner_source: Option<OwnerSource>,
    #[serde(default)]
    caller_user_id: Option<String>,
    caller_ura: String,
    principal_kind: crate::daemon::invocation::admission::decision::PrincipalKind,
    principal_id: String,
    #[serde(default)]
    token_id: Option<String>,
    #[serde(default)]
    token_class: Option<crate::daemon::invocation::admission::decision::TokenClass>,
    callee_ura: String,
    subject_ura: String,
    ability_ura: String,
    action: crate::daemon::invocation::admission::decision::AccessAction,
    #[serde(default)]
    safe_read: bool,
    #[serde(default)]
    interactive_context_available: bool,
    #[serde(default)]
    canonical_hash: Option<String>,
    #[serde(default)]
    signature_key_id: Option<String>,
    #[serde(default)]
    authority_proof_id: Option<String>,
    #[serde(default)]
    rejector_ura: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PermissionRequestEnvelope {
    request: crate::daemon::invocation::admission::decision::PermissionRequest,
    #[serde(default)]
    actor_ura: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PermissionRequestResolutionEnvelope {
    request: crate::daemon::invocation::admission::decision::PermissionRequest,
    #[serde(default)]
    created_grant: Option<crate::daemon::invocation::admission::grant_matcher::PermissionGrant>,
    #[serde(default)]
    authority_proof: Option<crate::daemon::invocation::admission::authority_proof::AuthorityProof>,
    #[serde(default)]
    actor_ura: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ListRequestsRequest {
    owner_user_id: String,
    #[serde(default)]
    principal_id: Option<String>,
    #[serde(default)]
    token_id: Option<String>,
    #[serde(default)]
    status: Option<crate::daemon::invocation::admission::decision::PermissionRequestStatus>,
    #[serde(default)]
    callee_ura: Option<String>,
    #[serde(default)]
    ability_ura: Option<String>,
    #[serde(default)]
    subject_ura: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    created_at_or_after: Option<String>,
    #[serde(default)]
    created_at_or_before: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ExplainRequest {
    observer_ura: String,
    #[serde(default)]
    invocation_id: Option<String>,
    #[serde(default)]
    request_id: Option<String>,
    #[serde(default)]
    trace_id: Option<String>,
    #[serde(default)]
    root_id: Option<String>,
}

impl ExplainRequest {
    fn selector(&self) -> anyhow::Result<InvocationLedgerFetchKey> {
        let selectors = [
            self.invocation_id.as_deref(),
            self.request_id.as_deref(),
            self.trace_id.as_deref(),
            self.root_id.as_deref(),
        ]
        .into_iter()
        .flatten()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
        if selectors.len() != 1 {
            anyhow::bail!("admission.explain: exactly one invocation_id, request_id, trace_id, or root_id is required");
        }
        let value = selectors[0].to_string();
        if self.trace_id.as_deref().is_some() || self.root_id.as_deref().is_some() {
            Ok(InvocationLedgerFetchKey::TraceId(value))
        } else if self.invocation_id.as_deref().is_some() && value.starts_with("easynet:") {
            Ok(InvocationLedgerFetchKey::InvocationUra(value))
        } else {
            Ok(InvocationLedgerFetchKey::RequestId(value))
        }
    }
}

pub fn description_for(name: &str) -> &'static str {
    match name {
        AUTHORITY_BINDING_GRANT => "Create or return an idempotent RFC-014 PermissionGrant.",
        AUTHORITY_BINDING_REVOKE => "Revoke an RFC-014 PermissionGrant monotonically.",
        AUTHORITY_BINDING_LIST => "List RFC-014 PermissionGrant records by owner and filters.",
        AUTHORITY_BINDING_CHECK => "Run RFC-014 PolicyEngine.check in read-only mode.",
        POLICY_REQUEST_CREATE => {
            "Create or return an idempotent pending RFC-014 PermissionRequest."
        }
        POLICY_REQUEST_RESOLVE => "Resolve one RFC-014 PermissionRequest terminal transition.",
        POLICY_REQUEST_LIST => "List RFC-014 PermissionRequest records by owner and filters.",
        ADMISSION_EXPLAIN => "Return an observer-scoped RFC-014 admission diagnostic projection.",
        _ => "RFC-014 access-control governance ability.",
    }
}

pub fn input_schema_for(name: &str) -> Value {
    match name {
        AUTHORITY_BINDING_GRANT => json!({
            "type": "object",
            "required": ["grant"],
            "properties": {
                "grant": {"type": "object"},
                "actor_ura": {"type": "string"}
            },
            "additionalProperties": false
        }),
        AUTHORITY_BINDING_REVOKE => json!({
            "type": "object",
            "required": ["grant_id", "owner_user_id"],
            "properties": {
                "grant_id": {"type": "string"},
                "owner_user_id": {"type": "string"},
                "reason": {"type": "string"},
                "actor_ura": {"type": "string"}
            },
            "additionalProperties": false
        }),
        AUTHORITY_BINDING_LIST => json!({
            "type": "object",
            "required": ["owner_user_id"],
            "properties": {
                "owner_user_id": {"type": "string"},
                "principal_id": {"type": "string"},
                "token_id": {"type": "string"},
                "callee_ura": {"type": "string"},
                "ability_ura_pattern": {"type": "string"},
                "subject_ura_pattern": {"type": "string"},
                "action": {"type": "string"},
                "effect": {"type": "string"},
                "state": {"type": "string"}
            },
            "additionalProperties": false
        }),
        AUTHORITY_BINDING_CHECK => json!({
            "type": "object",
            "required": ["owner_user_id", "caller_ura", "principal_kind", "principal_id", "callee_ura", "subject_ura", "ability_ura", "action"],
            "properties": {
                "owner_user_id": {"type": "string"},
                "owner_ura": {"type": "string"},
                "owner_source": {"type": "string"},
                "caller_user_id": {"type": "string"},
                "caller_ura": {"type": "string"},
                "principal_kind": {"type": "string"},
                "principal_id": {"type": "string"},
                "token_id": {"type": "string"},
                "token_class": {"type": "string"},
                "callee_ura": {"type": "string"},
                "subject_ura": {"type": "string"},
                "ability_ura": {"type": "string"},
                "action": {"type": "string"},
                "safe_read": {"type": "boolean"},
                "interactive_context_available": {"type": "boolean"},
                "canonical_hash": {"type": "string"},
                "signature_key_id": {"type": "string"},
                "authority_proof_id": {"type": "string"},
                "rejector_ura": {"type": "string"}
            },
            "additionalProperties": false
        }),
        POLICY_REQUEST_CREATE => json!({
            "type": "object",
            "required": ["request"],
            "properties": {
                "request": {"type": "object"},
                "actor_ura": {"type": "string"}
            },
            "additionalProperties": false
        }),
        POLICY_REQUEST_RESOLVE => json!({
            "type": "object",
            "required": ["request"],
            "properties": {
                "request": {"type": "object"},
                "created_grant": {"type": "object"},
                "authority_proof": {"type": "object"},
                "actor_ura": {"type": "string"}
            },
            "additionalProperties": false
        }),
        POLICY_REQUEST_LIST => json!({
            "type": "object",
            "required": ["owner_user_id"],
            "properties": {
                "owner_user_id": {"type": "string"},
                "principal_id": {"type": "string"},
                "token_id": {"type": "string"},
                "status": {"type": "string"},
                "callee_ura": {"type": "string"},
                "ability_ura": {"type": "string"},
                "subject_ura": {"type": "string"},
                "created_at": {"type": "string", "format": "date-time"},
                "created_at_or_after": {"type": "string", "format": "date-time"},
                "created_at_or_before": {"type": "string", "format": "date-time"}
            },
            "additionalProperties": false
        }),
        ADMISSION_EXPLAIN => json!({
            "type": "object",
            "required": ["observer_ura"],
            "properties": {
                "observer_ura": {"type": "string"},
                "invocation_id": {"type": "string"},
                "request_id": {"type": "string"},
                "trace_id": {"type": "string"},
                "root_id": {"type": "string"}
            },
            "additionalProperties": false
        }),
        _ => json!({"type": "object"}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::commands::test_support::HomeGuard;
    use crate::daemon::ability::dispatch::AxonAbilityCatalog;

    #[test]
    fn registration_makes_rfc014_abilities_dispatchable() {
        let mut reg = AxonAbilityCatalog::new();
        register(&mut reg);
        for ability in [
            AUTHORITY_BINDING_GRANT,
            AUTHORITY_BINDING_REVOKE,
            AUTHORITY_BINDING_LIST,
            AUTHORITY_BINDING_CHECK,
            POLICY_REQUEST_CREATE,
            POLICY_REQUEST_RESOLVE,
            POLICY_REQUEST_LIST,
            ADMISSION_EXPLAIN,
        ] {
            assert!(reg.get_rpc(ability).is_some(), "{ability} missing");
        }
    }

    #[test]
    fn admission_explain_reads_ledger_and_redacts_unrelated_observers() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ledger =
            Arc::new(InvocationLedger::open(dir.path().join("invocations.redb")).expect("ledger"));
        ledger
            .put(
                &easynet_axon::invocation::InvocationLedgerRecordBuilder::new()
                    .invocation_ura("easynet:///r/test/resource/alice.invocations/req-1")
                    .request_id("req-1")
                    .trace_id("trace-1")
                    .span_id("span-1")
                    .caller_ura("easynet:///r/test/hub")
                    .callee_ura("easynet:///r/test/device/dev-a")
                    .subject_ura("easynet:///r/test/user/alice")
                    .ability_ura("easynet:///r/test/ability/device.dev-a.terminal.create")
                    .ability_name("terminal.create")
                    .state("failed")
                    .started_unix_ms(1)
                    .args(easynet_axon::invocation::LedgerEventPayload::digest(
                        "application/json",
                        b"{}",
                    ))
                    .build()
                    .expect("record"),
            )
            .expect("put record");

        let reader = AdmissionExplainReader {
            ledger: Some(ledger),
        };
        let visible = reader
            .explain(json!({
                "observer_ura": "easynet:///r/test/user/alice",
                "request_id": "req-1"
            }))
            .expect("visible explain");
        assert_eq!(visible["redacted"], false);
        assert_eq!(visible["root_trace"]["invocation_id"], "req-1");
        assert_eq!(visible["root_trace"]["stage"], "execution_failed");

        let hidden = reader
            .explain(json!({
                "observer_ura": "easynet:///r/test/user/bob",
                "request_id": "req-1"
            }))
            .expect("hidden explain");
        assert_eq!(hidden["redacted"], true);
        assert!(hidden.get("root_trace").is_none());
    }

    #[test]
    fn admission_explain_rejects_client_supplied_projection_fields() {
        let err = ExplainRequest::selector(
            &serde_json::from_value(json!({
                "observer_ura": "easynet:///r/test/user/alice",
                "redacted": true,
                "authority_reason": "forged"
            }))
            .expect("request"),
        )
        .expect_err("selector is required");
        assert!(err.to_string().contains("exactly one"));
    }

    #[test]
    fn authority_binding_list_supports_rfc014_scope_filters() {
        let _home = HomeGuard::new();
        grant_handler(json!({
            "grant": grant_payload("grant-target", "device.terminal.attach", "session-target")
        }))
        .expect("target grant");
        grant_handler(json!({
            "grant": grant_payload("grant-other", "device.files.read", "session-other")
        }))
        .expect("other grant");

        let output = list_grants_handler(json!({
            "owner_user_id": "alice",
            "principal_id": "token-principal-1",
            "token_id": "token-1",
            "callee_ura": "easynet:///r/example/device/dev-a",
            "ability_ura_pattern": "easynet:///r/example/ability/device.terminal.attach",
            "subject_ura_pattern": "easynet:///r/example/resource/session-target",
            "action": "stream",
            "effect": "allow",
            "state": "active"
        }))
        .expect("list grants");

        let grants = output["grants"].as_array().expect("grants array");
        assert_eq!(grants.len(), 1);
        assert_eq!(grants[0]["grant_id"], "grant-target");
    }

    #[test]
    fn policy_request_list_supports_rfc014_scope_and_creation_filters() {
        let _home = HomeGuard::new();
        request_create_handler(json!({
            "request": request_payload(
                "req-target",
                "device.terminal.attach",
                "session-target",
                "2026-07-09T00:00:00Z"
            )
        }))
        .expect("target request");
        request_create_handler(json!({
            "request": request_payload(
                "req-other",
                "device.files.read",
                "session-other",
                "2026-07-09T00:10:00Z"
            )
        }))
        .expect("other request");

        let output = request_list_handler(json!({
            "owner_user_id": "alice",
            "principal_id": "token-principal-1",
            "token_id": "token-1",
            "status": "pending",
            "callee_ura": "easynet:///r/example/device/dev-a",
            "ability_ura": "easynet:///r/example/ability/device.terminal.attach",
            "subject_ura": "easynet:///r/example/resource/session-target",
            "created_at_or_after": "2026-07-09T00:00:00Z",
            "created_at_or_before": "2026-07-09T00:05:00Z"
        }))
        .expect("list requests");

        let requests = output["requests"].as_array().expect("requests array");
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0]["request_id"], "req-target");
    }

    #[test]
    fn policy_request_list_rejects_invalid_creation_filter_window() {
        let err = request_list_handler(json!({
            "owner_user_id": "alice",
            "created_at_or_after": "2026-07-09T00:05:00Z",
            "created_at_or_before": "2026-07-09T00:00:00Z"
        }))
        .expect_err("invalid creation filter window must fail");
        assert!(
            err.to_string()
                .contains("created_at_or_after must not be after created_at_or_before"),
            "{err}"
        );
    }

    fn grant_payload(grant_id: &str, ability: &str, subject: &str) -> Value {
        json!({
            "grant_id": grant_id,
            "owner_user_id": "alice",
            "principal_kind": "token",
            "principal_id": "token-principal-1",
            "token_id": "token-1",
            "token_class": "hub_link",
            "callee_ura": "easynet:///r/example/device/dev-a",
            "subject_ura_pattern": format!("easynet:///r/example/resource/{subject}"),
            "ability_ura_pattern": format!("easynet:///r/example/ability/{ability}"),
            "actions": ["stream"],
            "effect": "allow",
            "lifetime": "session",
            "state": "active",
            "created_by": "easynet:///r/example/user/alice",
            "created_at": "2026-07-09T00:00:00Z"
        })
    }

    fn request_payload(request_id: &str, ability: &str, subject: &str, created_at: &str) -> Value {
        json!({
            "request_id": request_id,
            "owner_user_id": "alice",
            "caller_ura": "easynet:///r/example/hub",
            "principal_kind": "token",
            "principal_id": "token-principal-1",
            "token_id": "token-1",
            "token_class": "hub_link",
            "callee_ura": "easynet:///r/example/device/dev-a",
            "subject_ura": format!("easynet:///r/example/resource/{subject}"),
            "ability_ura": format!("easynet:///r/example/ability/{ability}"),
            "action": "stream",
            "canonical_hash": format!("sha256:{request_id}"),
            "requested_lifetimes": ["once", "session"],
            "status": "pending",
            "created_at": created_at,
            "expires_at": "2026-07-09T01:00:00Z"
        })
    }
}
