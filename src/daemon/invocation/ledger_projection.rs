// EasyNet Daemon — Invocation Ledger Projection
// ===============================================
//
// File: src/daemon/invocation/ledger_projection.rs
// Description: Builds the Axon `InvocationLedgerRecord` for unary
//              invokes (commit-plan-2 E5): caller/callee/ability URAs,
//              typed error projection from tonic Status / in-band
//              InvokeResponse errors, authority-form classification,
//              causal links recovered from the envelope, and the
//              invocation resource URA helpers. Axon stays the verifier
//              owner; this module only projects wire outcomes into
//              ledger rows.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use tonic::{Response, Status};

use easynet_axon::pb::axon::v1::{causal_context, Envelope, InvokeRequest, InvokeResponse};

use std::collections::BTreeMap;

use crate::daemon::invocation::bidi_dispatcher::terminal_failure_message;
use crate::daemon::invocation::federation_wrappers::{
    ABILITY_FEDERATION_ADVERTISE_AGENT, ABILITY_RUNTIME_BOOTSTRAP_SELF_IDENTITY,
};
use crate::daemon::invocation::invocation_wire::{
    DELEGATION_METADATA_KEY, SESSION_AUTHORITY_METADATA_KEY,
};
use crate::daemon::invocation::list_user_pubkeys::ABILITY_IDENTITY_LIST_USER_PUBKEYS;
use crate::daemon::invocation::register_device_pubkey::parse_realm_from_ura;
use crate::daemon::invocation::register_device_pubkey::ABILITY_IDENTITY_REGISTER_PUBKEY;
use crate::daemon::invocation::revoke_user_pubkey::ABILITY_IDENTITY_REVOKE_USER_PUBKEY;
use crate::daemon::invocation::target_gate::RESOLVE_SELECTED_HOST_UNAVAILABLE_CODE;
use crate::runtime::ability::HOSTED_AGENT_DELEGATION_METADATA_KEY;

pub(crate) fn build_unary_ledger_record(
    request: &InvokeRequest,
    started_unix_ms: i64,
    completed_unix_ms: i64,
    result: &Result<Response<InvokeResponse>, Status>,
) -> Result<easynet_axon::invocation::InvocationLedgerRecord, anyhow::Error> {
    let envelope = required_envelope(request)?;
    let caller_ura = required_ura(
        envelope
            .caller
            .as_ref()
            .map(|identity| identity.ura.as_str()),
        "envelope.caller.ura",
    )?;
    let realm = parse_realm_from_ura(&caller_ura).ok_or_else(|| {
        anyhow::anyhow!("envelope.caller.ura does not carry a parseable realm: {caller_ura}")
    })?;
    let callee_ura = required_ura(
        envelope
            .callee
            .as_ref()
            .map(|identity| identity.ura.as_str()),
        "envelope.callee.ura",
    )?;
    let subject_ura = ledger_subject_ura(envelope)?;
    require_invocation_nonce(envelope)?;
    let request_id = ledger_request_id(envelope)?;
    let trace_id = envelope.trace_id.clone();
    let span_id = envelope.span_id.clone();
    let invocation_ura =
        invocation_resource_ura(&realm, &request_id, &subject_ura, &callee_ura, &caller_ura)?;
    let elapsed_ms = completed_unix_ms.saturating_sub(started_unix_ms) as u64;
    let authority_form = ledger_authority_form_for_request(request);
    let ability_ura = ledger_ability_ura(&callee_ura, &request.function_name)?;

    let mut builder = easynet_axon::invocation::InvocationLedgerRecordBuilder::new()
        .invocation_ura(invocation_ura)
        .request_id(request_id)
        .trace_id(trace_id)
        .span_id(span_id)
        .caller_ura(caller_ura)
        .callee_ura(callee_ura)
        .subject_ura(subject_ura)
        .ability_ura(ability_ura)
        .ability_name(request.function_name.clone())
        .started_unix_ms(started_unix_ms)
        .completed_unix_ms(completed_unix_ms)
        .elapsed_ms(elapsed_ms)
        .causal_links(causal_links_from_envelope(Some(envelope)))
        .authority_form(authority_form)
        .args(easynet_axon::invocation::LedgerEventPayload::digest(
            "application/octet-stream",
            &request.arguments,
        ));

    match result {
        Ok(response) => {
            let body = response.get_ref();
            let state = if body.state
                == easynet_axon::invocation::InvocationState::Completed.to_wire_i32()
            {
                "completed"
            } else if body.state == easynet_axon::invocation::InvocationState::Failed.to_wire_i32()
            {
                "failed"
            } else {
                "unknown"
            };
            builder = builder.state(state.to_string());
            if state == "failed" {
                let error = ledger_error_from_invoke_response(
                    body,
                    completed_unix_ms,
                    &request.function_name,
                );
                builder = builder
                    .error(error.clone())
                    .diagnostics(vec![ledger_error_diagnostic(
                        completed_unix_ms,
                        error.clone(),
                    )]);
            } else {
                builder = builder.result(easynet_axon::invocation::LedgerEventPayload::digest(
                    body.result_content_type.clone(),
                    &body.result,
                ));
            }
        }
        Err(status) => {
            let error = ledger_error_from_status(status, &request.function_name);
            builder = builder
                .state("failed".to_string())
                .error(error.clone())
                .diagnostics(vec![ledger_error_diagnostic(completed_unix_ms, error)]);
        }
    }

    Ok(builder.build()?)
}

fn ledger_subject_ura(envelope: &Envelope) -> Result<String, anyhow::Error> {
    required_ura(
        envelope
            .subject
            .as_ref()
            .map(|identity| identity.ura.as_str()),
        "envelope.subject.ura",
    )
}

fn ledger_request_id(envelope: &Envelope) -> Result<String, anyhow::Error> {
    match envelope.request_id.trim() {
        "" => Ok(format!(
            "legacy-{}",
            hex::encode(&envelope.invocation_nonce)
        )),
        request_id => Ok(request_id.to_string()),
    }
}

fn required_envelope(request: &InvokeRequest) -> Result<&Envelope, anyhow::Error> {
    request
        .envelope
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("InvokeRequest.envelope is required for ledger projection"))
}

fn required_ura(value: Option<&str>, field: &str) -> Result<String, anyhow::Error> {
    let value = required_optional_non_empty(value, field)?;
    crate::ura::parse_ura(&value)
        .map_err(|err| anyhow::anyhow!("{field} is not a valid URA: {err}"))?;
    Ok(value)
}

fn required_optional_non_empty(value: Option<&str>, field: &str) -> Result<String, anyhow::Error> {
    let value = value.ok_or_else(|| anyhow::anyhow!("{field} is required"))?;
    required_non_empty(value, field)
}

fn required_non_empty(value: &str, field: &str) -> Result<String, anyhow::Error> {
    let value = value.trim();
    if value.is_empty() {
        anyhow::bail!("{field} must not be empty");
    }
    Ok(value.to_string())
}

fn require_invocation_nonce(envelope: &Envelope) -> Result<(), anyhow::Error> {
    if envelope.invocation_nonce.len() != 16 {
        anyhow::bail!(
            "envelope.invocation_nonce must be exactly 16 bytes, got {}",
            envelope.invocation_nonce.len()
        );
    }
    Ok(())
}

fn ledger_error_from_status(
    status: &Status,
    ability_name: &str,
) -> easynet_axon::invocation::LedgerErrorRecord {
    let fallback = status_fallback_failure_code(status.code());
    let code = crate::runtime::failure_codes::FailureCodeClassifier::classify_or(
        status.message(),
        fallback,
    );
    let mut context = BTreeMap::from([
        ("ability_name".to_string(), ability_name.to_string()),
        (
            "transport_status".to_string(),
            format!("{:?}", status.code()).to_ascii_lowercase(),
        ),
    ]);
    let failure_class =
        crate::runtime::failure_codes::FailureCodeClassifier::classify_error_class(&code);
    context.insert(
        "error_stage".to_string(),
        format!("{:?}", failure_class.stage),
    );
    context.insert(
        "security_class".to_string(),
        format!("{:?}", failure_class.security_class),
    );
    easynet_axon::invocation::LedgerErrorRecord {
        source: "daemon_invocation_service".to_string(),
        code,
        message: status.message().to_string(),
        retryable: status_code_retryable(status.code()),
        context,
    }
}

fn ledger_error_from_invoke_response(
    response: &InvokeResponse,
    completed_unix_ms: i64,
    ability_name: &str,
) -> easynet_axon::invocation::LedgerErrorRecord {
    let default_message = if response.scheduling_reason.trim().is_empty() {
        "invocation completed with failed state".to_string()
    } else {
        response.scheduling_reason.clone()
    };
    let Some(error) = response.error.as_ref() else {
        let code = crate::runtime::failure_codes::FailureCodeClassifier::classify_or(
            &default_message,
            "INVOCATION_FAILED",
        );
        let failure_class =
            crate::runtime::failure_codes::FailureCodeClassifier::classify_error_class(&code);
        return easynet_axon::invocation::LedgerErrorRecord {
            source: "daemon_invocation_service".to_string(),
            code,
            message: default_message,
            retryable: false,
            context: BTreeMap::from([
                ("ability_name".to_string(), ability_name.to_string()),
                (
                    "completed_unix_ms".to_string(),
                    completed_unix_ms.to_string(),
                ),
                (
                    "error_stage".to_string(),
                    format!("{:?}", failure_class.stage),
                ),
                (
                    "security_class".to_string(),
                    format!("{:?}", failure_class.security_class),
                ),
            ]),
        };
    };
    let code = crate::runtime::failure_codes::FailureCodeClassifier::explicit_or_reason(
        Some(error.code.as_str()),
        &error.message,
        "INVOCATION_FAILED",
    );
    let failure_class =
        crate::runtime::failure_codes::FailureCodeClassifier::classify_error_class(&code);
    easynet_axon::invocation::LedgerErrorRecord {
        source: "daemon_invocation_service".to_string(),
        code,
        message: terminal_failure_message(&error.message, "INVOCATION_FAILED"),
        retryable: error.retryable,
        context: BTreeMap::from([
            ("ability_name".to_string(), ability_name.to_string()),
            (
                "completed_unix_ms".to_string(),
                completed_unix_ms.to_string(),
            ),
            (
                "error_stage".to_string(),
                format!("{:?}", failure_class.stage),
            ),
            (
                "security_class".to_string(),
                format!("{:?}", failure_class.security_class),
            ),
        ]),
    }
}

fn ledger_error_diagnostic(
    completed_unix_ms: i64,
    error: easynet_axon::invocation::LedgerErrorRecord,
) -> easynet_axon::invocation::LedgerDiagnosticRecord {
    easynet_axon::invocation::LedgerDiagnosticRecord {
        timestamp_unix_ms: completed_unix_ms,
        level: "error".to_string(),
        source: error.source,
        code: error.code,
        message: error.message,
        retryable: error.retryable,
        payload: None,
    }
}

fn status_fallback_failure_code(code: tonic::Code) -> &'static str {
    match code {
        tonic::Code::InvalidArgument => "INVALID_ARGUMENT",
        tonic::Code::DeadlineExceeded => "INVOCATION_TIMED_OUT",
        tonic::Code::Cancelled => "INVOCATION_CANCELLED",
        tonic::Code::Unavailable => RESOLVE_SELECTED_HOST_UNAVAILABLE_CODE,
        _ => "INVOCATION_FAILED",
    }
}

fn status_code_retryable(code: tonic::Code) -> bool {
    matches!(
        code,
        tonic::Code::Unavailable | tonic::Code::ResourceExhausted | tonic::Code::DeadlineExceeded
    )
}

pub(crate) fn ledger_authority_form_for_request(request: &InvokeRequest) -> &'static str {
    if bootstrap_authority_ability_for_ledger(&request.function_name) {
        "bootstrap"
    } else if has_non_empty_metadata(request, HOSTED_AGENT_DELEGATION_METADATA_KEY)
        || has_non_empty_metadata(request, DELEGATION_METADATA_KEY)
    {
        "delegated"
    } else if has_non_empty_metadata(request, SESSION_AUTHORITY_METADATA_KEY) {
        "session"
    } else {
        "self"
    }
}

fn has_non_empty_metadata(request: &InvokeRequest, key: &str) -> bool {
    request
        .metadata
        .get(key)
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
}

fn bootstrap_authority_ability_for_ledger(function: &str) -> bool {
    matches!(
        function,
        ABILITY_IDENTITY_REGISTER_PUBKEY
            | ABILITY_RUNTIME_BOOTSTRAP_SELF_IDENTITY
            | ABILITY_FEDERATION_ADVERTISE_AGENT
            | ABILITY_IDENTITY_LIST_USER_PUBKEYS
            | ABILITY_IDENTITY_REVOKE_USER_PUBKEY
    )
}

fn causal_links_from_envelope(
    envelope: Option<&Envelope>,
) -> Vec<easynet_axon::invocation::InvocationCausalLink> {
    let Some(form) = envelope
        .and_then(|env| env.causal_context.as_ref())
        .and_then(|ctx| ctx.form.as_ref())
    else {
        return Vec::new();
    };
    match form {
        causal_context::Form::None(_) => Vec::new(),
        causal_context::Form::Scalar(receipt) => {
            vec![causal_link_from_receipt_ref(receipt, "causal")]
        }
        causal_context::Form::List(list) => list
            .prior
            .iter()
            .map(|receipt| causal_link_from_receipt_ref(receipt, "causal_join"))
            .collect(),
        causal_context::Form::Merkle(root) => {
            vec![easynet_axon::invocation::InvocationCausalLink {
                source_invocation_ura: None,
                source_receipt_ura: root.proof_ura.clone(),
                source_receipt_hash: hex::encode(&root.root),
                relation: "causal_merkle".to_string(),
            }]
        }
    }
}

fn causal_link_from_receipt_ref(
    receipt: &easynet_axon::pb::axon::v1::ReceiptRef,
    relation: &str,
) -> easynet_axon::invocation::InvocationCausalLink {
    easynet_axon::invocation::InvocationCausalLink {
        source_invocation_ura: invocation_ura_from_receipt_ura(&receipt.receipt_ura),
        source_receipt_ura: receipt.receipt_ura.clone(),
        source_receipt_hash: hex::encode(&receipt.receipt_hash),
        relation: relation.to_string(),
    }
}

fn invocation_ura_from_receipt_ura(receipt_ura: &str) -> Option<String> {
    receipt_ura
        .rsplit_once("/receipt/")
        .map(|(invocation_ura, _)| invocation_ura.to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InvocationResourceOwner {
    owner_id: String,
    path_prefix: String,
}

pub(crate) fn invocation_resource_ura(
    realm: &str,
    request_id: &str,
    subject_ura: &str,
    callee_ura: &str,
    caller_ura: &str,
) -> Result<String, anyhow::Error> {
    let owner = invocation_resource_owner_from_ura(subject_ura)
        .or_else(|| invocation_resource_owner_from_ura(callee_ura))
        .or_else(|| invocation_resource_owner_from_ura(caller_ura))
        .or_else(local_invocation_resource_owner)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "cannot derive invocation resource owner from subject/callee/caller/local device URA"
            )
        })?;
    let request_segment = safe_resource_path_segment(request_id);
    let path = if owner.path_prefix.is_empty() {
        request_segment
    } else {
        format!("{}/{}", owner.path_prefix, request_segment)
    };
    Ok(crate::ura::resource_dot_ura(realm, &owner.owner_id, &path))
}

fn invocation_resource_owner_from_ura(ura: &str) -> Option<InvocationResourceOwner> {
    let parsed = crate::ura::parse_ura(ura).ok()?;
    match parsed.kind {
        crate::ura::URAKind::User => Some(InvocationResourceOwner {
            owner_id: format!("{}.invocations", parsed.user_id()?),
            path_prefix: String::new(),
        }),
        crate::ura::URAKind::Agent => {
            let (user_id, agent_id) = parsed.agent_ids()?;
            Some(InvocationResourceOwner {
                owner_id: format!("{user_id}.invocations"),
                path_prefix: format!("agents/{agent_id}/invocations"),
            })
        }
        crate::ura::URAKind::Device => Some(InvocationResourceOwner {
            owner_id: format!("device.{}", parsed.device_id()?),
            path_prefix: "invocations".to_string(),
        }),
        _ => None,
    }
}

fn local_invocation_resource_owner() -> Option<InvocationResourceOwner> {
    let local = crate::persistence::local_agents::load().ok()?;
    invocation_resource_owner_from_ura(&local.host_device_agent_ura)
}

fn safe_resource_path_segment(raw: &str) -> String {
    let trimmed = raw.trim();
    let mut out = String::with_capacity(trimmed.len());
    for ch in trimmed.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
            out.push(ch);
        } else {
            out.push('-');
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        return format!("request-{}", short_hash(raw.as_bytes()));
    }
    if out == trimmed {
        out
    } else {
        format!("{}-{}", out, short_hash(raw.as_bytes()))
    }
}

fn ledger_ability_ura(callee_ura: &str, ability_name: &str) -> anyhow::Result<String> {
    let public_name = crate::ura::owner_local_ability_name(callee_ura, ability_name);
    crate::ura::owner_ability_ura(callee_ura, &public_name).ok_or_else(|| {
        anyhow::anyhow!(
            "derive ledger ability URA for callee {callee_ura:?} ability {ability_name:?}"
        )
    })
}

fn short_hash(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let full = hex::encode(hasher.finalize());
    full[..16].to_string()
}

/// Project a callee-signed carrier-v1 execution receipt into THIS
/// hub's ledger (DEC-F004: the hub stays OFF the receipt chain — its
/// account of the forwarded call lives in its own ledger, not in the
/// chain). The receipt's axiom echo carries caller/callee/subject
/// bindings but not ability (landing audit 3), so the consumer side
/// supplies the dispatch context it already holds.
pub(crate) fn ledger_record_from_remote_receipt(
    receipt: &easynet_axon::pb::axon::v1::InvocationReceipt,
    ability_name: &str,
    started_unix_ms: i64,
) -> anyhow::Result<easynet_axon::invocation::InvocationLedgerRecord> {
    use easynet_axon::invocation::{InvocationLedgerRecord, LedgerEventPayload};

    let caller_ura = receipt
        .caller_binding
        .as_ref()
        .map(|c| c.ura.clone())
        .unwrap_or_default();
    let callee_ura = receipt
        .callee_binding
        .as_ref()
        .map(|c| c.ura.clone())
        .unwrap_or_default();
    let subject_ura = receipt
        .subject_binding
        .as_ref()
        .map(|s| s.ura.clone())
        .unwrap_or_default();
    let realm = parse_realm_from_ura(&callee_ura).unwrap_or_else(|| "localhost".to_string());
    // Error propagation, not shape-minting: a failed derivation must
    // never put a wild URA into the ledger (F-042; round-31 audit).
    // Same discipline as the unary path above.
    let invocation_ura = invocation_resource_ura(
        &realm,
        &receipt.invocation_id,
        &subject_ura,
        &callee_ura,
        &caller_ura,
    )?;
    // pb enum number → pb screaming name → domain wire label, the
    // same lowercase format every other ledger row uses.
    let state = easynet_axon::pb::axon::v1::InvocationState::try_from(receipt.state)
        .map(|s| {
            easynet_axon::invocation::InvocationState::from_wire_str(s.as_str_name())
                .as_str()
                .to_string()
        })
        .unwrap_or_else(|_| format!("state_{}", receipt.state));
    let proof_diagnostics = remote_receipt_proof_diagnostics(receipt);
    let ability_ura = ledger_ability_ura(&callee_ura, ability_name)?;
    Ok(InvocationLedgerRecord {
        invocation_ura,
        // Signed usage rides the wire receipt verbatim into the row —
        // never fabricated here (seven-axes DEC-010 card ①).
        usage: receipt
            .usage
            .as_ref()
            .map(|u| easynet_axon::invocation::axiom::InvocationUsage {
                tokens_in: u.tokens_in,
                tokens_out: u.tokens_out,
                duration_ms: u.duration_ms,
                external_calls: u.external_calls,
            })
            .unwrap_or_default(),
        request_id: receipt.invocation_id.clone(),
        caller_ura,
        subject_ura,
        callee_ura,
        ability_name: ability_name.to_string(),
        state,
        ability_ura,
        trace_id: String::new(),
        span_id: String::new(),
        started_unix_ms,
        completed_unix_ms: Some(receipt.timestamp_unix_ms),
        elapsed_ms: u64::try_from(receipt.timestamp_unix_ms - started_unix_ms).ok(),
        // The hub never sees plaintext args for a forwarded call —
        // record an empty digest; the callee's own ledger row holds
        // the full payload story.
        args: LedgerEventPayload::digest("application/octet-stream", &[]),
        result: None,
        error: None,
        diagnostics: proof_diagnostics,
        causal_links: Vec::new(),
        receipt_chain: Default::default(),
        visibility: Default::default(),
        // A forwarded receipt carries identity bindings but NOT the
        // caller's authority form (the delegation/session metadata lived
        // on the originating request, which the hub never sees here).
        // Record "unknown" rather than minting a classification the
        // receipt did not assert — the callee's own ledger row holds the
        // authoritative form. (seven-axes C2: never render a fabricated
        // authority as fact.)
        authority_form: "unknown".to_string(),
    })
}

fn remote_receipt_proof_diagnostics(
    receipt: &easynet_axon::pb::axon::v1::InvocationReceipt,
) -> Vec<easynet_axon::invocation::LedgerDiagnosticRecord> {
    let descriptor_version = receipt.descriptor_version.trim();
    let schema_hash = proof_hash_label(&receipt.schema_hash);
    let impl_hash = proof_hash_label(&receipt.impl_hash);
    let runtime_env = receipt.runtime_env.trim();
    if descriptor_version.is_empty()
        && schema_hash.is_empty()
        && impl_hash.is_empty()
        && runtime_env.is_empty()
    {
        return Vec::new();
    }
    let payload = serde_json::json!({
        "descriptor_version": descriptor_version,
        "schema_hash": schema_hash,
        "impl_hash": impl_hash,
        "runtime_env": runtime_env,
    });
    vec![easynet_axon::invocation::LedgerDiagnosticRecord {
        timestamp_unix_ms: receipt.timestamp_unix_ms,
        level: "info".to_string(),
        source: "remote_receipt".to_string(),
        code: "REMOTE_RECEIPT_PROOF_FACTS".to_string(),
        message: format!(
            "remote receipt proof facts: descriptor_version={descriptor_version:?}, \
             schema_hash={schema_hash:?}, impl_hash={impl_hash:?}, runtime_env={runtime_env:?}"
        ),
        retryable: false,
        payload: Some(easynet_axon::invocation::LedgerEventPayload::digest(
            "application/json",
            payload.to_string().as_bytes(),
        )),
    }]
}

fn proof_hash_label(bytes: &[u8]) -> String {
    if bytes.is_empty() || bytes.iter().all(|byte| *byte == 0) {
        String::new()
    } else {
        format!("sha256:{}", hex::encode(bytes))
    }
}
