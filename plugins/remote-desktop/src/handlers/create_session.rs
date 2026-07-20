// EasyNet CLI — remote desktop create-session handler
// ===================================================

use std::sync::Arc;

use serde_json::Value;

use crate::daemon::ability::dispatch::EnvelopeContext;
use crate::daemon::plugins::remote_desktop::constants::{
    ABILITY_CREATE_SESSION, REASON_INVALID_ARGUMENT, REASON_SESSION_STORE_FULL,
};
use crate::daemon::plugins::remote_desktop::request::{
    mint_session_id, mint_session_token, parse_input_policy, parse_lease_ttl_ms, parse_mode,
    parse_optional_session_id, parse_transport_preferences, parse_video_constraints,
};
use crate::daemon::plugins::remote_desktop::resource::resolve_screen_resource_from_envelope;
use crate::daemon::plugins::remote_desktop::runtime::RemoteDesktopPlugin;
use crate::daemon::plugins::remote_desktop::session::{
    now_ms, RemoteDesktopSession, RemoteDesktopSessionInit,
};
use crate::daemon::plugins::remote_desktop::session_consent::RemoteDesktopConsentGrant;
use crate::daemon::plugins::remote_desktop::session_lifecycle::{
    prune_inactive_sessions, spawn_session_lease_watchdog,
};
use crate::daemon::plugins::remote_desktop::view::serialize_session_with_token;

/// Handle `remote_desktop.create_session`.
pub(in crate::daemon::plugins::remote_desktop) fn handle(
    plugin: Arc<RemoteDesktopPlugin>,
    env: EnvelopeContext,
    args: Value,
) -> anyhow::Result<Value> {
    let entry = resolve_screen_resource_from_envelope(ABILITY_CREATE_SESSION, &env, &args)?;
    let mode = parse_mode(&args)?;
    let lease_ttl_ms = parse_lease_ttl_ms(&args)?;
    let transport_preferences = parse_transport_preferences(&args)?;
    let video = parse_video_constraints(&args)?;
    let input_policy = parse_input_policy(&args, &mode)?;
    let session_id = parse_optional_session_id(&args)?.unwrap_or_else(mint_session_id);
    let session_token = mint_session_token();
    let consent = RemoteDesktopConsentGrant::required_from_envelope(
        ABILITY_CREATE_SESSION,
        &session_id,
        &env,
    )?;
    let session = RemoteDesktopSession::new(RemoteDesktopSessionInit {
        session_id: session_id.clone(),
        session_token,
        creator_caller_ura: Some(env.caller().to_string()),
        consent,
        subject_ura: entry.resource_ura.clone(),
        subject_type: entry.kind,
        subject_display_name: entry.display_name.clone(),
        mode,
        lease_ttl_ms,
        transport_preferences,
        video,
        input_policy,
    });
    let now = now_ms();
    let (watchdog_session_id, lease_expires_at_ms, view) =
        plugin
            .session_store()
            .with_sessions(|sessions| -> anyhow::Result<_> {
                prune_inactive_sessions(&plugin, sessions, now);
                if sessions.len() >= plugin.config().max_sessions() {
                    anyhow::bail!(
                        "{ABILITY_CREATE_SESSION}: remote desktop session store is full; \
                         max_sessions={}; reason={REASON_SESSION_STORE_FULL}",
                        plugin.config().max_sessions()
                    );
                }
                if sessions.contains_key(&session_id) {
                    anyhow::bail!(
                        "{ABILITY_CREATE_SESSION}: session_id {session_id:?} already exists; reason={REASON_INVALID_ARGUMENT}"
                    );
                }
                let watchdog_session_id = session_id.clone();
                let lease_expires_at_ms = session.lease_expires_at_ms();
                let view = serialize_session_with_token(&session);
                sessions.insert(session_id, session);
                Ok((watchdog_session_id, lease_expires_at_ms, view))
            })?;
    spawn_session_lease_watchdog(plugin, watchdog_session_id, lease_expires_at_ms);
    Ok(view)
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::json;

    use crate::daemon::persistence::{resources, resources::ResourcesFile};
    use crate::daemon::plugins::remote_desktop::test_support::{
        reset_store, seed_display, test_lock, test_plugin,
    };

    #[test]
    fn create_session_requires_subject() {
        let _lock = test_lock();
        let plugin = test_plugin();
        reset_store(&plugin);
        let err = handle(
            Arc::clone(&plugin),
            EnvelopeContext::for_test(
                "easynet:///r/acme/user/alice",
                "easynet:///r/acme/user/alice",
            ),
            json!({}),
        )
        .unwrap_err();
        assert!(err.to_string().contains("subject_required"));
    }

    #[test]
    fn create_session_rejects_subject_in_args() {
        let _lock = test_lock();
        let plugin = test_plugin();
        reset_store(&plugin);
        let err = handle(
            Arc::clone(&plugin),
            EnvelopeContext::for_test(
                "easynet:///r/acme/user/alice",
                "easynet:///r/acme/resource/01",
            ),
            json!({"subject": "bad"}),
        )
        .unwrap_err();
        assert!(err.to_string().contains("subject_in_args"));
    }

    #[test]
    fn create_session_requires_local_user_consent_receipt() {
        let _lock = test_lock();
        let plugin = test_plugin();
        reset_store(&plugin);
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let ura = seed_display(&mut file, "remote-desktop-no-consent-display");
        resources::save(&file).unwrap();
        let err = handle(
            Arc::clone(&plugin),
            EnvelopeContext::for_test("easynet:///r/acme/user/alice", ura),
            json!({"session_id": "rd-no-consent"}),
        )
        .unwrap_err();
        assert!(err.to_string().contains("consent_receipt_required"));
    }
}
