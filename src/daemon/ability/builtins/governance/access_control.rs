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

use serde::Deserialize;
use serde_json::{json, Value};

use crate::daemon::ability::catalog::system_manifest::registry_manifest;
use crate::daemon::ability::dispatch::{AxonAbilityCatalog, OwnerKind};
use crate::daemon::invocation::admission::decision::{
    AbilityCallTrace, AdmissionExplainResult, OwnerResolution, OwnerSource, PolicyDecision,
    SignatureDecision,
};
use crate::daemon::invocation::admission::policy_engine::{PolicyEngine, PolicyInput};
use crate::daemon::persistence::access_control::AccessControlStore;

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
        let handler = match ability {
            AUTHORITY_BINDING_GRANT => grant_handler,
            AUTHORITY_BINDING_REVOKE => revoke_handler,
            AUTHORITY_BINDING_LIST => list_grants_handler,
            AUTHORITY_BINDING_CHECK => check_handler,
            POLICY_REQUEST_CREATE => request_create_handler,
            POLICY_REQUEST_RESOLVE => request_resolve_handler,
            POLICY_REQUEST_LIST => request_list_handler,
            ADMISSION_EXPLAIN => explain_handler,
            _ => unreachable!("static RFC-014 ability list"),
        };
        reg.register_rpc_with_spec(
            ability,
            OwnerKind::Device,
            registry_manifest(ability, description_for(ability), input_schema_for(ability)),
            std::sync::Arc::new(handler),
        );
    }
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
        authority_proof_id: request.authority_proof_id,
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
    let request: PermissionRequestEnvelope = serde_json::from_value(args)?;
    let actor_ura = request
        .actor_ura
        .clone()
        .unwrap_or_else(|| request.request.resolver_ura.clone().unwrap_or_default());
    let mut store = AccessControlStore::open_or_create(request.request.owner_user_id.clone())?;
    let request = store.resolve_permission_request(request.request, &actor_ura)?;
    Ok(json!({ "request": request }))
}

fn request_list_handler(args: Value) -> anyhow::Result<Value> {
    let request: ListRequestsRequest = serde_json::from_value(args)?;
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
    Ok(json!({ "requests": requests }))
}

fn explain_handler(args: Value) -> anyhow::Result<Value> {
    let request: ExplainRequest = serde_json::from_value(args)?;
    let result = AdmissionExplainResult {
        observer_ura: request.observer_ura,
        redacted: request.redacted,
        root_trace: request.root_trace,
        signature_decision: request.signature_decision,
        policy_decision: request.policy_decision,
        authority_reason: request.authority_reason,
        route_ref: request.route_ref,
        rejector_ura: request.rejector_ura,
        redaction_reason: request.redaction_reason,
    };
    Ok(serde_json::to_value(result)?)
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
struct ListRequestsRequest {
    owner_user_id: String,
    #[serde(default)]
    principal_id: Option<String>,
    #[serde(default)]
    token_id: Option<String>,
    #[serde(default)]
    status: Option<crate::daemon::invocation::admission::decision::PermissionRequestStatus>,
}

#[derive(Debug, Deserialize)]
struct ExplainRequest {
    observer_ura: String,
    #[serde(default)]
    redacted: bool,
    #[serde(default)]
    root_trace: Option<AbilityCallTrace>,
    #[serde(default)]
    signature_decision: Option<SignatureDecision>,
    #[serde(default)]
    policy_decision: Option<PolicyDecision>,
    #[serde(default)]
    authority_reason: Option<String>,
    #[serde(default)]
    route_ref: Option<String>,
    #[serde(default)]
    rejector_ura: Option<String>,
    #[serde(default)]
    redaction_reason: Option<crate::daemon::invocation::admission::decision::RedactionReason>,
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
        POLICY_REQUEST_CREATE | POLICY_REQUEST_RESOLVE => json!({
            "type": "object",
            "required": ["request"],
            "properties": {
                "request": {"type": "object"},
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
                "status": {"type": "string"}
            },
            "additionalProperties": false
        }),
        ADMISSION_EXPLAIN => json!({
            "type": "object",
            "required": ["observer_ura"],
            "properties": {
                "observer_ura": {"type": "string"},
                "redacted": {"type": "boolean"},
                "root_trace": {"type": "object"},
                "signature_decision": {"type": "object"},
                "policy_decision": {"type": "object"},
                "authority_reason": {"type": "string"},
                "route_ref": {"type": "string"},
                "rejector_ura": {"type": "string"},
                "redaction_reason": {"type": "string"}
            },
            "additionalProperties": false
        }),
        _ => json!({"type": "object"}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn admission_explain_is_read_only_projection() {
        let output = explain_handler(json!({
            "observer_ura": "easynet:///r/test/user/alice",
            "redacted": true,
            "authority_reason": "AUTHORITY_PROOF_MISSING"
        }))
        .expect("explain");
        assert_eq!(output["observer_ura"], "easynet:///r/test/user/alice");
        assert_eq!(output["redacted"], true);
        assert_eq!(output["authority_reason"], "AUTHORITY_PROOF_MISSING");
    }
}
