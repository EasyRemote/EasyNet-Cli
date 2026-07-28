//! Daemon causal-context JSON projection.
//!
//! Axon owns the typed [`CausalContext`] model. The daemon needs a JSON
//! projection for ability envelope contexts, child-invocation records, plugin
//! receipts, and bridge adapters. This module is the single daemon-owned
//! projection boundary so those consumers do not preserve independent protocol
//! dialects.

use axon_sdk::invocation::CausalContext;
use serde_json::Value;

pub(crate) fn causal_context_projection(causal: &CausalContext) -> Value {
    match causal {
        CausalContext::None => serde_json::json!({"form": "none"}),
        CausalContext::Scalar(receipt) => serde_json::json!({
            "form": "scalar",
            "receipt_hash": hex::encode(receipt.receipt_hash),
            "receipt_ura": receipt.receipt_ura,
        }),
        CausalContext::List(receipts) => serde_json::json!({
            "form": "list",
            "receipts": receipts.iter().map(|receipt| serde_json::json!({
                "receipt_hash": hex::encode(receipt.receipt_hash),
                "receipt_ura": receipt.receipt_ura,
            })).collect::<Vec<_>>(),
        }),
        CausalContext::Merkle { root, proof_ura } => serde_json::json!({
            "form": "merkle",
            "root": hex::encode(root),
            "proof_ura": proof_ura,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axon_sdk::invocation::ReceiptRef;

    #[test]
    fn projection_uses_canonical_form_field() {
        assert_eq!(
            causal_context_projection(&CausalContext::None),
            serde_json::json!({"form": "none"})
        );

        let scalar = causal_context_projection(&CausalContext::Scalar(ReceiptRef {
            receipt_ura: "easynet:///r/test/resource/job/receipt/1".to_string(),
            receipt_hash: [0xab; 32],
        }));
        assert_eq!(scalar["form"], "scalar");
        assert!(scalar.get("kind").is_none());
    }
}
