// EasyNet CLI — remote desktop consent binding
// ============================================
//
// File: plugins/remote-desktop/src/session_consent.rs
// Description: Immutable local-user consent grant captured at session creation.

use serde_json::{json, Value};

use crate::daemon::ability::dispatch::EnvelopeContext;
use crate::daemon::plugins::remote_desktop::errors::{RemoteDesktopError, RemoteDesktopResult};

const POLICY_LOCAL_USER_CONSENT: &str = "local_user_consent";

/// Receipt reference that authorized a remote desktop session.
///
/// Invariant 1: `receipt_ura` and `receipt_hash` are copied from Axon's
/// causal-context projection; the remote desktop plugin never fabricates them.
/// Invariant 2: equality is byte/string exact after projection. This type is
/// not a receipt verifier; Axon owns receipt verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) struct RemoteDesktopConsentReceipt {
    receipt_ura: String,
    receipt_hash: String,
}

impl RemoteDesktopConsentReceipt {
    fn from_value(ability: &'static str, value: &Value) -> RemoteDesktopResult<Self> {
        let receipt_ura = required_receipt_field(ability, value, "receipt_ura")?;
        let receipt_hash = required_receipt_field(ability, value, "receipt_hash")?;
        Ok(Self {
            receipt_ura,
            receipt_hash,
        })
    }

    pub(in crate::daemon::plugins::remote_desktop) fn receipt_ura(&self) -> &str {
        &self.receipt_ura
    }

    pub(in crate::daemon::plugins::remote_desktop) fn to_value(&self) -> Value {
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
pub(in crate::daemon::plugins::remote_desktop) struct RemoteDesktopConsentGrant {
    policy: &'static str,
    approval_actor_ura: Option<String>,
    approval_receipt: Option<RemoteDesktopConsentReceipt>,
}

impl RemoteDesktopConsentGrant {
    /// Capture the approval actor and required receipt link from the creation
    /// invocation envelope.
    ///
    /// Remote desktop is a local-user-consent ability. Session creation is
    /// fail-closed unless Axon has admitted and projected a causal consent
    /// receipt into the envelope. The plugin stores and compares that receipt,
    /// but does not verify its signature; Axon admission owns verification.
    /// Local owner, device, and process-local callers still need the same
    /// receipt fact so the product plugin cannot become a second authority.
    pub(in crate::daemon::plugins::remote_desktop) fn required_from_envelope(
        ability: &'static str,
        session_id: &str,
        env: &EnvelopeContext,
    ) -> RemoteDesktopResult<Self> {
        if let Some(approval_receipt) =
            first_receipt_from_causal_context(ability, env.causal_context())?
        {
            return Ok(Self {
                policy: POLICY_LOCAL_USER_CONSENT,
                approval_actor_ura: Some(env.caller().to_string()),
                approval_receipt: Some(approval_receipt),
            });
        }
        Err(RemoteDesktopError::ConsentReceiptRequired {
            ability,
            session_id: session_id.to_string(),
        })
    }

    #[cfg(test)]
    pub(in crate::daemon::plugins::remote_desktop) fn from_envelope_for_test(
        env: &EnvelopeContext,
    ) -> Self {
        Self {
            policy: POLICY_LOCAL_USER_CONSENT,
            approval_actor_ura: Some(env.caller().to_string()),
            approval_receipt: first_receipt_from_causal_context(
                "test.ability",
                env.causal_context(),
            )
            .expect("test causal context must be valid"),
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn approval_receipt(
        &self,
    ) -> Option<&RemoteDesktopConsentReceipt> {
        self.approval_receipt.as_ref()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn to_value(&self) -> Value {
        json!({
            "policy": self.policy,
            "approval_actor_ura": self.approval_actor_ura,
            "approval_receipt": self.approval_receipt.as_ref().map(RemoteDesktopConsentReceipt::to_value),
        })
    }
}

/// Return whether `causal_context` contains `expected`.
pub(in crate::daemon::plugins::remote_desktop) fn causal_context_contains_receipt(
    ability: &'static str,
    causal_context: Option<&Value>,
    expected: &RemoteDesktopConsentReceipt,
) -> RemoteDesktopResult<bool> {
    let Some(causal_context) = causal_context else {
        return Ok(false);
    };
    Ok(receipts_from_causal_context(ability, causal_context)?
        .iter()
        .any(|receipt| receipt == expected))
}

fn first_receipt_from_causal_context(
    ability: &'static str,
    value: &Value,
) -> RemoteDesktopResult<Option<RemoteDesktopConsentReceipt>> {
    Ok(receipts_from_causal_context(ability, value)?
        .into_iter()
        .next())
}

fn receipts_from_causal_context(
    ability: &'static str,
    value: &Value,
) -> RemoteDesktopResult<Vec<RemoteDesktopConsentReceipt>> {
    match value.get("form").and_then(Value::as_str) {
        Some("none" | "merkle") => Ok(Vec::new()),
        Some("scalar") => Ok(vec![RemoteDesktopConsentReceipt::from_value(
            ability, value,
        )?]),
        Some("list") => {
            let receipts = value
                .get("receipts")
                .and_then(Value::as_array)
                .ok_or_else(|| RemoteDesktopError::InvalidArgument {
                    ability,
                    detail:
                        "causal_context list requires a receipts array of consent receipt facts"
                            .to_string(),
                })?;
            receipts
                .iter()
                .map(|receipt| RemoteDesktopConsentReceipt::from_value(ability, receipt))
                .collect()
        }
        None if value.get("kind").is_some() => Err(RemoteDesktopError::InvalidArgument {
            ability,
            detail: "causal_context uses retired kind field; use form".to_string(),
        }),
        other => Err(RemoteDesktopError::InvalidArgument {
            ability,
            detail: format!(
                "unsupported causal_context form {:?} for remote desktop consent receipt",
                other.unwrap_or("<missing>")
            ),
        }),
    }
}

fn required_receipt_field(
    ability: &'static str,
    value: &Value,
    field: &'static str,
) -> RemoteDesktopResult<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| RemoteDesktopError::InvalidArgument {
            ability,
            detail: format!("causal_context receipt requires non-empty {field}"),
        })
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
            "form": "scalar",
            "receipt_ura": "easynet:///r/acme/resource/alice.invocations/1",
            "receipt_hash": "ab",
        });
        let expected = first_receipt_from_causal_context("rd.test", &scalar)
            .unwrap()
            .unwrap();
        assert!(causal_context_contains_receipt("rd.test", Some(&scalar), &expected).unwrap());

        let list = json!({
            "form": "list",
            "receipts": [
                {"receipt_ura": "other", "receipt_hash": "00"},
                expected.to_value(),
            ],
        });
        assert!(causal_context_contains_receipt("rd.test", Some(&list), &expected).unwrap());
    }

    #[test]
    fn causal_context_receipt_projection_rejects_malformed_list_rows() {
        let expected = RemoteDesktopConsentReceipt {
            receipt_ura: "easynet:///r/acme/resource/alice.invocations/1".to_string(),
            receipt_hash: "ab".to_string(),
        };
        let list = json!({
            "form": "list",
            "receipts": [
                expected.to_value(),
                {"receipt_ura": "easynet:///r/acme/resource/alice.invocations/2"}
            ],
        });

        let err = causal_context_contains_receipt("rd.test", Some(&list), &expected)
            .expect_err("malformed receipt rows must not be skipped");
        assert!(err
            .to_string()
            .contains("causal_context receipt requires non-empty receipt_hash"));
    }

    #[test]
    fn causal_context_receipt_projection_rejects_retired_kind_field() {
        let expected = RemoteDesktopConsentReceipt {
            receipt_ura: "easynet:///r/acme/resource/alice.invocations/1".to_string(),
            receipt_hash: "ab".to_string(),
        };
        let retired = json!({
            "kind": "scalar",
            "receipt_ura": expected.receipt_ura(),
            "receipt_hash": "ab",
        });

        let err = causal_context_contains_receipt("rd.test", Some(&retired), &expected)
            .expect_err("retired causal-context kind field must fail closed");
        assert!(err.to_string().contains("retired kind field"));
    }

    #[test]
    fn malformed_causal_context_rejects_before_consent_policy() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        pair_device("acme", "dev-1", "alice");
        let env = EnvelopeContext::for_test(
            "easynet:///r/acme/user/user-alice",
            "easynet:///r/acme/user/user-alice",
        )
        .with_causal_context(json!({
            "form": "scalar",
            "receipt_ura": "easynet:///r/acme/resource/user-alice.invocations/1"
        }));

        let err = RemoteDesktopConsentGrant::required_from_envelope("rd.create", "s-bad", &env)
            .expect_err("malformed causal context must fail before consent policy");
        assert!(err
            .to_string()
            .contains("causal_context receipt requires non-empty receipt_hash"));
    }

    fn pair_device(realm: &str, node_id: &str, username: &str) {
        crate::daemon::persistence::config::save_credentials(
            &crate::daemon::persistence::config::Credentials {
                node_id: node_id.to_string(),
                credential_token: "tok".into(),
                hub_endpoint: "https://127.0.0.1:50443".into(),
                realm: realm.to_string(),
                username: Some(username.to_string()),
                user_id: Some(format!("user-{username}")),
                ..Default::default()
            },
        )
        .expect("save credentials");
    }

    #[test]
    fn owner_user_caller_still_requires_consent_receipt() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        pair_device("acme", "dev-1", "alice");
        let env = EnvelopeContext::for_test(
            "easynet:///r/acme/user/user-alice",
            "easynet:///r/acme/user/user-alice",
        );
        let err =
            RemoteDesktopConsentGrant::required_from_envelope("rd.create", "s1", &env).unwrap_err();
        assert!(err.to_string().contains("consent_receipt_required"));
    }

    #[test]
    fn owner_device_caller_still_requires_consent_receipt() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        pair_device("acme", "dev-1", "alice");
        let env = EnvelopeContext::for_test(
            "easynet:///r/acme/device/dev-1",
            "easynet:///r/acme/device/dev-1",
        );
        let err =
            RemoteDesktopConsentGrant::required_from_envelope("rd.create", "s2", &env).unwrap_err();
        assert!(err.to_string().contains("consent_receipt_required"));
    }

    #[test]
    fn local_system_caller_still_requires_consent_receipt() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        pair_device("acme", "dev-1", "alice");
        let env = EnvelopeContext::for_test(
            crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA,
            "easynet:///r/acme/resource/display-1",
        );
        let err = RemoteDesktopConsentGrant::required_from_envelope("rd.create", "s-local", &env)
            .unwrap_err();
        assert!(err.to_string().contains("consent_receipt_required"));
    }

    #[test]
    fn unpaired_local_system_caller_still_requires_consent_receipt() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let env = EnvelopeContext::for_test(
            crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA,
            "easynet:///r/acme/resource/display-1",
        );
        let err =
            RemoteDesktopConsentGrant::required_from_envelope("rd.create", "s-unpaired", &env)
                .unwrap_err();
        assert!(err.to_string().contains("consent_receipt_required"));
    }

    #[test]
    fn foreign_or_mismatched_callers_stay_fail_closed_without_receipt() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        pair_device("acme", "dev-1", "alice");
        for caller in [
            "easynet:///r/acme/user/mallory",            // different user
            "easynet:///r/other/user/user-alice",        // different realm
            "easynet:///r/acme/device/dev-2",            // different device
            "easynet:///r/acme/agent/user-alice.helper", // agents cannot authorize consent
        ] {
            let env = EnvelopeContext::for_test(caller, "easynet:///r/acme/device/dev-1");
            let err = RemoteDesktopConsentGrant::required_from_envelope("rd.create", "s3", &env)
                .unwrap_err();
            assert!(
                err.to_string().contains("consent_receipt_required"),
                "caller {caller} must stay fail-closed; got: {err}"
            );
        }
    }

    #[test]
    fn causal_receipt_grants_consent_for_owner_caller() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        pair_device("acme", "dev-1", "alice");
        let env = EnvelopeContext::for_test(
            "easynet:///r/acme/user/user-alice",
            "easynet:///r/acme/user/user-alice",
        )
        .with_causal_context(json!({
            "form": "scalar",
            "receipt_ura": "easynet:///r/acme/resource/user-alice.invocations/1",
            "receipt_hash": "ab",
        }));
        let grant =
            RemoteDesktopConsentGrant::required_from_envelope("rd.create", "s4", &env).unwrap();
        assert_eq!(grant.policy, POLICY_LOCAL_USER_CONSENT);
        assert!(grant.approval_receipt().is_some());
    }
}
