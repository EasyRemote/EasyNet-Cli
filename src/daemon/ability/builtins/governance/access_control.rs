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
use std::sync::{Arc, OnceLock};

use crate::core::identity::RuntimeIdentityUra;
use crate::core::ura::URAKind;
use crate::daemon::ability::catalog::system_manifest::registry_manifest;
use crate::daemon::ability::dispatch::{AxonAbilityCatalog, LocalRpcHandler, OwnerKind};
use crate::daemon::invocation::admission::decision::{
    AbilityCallTrace, AdmissionExplainResult, OwnerResolution, OwnerSource,
    PermissionRequestStatus, PolicyDecision, RedactionReason, SignatureDecision,
    SignatureDecisionOutcome, SignatureDecisionReason, TraceStage,
};
use crate::daemon::invocation::admission::policy_engine::{PolicyEngine, PolicyInput};
use crate::daemon::persistence::access_control::AccessControlStoreRegistry;
use axon_sdk::invocation::{InvocationLedger, InvocationLedgerFetchKey, InvocationLedgerQuery};

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
    register_with_ledger(
        reg,
        None,
        Arc::new(OnceLock::new()),
        Arc::new(AccessControlStoreRegistry::default()),
    );
}

pub fn register_with_ledger(
    reg: &mut AxonAbilityCatalog,
    ledger: Option<Arc<InvocationLedger>>,
    _catalog: Arc<OnceLock<Arc<AxonAbilityCatalog>>>,
    access_control_stores: Arc<AccessControlStoreRegistry>,
) {
    let runtime_governance_owners = [
        OwnerKind::runtime_governance_system(),
        OwnerKind::RealmAuthority,
    ];
    for ability in [
        AUTHORITY_BINDING_GRANT,
        AUTHORITY_BINDING_REVOKE,
        AUTHORITY_BINDING_LIST,
        AUTHORITY_BINDING_CHECK,
        POLICY_REQUEST_CREATE,
        POLICY_REQUEST_RESOLVE,
        POLICY_REQUEST_LIST,
    ] {
        let handler: LocalRpcHandler = match ability {
            AUTHORITY_BINDING_GRANT => {
                let stores = Arc::clone(&access_control_stores);
                Arc::new(move |args| grant_handler(args, stores.as_ref()))
            }
            AUTHORITY_BINDING_REVOKE => {
                let stores = Arc::clone(&access_control_stores);
                Arc::new(move |args| revoke_handler(args, stores.as_ref()))
            }
            AUTHORITY_BINDING_LIST => {
                let stores = Arc::clone(&access_control_stores);
                Arc::new(move |args| list_grants_handler(args, stores.as_ref()))
            }
            AUTHORITY_BINDING_CHECK => {
                let stores = Arc::clone(&access_control_stores);
                Arc::new(move |args| check_handler(args, stores.as_ref()))
            }
            POLICY_REQUEST_CREATE => {
                let stores = Arc::clone(&access_control_stores);
                Arc::new(move |args| request_create_handler(args, stores.as_ref()))
            }
            POLICY_REQUEST_RESOLVE => {
                let stores = Arc::clone(&access_control_stores);
                Arc::new(move |args| request_resolve_handler(args, stores.as_ref()))
            }
            POLICY_REQUEST_LIST => {
                let stores = Arc::clone(&access_control_stores);
                Arc::new(move |args| request_list_handler(args, stores.as_ref()))
            }
            _ => unreachable!("static RFC-014 ability list"),
        };
        for owner in runtime_governance_owners.iter().cloned() {
            reg.register_rpc_with_spec(
                ability,
                owner,
                registry_manifest(ability, description_for(ability), input_schema_for(ability)),
                Arc::clone(&handler),
            );
        }
    }

    let reader = Arc::new(AdmissionExplainReader { ledger });
    for owner in runtime_governance_owners.iter().cloned() {
        let reader = Arc::clone(&reader);
        reg.register_rpc_with_spec(
            ADMISSION_EXPLAIN,
            owner,
            registry_manifest(
                ADMISSION_EXPLAIN,
                description_for(ADMISSION_EXPLAIN),
                input_schema_for(ADMISSION_EXPLAIN),
            ),
            Arc::new(move |args| reader.explain(args)),
        );
    }
}

fn grant_handler(args: Value, stores: &AccessControlStoreRegistry) -> anyhow::Result<Value> {
    let request: GrantRequest = serde_json::from_value(args)?;
    let grant = grant_from_wire_mutation_boundary(
        request.grant,
        request.owner_ura.as_str(),
        request.principal_ura.as_deref(),
    )?;
    let actor_ura = require_actor_ura(request.actor_ura.as_str())?;
    let owner_user_ura = grant.owner_user_ura.clone();
    let result = stores.with_store(&owner_user_ura, |store| {
        store.create_grant(grant, actor_ura)
    })??;
    Ok(serde_json::to_value(result)?)
}

fn revoke_handler(args: Value, stores: &AccessControlStoreRegistry) -> anyhow::Result<Value> {
    let request: RevokeRequest = serde_json::from_value(args)?;
    let owner_user_ura = owner_user_ura_from_mutation_boundary(request.owner_ura.as_str())?;
    let actor_ura = require_actor_ura(request.actor_ura.as_str())?;
    let grant = stores.with_store(&owner_user_ura, |store| {
        store.revoke_grant(
            &request.grant_id,
            &owner_user_ura,
            actor_ura,
            request.reason,
        )
    })??;
    Ok(json!({ "grant": grant }))
}

fn list_grants_handler(args: Value, stores: &AccessControlStoreRegistry) -> anyhow::Result<Value> {
    let request: ListGrantsRequest = serde_json::from_value(args)?;
    let owner_user_ura = owner_user_ura_from_boundary(request.owner_ura.as_str())?;
    let principal_id = principal_id_from_boundary(
        request.principal_kind,
        request.principal_ura.as_deref(),
        request.token_id.as_deref(),
    )?;
    let mut grants = stores.with_store(&owner_user_ura, |store| store.grants())?;
    if let Some(principal_id) = principal_id {
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

fn check_handler(args: Value, stores: &AccessControlStoreRegistry) -> anyhow::Result<Value> {
    let request: CheckRequest = serde_json::from_value(args)?;
    let owner_user_ura = owner_user_ura_from_boundary(request.owner_ura.as_str())?;
    let principal_id = principal_id_from_boundary(
        Some(request.principal_kind),
        request.principal_ura.as_deref(),
        request.token_id.as_deref(),
    )?
    .ok_or_else(|| anyhow::anyhow!("principal_ura or token_id is required for policy checks"))?;
    let grants = stores.with_store(&owner_user_ura, |store| store.grants())?;
    let owner = OwnerResolution {
        owner_user_ura: Some(owner_user_ura),
        owner_ura: Some(request.owner_ura),
        owner_source: request.owner_source,
        audit_warnings: vec![],
    };
    let decision = PolicyEngine::check(PolicyInput {
        owner,
        caller_user_ura: None,
        caller_ura: request.caller_ura,
        principal_kind: request.principal_kind,
        principal_id,
        token_id: request.token_id,
        token_class: request.token_class,
        callee_ura: request.callee_ura,
        subject_ura: request.subject_ura,
        ability_ura: request.ability_ura,
        action: request.action,
        safe_read: request.safe_read,
        system_rule_matches: Vec::new(),
        invocation_lifecycle_control: false,
        interactive_context_available: request.interactive_context_available,
        canonical_hash: request.canonical_hash,
        signature_key_id: request.signature_key_id,
        verified_authority_id: request.authority_proof_id,
        verified_session_id: None,
        rejector_ura: request.rejector_ura,
        now: chrono::Utc::now(),
        grants,
    });
    Ok(json!({ "policy_decision": decision }))
}

fn request_create_handler(
    args: Value,
    stores: &AccessControlStoreRegistry,
) -> anyhow::Result<Value> {
    let request: PermissionRequestEnvelope = serde_json::from_value(args)?;
    let permission_request = permission_request_from_wire_mutation_boundary(
        request.request,
        request.owner_ura.as_str(),
        request.principal_ura.as_deref(),
    )?;
    let actor_ura = require_actor_ura(request.actor_ura.as_str())?;
    let owner_user_ura = permission_request.owner_user_ura.clone();
    let request = stores.with_store(&owner_user_ura, |store| {
        store.upsert_permission_request(permission_request, actor_ura)
    })??;
    Ok(json!({ "request": request }))
}

fn request_resolve_handler(
    args: Value,
    stores: &AccessControlStoreRegistry,
) -> anyhow::Result<Value> {
    let request: PermissionRequestResolutionEnvelope = serde_json::from_value(args)?;
    let permission_request = permission_request_from_wire_mutation_boundary(
        request.request,
        request.owner_ura.as_str(),
        request.principal_ura.as_deref(),
    )?;
    let actor_ura = require_actor_ura(request.actor_ura.as_str())?;
    let owner_user_ura = permission_request.owner_user_ura.clone();
    stores.with_store(&owner_user_ura, |store| {
        if permission_request.status == PermissionRequestStatus::Approved {
            if let Some(grant) = request.created_grant {
                let grant = grant_from_wire_mutation_boundary(
                    grant,
                    request.owner_ura.as_str(),
                    request.principal_ura.as_deref(),
                )?;
                let result = store.resolve_permission_request_with_grant(
                    permission_request,
                    grant,
                    actor_ura,
                )?;
                return Ok(serde_json::to_value(result)?);
            }
            if let Some(proof) = request.authority_proof {
                let result = store.resolve_permission_request_with_authority_proof(
                    permission_request,
                    proof,
                    actor_ura,
                )?;
                return Ok(serde_json::to_value(result)?);
            }
        }
        let request = store.resolve_permission_request(permission_request, actor_ura)?;
        Ok(json!({ "request": request }))
    })?
}

fn request_list_handler(args: Value, stores: &AccessControlStoreRegistry) -> anyhow::Result<Value> {
    let request: ListRequestsRequest = serde_json::from_value(args)?;
    let owner_user_ura = owner_user_ura_from_boundary(request.owner_ura.as_str())?;
    let principal_id = principal_id_from_boundary(
        request.principal_kind,
        request.principal_ura.as_deref(),
        request.token_id.as_deref(),
    )?;
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
    let mut requests = stores.with_store(&owner_user_ura, |store| store.requests())?;
    if let Some(principal_id) = principal_id {
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

fn grant_from_wire_mutation_boundary(
    grant: WirePermissionGrant,
    owner_ura: &str,
    principal_ura: Option<&str>,
) -> anyhow::Result<crate::daemon::invocation::admission::grant_matcher::PermissionGrant> {
    let owner_user_ura = owner_user_ura_from_mutation_boundary(owner_ura)?;
    let principal_id = principal_id_from_mutation_boundary(
        Some(grant.principal_kind),
        principal_ura,
        grant.token_id.as_deref(),
    )?
    .ok_or_else(|| anyhow::anyhow!("principal_ura or token_id is required for policy mutation"))?;
    Ok(grant.into_permission_grant(owner_user_ura, principal_id))
}

fn permission_request_from_wire_mutation_boundary(
    request: WirePermissionRequest,
    owner_ura: &str,
    principal_ura: Option<&str>,
) -> anyhow::Result<crate::daemon::invocation::admission::decision::PermissionRequest> {
    let owner_user_ura = owner_user_ura_from_mutation_boundary(owner_ura)?;
    let principal_id = principal_id_from_mutation_boundary(
        Some(request.principal_kind),
        principal_ura,
        request.token_id.as_deref(),
    )?
    .ok_or_else(|| anyhow::anyhow!("principal_ura or token_id is required for policy mutation"))?;
    Ok(request.into_permission_request(owner_user_ura, principal_id))
}

fn owner_user_ura_from_boundary(owner_ura: &str) -> anyhow::Result<String> {
    let owner_ura = owner_ura.trim();
    if owner_ura.is_empty() {
        anyhow::bail!("owner_ura is required");
    }
    let parsed = RuntimeIdentityUra::parse(owner_ura)
        .map_err(|err| anyhow::anyhow!("owner_ura must be an admissible User URA: {err}"))?;
    if parsed.kind() != URAKind::User {
        anyhow::bail!("owner_ura must be a canonical User URA");
    }
    let parsed_ura = crate::core::ura::parse_ura(parsed.as_str())
        .map_err(|err| anyhow::anyhow!("owner_ura must be a canonical User URA: {err}"))?;
    if parsed_ura.user_id().is_none() {
        anyhow::bail!("owner_ura must include a user id");
    }
    Ok(parsed.into_string())
}

fn owner_user_ura_from_mutation_boundary(owner_ura: &str) -> anyhow::Result<String> {
    let owner_ura = owner_ura.trim();
    if owner_ura.is_empty() {
        anyhow::bail!("owner_ura is required for a policy mutation");
    }
    owner_user_ura_from_boundary(owner_ura)
}

fn require_actor_ura(actor_ura: &str) -> anyhow::Result<&str> {
    let actor_ura = actor_ura.trim();
    if actor_ura.is_empty() {
        anyhow::bail!("actor_ura is required for an audited mutation");
    }
    RuntimeIdentityUra::parse(actor_ura).map_err(|err| {
        anyhow::anyhow!("actor_ura must be a canonical URA and admissible runtime identity: {err}")
    })?;
    Ok(actor_ura)
}

fn principal_id_from_boundary(
    kind: Option<crate::daemon::invocation::admission::decision::PrincipalKind>,
    principal_ura: Option<&str>,
    token_id: Option<&str>,
) -> anyhow::Result<Option<String>> {
    let token_id = token_id.map(str::trim).filter(|value| !value.is_empty());
    let Some(principal_ura) = principal_ura
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return match kind {
            Some(crate::daemon::invocation::admission::decision::PrincipalKind::Token) => {
                Ok(token_id.map(str::to_string))
            }
            Some(_) => anyhow::bail!("principal_ura is required for non-token principals"),
            None => Ok(None),
        };
    };
    let parsed = RuntimeIdentityUra::parse(principal_ura)
        .map_err(|err| anyhow::anyhow!("principal_ura must be admissible: {err}"))?;
    let canonical = match kind {
        Some(crate::daemon::invocation::admission::decision::PrincipalKind::User) => {
            if parsed.kind() != URAKind::User {
                anyhow::bail!("principal_ura for user principal must be a User URA");
            }
            let parsed_ura = crate::core::ura::parse_ura(parsed.as_str())
                .map_err(|err| anyhow::anyhow!("principal_ura must be canonical: {err}"))?;
            if parsed_ura.user_id().is_none() {
                anyhow::bail!("principal_ura must include a user id");
            }
            parsed.into_string()
        }
        Some(crate::daemon::invocation::admission::decision::PrincipalKind::Agent) => {
            if parsed.kind() != URAKind::Agent {
                anyhow::bail!("principal_ura for agent principal must be an Agent URA");
            }
            parsed.into_string()
        }
        Some(crate::daemon::invocation::admission::decision::PrincipalKind::Token) => {
            return token_id
                .map(|value| Ok(Some(value.to_string())))
                .unwrap_or_else(|| anyhow::bail!("token_id is required for token principals"));
        }
        _ => parsed.into_string(),
    };
    Ok(Some(canonical))
}

fn principal_id_from_mutation_boundary(
    kind: Option<crate::daemon::invocation::admission::decision::PrincipalKind>,
    principal_ura: Option<&str>,
    token_id: Option<&str>,
) -> anyhow::Result<Option<String>> {
    use crate::daemon::invocation::admission::decision::PrincipalKind;

    if kind != Some(PrincipalKind::Token)
        && principal_ura
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_none()
    {
        anyhow::bail!("principal_ura is required for a non-token policy mutation");
    }
    principal_id_from_boundary(kind, principal_ura, token_id)
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
        let action: crate::daemon::invocation::admission::decision::AccessAction =
            serde_json::from_value(Value::String(record.admission_action.clone())).map_err(
                |error| {
                    anyhow::anyhow!(
                        "admission.explain: ledger record lacks bound admission action: {error}"
                    )
                },
            )?;
        let signed_action =
            axon_sdk::invocation::admission_action_from_descriptor_ref(&record.descriptor_ref)
                .map_err(|error| {
                    anyhow::anyhow!(
                "admission.explain: ledger record has an invalid descriptor reference: {error}"
            )
                })?;
        if signed_action != record.admission_action
            || record.safe_read
                != (action == crate::daemon::invocation::admission::decision::AccessAction::Read)
        {
            anyhow::bail!(
                "admission.explain: ledger descriptor facts disagree with signed evidence"
            );
        }

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
            let failure = record
                .error
                .as_ref()
                .map(|error| project_failure(error, &record));
            let trace = AbilityCallTrace {
                invocation_id: record.request_id.clone(),
                parent_invocation_id: None,
                root_invocation_id: record.trace_id.clone(),
                caller_ura: record.caller_ura.clone(),
                callee_ura: record.callee_ura.clone(),
                subject_ura: record.subject_ura.clone(),
                ability_ura: record.ability_ura.clone(),
                action,
                route_ref: Some(record.descriptor_ref.clone()),
                execution_host_ura: None,
                rejector_ura: failure
                    .as_ref()
                    .and_then(|failure| failure.rejector_ura.clone()),
                stage: trace_stage_for(record.state.as_str()),
                signature_decision: failure
                    .as_ref()
                    .and_then(|failure| failure.signature_decision.clone()),
                policy_decision: failure
                    .as_ref()
                    .and_then(|failure| failure.policy_decision.clone()),
                authority_proof_id: failure
                    .as_ref()
                    .and_then(|failure| failure.authority_proof_id.clone()),
                redacted: false,
                child_failure_class: None,
                redaction_reason: None,
                children: Vec::new(),
            };
            AdmissionExplainResult {
                observer_ura: observer.to_string(),
                redacted: false,
                root_trace: Some(trace),
                signature_decision: failure
                    .as_ref()
                    .and_then(|failure| failure.signature_decision.clone()),
                policy_decision: failure
                    .as_ref()
                    .and_then(|failure| failure.policy_decision.clone()),
                authority_reason,
                route_ref: Some(record.descriptor_ref.clone()),
                rejector_ura: failure.and_then(|failure| failure.rejector_ura),
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

#[derive(Default)]
struct FailureProjection {
    signature_decision: Option<SignatureDecision>,
    policy_decision: Option<PolicyDecision>,
    rejector_ura: Option<String>,
    authority_proof_id: Option<String>,
}

/// Decode only the structured denial that admission already persisted in the
/// ledger error. This is a projection, not a second policy engine and never
/// trusts caller-supplied explain fields.
fn project_failure(
    error: &axon_sdk::invocation::LedgerErrorRecord,
    record: &axon_sdk::invocation::InvocationLedgerRecord,
) -> FailureProjection {
    let Some((prefix, encoded)) = error.message.split_once(": ") else {
        return FailureProjection::default();
    };
    let Ok(value) = serde_json::from_str::<Value>(encoded) else {
        return FailureProjection::default();
    };
    let mut projection = FailureProjection {
        rejector_ura: value
            .get("rejector_ura")
            .and_then(Value::as_str)
            .map(str::to_string),
        ..FailureProjection::default()
    };

    if prefix == "POLICY_DENIED" {
        if let Ok(policy) = serde_json::from_value::<PolicyDecision>(value.clone()) {
            projection.rejector_ura = policy.rejector_ura.clone();
            projection.authority_proof_id = policy.authority_proof_id.clone();
            projection.policy_decision = Some(policy);
        }
    } else if prefix == "SIGNATURE_DENIED" {
        let reason = value
            .get("target_reason")
            .and_then(Value::as_str)
            .unwrap_or(error.code.as_str());
        let canonical_hash = value
            .get("canonical_hash")
            .and_then(Value::as_str)
            .unwrap_or("");
        let verifier_ura = projection.rejector_ura.as_deref().unwrap_or("");
        if !canonical_hash.is_empty() && !verifier_ura.is_empty() {
            projection.signature_decision = Some(SignatureDecision {
                decision: SignatureDecisionOutcome::Invalid,
                reason: SignatureDecisionReason::from_admission_detail(reason),
                caller_ura: record.caller_ura.clone(),
                callee_ura: record.callee_ura.clone(),
                ability_ura: record.ability_ura.clone(),
                subject_ura: record.subject_ura.clone(),
                canonical_hash: canonical_hash.to_string(),
                signature_key_id: value
                    .get("signature_key_id")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                presented_pubkey_fingerprint: None,
                verifier_ura: verifier_ura.to_string(),
            });
        }
    }
    projection
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
#[serde(deny_unknown_fields)]
struct GrantRequest {
    grant: WirePermissionGrant,
    owner_ura: String,
    #[serde(default)]
    principal_ura: Option<String>,
    actor_ura: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WirePermissionGrant {
    grant_id: String,
    principal_kind: crate::daemon::invocation::admission::decision::PrincipalKind,
    #[serde(default)]
    token_id: Option<String>,
    #[serde(default)]
    token_class: Option<crate::daemon::invocation::admission::decision::TokenClass>,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    session_expires_at: Option<String>,
    #[serde(default)]
    callee_ura: Option<String>,
    #[serde(default)]
    subject_ura_pattern: Option<String>,
    #[serde(default)]
    ability_ura_pattern: Option<String>,
    actions: Vec<crate::daemon::invocation::admission::decision::AccessAction>,
    #[serde(default)]
    constraints: Option<crate::daemon::invocation::admission::grant_matcher::PermissionConstraints>,
    effect: crate::daemon::invocation::admission::grant_matcher::PermissionEffect,
    lifetime: crate::daemon::invocation::admission::grant_matcher::PermissionGrantLifetime,
    state: crate::daemon::invocation::admission::grant_matcher::PermissionGrantState,
    #[serde(default)]
    expires_at: Option<String>,
    #[serde(default)]
    review_required_after: Option<String>,
    #[serde(default)]
    last_reviewed_at: Option<String>,
    #[serde(default)]
    last_used_at: Option<String>,
    created_by: String,
    created_at: String,
    #[serde(default)]
    updated_at: Option<String>,
    #[serde(default)]
    revoked_at: Option<String>,
    #[serde(default)]
    reason: Option<String>,
}

impl WirePermissionGrant {
    fn into_permission_grant(
        self,
        owner_user_ura: String,
        principal_id: String,
    ) -> crate::daemon::invocation::admission::grant_matcher::PermissionGrant {
        crate::daemon::invocation::admission::grant_matcher::PermissionGrant {
            grant_id: self.grant_id,
            owner_user_ura,
            principal_kind: self.principal_kind,
            principal_id,
            token_id: self.token_id,
            token_class: self.token_class,
            session_id: self.session_id,
            session_expires_at: self.session_expires_at,
            callee_ura: self.callee_ura,
            subject_ura_pattern: self.subject_ura_pattern,
            ability_ura_pattern: self.ability_ura_pattern,
            actions: self.actions,
            constraints: self.constraints,
            effect: self.effect,
            lifetime: self.lifetime,
            state: self.state,
            expires_at: self.expires_at,
            review_required_after: self.review_required_after,
            last_reviewed_at: self.last_reviewed_at,
            last_used_at: self.last_used_at,
            created_by: self.created_by,
            created_at: self.created_at,
            updated_at: self.updated_at,
            revoked_at: self.revoked_at,
            reason: self.reason,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RevokeRequest {
    grant_id: String,
    owner_ura: String,
    #[serde(default)]
    reason: Option<String>,
    actor_ura: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListGrantsRequest {
    owner_ura: String,
    #[serde(default)]
    principal_kind: Option<crate::daemon::invocation::admission::decision::PrincipalKind>,
    #[serde(default)]
    principal_ura: Option<String>,
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
#[serde(deny_unknown_fields)]
struct CheckRequest {
    owner_ura: String,
    owner_source: OwnerSource,
    caller_ura: String,
    principal_kind: crate::daemon::invocation::admission::decision::PrincipalKind,
    #[serde(default)]
    principal_ura: Option<String>,
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
#[serde(deny_unknown_fields)]
struct PermissionRequestEnvelope {
    request: WirePermissionRequest,
    owner_ura: String,
    #[serde(default)]
    principal_ura: Option<String>,
    actor_ura: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PermissionRequestResolutionEnvelope {
    request: WirePermissionRequest,
    owner_ura: String,
    #[serde(default)]
    principal_ura: Option<String>,
    #[serde(default)]
    created_grant: Option<WirePermissionGrant>,
    #[serde(default)]
    authority_proof: Option<crate::daemon::invocation::admission::authority_proof::AuthorityProof>,
    actor_ura: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WirePermissionRequest {
    request_id: String,
    caller_ura: String,
    principal_kind: crate::daemon::invocation::admission::decision::PrincipalKind,
    #[serde(default)]
    token_id: Option<String>,
    #[serde(default)]
    token_class: Option<crate::daemon::invocation::admission::decision::TokenClass>,
    callee_ura: String,
    subject_ura: String,
    ability_ura: String,
    action: crate::daemon::invocation::admission::decision::AccessAction,
    #[serde(default)]
    nonce: Option<String>,
    #[serde(default)]
    canonical_hash: Option<String>,
    requested_lifetimes: Vec<crate::daemon::invocation::admission::decision::PermissionLifetime>,
    status: crate::daemon::invocation::admission::decision::PermissionRequestStatus,
    created_at: String,
    expires_at: String,
    #[serde(default)]
    resolver_ura: Option<String>,
    #[serde(default)]
    resolved_lifetime: Option<crate::daemon::invocation::admission::decision::PermissionLifetime>,
    #[serde(default)]
    created_grant_id: Option<String>,
    #[serde(default)]
    authority_proof_id: Option<String>,
    #[serde(default)]
    resolved_at: Option<String>,
    #[serde(default)]
    decision_reason: Option<String>,
}

impl WirePermissionRequest {
    fn into_permission_request(
        self,
        owner_user_ura: String,
        principal_id: String,
    ) -> crate::daemon::invocation::admission::decision::PermissionRequest {
        crate::daemon::invocation::admission::decision::PermissionRequest {
            request_id: self.request_id,
            owner_user_ura,
            caller_ura: self.caller_ura,
            principal_kind: self.principal_kind,
            principal_id,
            token_id: self.token_id,
            token_class: self.token_class,
            callee_ura: self.callee_ura,
            subject_ura: self.subject_ura,
            ability_ura: self.ability_ura,
            action: self.action,
            nonce: self.nonce,
            canonical_hash: self.canonical_hash,
            requested_lifetimes: self.requested_lifetimes,
            status: self.status,
            created_at: self.created_at,
            expires_at: self.expires_at,
            resolver_ura: self.resolver_ura,
            resolved_lifetime: self.resolved_lifetime,
            created_grant_id: self.created_grant_id,
            authority_proof_id: self.authority_proof_id,
            resolved_at: self.resolved_at,
            decision_reason: self.decision_reason,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListRequestsRequest {
    owner_ura: String,
    #[serde(default)]
    principal_kind: Option<crate::daemon::invocation::admission::decision::PrincipalKind>,
    #[serde(default)]
    principal_ura: Option<String>,
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
#[serde(deny_unknown_fields)]
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
            anyhow::bail!(
                "admission.explain: exactly one invocation_id, request_id, trace_id, or root_id is required"
            );
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
            "required": ["grant", "owner_ura", "actor_ura"],
            "properties": {
                "grant": {"type": "object"},
                "owner_ura": {"type": "string"},
                "principal_ura": {"type": "string"},
                "actor_ura": {"type": "string"}
            },
            "additionalProperties": false
        }),
        AUTHORITY_BINDING_REVOKE => json!({
            "type": "object",
            "required": ["grant_id", "owner_ura", "actor_ura"],
            "properties": {
                "grant_id": {"type": "string"},
                "owner_ura": {"type": "string"},
                "reason": {"type": "string"},
                "actor_ura": {"type": "string"}
            },
            "additionalProperties": false
        }),
        AUTHORITY_BINDING_LIST => json!({
            "type": "object",
            "required": ["owner_ura"],
            "properties": {
                "owner_ura": {"type": "string"},
                "principal_kind": {"type": "string"},
                "principal_ura": {"type": "string"},
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
            "required": ["owner_ura", "owner_source", "caller_ura", "principal_kind", "callee_ura", "subject_ura", "ability_ura", "action"],
            "properties": {
                "owner_ura": {"type": "string"},
                "owner_source": {"type": "string"},
                "caller_ura": {"type": "string"},
                "principal_kind": {"type": "string"},
                "principal_ura": {"type": "string"},
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
            "required": ["request", "owner_ura", "actor_ura"],
            "properties": {
                "request": {"type": "object"},
                "owner_ura": {"type": "string"},
                "principal_ura": {"type": "string"},
                "actor_ura": {"type": "string"}
            },
            "additionalProperties": false
        }),
        POLICY_REQUEST_RESOLVE => json!({
            "type": "object",
            "required": ["request", "owner_ura", "actor_ura"],
            "properties": {
                "request": {"type": "object"},
                "owner_ura": {"type": "string"},
                "principal_ura": {"type": "string"},
                "created_grant": {"type": "object"},
                "authority_proof": {"type": "object"},
                "actor_ura": {"type": "string"}
            },
            "additionalProperties": false
        }),
        POLICY_REQUEST_LIST => json!({
            "type": "object",
            "required": ["owner_ura"],
            "properties": {
                "owner_ura": {"type": "string"},
                "principal_kind": {"type": "string"},
                "principal_ura": {"type": "string"},
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
            "oneOf": [
                {"required": ["invocation_id"]},
                {"required": ["request_id"]},
                {"required": ["trace_id"]},
                {"required": ["root_id"]}
            ],
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

    fn terminal_system_agent_ura(realm: &str) -> String {
        crate::core::ura::device_agent_ura(
            realm,
            "dev-a",
            crate::daemon::ability::names::device_control::TERMINAL_SYSTEM_AGENT_ID,
        )
    }

    fn terminal_system_ability_ura(realm: &str, public_name: &str) -> String {
        crate::core::ura::owner_ability_ura(&terminal_system_agent_ura(realm), public_name)
            .expect("terminal SystemAgent ability URA")
    }

    fn signature_failure_record() -> axon_sdk::invocation::InvocationLedgerRecord {
        let callee_ura = terminal_system_agent_ura("test");
        let ability_ura = terminal_system_ability_ura("test", "terminal.attach");
        axon_sdk::invocation::InvocationLedgerRecordBuilder::new()
            .invocation_ura("easynet:///r/test/resource/alice.invocations/req-signature")
            .request_id("req-signature")
            .trace_id("trace-signature")
            .span_id("span-signature")
            .caller_ura("easynet:///r/test/user/alice")
            .callee_ura(callee_ura)
            .subject_ura("easynet:///r/test/resource/user.alice/session/session-target")
            .ability_ura(ability_ura.clone())
            .ability_name("terminal.attach")
            .descriptor_ref(format!("{ability_ura}@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!stream"))
            .admission_action("stream")
            .authority_form("self")
            .safe_read(false)
            .state("failed")
            .started_unix_ms(1)
            .args(axon_sdk::invocation::LedgerEventPayload::digest(
                "application/json",
                b"{}",
            ))
            .build()
            .expect("signature failure record")
    }

    fn signature_failure_error(mut body: Value) -> axon_sdk::invocation::LedgerErrorRecord {
        let object = body
            .as_object_mut()
            .expect("signature failure body must be an object");
        object.insert("decision".to_string(), Value::String("deny".to_string()));
        object.insert(
            "target_reason".to_string(),
            Value::String("CALLER_SIGNATURE_VERIFY_FAILED".to_string()),
        );
        object.insert(
            "canonical_hash".to_string(),
            Value::String("sha256:signature".to_string()),
        );
        object.insert(
            "signature_key_id".to_string(),
            Value::String("ed25519:key".to_string()),
        );
        axon_sdk::invocation::LedgerErrorRecord {
            source: "daemon_invocation_service".to_string(),
            code: "SIGNATURE_DENIED".to_string(),
            message: format!("SIGNATURE_DENIED: {body}"),
            retryable: false,
            context: std::collections::BTreeMap::new(),
        }
    }

    #[test]
    fn registration_makes_rfc014_abilities_dispatchable() {
        let mut reg = AxonAbilityCatalog::new_test_metadata_for_device_authority(
            "easynet:///r/test/device/access-control",
        );
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
    fn registration_publishes_admission_explain_selector_contract() {
        let mut reg = AxonAbilityCatalog::new_test_metadata_for_device_authority(
            "easynet:///r/test/device/access-control",
        );
        register(&mut reg);

        let record = reg
            .control_plane_record_for_mode(ADMISSION_EXPLAIN, crate::daemon::ability::CallMode::Rpc)
            .expect("control-plane lookup")
            .expect("admission explain control-plane record");
        let selector_contract = record.descriptor().input_schema()["oneOf"]
            .as_array()
            .expect("descriptor must publish exact-one selector contract");

        assert_eq!(selector_contract.len(), 4);
    }

    #[test]
    fn access_control_routes_are_generated_from_manifest() {
        use sha2::Digest as _;

        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("provider_routes/runtime-access-control-routes.v1.json");
        let digest = sha2::Sha256::digest(std::fs::read(manifest).expect("read manifest"));

        assert_eq!(
            crate::daemon::ability::access_control_routes_gen::ACCESS_CONTROL_ROUTE_MANIFEST_SHA256,
            hex::encode(digest)
        );
        assert_eq!(
            crate::daemon::ability::access_control_routes_gen::ACCESS_CONTROL_PROFILE,
            "access_control"
        );
        assert_eq!(
            AUTHORITY_BINDING_GRANT,
            crate::daemon::ability::access_control_routes_gen::AUTHORITY_BINDING_GRANT
        );
    }

    #[test]
    fn admission_explain_signature_projection_rejects_missing_verifier_ura() {
        let record = signature_failure_record();
        let error = signature_failure_error(json!({}));

        let projection = project_failure(&error, &record);

        assert_eq!(projection.rejector_ura, None);
        assert!(
            projection.signature_decision.is_none(),
            "signature decision must not be projected with an empty verifier_ura"
        );
    }

    #[test]
    fn admission_explain_signature_projection_uses_rejector_as_verifier_ura() {
        let record = signature_failure_record();
        let error = signature_failure_error(json!({
            "rejector_ura": "easynet:///r/test/device/dev-a"
        }));

        let projection = project_failure(&error, &record);
        let signature = projection
            .signature_decision
            .expect("complete signature failure should project a signature decision");

        assert_eq!(
            signature.verifier_ura, "easynet:///r/test/device/dev-a",
            "verifier_ura must be the structured rejector, never a default"
        );
        assert_eq!(signature.caller_ura, "easynet:///r/test/user/alice");
        assert_eq!(
            signature.reason,
            SignatureDecisionReason::CallerSignatureVerifyFailed
        );
    }

    #[test]
    fn access_control_schemas_require_canonical_owner_ura() {
        for ability in [
            AUTHORITY_BINDING_LIST,
            AUTHORITY_BINDING_CHECK,
            POLICY_REQUEST_LIST,
        ] {
            let schema = input_schema_for(ability);
            let required = schema["required"].as_array().expect("required array");
            assert!(
                required.iter().any(|item| item == "owner_ura"),
                "{ability} must require owner_ura"
            );
            assert!(
                schema["properties"].get("owner_user_id").is_none(),
                "{ability} must not expose owner_user_id as request input"
            );
            assert!(
                schema["properties"].get("principal_id").is_none(),
                "{ability} must not expose principal_id as request input"
            );
            assert!(
                schema["properties"].get("caller_user_id").is_none(),
                "{ability} must not expose caller_user_id as request input"
            );
            assert!(
                !required.iter().any(|item| item == "principal_id"),
                "{ability} must not require principal_id"
            );
        }
    }

    #[test]
    fn authority_binding_check_requires_explicit_owner_source() {
        let _home = HomeGuard::new();
        let stores = AccessControlStoreRegistry::ephemeral();
        let missing = check_handler(
            json!({
                "owner_ura": "easynet:///r/example/user/alice",
                "caller_ura": "easynet:///r/example/authority",
                "principal_kind": "user",
                "principal_ura": "easynet:///r/example/user/bob",
                "callee_ura": terminal_system_agent_ura("example"),
                "subject_ura": "easynet:///r/example/resource/user.alice/session/session-target",
                "ability_ura": terminal_system_ability_ura("example", "terminal.attach"),
                "action": "stream"
            }),
            &stores,
        )
        .expect_err("policy checks must not infer owner_source from subject");
        assert!(missing.to_string().contains("missing field `owner_source`"));

        let schema = input_schema_for(AUTHORITY_BINDING_CHECK);
        let required = schema["required"].as_array().expect("required array");
        assert!(required.iter().any(|item| item == "owner_source"));
    }

    #[test]
    fn admission_explain_schema_requires_exactly_one_ledger_selector() {
        let schema = input_schema_for(ADMISSION_EXPLAIN);
        let required = schema["required"].as_array().expect("required array");
        assert!(required.iter().any(|item| item == "observer_ura"));

        let selectors = schema["oneOf"].as_array().expect("oneOf selector array");
        let selector_fields = selectors
            .iter()
            .map(|branch| {
                branch["required"]
                    .as_array()
                    .and_then(|required| required.first())
                    .and_then(|field| field.as_str())
                    .expect("selector branch must require one field")
            })
            .collect::<Vec<_>>();

        assert_eq!(
            selector_fields,
            vec!["invocation_id", "request_id", "trace_id", "root_id"]
        );
    }

    #[test]
    fn revoke_requires_a_canonical_actor_ura_before_opening_the_store() {
        let _home = HomeGuard::new();
        let stores = AccessControlStoreRegistry::ephemeral();
        let missing = revoke_handler(
            json!({
                "grant_id": "grant-1",
                "owner_ura": "easynet:///r/test/user/alice"
            }),
            &stores,
        )
        .expect_err("an audited revoke cannot infer its actor from owner_user_id");
        assert!(missing.to_string().contains("missing field `actor_ura`"));

        let invalid = revoke_handler(
            json!({
                "grant_id": "grant-1",
                "owner_ura": "easynet:///r/test/user/alice",
                "actor_ura": "alice"
            }),
            &stores,
        )
        .expect_err("a scalar actor must not be persisted as an actor URA");
        assert!(invalid
            .to_string()
            .contains("actor_ura must be a canonical URA"));
    }

    #[test]
    fn policy_mutations_reject_scalar_only_identity_boundaries() {
        let _home = HomeGuard::new();
        let stores = AccessControlStoreRegistry::ephemeral();
        let missing_owner = grant_handler(
            json!({
                "actor_ura": "easynet:///r/example/user/alice",
                "grant": grant_payload("grant-scalar-owner", "device.files.read", "session-1")
            }),
            &stores,
        )
        .expect_err("owner_user_id must not replace owner_ura on a mutation");
        assert!(missing_owner
            .to_string()
            .contains("missing field `owner_ura`"));

        let missing_principal = grant_handler(
            json!({
                "owner_ura": "easynet:///r/example/user/alice",
                "actor_ura": "easynet:///r/example/user/alice",
                "grant": user_grant_payload("grant-scalar-principal")
            }),
            &stores,
        )
        .expect_err("a non-token mutation must carry principal_ura");
        assert!(missing_principal
            .to_string()
            .contains("principal_ura is required for a non-token policy mutation"));
    }

    #[test]
    fn agent_grant_boundary_requires_agent_principal_ura() {
        let _home = HomeGuard::new();
        let stores = AccessControlStoreRegistry::ephemeral();
        let invalid = grant_handler(
            json!({
                "owner_ura": "easynet:///r/example/user/alice",
                "principal_ura": "easynet:///r/example/device/dev-a",
                "actor_ura": "easynet:///r/example/user/alice",
                "grant": agent_grant_payload("grant-agent-invalid")
            }),
            &stores,
        )
        .expect_err("Agent grant must not accept a Device URA as principal");
        assert!(
            invalid
                .to_string()
                .contains("principal_ura for agent principal must be an Agent URA"),
            "{invalid}"
        );

        let valid = grant_handler(
            json!({
                "owner_ura": "easynet:///r/example/user/alice",
                "principal_ura": "easynet:///r/example/agent/alice.worker",
                "actor_ura": "easynet:///r/example/user/alice",
                "grant": agent_grant_payload("grant-agent-valid")
            }),
            &stores,
        )
        .expect("Agent grant with Agent URA");
        assert_eq!(
            valid["grant"]["principal_id"],
            "easynet:///r/example/agent/alice.worker"
        );

        let listed = list_grants_handler(
            json!({
                "owner_ura": "easynet:///r/example/user/alice",
                "principal_kind": "agent",
                "principal_ura": "easynet:///r/example/agent/alice.worker"
            }),
            &stores,
        )
        .expect("list Agent grant by Agent URA");
        let grants = listed["grants"].as_array().expect("grants array");
        assert_eq!(grants.len(), 1);
        assert_eq!(grants[0]["grant_id"], "grant-agent-valid");

        let invalid_list = list_grants_handler(
            json!({
                "owner_ura": "easynet:///r/example/user/alice",
                "principal_kind": "agent",
                "principal_ura": "easynet:///r/example/device/dev-a"
            }),
            &stores,
        )
        .expect_err("Agent grant list filter must not accept a Device URA");
        assert!(
            invalid_list
                .to_string()
                .contains("principal_ura for agent principal must be an Agent URA"),
            "{invalid_list}"
        );
    }

    #[test]
    fn policy_read_boundaries_reject_scalar_only_owner_identity() {
        let _home = HomeGuard::new();
        let stores = AccessControlStoreRegistry::ephemeral();
        let list_error = list_grants_handler(
            json!({
                "owner_user_id": "alice",
                "token_id": "token-1"
            }),
            &stores,
        )
        .expect_err("owner_user_id must not replace owner_ura on grant reads");
        assert!(list_error.to_string().contains("owner_user_id"));

        let check_error = check_handler(
            json!({
                "owner_user_id": "alice",
                "caller_ura": "easynet:///r/example/authority",
                "principal_kind": "token",
                "token_id": "token-1",
                "callee_ura": terminal_system_agent_ura("example"),
                "subject_ura": "easynet:///r/example/resource/user.alice/session/session-target",
                "ability_ura": terminal_system_ability_ura("example", "terminal.attach"),
                "action": "stream"
            }),
            &stores,
        )
        .expect_err("owner_user_id must not replace owner_ura on policy checks");
        assert!(check_error.to_string().contains("owner_user_id"));

        let requests_error = request_list_handler(
            json!({
                "owner_user_id": "alice",
                "token_id": "token-1"
            }),
            &stores,
        )
        .expect_err("owner_user_id must not replace owner_ura on request reads");
        assert!(requests_error.to_string().contains("owner_user_id"));
    }

    #[test]
    fn admission_explain_is_stable_after_the_live_descriptor_changes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ledger =
            Arc::new(InvocationLedger::open(dir.path().join("invocations.redb")).expect("ledger"));
        ledger
            .put(
                &axon_sdk::invocation::InvocationLedgerRecordBuilder::new()
                    .invocation_ura("easynet:///r/test/resource/alice.invocations/req-1")
                    .request_id("req-1")
                    .trace_id("trace-1")
                    .span_id("span-1")
                    .caller_ura("easynet:///r/test/authority")
                    .callee_ura(terminal_system_agent_ura("test"))
                    .subject_ura("easynet:///r/test/user/alice")
                    .ability_ura(terminal_system_ability_ura("test", "terminal.create"))
                    .ability_name("terminal.create")
                    .descriptor_ref(format!("{}@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!read", terminal_system_ability_ura("test", "terminal.create")))
                    .admission_action("read")
                    .authority_form("self")
                    .safe_read(true)
                    .state("failed")
                    .error(axon_sdk::invocation::LedgerErrorRecord {
                        source: "daemon_invocation_service".to_string(),
                        code: "POLICY_DENIED".to_string(),
                        message: format!(
                            "POLICY_DENIED: {}",
                            serde_json::json!({
                                "decision": "deny",
                                "reason": "TOKEN_SCOPE_DENIED",
                                "owner_user_id": "alice",
                                "owner_source": "subject",
                                "caller_ura": "easynet:///r/test/authority",
                                "principal_kind": "token",
                                "principal_id": "token-1",
                                "callee_ura": terminal_system_agent_ura("test"),
                                "subject_ura": "easynet:///r/test/user/alice",
                                "ability_ura": terminal_system_ability_ura("test", "terminal.create"),
                                "action": "stream",
                                "canonical_hash": "sha256:abc",
                                "signature_key_id": "ed25519:key",
                                "authority_proof_id": "proof-1",
                                "rejector_ura": terminal_system_agent_ura("test")
                            })
                        ),
                        retryable: false,
                        context: std::collections::BTreeMap::new(),
                    })
                    .started_unix_ms(1)
                    .args(axon_sdk::invocation::LedgerEventPayload::digest(
                        "application/json",
                        b"{}",
                    ))
                    .build()
                    .expect("record"),
            )
            .expect("put record");

        let callee_ura = "easynet:///r/test/device/dev-a";
        let mut catalog = AxonAbilityCatalog::new_with_runtime_and_authority_context(
            crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
                crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
                None,
            ),
            crate::daemon::ability::dispatch::AbilityAuthorityContext::for_device_authority_root(
                callee_ura,
            )
            .expect("Device authority context"),
        );
        catalog.register_rpc_with_spec(
            "terminal.create",
            OwnerKind::DeviceProfileProjection,
            registry_manifest(
                "terminal.create",
                "Admission explain descriptor fixture.",
                json!({"type": "object"}),
            ),
            Arc::new(|_args| Ok(json!({}))),
        );
        let catalog_handle = Arc::new(OnceLock::new());
        catalog_handle
            .set(Arc::new(catalog))
            .expect("set admission explain catalog fixture");
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
        assert_eq!(
            visible["root_trace"]["policy_decision"]["reason"],
            "TOKEN_SCOPE_DENIED"
        );
        assert_eq!(visible["root_trace"]["authority_proof_id"], "proof-1");
        assert_eq!(visible["root_trace"]["action"], "read");
        assert_eq!(
            visible["route_ref"],
            format!(
                "{}@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!read",
                terminal_system_ability_ura("test", "terminal.create")
            )
        );
        assert_eq!(visible["rejector_ura"], terminal_system_agent_ura("test"));

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
    fn admission_explain_projects_voice_actions_from_signed_descriptor_facts() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ledger =
            Arc::new(InvocationLedger::open(dir.path().join("invocations.redb")).expect("ledger"));

        fn put_voice_record(
            ledger: &InvocationLedger,
            request_id: &str,
            ability_name: &str,
            action: &str,
            hash_char: char,
        ) {
            let digest = hash_char.to_string().repeat(64);
            let ability_ura = crate::core::ura::hub_ability_ura("test", ability_name);
            let descriptor_ref = format!("{ability_ura}@1.0.0#{digest}!{action}");
            ledger
                .put(
                    &axon_sdk::invocation::InvocationLedgerRecordBuilder::new()
                        .invocation_ura(format!(
                            "easynet:///r/test/resource/alice.invocations/{request_id}"
                        ))
                        .request_id(request_id)
                        .trace_id(format!("trace-{request_id}"))
                        .span_id(format!("span-{request_id}"))
                        .caller_ura("easynet:///r/test/user/alice")
                        .callee_ura("easynet:///r/test/authority")
                        .subject_ura("easynet:///r/test/resource/authority.voice-call/call-1")
                        .ability_ura(ability_ura)
                        .ability_name(ability_name)
                        .descriptor_ref(descriptor_ref)
                        .admission_action(action)
                        .authority_form("self")
                        .safe_read(action == "read")
                        .state("completed")
                        .started_unix_ms(1)
                        .args(axon_sdk::invocation::LedgerEventPayload::digest(
                            "application/json",
                            b"{}",
                        ))
                        .build()
                        .expect("voice ledger record"),
                )
                .expect("put voice record");
        }

        let reader = AdmissionExplainReader {
            ledger: Some(Arc::clone(&ledger)),
        };
        let voice_actions = [
            ("voice.create_call", "invoke", 'b'),
            ("voice.join_call", "invoke", 'c'),
            ("voice.leave_call", "invoke", 'd'),
            ("voice.end_call", "invoke", 'e'),
            ("voice.report_metrics", "invoke", 'f'),
            ("voice.show_call", "read", 'a'),
            ("voice.watch_call", "read", '1'),
            ("voice.list_calls", "read", '2'),
            ("voice.subscribe", "stream", '3'),
            ("voice.transcribe", "stream", '4'),
        ];

        for (ability, action, hash_char) in voice_actions {
            let request_id = ability.replace('.', "-");
            put_voice_record(ledger.as_ref(), &request_id, ability, action, hash_char);
            let explain = reader
                .explain(json!({
                    "observer_ura": "easynet:///r/test/user/alice",
                    "request_id": request_id
                }))
                .unwrap_or_else(|error| panic!("{ability} explain failed: {error}"));

            assert_eq!(
                explain["root_trace"]["action"], action,
                "{ability} must project the action bound into its signed descriptor ref"
            );
        }
    }

    #[test]
    fn admission_explain_rejects_client_supplied_projection_fields() {
        let err = serde_json::from_value::<ExplainRequest>(json!({
            "observer_ura": "easynet:///r/test/user/alice",
            "request_id": "req-1",
            "redacted": true,
            "authority_reason": "forged"
        }))
        .expect_err("admission.explain must reject client-supplied projection fields");
        assert!(err.to_string().contains("unknown field"));
    }

    #[test]
    fn authority_binding_list_supports_rfc014_scope_filters() {
        let _home = HomeGuard::new();
        let stores = AccessControlStoreRegistry::ephemeral();
        grant_handler(
            json!({
                "owner_ura": "easynet:///r/example/user/alice",
                "actor_ura": "easynet:///r/example/user/alice",
                "grant": grant_payload("grant-target", "device.terminal.attach", "session-target")
            }),
            &stores,
        )
        .expect("target grant");
        grant_handler(
            json!({
                "owner_ura": "easynet:///r/example/user/alice",
                "actor_ura": "easynet:///r/example/user/alice",
                "grant": grant_payload("grant-other", "device.files.read", "session-other")
            }),
            &stores,
        )
        .expect("other grant");

        let output = list_grants_handler(json!({
            "owner_ura": "easynet:///r/example/user/alice",
            "token_id": "token-1",
            "callee_ura": terminal_system_agent_ura("example"),
            "ability_ura_pattern": terminal_system_ability_ura("example", "terminal.attach"),
            "subject_ura_pattern": "easynet:///r/example/resource/user.alice/session/session-target",
            "action": "stream",
            "effect": "allow",
            "state": "active"
        }), &stores)
        .expect("list grants");

        let grants = output["grants"].as_array().expect("grants array");
        assert_eq!(grants.len(), 1);
        assert_eq!(grants[0]["grant_id"], "grant-target");
    }

    #[test]
    fn authority_binding_grant_derives_owner_and_user_principal_from_ura() {
        let _home = HomeGuard::new();
        let stores = AccessControlStoreRegistry::ephemeral();
        let output = grant_handler(
            json!({
                "owner_ura": "easynet:///r/example/user/alice",
                "principal_ura": "easynet:///r/example/user/alice",
                "actor_ura": "easynet:///r/example/user/alice",
                "grant": user_grant_payload("grant-user")
            }),
            &stores,
        )
        .expect("user grant");

        assert_eq!(
            output["grant"]["owner_user_id"],
            "easynet:///r/example/user/alice"
        );
        assert_eq!(
            output["grant"]["principal_id"],
            "easynet:///r/example/user/alice"
        );

        let listed = list_grants_handler(
            json!({
                "owner_ura": "easynet:///r/example/user/alice",
                "principal_kind": "user",
                "principal_ura": "easynet:///r/example/user/alice"
            }),
            &stores,
        )
        .expect("list by URA");
        let grants = listed["grants"].as_array().expect("grants array");
        assert_eq!(grants.len(), 1);
        assert_eq!(grants[0]["grant_id"], "grant-user");
    }

    #[test]
    fn authority_binding_rejects_nested_scalar_identity_fields() {
        let _home = HomeGuard::new();
        let stores = AccessControlStoreRegistry::ephemeral();
        let grant_error = grant_handler(
            json!({
                "owner_ura": "easynet:///r/example/user/alice",
                "principal_ura": "easynet:///r/example/user/alice",
                "actor_ura": "easynet:///r/example/user/alice",
                "grant": {
                    "grant_id": "grant-scalar",
                    "owner_user_id": "bob",
                    "principal_kind": "user",
                    "principal_id": "bob",
                    "actions": ["invoke"],
                    "effect": "allow",
                    "lifetime": "session",
                    "state": "active",
                    "created_by": "easynet:///r/example/user/bob",
                    "created_at": "2026-07-09T00:00:00Z"
                }
            }),
            &stores,
        )
        .expect_err("nested scalar identity fields must not be accepted");
        assert!(
            grant_error.to_string().contains("owner_user_id")
                || grant_error.to_string().contains("principal_id"),
            "{grant_error}"
        );

        let request_error = request_create_handler(
            json!({
                "owner_ura": "easynet:///r/example/user/alice",
                "principal_ura": "easynet:///r/example/user/alice",
                "actor_ura": "easynet:///r/example/user/alice",
                "request": {
                    "request_id": "req-scalar",
                    "owner_user_id": "bob",
                    "caller_ura": "easynet:///r/example/authority",
                    "principal_kind": "user",
                    "principal_id": "bob",
                    "callee_ura": "easynet:///r/example/agent/device.dev-a.agent-management",
                    "subject_ura": "easynet:///r/example/user/alice",
                    "ability_ura": "easynet:///r/example/ability/system-agent.dev-a.agent-management.agent.list",
                    "action": "invoke",
                    "requested_lifetimes": ["session"],
                    "status": "pending",
                    "created_at": "2026-07-09T00:00:00Z",
                    "expires_at": "2026-07-09T01:00:00Z"
                }
            }),
            &stores,
        )
        .expect_err("nested request scalar identity fields must not be accepted");
        assert!(
            request_error.to_string().contains("owner_user_id")
                || request_error.to_string().contains("principal_id"),
            "{request_error}"
        );
    }

    #[test]
    fn policy_boundaries_reject_all_zero_user_uras() {
        let all_zero_user = "easynet:///r/example/user/00000000-0000-0000-0000-000000000000";
        assert!(owner_user_ura_from_boundary(all_zero_user).is_err());
        assert!(principal_id_from_boundary(
            Some(crate::daemon::invocation::admission::decision::PrincipalKind::User),
            Some(all_zero_user),
            None,
        )
        .is_err());
        assert!(require_actor_ura(all_zero_user).is_err());
    }

    #[test]
    fn policy_request_list_supports_rfc014_scope_and_creation_filters() {
        let _home = HomeGuard::new();
        let stores = AccessControlStoreRegistry::ephemeral();
        request_create_handler(
            json!({
                "owner_ura": "easynet:///r/example/user/alice",
                "actor_ura": "easynet:///r/example/user/alice",
                "request": request_payload(
                    "req-target",
                    "device.terminal.attach",
                    "session-target",
                    "2026-07-09T00:00:00Z"
                )
            }),
            &stores,
        )
        .expect("target request");
        request_create_handler(
            json!({
                "owner_ura": "easynet:///r/example/user/alice",
                "actor_ura": "easynet:///r/example/user/alice",
                "request": request_payload(
                    "req-other",
                    "device.files.read",
                    "session-other",
                    "2026-07-09T00:10:00Z"
                )
            }),
            &stores,
        )
        .expect("other request");

        let output = request_list_handler(
            json!({
                "owner_ura": "easynet:///r/example/user/alice",
                "token_id": "token-1",
                "status": "pending",
                "callee_ura": terminal_system_agent_ura("example"),
                "ability_ura": terminal_system_ability_ura("example", "terminal.attach"),
                "subject_ura": "easynet:///r/example/resource/user.alice/session/session-target",
                "created_at_or_after": "2026-07-09T00:00:00Z",
                "created_at_or_before": "2026-07-09T00:05:00Z"
            }),
            &stores,
        )
        .expect("list requests");

        let requests = output["requests"].as_array().expect("requests array");
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0]["request_id"], "req-target");
    }

    #[test]
    fn policy_request_list_rejects_invalid_creation_filter_window() {
        let stores = AccessControlStoreRegistry::ephemeral();
        let err = request_list_handler(
            json!({
                "owner_ura": "easynet:///r/example/user/alice",
                "created_at_or_after": "2026-07-09T00:05:00Z",
                "created_at_or_before": "2026-07-09T00:00:00Z"
            }),
            &stores,
        )
        .expect_err("invalid creation filter window must fail");
        assert!(
            err.to_string()
                .contains("created_at_or_after must not be after created_at_or_before"),
            "{err}"
        );
    }

    fn grant_payload(grant_id: &str, ability: &str, subject: &str) -> Value {
        let ability_ura = terminal_system_ability_ura(
            "example",
            ability.strip_prefix("device.").unwrap_or(ability),
        );
        let subject_ura = crate::core::ura::resource_dot_ura(
            "example",
            "user.alice",
            &format!("session/{subject}"),
        );
        json!({
            "grant_id": grant_id,
            "principal_kind": "token",
            "token_id": "token-1",
            "token_class": "hub_link",
            "callee_ura": terminal_system_agent_ura("example"),
            "subject_ura_pattern": subject_ura,
            "ability_ura_pattern": ability_ura,
            "actions": ["stream"],
            "effect": "allow",
            "lifetime": "permanent",
            "state": "active",
            "created_by": "easynet:///r/example/user/alice",
            "created_at": "2026-07-09T00:00:00Z"
        })
    }

    fn user_grant_payload(grant_id: &str) -> Value {
        json!({
            "grant_id": grant_id,
            "principal_kind": "user",
            "callee_ura": "easynet:///r/example/agent/device.dev-a.agent-management",
            "subject_ura_pattern": "easynet:///r/example/user/alice",
            "ability_ura_pattern": "easynet:///r/example/ability/system-agent.dev-a.agent-management.agent.list",
            "actions": ["invoke"],
            "effect": "allow",
            "lifetime": "permanent",
            "state": "active",
            "created_by": "easynet:///r/example/user/alice",
            "created_at": "2026-07-09T00:00:00Z"
        })
    }

    fn agent_grant_payload(grant_id: &str) -> Value {
        json!({
            "grant_id": grant_id,
            "principal_kind": "agent",
            "callee_ura": "easynet:///r/example/agent/alice.worker",
            "subject_ura_pattern": "easynet:///r/example/user/alice",
            "ability_ura_pattern": "easynet:///r/example/ability/agent.alice.worker.remote_desktop.attach",
            "actions": ["stream"],
            "effect": "allow",
            "lifetime": "permanent",
            "state": "active",
            "created_by": "easynet:///r/example/user/alice",
            "created_at": "2026-07-09T00:00:00Z"
        })
    }

    fn request_payload(request_id: &str, ability: &str, subject: &str, created_at: &str) -> Value {
        let ability_ura = terminal_system_ability_ura(
            "example",
            ability.strip_prefix("device.").unwrap_or(ability),
        );
        let subject_ura = crate::core::ura::resource_dot_ura(
            "example",
            "user.alice",
            &format!("session/{subject}"),
        );
        json!({
            "request_id": request_id,
            "caller_ura": "easynet:///r/example/authority",
            "principal_kind": "token",
            "token_id": "token-1",
            "token_class": "hub_link",
            "callee_ura": terminal_system_agent_ura("example"),
            "subject_ura": subject_ura,
            "ability_ura": ability_ura,
            "action": "stream",
            "canonical_hash": format!("sha256:{request_id}"),
            "requested_lifetimes": ["once", "session"],
            "status": "pending",
            "created_at": created_at,
            "expires_at": "2026-07-09T01:00:00Z"
        })
    }
}
