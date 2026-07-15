//! Verification of finalized invocations forwarded across daemon sessions.
//!
//! This module verifies two lifecycle checkpoints, not a complete receipt
//! chain. Admission and terminal receipts need not be adjacent. Full-chain
//! verification remains an Axon verifier concern and requires all intermediate
//! receipts or a separately verifiable chain proof.

use std::sync::Arc;

use easynet_axon::invocation::{
    sha256, InvocationState, KeyResolver, SignedInvocationReceipt, VerifiedFinalizationCheckpoints,
};
use easynet_axon::pb::axon::v1::{
    Envelope, Error, InvocationReceipt as WireInvocationReceipt, InvokeRequest, InvokeResponse,
    ResponseHeader,
};
use tonic::Status;

use crate::daemon::invocation::receipts::finalization_projection::{
    self, FinalizationProjectionError, ReceiptCheckpointStage,
};

use super::invocation_wire::SIGNED_DESCRIPTOR_REF_METADATA_KEY;

#[derive(Debug, Clone)]
pub(crate) struct ForwardedInvocationBinding {
    envelope: Envelope,
    ability_binding: String,
    input_hash: [u8; 32],
}

impl ForwardedInvocationBinding {
    pub(crate) fn from_request(request: &InvokeRequest) -> Result<Self, Status> {
        let envelope = request
            .envelope
            .clone()
            .ok_or_else(|| invalid("forwarded invocation is missing its seven-tuple envelope"))?;
        let ability_binding = request
            .metadata
            .get(SIGNED_DESCRIPTOR_REF_METADATA_KEY)
            .map(String::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                invalid(format!(
                    "forwarded invocation is missing `{SIGNED_DESCRIPTOR_REF_METADATA_KEY}`"
                ))
            })?
            .to_string();
        Ok(Self {
            envelope,
            ability_binding,
            input_hash: sha256(&request.arguments),
        })
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
    pub(crate) fn verify(
        binding: &ForwardedInvocationBinding,
        admission: WireInvocationReceipt,
        terminal: WireInvocationReceipt,
        resolver: &dyn KeyResolver,
    ) -> Result<Self, Status> {
        let verified = verify_finalization_proofs(admission, terminal, resolver)?;
        Self::from_verified(binding, verified)
    }

    fn from_verified(
        binding: &ForwardedInvocationBinding,
        verified: VerifiedFinalizationCheckpoints,
    ) -> Result<Self, Status> {
        let admission = easynet_axon::invocation::wire::receipt_to_wire(verified.admission())
            .map_err(|error| invalid(format!("project verified admission checkpoint: {error}")))?;
        let terminal = easynet_axon::invocation::wire::receipt_to_wire(verified.terminal())
            .map_err(|error| invalid(format!("project verified terminal checkpoint: {error}")))?;
        Self::verify_structure(binding, admission, terminal)
    }

    /// Verify and canonicalize one forwarded unary response. Every response
    /// field is reconstructed from the signed terminal checkpoint; untrusted
    /// transport duplicates such as `state`, `result`, and `error` are never
    /// consumed after verification.
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
        Self::verify(binding, admission, terminal, resolver)
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
}

/// Ordered verifier shared by remote stream and bidi forwarding.
pub(crate) struct ForwardedFinalizationVerifier {
    binding: ForwardedInvocationBinding,
    resolver: Arc<dyn KeyResolver>,
    admission: Option<SignedInvocationReceipt>,
    terminal_seen: bool,
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
        let canonical = easynet_axon::invocation::wire::receipt_to_wire(&receipt)
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
    if receipt.output_hash.iter().any(|byte| *byte != 0) {
        return Err(invalid(
            "admission checkpoint must not bind a terminal output hash",
        ));
    }
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
    use easynet_axon::pb::axon::v1::{
        causal_context, AgentIdentity, AuthorityBinding, CalleeSignature, CausalContext, Empty,
        EntityRef, InvocationAuthorityProof, SubjectIdentity,
    };
    use ed25519_dalek::VerifyingKey;

    const DESCRIPTOR: &str = "easynet:///r/acme/device/callee/ability/job.run@1.0.0";

    struct RejectingResolver;

    impl KeyResolver for RejectingResolver {
        fn resolve(
            &self,
            _agent_ura: &str,
        ) -> Result<VerifyingKey, easynet_axon::invocation::AxonError> {
            Err(easynet_axon::invocation::AxonError::permission_denied(
                "test resolver rejects forged receipt",
            ))
        }
    }

    fn request() -> InvokeRequest {
        let mut request = InvokeRequest {
            envelope: Some(Envelope {
                caller: Some(agent("easynet:///r/acme/agent/caller")),
                callee: Some(agent("easynet:///r/acme/device/callee")),
                subject: Some(SubjectIdentity {
                    ura: "easynet:///r/acme/resource/job/1".to_string(),
                    profile: "easynet-strict-v2".to_string(),
                }),
                invocation_nonce: vec![7; 16],
                causal_context: Some(CausalContext {
                    form: Some(causal_context::Form::None(Empty {})),
                }),
                ..Envelope::default()
            }),
            function_name: "job.run".to_string(),
            arguments: b"{}".to_vec(),
            ..InvokeRequest::default()
        };
        request.metadata.insert(
            SIGNED_DESCRIPTOR_REF_METADATA_KEY.to_string(),
            DESCRIPTOR.to_string(),
        );
        request
    }

    fn agent(ura: &str) -> AgentIdentity {
        AgentIdentity {
            ura: ura.to_string(),
            profile: "easynet-strict-v2".to_string(),
        }
    }

    fn checkpoint(state: InvocationState, index: u64, payload: &[u8]) -> WireInvocationReceipt {
        let request = request();
        let envelope = request.envelope.expect("envelope");
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
            ability_binding: DESCRIPTOR.to_string(),
            subject_ref: Some(EntityRef {
                ura: "easynet:///r/acme/resource/job/1".to_string(),
                profile: "easynet-strict-v2".to_string(),
                ..EntityRef::default()
            }),
            descriptor_version: "1.0.0".to_string(),
            schema_hash: vec![0x31; 32],
            impl_hash: vec![0x32; 32],
            runtime_env: "test".to_string(),
            authority_proof: Some(InvocationAuthorityProof::default()),
            input_hash: sha256(&request.arguments).to_vec(),
            output_hash: if state.is_terminal() {
                sha256(payload).to_vec()
            } else {
                vec![0; 32]
            },
            ..WireInvocationReceipt::default()
        }
    }

    #[test]
    fn accepts_non_adjacent_checkpoints_and_empty_output() {
        let binding = ForwardedInvocationBinding::from_request(&request()).unwrap();
        let admission = checkpoint(InvocationState::Admitted, 1, b"");
        let terminal = checkpoint(InvocationState::Completed, 7, b"");
        let finalized =
            ForwardedFinalizedInvocation::verify_structure(&binding, admission, terminal).unwrap();
        assert_eq!(finalized.terminal_state, InvocationState::Completed);
        assert!(finalized.output.is_empty());
    }

    #[test]
    fn rejects_changed_real_invocation_binding() {
        let binding = ForwardedInvocationBinding::from_request(&request()).unwrap();
        let admission = checkpoint(InvocationState::Admitted, 1, b"");
        let mut terminal = checkpoint(InvocationState::Completed, 5, b"done");
        terminal.invocation_nonce[0] ^= 1;
        let error = ForwardedFinalizedInvocation::verify_structure(&binding, admission, terminal)
            .expect_err("changed nonce must fail closed");
        assert!(error.message().contains("seven-tuple"));
    }

    #[test]
    fn ordered_verifier_rejects_checkpoints_after_terminal() {
        let binding = ForwardedInvocationBinding::from_request(&request()).unwrap();
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
            let binding = ForwardedInvocationBinding::from_request(&request()).unwrap();
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
    fn public_verifier_rejects_receipts_without_valid_cryptographic_proof() {
        let binding = ForwardedInvocationBinding::from_request(&request()).unwrap();
        let admission = checkpoint(InvocationState::Admitted, 1, b"");
        let terminal = checkpoint(InvocationState::Completed, 4, b"done");

        let error =
            ForwardedFinalizedInvocation::verify(&binding, admission, terminal, &RejectingResolver)
                .expect_err("shape-correct forged receipts must fail closed");

        assert!(error.message().contains("proof"));
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

        easynet_axon::invocation::wire::try_receipt_from_wire(terminal)
            .expect_err("forged wire receipt must not enter the trusted domain");
    }
}
