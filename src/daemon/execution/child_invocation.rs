//! Canonical child-invocation receipt facts shared by composite executors.
//!
//! Product-specific executors may request child work, but dependency edges
//! between nested invocations are runtime facts: an admitted child envelope,
//! its canonical invocation URA, and the signed terminal receipt anchor.

use axon_sdk::invocation::InvocationEnvelope;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ChildInvocationReceiptAnchor {
    invocation_ura: String,
    receipt_ura: String,
    receipt_hash: [u8; 32],
}

impl ChildInvocationReceiptAnchor {
    pub(crate) fn new(
        invocation_ura: impl Into<String>,
        receipt_ura: impl Into<String>,
        receipt_hash: [u8; 32],
    ) -> Self {
        Self {
            invocation_ura: invocation_ura.into(),
            receipt_ura: receipt_ura.into(),
            receipt_hash,
        }
    }

    pub(crate) fn projection(&self) -> Value {
        serde_json::json!({
            "invocation_ura": self.invocation_ura,
            "receipt_ura": self.receipt_ura,
            "receipt_hash": hex::encode(self.receipt_hash),
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ChildInvocationRecord {
    envelope: InvocationEnvelope,
    invocation_ura: String,
    terminal_receipt: ChildInvocationReceiptAnchor,
    dependency_receipts: Vec<ChildInvocationReceiptAnchor>,
}

impl ChildInvocationRecord {
    pub(crate) fn new(
        envelope: InvocationEnvelope,
        invocation_ura: impl Into<String>,
        terminal_receipt: ChildInvocationReceiptAnchor,
        dependency_receipts: Vec<ChildInvocationReceiptAnchor>,
    ) -> Self {
        Self {
            envelope,
            invocation_ura: invocation_ura.into(),
            terminal_receipt,
            dependency_receipts,
        }
    }

    pub(crate) fn terminal_receipt(&self) -> &ChildInvocationReceiptAnchor {
        &self.terminal_receipt
    }

    pub(crate) fn projection(&self) -> Value {
        serde_json::json!({
            "invocation_ura": self.invocation_ura,
            "caller_ura": self.envelope.caller.ura,
            "callee_ura": self.envelope.callee.ura,
            "ability": self.envelope.ability,
            "subject_ura": self.envelope.subject.ura,
            "invocation_nonce": hex::encode(self.envelope.invocation_nonce),
            "causal_context": crate::daemon::invocation::causal_context_projection::causal_context_projection(
                &self.envelope.causal_context,
            ),
            "dependency_receipts": self.dependency_receipts.iter()
                .map(ChildInvocationReceiptAnchor::projection)
                .collect::<Vec<_>>(),
            "terminal_receipt": self.terminal_receipt.projection(),
        })
    }

    #[cfg(test)]
    pub(crate) fn for_test(ability: &str, marker: u8) -> Self {
        let caller = axon_sdk::invocation::AgentIdentity::new(
            "easynet:///r/test/device/child",
            axon_sdk::invocation::UraProfile::StrictV2,
        );
        let callee = axon_sdk::invocation::AgentIdentity::new(
            "easynet:///r/test/agent/worker",
            axon_sdk::invocation::UraProfile::StrictV2,
        );
        let subject = axon_sdk::invocation::SubjectIdentity::new(
            "easynet:///r/test/resource/child",
            axon_sdk::invocation::UraProfile::StrictV2,
        );
        let invocation_id = format!("child-test-{marker:02x}");
        let invocation_ura = crate::core::ura::invocation_record_ura_for_binding(
            &subject.ura,
            &callee.ura,
            &caller.ura,
            &invocation_id,
        )
        .expect("test identities own canonical Invocation records");
        let envelope = axon_sdk::invocation::CanonicalEnvelopeBuilder::new(
            caller,
            callee,
            subject,
            axon_sdk::invocation::InvocationDerivationPolicy::Explicit {
                invocation_nonce: [marker; 16],
                causal_context: axon_sdk::invocation::CausalContext::None,
            },
        )
        .and_then(|builder| builder.invocation_envelope(ability, &[marker]))
        .expect("construct canonical child Invocation");
        let terminal_receipt = ChildInvocationReceiptAnchor::new(
            invocation_ura.clone(),
            format!("{invocation_ura}/receipt/1"),
            [marker; 32],
        );
        Self::new(envelope, invocation_ura, terminal_receipt, Vec::new())
    }
}

#[derive(Debug)]
pub(crate) struct ChildInvocationOutcome {
    pub(crate) value: Value,
    pub(crate) invocation: ChildInvocationRecord,
}
