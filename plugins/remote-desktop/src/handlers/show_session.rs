// EasyNet CLI — remote desktop show-session handler
// =================================================

use std::sync::Arc;

use serde_json::Value;

use crate::daemon::ability::dispatch::EnvelopeContext;
use crate::daemon::plugins::remote_desktop::constants::{
    ABILITY_SHOW_SESSION, REASON_SESSION_NOT_FOUND,
};
use crate::daemon::plugins::remote_desktop::request::require_str;
use crate::daemon::plugins::remote_desktop::runtime::RemoteDesktopPlugin;
use crate::daemon::plugins::remote_desktop::session_lifecycle::ensure_session_control_access;
use crate::daemon::plugins::remote_desktop::view::serialize_session;

/// Handle `remote_desktop.show_session`.
pub(in crate::daemon::plugins::remote_desktop) fn handle(
    plugin: Arc<RemoteDesktopPlugin>,
    env: EnvelopeContext,
    args: Value,
) -> anyhow::Result<Value> {
    let session_id = require_str(&args, "session_id", ABILITY_SHOW_SESSION)?;
    plugin
        .session_store()
        .with_sessions(|sessions| -> anyhow::Result<Value> {
            let session = sessions.get_mut(session_id).ok_or_else(|| {
                anyhow::anyhow!(
                    "{ABILITY_SHOW_SESSION}: session {session_id:?} not found; reason={REASON_SESSION_NOT_FOUND}"
                )
            })?;
            ensure_session_control_access(&plugin, ABILITY_SHOW_SESSION, &env, &args, session)?;
            Ok(serialize_session(session))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::json;

    use crate::daemon::persistence::resources::{self, ResourcesFile};
    use crate::daemon::plugins::remote_desktop::constants::{
        REASON_CONSENT_RECEIPT_MISMATCH, REASON_SESSION_CALLER_MISMATCH,
        REASON_SESSION_TOKEN_MISMATCH, REASON_SESSION_TOKEN_REQUIRED,
    };
    use crate::daemon::plugins::remote_desktop::test_support::{
        env_for, env_for_caller, env_for_caller_with_causal, reset_store, seed_display, test_lock,
        test_plugin,
    };

    #[test]
    fn session_token_blocks_session_id_takeover() {
        let _lock = test_lock();
        let plugin = test_plugin();
        reset_store(&plugin);
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let ura = seed_display(&mut file, "remote-desktop-token-display");
        resources::save(&file).unwrap();

        let created = crate::daemon::plugins::remote_desktop::handlers::create_session::handle(
            Arc::clone(&plugin),
            env_for(&ura),
            json!({
                "session_id": "rd-token-test",
                "mode": "view_only",
                "lease_ttl_ms": 5000,
            }),
        )
        .unwrap();
        let token = created["session_token"].as_str().unwrap();

        let missing = handle(
            Arc::clone(&plugin),
            env_for(&ura),
            json!({"session_id": "rd-token-test"}),
        )
        .unwrap_err();
        assert!(missing.to_string().contains(REASON_SESSION_TOKEN_REQUIRED));

        let wrong = handle(
            Arc::clone(&plugin),
            env_for(&ura),
            json!({"session_id": "rd-token-test", "session_token": "wrong-token"}),
        )
        .unwrap_err();
        assert!(wrong.to_string().contains(REASON_SESSION_TOKEN_MISMATCH));

        let shown = handle(
            Arc::clone(&plugin),
            env_for(&ura),
            json!({"session_id": "rd-token-test", "session_token": token}),
        )
        .unwrap();
        assert_eq!(shown["session_id"], json!("rd-token-test"));
        assert!(
            shown.get("session_token").is_none(),
            "show_session must not echo the secret session_token"
        );
    }

    #[test]
    fn session_token_is_bound_to_creator_caller_when_envelope_carries_caller() {
        let _lock = test_lock();
        let plugin = test_plugin();
        reset_store(&plugin);
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let ura = seed_display(&mut file, "remote-desktop-caller-bound-display");
        resources::save(&file).unwrap();

        let creator = "easynet:///r/acme/user/alice";
        let attacker = "easynet:///r/acme/user/bob";
        let created = crate::daemon::plugins::remote_desktop::handlers::create_session::handle(
            Arc::clone(&plugin),
            env_for_caller(&ura, creator),
            json!({
                "session_id": "rd-caller-bound",
                "mode": "view_only",
                "lease_ttl_ms": 5000,
            }),
        )
        .unwrap();
        let token = created["session_token"].as_str().unwrap();

        let err = handle(
            Arc::clone(&plugin),
            env_for_caller(&ura, attacker),
            json!({"session_id": "rd-caller-bound", "session_token": token}),
        )
        .unwrap_err();
        assert!(err.to_string().contains(REASON_SESSION_CALLER_MISMATCH));

        let shown = handle(
            Arc::clone(&plugin),
            env_for_caller(&ura, creator),
            json!({"session_id": "rd-caller-bound", "session_token": token}),
        )
        .unwrap();
        assert_eq!(shown["session_id"], json!("rd-caller-bound"));
    }

    #[test]
    fn session_access_preserves_consent_receipt_binding() {
        let _lock = test_lock();
        let plugin = test_plugin();
        reset_store(&plugin);
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let ura = seed_display(&mut file, "remote-desktop-consent-receipt-display");
        resources::save(&file).unwrap();

        let caller = "easynet:///r/acme/user/alice";
        // Borrowed receipt-URA shape (ledger.rs test convention) —
        // no production builder yet; RFC-007/008 (F-042).
        let consent_receipt = json!({
            "kind": "scalar",
            "receipt_ura": "easynet:///r/acme/resource/alice.invocations/approve-rd",
            "receipt_hash": "aa",
        });
        let created = crate::daemon::plugins::remote_desktop::handlers::create_session::handle(
            Arc::clone(&plugin),
            env_for_caller_with_causal(&ura, caller, consent_receipt.clone()),
            json!({
                "session_id": "rd-consent-bound",
                "mode": "view_only",
                "lease_ttl_ms": 5000,
            }),
        )
        .unwrap();
        assert_eq!(
            created["consent"]["approval_receipt"]["receipt_ura"],
            json!("easynet:///r/acme/resource/alice.invocations/approve-rd")
        );
        let token = created["session_token"].as_str().unwrap();

        let missing = handle(
            Arc::clone(&plugin),
            EnvelopeContext::for_test(caller, ura.clone()),
            json!({"session_id": "rd-consent-bound", "session_token": token}),
        )
        .unwrap_err();
        // A session bound to an approval receipt rejects any access whose
        // causal context does not carry that receipt — absence included.
        // The contract folds "no receipt" into the single mismatch reason
        // rather than a separate "required" code.
        assert!(missing
            .to_string()
            .contains(REASON_CONSENT_RECEIPT_MISMATCH));

        let shown = handle(
            Arc::clone(&plugin),
            env_for_caller_with_causal(&ura, caller, consent_receipt),
            json!({"session_id": "rd-consent-bound", "session_token": token}),
        )
        .unwrap();
        assert_eq!(shown["session_id"], json!("rd-consent-bound"));
    }
}
