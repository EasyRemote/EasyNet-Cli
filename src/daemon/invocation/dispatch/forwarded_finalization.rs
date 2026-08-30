//! EasyNet CLI — forwarded invocation finalization
//! ================================================
//!
//! File: src/daemon/invocation/dispatch/forwarded_finalization.rs
//! Description: Verifies descriptor-bound lifecycle closure returned across
//! daemon forwarding boundaries.
//!
//! Protocol Responsibility:
//! - Bind forwarded request tuples and descriptor refs to resolver-selected
//!   owner, callee, route, and execution-host facts.
//! - Verify admission and terminal checkpoints before projecting results.
//!
//! Implementation Approach:
//! - Axon verifies receipt canonical bytes, signatures, hosted attestation, and
//!   checkpoint continuity.
//! - EasyNet then applies the product route policy Axon intentionally leaves to
//!   the directory layer: a selected-route receipt's effective signer must be
//!   the resolver-selected execution host.
//!
//! Usage Contract:
//! - Direct selected-route forwarding must use `for_selected_route`.
//! - Delegated forwarding may use `for_delegated_request` only when this daemon
//!   has not selected the final execution host.
//! - Two verified checkpoints prove finalization, not a complete receipt chain;
//!   full-chain verification still requires all intermediate receipts or a
//!   separately verifiable chain proof.
//!
//! Architectural Position:
//! - EasyNet Runtime dispatch policy above Axon's protocol verifier and below
//!   unary, stream, and bidi carrier projection.

use std::sync::Arc;

use axon_sdk::invocation::{
    sha256, InvocationState, KeyResolver, SignedInvocationReceipt, VerifiedFinalizationCheckpoints,
};
use axon_sdk::pb::axon::v1::{
    Envelope, Error, InvocationReceipt as WireInvocationReceipt, InvokeRequest, InvokeResponse,
    ResponseHeader,
};
use tonic::Status;

use crate::daemon::invocation::admission::device_trust_sync::DeviceTrustSync;
use crate::daemon::invocation::receipts::finalization_projection::{
    self, FinalizationProjectionError, ReceiptCheckpointStage,
};
use crate::daemon::invocation::routing::route_resolver::{SelectedInvokeRoute, SelectedRouteKind};

#[derive(Debug, Clone)]
enum ForwardedReceiptAuthority {
    /// The final execution host is selected by a remote authority. This daemon
    /// can bind the request and verify the returned proof, but must not invent a
    /// local route-host constraint.
    Delegated,
    /// The local resolver selected the exact host that receives the forwarded
    /// carrier. Its identity is therefore part of the receipt acceptance
    /// contract, not merely a key-prefetch hint.
    SelectedExecutionHost(String),
}

#[derive(Debug, Clone)]
pub(crate) struct ForwardedInvocationBinding {
    envelope: Envelope,
    ability_binding: String,
    input_hash: [u8; 32],
    receipt_authority: ForwardedReceiptAuthority,
}

impl ForwardedInvocationBinding {
    /// Bind a request whose final execution host is selected beyond this
    /// daemon's routing boundary.
    pub(crate) fn for_delegated_request(request: &InvokeRequest) -> Result<Self, Status> {
        Self::from_request(request, ForwardedReceiptAuthority::Delegated)
    }

    /// Bind a direct selected-route forwarding request.
    ///
    /// This is the single contract seam joining descriptor identity, ability
    /// ownership, route selection, and forwarded receipt authority.
    pub(crate) fn for_selected_route(
        request: &InvokeRequest,
        route: &SelectedInvokeRoute,
    ) -> Result<Self, Status> {
        let binding = Self::from_request(
            request,
            ForwardedReceiptAuthority::SelectedExecutionHost(route.execution_host_ura.clone()),
        )?;
        binding.verify_selected_route(route)?;
        Ok(binding)
    }

    fn from_request(
        request: &InvokeRequest,
        receipt_authority: ForwardedReceiptAuthority,
    ) -> Result<Self, Status> {
        let envelope = request
            .envelope
            .clone()
            .ok_or_else(|| invalid("forwarded invocation is missing its seven-tuple envelope"))?;
        let callee_ura = envelope
            .callee
            .as_ref()
            .map(|callee| callee.ura.trim())
            .filter(|ura| !ura.is_empty())
            .ok_or_else(|| invalid("forwarded invocation is missing its callee binding"))?;
        let ability_binding = super::invocation_wire::descriptor_ref_from_invocation_target(
            "forwarded invocation",
            callee_ura,
            request.target.as_ref(),
        )
        .map_err(|status| invalid(status.message()))?;
        Ok(Self {
            envelope,
            ability_binding,
            input_hash: sha256(&request.arguments),
            receipt_authority,
        })
    }

    fn verify_selected_route(&self, route: &SelectedInvokeRoute) -> Result<(), Status> {
        let callee_ura = self
            .envelope
            .callee
            .as_ref()
            .map(|callee| callee.ura.trim())
            .filter(|ura| !ura.is_empty())
            .ok_or_else(|| invalid("forwarded invocation is missing its callee binding"))?;
        let selector =
            crate::daemon::axon_bridge::descriptor_ref::ability_selector_from_descriptor_ref(
                &self.ability_binding,
            )
            .map_err(|error| {
                invalid(format!(
                    "forwarded invocation descriptor ref cannot project its ability owner: {error}"
                ))
            })?;

        if route.owner_ura != route.callee_ura
            || selector.owner_ura() != route.owner_ura
            || callee_ura != route.callee_ura
        {
            return Err(invalid(format!(
                "forwarded descriptor owner `{}` must equal selected owner `{}`, callee `{}`, and request callee `{callee_ura}`",
                selector.owner_ura(), route.owner_ura, route.callee_ura,
            )));
        }
        if selector.ability_ura() != route.ability_ura {
            return Err(invalid(format!(
                "forwarded descriptor ability `{}` does not match selected route ability `{}`",
                selector.ability_ura(),
                route.ability_ura,
            )));
        }
        if self.ability_binding != route.descriptor_ref {
            return Err(invalid(format!(
                "forwarded descriptor ref does not match selected route `{}` descriptor proof",
                route.route_ura,
            )));
        }

        let execution_host =
            crate::core::ura::parse_ura(&route.execution_host_ura).map_err(|error| {
                invalid(format!(
                    "selected execution host `{}` is not canonical: {error}",
                    route.execution_host_ura,
                ))
            })?;
        match route.kind() {
            SelectedRouteKind::RealmAuthorityOwned => {
                if selector.owner_kind() != "authority"
                    || execution_host.kind != crate::core::ura::URAKind::Authority
                    || route.execution_host_ura != route.callee_ura
                {
                    return Err(invalid(
                        "Authority-owned selected route must use the same Authority as owner, callee, and execution host",
                    ));
                }
            }
            SelectedRouteKind::RoutableAgentOwned => {
                if !matches!(selector.owner_kind(), "agent" | "system-agent" | "service")
                    || execution_host.kind != crate::core::ura::URAKind::Device
                {
                    return Err(invalid(
                        "Agent/SystemAgent/Service selected route must keep the actor as owner/callee and a Device as execution host",
                    ));
                }
            }
        }
        Ok(())
    }

    pub(crate) fn selected_execution_host_ura(&self) -> Option<&str> {
        match &self.receipt_authority {
            ForwardedReceiptAuthority::Delegated => None,
            ForwardedReceiptAuthority::SelectedExecutionHost(execution_host_ura) => {
                Some(execution_host_ura)
            }
        }
    }

    fn verify_receipt_authority(
        &self,
        receipt: &WireInvocationReceipt,
        stage: &str,
    ) -> Result<(), Status> {
        let Some(expected_signer_ura) = self.selected_execution_host_ura() else {
            return Ok(());
        };
        let actual_signer_ura = receipt_signer_ura(receipt).ok_or_else(|| {
            invalid(format!(
                "{stage} checkpoint has no effective receipt signer binding"
            ))
        })?;
        if actual_signer_ura != expected_signer_ura {
            return Err(invalid(format!(
                "{stage} checkpoint signer `{actual_signer_ura}` does not match selected execution host `{expected_signer_ura}`"
            )));
        }
        Ok(())
    }
}

/// Immutable projection of one verified forwarded lifecycle closure.
#[derive(Debug, Clone)]
pub(crate) struct ForwardedFinalizedInvocation {
    pub(crate) admission_receipt: WireInvocationReceipt,
    pub(crate) terminal_receipt: WireInvocationReceipt,
    pub(crate) terminal_state: InvocationState,
    pub(crate) output: Vec<u8>,
    pub(crate) output_content_type: String,
    pub(crate) failure: Option<Error>,
}

impl ForwardedFinalizedInvocation {
    pub(crate) fn verify_with_carrier_result(
        binding: &ForwardedInvocationBinding,
        admission: WireInvocationReceipt,
        terminal: WireInvocationReceipt,
        carrier_payload: Vec<u8>,
        carrier_result_content_type: String,
        resolver: &dyn KeyResolver,
    ) -> Result<Self, Status> {
        let verified = verify_finalization_proofs(admission, terminal, resolver)?;
        let mut finalized = Self::from_verified(binding, verified)?;
        finalized.bind_carrier_result(carrier_payload, carrier_result_content_type)?;
        Ok(finalized)
    }

    fn from_verified(
        binding: &ForwardedInvocationBinding,
        verified: VerifiedFinalizationCheckpoints,
    ) -> Result<Self, Status> {
        let admission = axon_sdk::invocation::wire::receipt_to_wire(verified.admission())
            .map_err(|error| invalid(format!("project verified admission checkpoint: {error}")))?;
        let terminal = axon_sdk::invocation::wire::receipt_to_wire(verified.terminal())
            .map_err(|error| invalid(format!("project verified terminal checkpoint: {error}")))?;
        Self::verify_structure(binding, admission, terminal)
    }

    /// Verify and canonicalize one forwarded unary response. Receipt fields
    /// prove lifecycle closure; the carrier payload/result content type is
    /// accepted only after the payload is byte-bound to the signed terminal
    /// checkpoint.
    pub(crate) fn verify_response(
        binding: &ForwardedInvocationBinding,
        response: InvokeResponse,
        resolver: &dyn KeyResolver,
    ) -> Result<Self, Status> {
        let admission = response
            .admission_receipt
            .ok_or_else(|| invalid("forwarded unary response omitted its admission checkpoint"))?;
        let terminal = response
            .terminal_receipt
            .ok_or_else(|| invalid("forwarded unary response omitted its terminal checkpoint"))?;
        Self::verify_with_carrier_result(
            binding,
            admission,
            terminal,
            response.result,
            response.result_content_type,
            resolver,
        )
    }

    pub(crate) fn into_response(self) -> InvokeResponse {
        let invocation_id = self.terminal_receipt.invocation_id.clone();
        InvokeResponse {
            header: Some(ResponseHeader {
                request_id: invocation_id,
                status: self.terminal_state.as_str().to_string(),
                ..ResponseHeader::default()
            }),
            state: self.terminal_state.to_wire_i32(),
            result: self.output,
            result_content_type: self.output_content_type,
            error: self.failure,
            admission_receipt: Some(self.admission_receipt),
            terminal_receipt: Some(self.terminal_receipt),
            ..InvokeResponse::default()
        }
    }

    fn verify_structure(
        binding: &ForwardedInvocationBinding,
        admission: WireInvocationReceipt,
        terminal: WireInvocationReceipt,
    ) -> Result<Self, Status> {
        verify_admission(binding, &admission)?;
        let terminal_state = verify_terminal(binding, &terminal)?;
        verify_checkpoint_pair(&admission, &terminal)?;

        let computed_output_hash = sha256(&terminal.payload);
        require_hash(
            "terminal.output_hash",
            &terminal.output_hash,
            Some(&computed_output_hash),
        )?;

        let (output, failure) = match terminal_state {
            InvocationState::Completed => {
                if terminal.failure.is_some() {
                    return Err(invalid("completed terminal checkpoint carries a failure"));
                }
                (terminal.payload.clone(), None)
            }
            InvocationState::Failed | InvocationState::TimedOut | InvocationState::Cancelled => {
                let failure = terminal.failure.clone().ok_or_else(|| {
                    invalid("non-completed terminal checkpoint is missing typed failure")
                })?;
                (Vec::new(), Some(failure))
            }
            _ => unreachable!("verify_terminal accepts only terminal states"),
        };

        Ok(Self {
            admission_receipt: admission,
            output_content_type: terminal.payload_content_type.clone(),
            terminal_receipt: terminal,
            terminal_state,
            output,
            failure,
        })
    }

    fn bind_carrier_result(
        &mut self,
        carrier_payload: Vec<u8>,
        carrier_result_content_type: String,
    ) -> Result<(), Status> {
        if self.terminal_state != InvocationState::Completed {
            return Ok(());
        }
        let content_type = carrier_result_content_type.trim();
        if content_type.is_empty() {
            return Err(invalid(
                "completed forwarded invocation omitted carrier result_content_type",
            ));
        }
        if carrier_payload != self.output {
            return Err(invalid(
                "carrier result payload does not match signed terminal checkpoint payload",
            ));
        }
        self.output = carrier_payload;
        self.output_content_type = content_type.to_string();
        Ok(())
    }
}

/// Ordered verifier shared by remote stream and bidi forwarding.
pub(crate) struct ForwardedFinalizationVerifier {
    binding: ForwardedInvocationBinding,
    resolver: Arc<dyn KeyResolver>,
    admission: Option<SignedInvocationReceipt>,
    terminal_seen: bool,
}

pub(crate) async fn ensure_forwarded_receipt_signer_key(
    resolver: &dyn KeyResolver,
    sync: Option<&Arc<DeviceTrustSync>>,
    execution_host_ura: &str,
    surface: &str,
) -> Result<(), Status> {
    ensure_forwarded_receipt_signer_keys(resolver, sync, [execution_host_ura], surface).await
}

pub(crate) async fn ensure_forwarded_response_receipt_signer_keys(
    resolver: &dyn KeyResolver,
    sync: Option<&Arc<DeviceTrustSync>>,
    response: &InvokeResponse,
    surface: &str,
) -> Result<(), Status> {
    let signers = [
        response
            .admission_receipt
            .as_ref()
            .and_then(receipt_signer_ura),
        response
            .terminal_receipt
            .as_ref()
            .and_then(receipt_signer_ura),
    ];
    ensure_forwarded_receipt_signer_keys(resolver, sync, signers.into_iter().flatten(), surface)
        .await
}

async fn ensure_forwarded_receipt_signer_keys<'a>(
    resolver: &dyn KeyResolver,
    sync: Option<&Arc<DeviceTrustSync>>,
    signer_uras: impl IntoIterator<Item = &'a str>,
    surface: &str,
) -> Result<(), Status> {
    let mut checked = Vec::<String>::new();
    for signer_ura in signer_uras {
        let signer_ura = signer_ura.trim();
        if signer_ura.is_empty() || checked.iter().any(|seen| seen == signer_ura) {
            continue;
        }
        checked.push(signer_ura.to_string());
        match resolver.resolve_all(signer_ura) {
            Ok(keys) if !keys.is_empty() => continue,
            Ok(_) => {}
            Err(_) => {}
        }
        let Some(sync) = sync else {
            return Err(Status::failed_precondition(format!(
                "{surface}: remote receipt signer `{signer_ura}` cannot be trusted for forwarded receipt finalization: receipt signer is not present in the canonical receipt trust authority"
            )));
        };
        let status = sync.ensure_caller_key_status(signer_ura, None).await;
        if status.trusted() {
            continue;
        }
        let diagnostic = status
            .diagnostic()
            .unwrap_or_else(|| "receipt signer is not syncable through hub trust".to_string());
        return Err(Status::failed_precondition(format!(
            "{surface}: remote receipt signer `{signer_ura}` cannot be trusted for forwarded receipt finalization: {diagnostic}"
        )));
    }
    Ok(())
}

fn receipt_signer_ura(receipt: &WireInvocationReceipt) -> Option<&str> {
    receipt
        .signer_binding
        .as_ref()
        .or(receipt.callee_binding.as_ref())
        .map(|identity| identity.ura.trim())
        .filter(|ura| !ura.is_empty())
}

impl ForwardedFinalizationVerifier {
    pub(crate) fn new(binding: ForwardedInvocationBinding, resolver: Arc<dyn KeyResolver>) -> Self {
        Self {
            binding,
            resolver,
            admission: None,
            terminal_seen: false,
        }
    }

    pub(crate) fn admit(&mut self, receipt: WireInvocationReceipt) -> Result<(), Status> {
        if self.terminal_seen {
            return Err(invalid("admission checkpoint arrived after terminal"));
        }
        if self.admission.is_some() {
            return Err(invalid(
                "forwarded invocation emitted admission more than once",
            ));
        }
        let receipt = verify_wire_receipt(
            receipt,
            self.resolver.as_ref(),
            ReceiptCheckpointStage::Admission,
        )?;
        let canonical = axon_sdk::invocation::wire::receipt_to_wire(&receipt)
            .map_err(|error| invalid(format!("project verified admission checkpoint: {error}")))?;
        verify_admission(&self.binding, &canonical)?;
        self.admission = Some(receipt);
        Ok(())
    }

    pub(crate) fn observe_data(&self) -> Result<(), Status> {
        if self.terminal_seen {
            return Err(invalid("forwarded data arrived after terminal"));
        }
        if self.admission.is_none() {
            return Err(invalid(
                "forwarded data arrived before the admission checkpoint",
            ));
        }
        Ok(())
    }

    pub(crate) fn finalize(
        &mut self,
        admission_on_terminal: Option<WireInvocationReceipt>,
        terminal: WireInvocationReceipt,
    ) -> Result<ForwardedFinalizedInvocation, Status> {
        if self.terminal_seen {
            return Err(invalid(
                "forwarded invocation emitted terminal more than once",
            ));
        }
        if let Some(admission) = admission_on_terminal {
            self.admit(admission)?;
        }
        self.terminal_seen = true;
        let admission = self.admission.clone().ok_or_else(|| {
            invalid("terminal checkpoint arrived before the admission checkpoint")
        })?;
        let terminal = verify_wire_receipt(
            terminal,
            self.resolver.as_ref(),
            ReceiptCheckpointStage::Terminal,
        )?;
        let verified = finalization_projection::verify_signed_finalization_checkpoints(
            &admission,
            &terminal,
            self.resolver.as_ref(),
        )
        .map_err(forwarded_finalization_error)?;
        ForwardedFinalizedInvocation::from_verified(&self.binding, verified)
    }

    /// Close a streamed/bidi lifecycle and prove that the carrier's terminal
    /// result is exactly the payload signed into the terminal checkpoint.
    pub(crate) fn finalize_with_carrier_result(
        &mut self,
        admission_on_terminal: Option<WireInvocationReceipt>,
        terminal: WireInvocationReceipt,
        carrier_payload: Vec<u8>,
        carrier_result_content_type: String,
    ) -> Result<ForwardedFinalizedInvocation, Status> {
        let mut finalized = self.finalize(admission_on_terminal, terminal)?;
        finalized.bind_carrier_result(carrier_payload, carrier_result_content_type)?;
        Ok(finalized)
    }
}

fn verify_wire_receipt(
    receipt: WireInvocationReceipt,
    resolver: &dyn KeyResolver,
    stage: ReceiptCheckpointStage,
) -> Result<SignedInvocationReceipt, Status> {
    finalization_projection::verify_wire_checkpoint(receipt, resolver, stage)
        .map_err(forwarded_finalization_error)
}

fn verify_finalization_proofs(
    admission: WireInvocationReceipt,
    terminal: WireInvocationReceipt,
    resolver: &dyn KeyResolver,
) -> Result<VerifiedFinalizationCheckpoints, Status> {
    finalization_projection::verify_wire_finalization_checkpoints(admission, terminal, resolver)
        .map_err(forwarded_finalization_error)
}

fn forwarded_finalization_error(error: FinalizationProjectionError) -> Status {
    match error {
        FinalizationProjectionError::Finalization { source } => {
            invalid(format!("finalization cryptographic proof failed: {source}"))
        }
        other => invalid(other.to_string()),
    }
}

fn verify_admission(
    binding: &ForwardedInvocationBinding,
    receipt: &WireInvocationReceipt,
) -> Result<(), Status> {
    let state = receipt_state(receipt, "admission")?;
    if state != InvocationState::Admitted || receipt.receipt_type != "admitted" {
        return Err(invalid(
            "admission checkpoint must have state Admitted and type admitted",
        ));
    }
    verify_request_binding(binding, receipt, "admission")?;
    let computed_output_hash = sha256(&receipt.payload);
    require_hash(
        "admission.output_hash",
        &receipt.output_hash,
        Some(&computed_output_hash),
    )?;
    Ok(())
}

fn verify_terminal(
    binding: &ForwardedInvocationBinding,
    receipt: &WireInvocationReceipt,
) -> Result<InvocationState, Status> {
    let state = receipt_state(receipt, "terminal")?;
    if !state.is_terminal() || receipt.receipt_type != state.as_str() {
        return Err(invalid(
            "terminal checkpoint state and receipt_type do not identify one terminal state",
        ));
    }
    if !receipt.cleanup_complete {
        return Err(invalid(
            "terminal checkpoint was published before cleanup completed",
        ));
    }
    verify_request_binding(binding, receipt, "terminal")?;
    Ok(state)
}

fn verify_request_binding(
    binding: &ForwardedInvocationBinding,
    receipt: &WireInvocationReceipt,
    stage: &str,
) -> Result<(), Status> {
    if receipt.invocation_id.trim().is_empty() {
        return Err(invalid(format!("{stage} checkpoint has no invocation_id")));
    }
    if receipt.caller_binding != binding.envelope.caller
        || receipt.callee_binding != binding.envelope.callee
        || receipt.subject_binding != binding.envelope.subject
        || receipt.invocation_nonce != binding.envelope.invocation_nonce
        || receipt.causal_binding != binding.envelope.causal_context
    {
        return Err(invalid(format!(
            "{stage} checkpoint does not bind the submitted invocation seven-tuple"
        )));
    }
    if receipt.ability_binding != binding.ability_binding {
        return Err(invalid(format!(
            "{stage} checkpoint ability binding differs from the signed descriptor ref"
        )));
    }
    binding.verify_receipt_authority(receipt, stage)?;
    require_hash(
        &format!("{stage}.input_hash"),
        &receipt.input_hash,
        Some(&binding.input_hash),
    )?;
    require_hash(&format!("{stage}.self_hash"), &receipt.self_hash, None)?;
    let signature = receipt
        .callee_signature
        .as_ref()
        .ok_or_else(|| invalid(format!("{stage} checkpoint is missing callee signature")))?;
    if signature.algorithm.trim().is_empty() || signature.signature.is_empty() {
        return Err(invalid(format!(
            "{stage} checkpoint has an incomplete callee signature"
        )));
    }
    if receipt.authority_binding.is_none()
        || receipt.authority_proof.is_none()
        || receipt.subject_ref.is_none()
        || receipt.descriptor_version.trim().is_empty()
        || receipt.runtime_env.trim().is_empty()
    {
        return Err(invalid(format!(
            "{stage} checkpoint is missing descriptor or authority proof binding"
        )));
    }
    require_hash(&format!("{stage}.schema_hash"), &receipt.schema_hash, None)?;
    require_hash(&format!("{stage}.impl_hash"), &receipt.impl_hash, None)?;
    Ok(())
}

fn verify_checkpoint_pair(
    admission: &WireInvocationReceipt,
    terminal: &WireInvocationReceipt,
) -> Result<(), Status> {
    if admission.invocation_id != terminal.invocation_id {
        return Err(invalid(
            "admission and terminal checkpoints have different invocation ids",
        ));
    }
    if terminal.index <= admission.index {
        return Err(invalid(
            "terminal checkpoint index must be greater than admission checkpoint index",
        ));
    }
    if admission.timestamp_unix_ms > terminal.timestamp_unix_ms {
        return Err(invalid(
            "terminal checkpoint timestamp precedes admission checkpoint",
        ));
    }
    if admission.caller_binding != terminal.caller_binding
        || admission.callee_binding != terminal.callee_binding
        || admission.subject_binding != terminal.subject_binding
        || admission.invocation_nonce != terminal.invocation_nonce
        || admission.causal_binding != terminal.causal_binding
        || admission.signer_binding != terminal.signer_binding
        || admission.host_attestation != terminal.host_attestation
        || admission.authority_binding != terminal.authority_binding
        || admission.ability_binding != terminal.ability_binding
        || admission.subject_ref != terminal.subject_ref
        || admission.descriptor_version != terminal.descriptor_version
        || admission.schema_hash != terminal.schema_hash
        || admission.impl_hash != terminal.impl_hash
        || admission.runtime_env != terminal.runtime_env
        || admission.authority_proof != terminal.authority_proof
        || admission.input_hash != terminal.input_hash
        || admission.parent_receipts != terminal.parent_receipts
    {
        return Err(invalid(
            "terminal checkpoint changed admission-bound invocation or proof facts",
        ));
    }
    Ok(())
}

fn receipt_state(receipt: &WireInvocationReceipt, stage: &str) -> Result<InvocationState, Status> {
    InvocationState::try_from(receipt.state)
        .map_err(|_| invalid(format!("{stage} checkpoint has an unknown state")))
}

fn require_hash(label: &str, actual: &[u8], expected: Option<&[u8; 32]>) -> Result<(), Status> {
    if actual.len() != 32 || actual.iter().all(|byte| *byte == 0) {
        return Err(invalid(format!("{label} must be a non-zero SHA-256 hash")));
    }
    if expected.is_some_and(|expected| actual != expected) {
        return Err(invalid(format!("{label} does not match canonical bytes")));
    }
    Ok(())
}

fn invalid(message: impl Into<String>) -> Status {
    Status::failed_precondition(format!(
        "FORWARDED_FINALIZATION_INVALID: {}",
        message.into()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axon_sdk::pb::axon::v1::{
        causal_context, AgentIdentity, AuthorityBinding, CalleeSignature, CausalContext, Empty,
        EntityRef, InvocationAuthorityProof, SubjectIdentity,
    };
    use ed25519_dalek::{SigningKey, VerifyingKey};

    const CALLEE: &str = "easynet:///r/acme/agent/device.edge-1.runtime-introspection";

    fn descriptor_ref() -> String {
        let binding = crate::daemon::axon_bridge::descriptor_ref::descriptor_binding_for_wire(
            "1.0.0", [0xaa; 32], "invoke",
        )
        .expect("test descriptor binding");
        crate::daemon::axon_bridge::descriptor_ref::ability_descriptor_ref_for_wire(
            CALLEE, "job.run", &binding,
        )
        .expect("test descriptor ref")
    }

    struct RejectingResolver;

    impl KeyResolver for RejectingResolver {
        fn resolve(
            &self,
            _agent_ura: &str,
        ) -> Result<VerifyingKey, axon_sdk::invocation::AxonError> {
            Err(axon_sdk::invocation::AxonError::permission_denied(
                "test resolver rejects forged receipt",
            ))
        }
    }

    struct SingleAgentResolver {
        agent_ura: &'static str,
        key: VerifyingKey,
    }

    impl SingleAgentResolver {
        fn new(agent_ura: &'static str) -> Self {
            Self {
                agent_ura,
                key: SigningKey::from_bytes(&[0x21; 32]).verifying_key(),
            }
        }
    }

    impl KeyResolver for SingleAgentResolver {
        fn resolve(
            &self,
            agent_ura: &str,
        ) -> Result<VerifyingKey, axon_sdk::invocation::AxonError> {
            if agent_ura == self.agent_ura {
                Ok(self.key)
            } else {
                Err(axon_sdk::invocation::AxonError::permission_denied(
                    "test resolver rejects unknown receipt signer",
                ))
            }
        }
    }

    fn request() -> InvokeRequest {
        InvokeRequest {
            envelope: Some(Envelope {
                caller: Some(agent("easynet:///r/acme/agent/caller")),
                callee: Some(agent(CALLEE)),
                subject: Some(SubjectIdentity {
                    ura: "easynet:///r/acme/resource/job/1".to_string(),
                    profile: "axon-strict-v2".to_string(),
                }),
                invocation_nonce: vec![7; 16],
                causal_context: Some(CausalContext {
                    form: Some(causal_context::Form::None(Empty {})),
                }),
                ..Envelope::default()
            }),
            target: Some(
                crate::daemon::invocation::dispatch::invocation_wire::wire_invocation_target(
                    descriptor_ref(),
                    "job.run",
                )
                .unwrap(),
            ),
            arguments: b"{}".to_vec(),
            ..InvokeRequest::default()
        }
    }

    fn agent(ura: &str) -> AgentIdentity {
        AgentIdentity {
            ura: ura.to_string(),
            profile: "axon-strict-v2".to_string(),
        }
    }

    fn checkpoint_for_request(
        request: &InvokeRequest,
        state: InvocationState,
        index: u64,
        payload: &[u8],
    ) -> WireInvocationReceipt {
        let envelope = request.envelope.clone().expect("envelope");
        let callee_ura = envelope.callee.as_ref().expect("callee").ura.as_str();
        let ability_binding = super::super::invocation_wire::descriptor_ref_from_invocation_target(
            "forwarded finalization test",
            callee_ura,
            request.target.as_ref(),
        )
        .expect("descriptor ref");
        WireInvocationReceipt {
            index,
            invocation_id: "inv-real-binding".to_string(),
            receipt_type: state.as_str().to_string(),
            state: state.to_wire_i32(),
            timestamp_unix_ms: 100 + index as i64,
            self_hash: vec![index as u8 + 1; 32],
            payload: payload.to_vec(),
            payload_content_type: "application/octet-stream".to_string(),
            cleanup_complete: state.is_terminal(),
            caller_binding: envelope.caller,
            callee_binding: envelope.callee,
            subject_binding: envelope.subject,
            invocation_nonce: envelope.invocation_nonce,
            causal_binding: envelope.causal_context,
            callee_signature: Some(CalleeSignature {
                algorithm: "ed25519".to_string(),
                signature: vec![index as u8 + 3; 64],
                ..CalleeSignature::default()
            }),
            authority_binding: Some(AuthorityBinding::default()),
            ability_binding,
            subject_ref: Some(EntityRef {
                ura: "easynet:///r/acme/resource/job/1".to_string(),
                profile: "axon-strict-v2".to_string(),
                ..EntityRef::default()
            }),
            descriptor_version: "1.0.0".to_string(),
            schema_hash: vec![0x31; 32],
            impl_hash: vec![0x32; 32],
            runtime_env: "test".to_string(),
            authority_proof: Some(InvocationAuthorityProof::default()),
            input_hash: sha256(&request.arguments).to_vec(),
            output_hash: sha256(payload).to_vec(),
            ..WireInvocationReceipt::default()
        }
    }

    fn checkpoint(state: InvocationState, index: u64, payload: &[u8]) -> WireInvocationReceipt {
        checkpoint_for_request(&request(), state, index, payload)
    }

    fn selected_route() -> SelectedInvokeRoute {
        SelectedInvokeRoute::test_local_runtime(CALLEE, "job.run", "job.run")
    }

    fn bind_receipt_to_host(receipt: &mut WireInvocationReceipt, host_ura: &str) {
        receipt.signer_binding = Some(agent(host_ura));
        receipt.host_attestation = vec![0x41; 64];
    }

    #[test]
    fn accepts_non_adjacent_checkpoints_and_empty_output() {
        let binding = ForwardedInvocationBinding::for_delegated_request(&request()).unwrap();
        let admission = checkpoint(InvocationState::Admitted, 1, b"");
        let terminal = checkpoint(InvocationState::Completed, 7, b"");
        let finalized =
            ForwardedFinalizedInvocation::verify_structure(&binding, admission, terminal).unwrap();
        assert_eq!(finalized.terminal_state, InvocationState::Completed);
        assert!(finalized.output.is_empty());
    }

    #[test]
    fn selected_route_binds_descriptor_owner_callee_and_receipt_host() {
        let route = selected_route();
        let request = request();
        let binding = ForwardedInvocationBinding::for_selected_route(&request, &route)
            .expect("selected route contract");
        let mut admission = checkpoint_for_request(&request, InvocationState::Admitted, 1, b"");
        let mut terminal = checkpoint_for_request(&request, InvocationState::Completed, 7, b"done");
        bind_receipt_to_host(&mut admission, &route.execution_host_ura);
        bind_receipt_to_host(&mut terminal, &route.execution_host_ura);

        let finalized =
            ForwardedFinalizedInvocation::verify_structure(&binding, admission, terminal)
                .expect("selected host receipt contract");

        assert_eq!(finalized.terminal_state, InvocationState::Completed);
    }

    #[test]
    fn selected_route_rejects_descriptor_proof_drift_before_forwarding() {
        let mut route = selected_route();
        route.descriptor_ref = route
            .descriptor_ref
            .replace(&"a".repeat(64), &"b".repeat(64));

        let error = ForwardedInvocationBinding::for_selected_route(&request(), &route)
            .expect_err("route descriptor drift must fail closed");

        assert!(error.message().contains("descriptor ref"));
    }

    #[test]
    fn selected_route_rejects_ability_owner_and_callee_drift_before_forwarding() {
        let mut route = selected_route();
        route.owner_ura = crate::core::ura::hub_ura("acme");

        let error = ForwardedInvocationBinding::for_selected_route(&request(), &route)
            .expect_err("route owner drift must fail closed");

        assert!(error.message().contains("descriptor owner"));
    }

    #[test]
    fn selected_route_rejects_non_selected_signer_at_post_axon_policy_boundary() {
        let route = selected_route();
        let request = request();
        let binding = ForwardedInvocationBinding::for_selected_route(&request, &route)
            .expect("selected route contract");
        let wrong_host = "easynet:///r/acme/device/edge-2";
        let mut admission = checkpoint_for_request(&request, InvocationState::Admitted, 1, b"");
        let mut terminal = checkpoint_for_request(&request, InvocationState::Completed, 7, b"done");
        bind_receipt_to_host(&mut admission, wrong_host);
        bind_receipt_to_host(&mut terminal, wrong_host);

        let error = ForwardedFinalizedInvocation::verify_structure(&binding, admission, terminal)
            .expect_err("Axon's projected signer must still match the locally selected host");

        assert!(error.message().contains("selected execution host"));
    }

    #[test]
    fn selected_authority_route_accepts_self_signed_receipt_authority() {
        let authority_ura = crate::core::ura::hub_ura("acme");
        let route = SelectedInvokeRoute::test_local_runtime(
            &authority_ura,
            "federation.status",
            "federation.status",
        );
        let request = InvokeRequest {
            envelope: Some(Envelope {
                caller: Some(agent("easynet:///r/acme/device/caller")),
                callee: Some(agent(&authority_ura)),
                subject: Some(SubjectIdentity {
                    ura: "easynet:///r/acme/resource/federation/status".to_string(),
                    profile: "axon-strict-v2".to_string(),
                }),
                invocation_nonce: vec![7; 16],
                causal_context: Some(CausalContext {
                    form: Some(causal_context::Form::None(Empty {})),
                }),
                ..Envelope::default()
            }),
            target: Some(
                crate::daemon::invocation::dispatch::invocation_wire::wire_invocation_target(
                    &route.descriptor_ref,
                    "federation.status",
                )
                .expect("Authority descriptor target"),
            ),
            arguments: b"{}".to_vec(),
            ..InvokeRequest::default()
        };
        let binding = ForwardedInvocationBinding::for_selected_route(&request, &route)
            .expect("selected Authority route contract");
        let admission = checkpoint_for_request(&request, InvocationState::Admitted, 1, b"");
        let terminal = checkpoint_for_request(&request, InvocationState::Completed, 7, b"done");

        ForwardedFinalizedInvocation::verify_structure(&binding, admission, terminal)
            .expect("Authority self-signature is the selected execution authority");
    }

    #[test]
    fn rejects_changed_real_invocation_binding() {
        let binding = ForwardedInvocationBinding::for_delegated_request(&request()).unwrap();
        let admission = checkpoint(InvocationState::Admitted, 1, b"");
        let mut terminal = checkpoint(InvocationState::Completed, 5, b"done");
        terminal.invocation_nonce[0] ^= 1;
        let error = ForwardedFinalizedInvocation::verify_structure(&binding, admission, terminal)
            .expect_err("changed nonce must fail closed");
        assert!(error.message().contains("seven-tuple"));
    }

    #[test]
    fn rejects_admission_payload_not_bound_by_output_hash() {
        let binding = ForwardedInvocationBinding::for_delegated_request(&request()).unwrap();
        let mut admission = checkpoint(InvocationState::Admitted, 1, b"");
        admission.payload = b"tampered admission payload".to_vec();
        let terminal = checkpoint(InvocationState::Completed, 5, b"done");

        let error = ForwardedFinalizedInvocation::verify_structure(&binding, admission, terminal)
            .expect_err("admission payload must remain bound to its proof facts");

        assert!(error.message().contains("admission.output_hash"));
    }

    #[test]
    fn ordered_verifier_rejects_checkpoints_after_terminal() {
        let binding = ForwardedInvocationBinding::for_delegated_request(&request()).unwrap();
        let mut verifier = ForwardedFinalizationVerifier {
            binding,
            resolver: Arc::new(RejectingResolver),
            admission: None,
            terminal_seen: true,
        };
        assert!(verifier
            .admit(checkpoint(InvocationState::Admitted, 1, b""))
            .is_err());
        assert!(verifier
            .finalize(None, checkpoint(InvocationState::Completed, 5, b"done"))
            .is_err());
    }

    #[test]
    fn projects_cancelled_and_timed_out_from_terminal_failure() {
        for state in [InvocationState::Cancelled, InvocationState::TimedOut] {
            let binding = ForwardedInvocationBinding::for_delegated_request(&request()).unwrap();
            let admission = checkpoint(InvocationState::Admitted, 2, b"");
            let mut terminal = checkpoint(state, 8, b"canonical failure");
            terminal.failure = Some(Error {
                code: state.as_str().to_ascii_uppercase(),
                message: "canonical failure".to_string(),
                ..Error::default()
            });
            let finalized =
                ForwardedFinalizedInvocation::verify_structure(&binding, admission, terminal)
                    .unwrap();
            assert_eq!(finalized.terminal_state, state);
            assert!(finalized.output.is_empty());
            assert!(finalized.failure.is_some());
        }
    }

    #[test]
    fn carrier_bound_verifier_rejects_receipts_without_valid_cryptographic_proof() {
        let binding = ForwardedInvocationBinding::for_delegated_request(&request()).unwrap();
        let admission = checkpoint(InvocationState::Admitted, 1, b"");
        let terminal = checkpoint(InvocationState::Completed, 4, b"done");

        let error = ForwardedFinalizedInvocation::verify_with_carrier_result(
            &binding,
            admission,
            terminal,
            b"done".to_vec(),
            "application/json".to_string(),
            &RejectingResolver,
        )
        .expect_err("shape-correct forged receipts must fail closed");

        assert!(error.message().contains("proof"));
    }

    #[test]
    fn completed_forwarded_result_requires_carrier_content_type() {
        let binding = ForwardedInvocationBinding::for_delegated_request(&request()).unwrap();
        let admission = checkpoint(InvocationState::Admitted, 1, b"");
        let terminal = checkpoint(InvocationState::Completed, 4, b"done");
        let mut finalized =
            ForwardedFinalizedInvocation::verify_structure(&binding, admission, terminal).unwrap();

        let error = finalized
            .bind_carrier_result(b"done".to_vec(), "  ".to_string())
            .expect_err("completed carrier result must name its content type");

        assert!(error.message().contains("result_content_type"));
    }

    #[test]
    fn completed_forwarded_result_payload_must_match_terminal_checkpoint() {
        let binding = ForwardedInvocationBinding::for_delegated_request(&request()).unwrap();
        let admission = checkpoint(InvocationState::Admitted, 1, b"");
        let terminal = checkpoint(InvocationState::Completed, 4, b"done");
        let mut finalized =
            ForwardedFinalizedInvocation::verify_structure(&binding, admission, terminal).unwrap();

        let error = finalized
            .bind_carrier_result(b"tampered".to_vec(), "application/json".to_string())
            .expect_err("carrier payload cannot diverge from terminal checkpoint payload");

        assert!(error.message().contains("payload"));
    }

    #[test]
    fn unsigned_wire_failure_duplicate_cannot_enter_trusted_domain() {
        let mut terminal = checkpoint(InvocationState::Cancelled, 4, b"canonical cancellation");
        terminal.reason = "canonical cancellation".to_string();
        terminal.failure = Some(Error {
            code: "TAMPERED".to_string(),
            message: "attacker controlled duplicate".to_string(),
            ..Error::default()
        });

        axon_sdk::invocation::wire::try_receipt_from_wire(terminal)
            .expect_err("forged wire receipt must not enter the trusted domain");
    }

    #[tokio::test]
    async fn forwarded_receipt_signer_accepts_canonical_receipt_trust_authority() {
        let authority_ura = "easynet:///r/acme/authority";
        let resolver = SingleAgentResolver::new(authority_ura);

        ensure_forwarded_receipt_signer_key(
            &resolver,
            None,
            authority_ura,
            "remote Invoke session escalation",
        )
        .await
        .expect(
            "authority signer present in receipt trust authority must pass without device sync",
        );
    }

    #[tokio::test]
    async fn forwarded_receipt_signer_rejects_unknown_signer_without_sync() {
        let resolver = SingleAgentResolver::new("easynet:///r/acme/authority");

        let error = ensure_forwarded_receipt_signer_key(
            &resolver,
            None,
            "easynet:///r/acme/device/unknown",
            "remote Invoke session escalation",
        )
        .await
        .expect_err("unknown receipt signer must fail closed when no sync source can attest it");

        assert!(
            error
                .message()
                .contains("canonical receipt trust authority"),
            "unexpected diagnostic: {error}"
        );
    }
}
