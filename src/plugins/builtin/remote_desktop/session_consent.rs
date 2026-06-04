// EasyNet CLI — remote desktop consent binding
// ============================================
//
// File: src/plugins/builtin/remote_desktop/session_consent.rs
// Description: Immutable local-user consent grant captured at session creation.

use serde_json::{json, Value};

use crate::plugins::remote_desktop::errors::{RemoteDesktopError, RemoteDesktopResult};
use crate::runtime::ability_dispatch::EnvelopeContext;

const POLICY_LOCAL_USER_CONSENT: &str = "local_user_consent";

/// Receipt reference that authorized a remote desktop session.
///
/// Invariant 1: `receipt_ura` and `receipt_hash` are copied from Axon's
/// causal-context projection; the remote desktop plugin never fabricates them.
/// Invariant 2: equality is byte/string exact after projection. This type is
/// not a receipt verifier; Axon owns receipt verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::plugins::builtin::remote_desktop) struct RemoteDesktopConsentReceipt {
    receipt_ura: String,
    receipt_hash: String,
}

impl RemoteDesktopConsentReceipt {
    fn from_value(value: &Value) -> Option<Self> {
        Some(Self {
            receipt_ura: value.get("receipt_ura")?.as_str()?.to_string(),
            receipt_hash: value.get("receipt_hash")?.as_str()?.to_string(),
        })
    }

    pub(in crate::plugins::builtin::remote_desktop) fn receipt_ura(&self) -> &str {
        &self.receipt_ura
    }

    pub(in crate::plugins::builtin::remote_desktop) fn to_value(&self) -> Value {
        json!({
            "receipt_ura": self.receipt_ura,
            "receipt_hash": self.receipt_hash,
        })
    }
}

/// Consent grant bound to one remote desktop session.
///
/// What this is NOT: a replacement for the consent broker. It is the session
/// row's immutable projection of the approval actor and optional Axon receipt
/// that created the session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::plugins::builtin::remote_desktop) struct RemoteDesktopConsentGrant {
    policy: &'static str,
    approval_actor_ura: Option<String>,
    approval_receipt: Option<RemoteDesktopConsentReceipt>,
}

impl RemoteDesktopConsentGrant {
    /// Capture the approval actor and required receipt link from the creation
    /// invocation envelope.
    ///
    /// Remote desktop is a local-user-consent ability. Session creation is
    /// fail-closed: callers must present an Axon causal receipt that represents
    /// the local approval event. The plugin stores and compares that receipt,
    /// but does not verify its signature; Axon admission owns verification.
    pub(in crate::plugins::builtin::remote_desktop) fn required_from_envelope(
        ability: &'static str,
        session_id: &str,
        env: &EnvelopeContext,
    ) -> RemoteDesktopResult<Self> {
        let approval_receipt = env
            .causal_context
            .as_ref()
            .and_then(first_receipt_from_causal_context)
            .ok_or_else(|| RemoteDesktopError::ConsentReceiptRequired {
                ability,
                session_id: session_id.to_string(),
            })?;
        Ok(Self {
            policy: POLICY_LOCAL_USER_CONSENT,
            approval_actor_ura: env.caller.clone(),
            approval_receipt: Some(approval_receipt),
        })
    }

    #[cfg(test)]
    pub(in crate::plugins::builtin::remote_desktop) fn from_envelope_for_test(
        env: &EnvelopeContext,
    ) -> Self {
        Self {
            policy: POLICY_LOCAL_USER_CONSENT,
            approval_actor_ura: env.caller.clone(),
            approval_receipt: env
                .causal_context
                .as_ref()
                .and_then(first_receipt_from_causal_context),
        }
    }

    pub(in crate::plugins::builtin::remote_desktop) fn approval_receipt(
        &self,
    ) -> Option<&RemoteDesktopConsentReceipt> {
        self.approval_receipt.as_ref()
    }

    pub(in crate::plugins::builtin::remote_desktop) fn to_value(&self) -> Value {
        json!({
            "policy": self.policy,
            "approval_actor_ura": self.approval_actor_ura,
            "approval_receipt": self.approval_receipt.as_ref().map(RemoteDesktopConsentReceipt::to_value),
        })
    }
}

/// Return whether `causal_context` contains `expected`.
pub(in crate::plugins::builtin::remote_desktop) fn causal_context_contains_receipt(
    causal_context: Option<&Value>,
    expected: &RemoteDesktopConsentReceipt,
) -> bool {
    let Some(causal_context) = causal_context else {
        return false;
    };
    receipts_from_causal_context(causal_context)
        .iter()
        .any(|receipt| receipt == expected)
}

fn first_receipt_from_causal_context(value: &Value) -> Option<RemoteDesktopConsentReceipt> {
    receipts_from_causal_context(value).into_iter().next()
}

fn receipts_from_causal_context(value: &Value) -> Vec<RemoteDesktopConsentReceipt> {
    match value.get("kind").and_then(Value::as_str) {
        Some("scalar") => RemoteDesktopConsentReceipt::from_value(value)
            .into_iter()
            .collect(),
        Some("list") => value
            .get("receipts")
            .and_then(Value::as_array)
            .map(|receipts| {
                receipts
                    .iter()
                    .filter_map(RemoteDesktopConsentReceipt::from_value)
                    .collect()
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn causal_context_receipt_matching_accepts_scalar_and_list_forms() {
        let scalar = json!({
            "kind": "scalar",
            "receipt_ura": "easynet:///r/acme/invocation/1/receipt/1",
            "receipt_hash": "ab",
        });
        let expected = first_receipt_from_causal_context(&scalar).unwrap();
        assert!(causal_context_contains_receipt(Some(&scalar), &expected));

        let list = json!({
            "kind": "list",
            "receipts": [
                {"receipt_ura": "other", "receipt_hash": "00"},
                expected.to_value(),
            ],
        });
        assert!(causal_context_contains_receipt(Some(&list), &expected));
    }
}
