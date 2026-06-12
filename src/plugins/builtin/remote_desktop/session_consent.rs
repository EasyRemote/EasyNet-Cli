// EasyNet CLI — remote desktop consent binding
// ============================================
//
// File: src/plugins/builtin/remote_desktop/session_consent.rs
// Description: Immutable local-user consent grant captured at session creation.

use serde_json::{json, Value};

use crate::plugins::remote_desktop::errors::{RemoteDesktopError, RemoteDesktopResult};
use crate::runtime::ability_dispatch::EnvelopeContext;

const POLICY_LOCAL_USER_CONSENT: &str = "local_user_consent";
const POLICY_OWNER_SELF_CONSENT: &str = "owner_self_consent";

/// Whether `caller` is this device's owner: the user this device is
/// paired to (console invokes sign as the user URA) or the device
/// identity itself (daemon-local CLI invokes). Realm must match the
/// pairing realm. Unpaired devices (no credentials) own nothing —
/// fail-closed.
fn caller_is_device_owner(caller: Option<&str>) -> bool {
    let Some(caller) = caller else { return false };
    let Ok(parsed) = crate::ura::parse_ura(caller) else {
        return false;
    };
    let Ok(creds) = crate::persistence::config::load_credentials() else {
        return false;
    };
    if parsed.realm != creds.realm.trim() {
        return false;
    }
    match parsed.kind {
        crate::ura::URAKind::Device => parsed.device_id() == Some(creds.node_id.trim()),
        crate::ura::URAKind::User => creds.username.as_deref().map(str::trim) == parsed.user_id(),
        _ => false,
    }
}

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
    /// fail-closed with one carve-out:
    ///
    ///   1. A causal receipt in the envelope (the local approval event)
    ///      grants under `local_user_consent`. The plugin stores and
    ///      compares that receipt, but does not verify its signature;
    ///      Axon admission owns verification.
    ///   2. No receipt, but the caller IS this device's owner (the
    ///      paired user, or the device identity itself): the signed,
    ///      admission-verified invocation is itself the owner's
    ///      approval act — viewing your own screen needs no second
    ///      approval artefact. Grants under `owner_self_consent`.
    ///
    /// Any other caller without a receipt is refused
    /// (`consent_receipt_required`), so cross-user and cross-realm
    /// session creation stays fail-closed.
    pub(in crate::plugins::builtin::remote_desktop) fn required_from_envelope(
        ability: &'static str,
        session_id: &str,
        env: &EnvelopeContext,
    ) -> RemoteDesktopResult<Self> {
        if let Some(approval_receipt) = env
            .causal_context
            .as_ref()
            .and_then(first_receipt_from_causal_context)
        {
            return Ok(Self {
                policy: POLICY_LOCAL_USER_CONSENT,
                approval_actor_ura: env.caller.clone(),
                approval_receipt: Some(approval_receipt),
            });
        }
        if caller_is_device_owner(env.caller.as_deref()) {
            return Ok(Self {
                policy: POLICY_OWNER_SELF_CONSENT,
                approval_actor_ura: env.caller.clone(),
                approval_receipt: None,
            });
        }
        Err(RemoteDesktopError::ConsentReceiptRequired {
            ability,
            session_id: session_id.to_string(),
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
        // Borrowed receipt-URA shape (ledger.rs test convention,
        // `resource/<owner>.invocations/<id>`) — no production
        // builder yet; RFC-007/008 tracks canonicalization (F-042).
        let scalar = json!({
            "kind": "scalar",
            "receipt_ura": "easynet:///r/acme/resource/alice.invocations/1",
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

    fn pair_device(realm: &str, node_id: &str, username: &str) {
        crate::persistence::config::save_credentials(&crate::persistence::config::Credentials {
            node_id: node_id.to_string(),
            credential_token: "tok".into(),
            hub_endpoint: "https://127.0.0.1:50443".into(),
            realm: realm.to_string(),
            username: Some(username.to_string()),
            ..Default::default()
        })
        .expect("save credentials");
    }

    #[test]
    fn owner_user_caller_grants_self_consent_without_receipt() {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        pair_device("acme", "dev-1", "alice");
        let env = EnvelopeContext {
            caller: Some("easynet:///r/acme/user/alice".into()),
            ..EnvelopeContext::default()
        };
        let grant =
            RemoteDesktopConsentGrant::required_from_envelope("rd.create", "s1", &env).unwrap();
        assert_eq!(grant.policy, POLICY_OWNER_SELF_CONSENT);
        assert!(grant.approval_receipt().is_none());
    }

    #[test]
    fn owner_device_caller_grants_self_consent_without_receipt() {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        pair_device("acme", "dev-1", "alice");
        let env = EnvelopeContext {
            caller: Some("easynet:///r/acme/device/dev-1".into()),
            ..EnvelopeContext::default()
        };
        let grant =
            RemoteDesktopConsentGrant::required_from_envelope("rd.create", "s2", &env).unwrap();
        assert_eq!(grant.policy, POLICY_OWNER_SELF_CONSENT);
    }

    #[test]
    fn foreign_or_mismatched_callers_stay_fail_closed_without_receipt() {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        pair_device("acme", "dev-1", "alice");
        for caller in [
            "easynet:///r/acme/user/mallory",       // different user
            "easynet:///r/other/user/alice",        // different realm
            "easynet:///r/acme/device/dev-2",       // different device
            "easynet:///r/acme/agent/alice.helper", // agents never self-consent
        ] {
            let env = EnvelopeContext {
                caller: Some(caller.into()),
                ..EnvelopeContext::default()
            };
            let err = RemoteDesktopConsentGrant::required_from_envelope("rd.create", "s3", &env)
                .unwrap_err();
            assert!(
                err.to_string().contains("consent_receipt_required"),
                "caller {caller} must stay fail-closed; got: {err}"
            );
        }
    }

    #[test]
    fn receipt_still_wins_over_owner_self_consent() {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        pair_device("acme", "dev-1", "alice");
        let env = EnvelopeContext {
            caller: Some("easynet:///r/acme/user/alice".into()),
            causal_context: Some(json!({
                "kind": "scalar",
                "receipt_ura": "easynet:///r/acme/resource/alice.invocations/1",
                "receipt_hash": "ab",
            })),
            ..EnvelopeContext::default()
        };
        let grant =
            RemoteDesktopConsentGrant::required_from_envelope("rd.create", "s4", &env).unwrap();
        assert_eq!(grant.policy, POLICY_LOCAL_USER_CONSENT);
        assert!(grant.approval_receipt().is_some());
    }
}
