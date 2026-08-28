// EasyNet CLI — authorized RemoteApp target-focus handler
// =======================================================

use std::sync::Arc;

use serde_json::{json, Value};

use crate::daemon::ability::dispatch::EnvelopeContext;
use crate::daemon::plugins::remote_desktop::constants::ABILITY_FOCUS_TARGET;
use crate::daemon::plugins::remote_desktop::request::require_str;
use crate::daemon::plugins::remote_desktop::runtime::RemoteDesktopPlugin;
use crate::daemon::plugins::remote_desktop::session::RemoteDesktopSession;
use crate::daemon::plugins::remote_desktop::session_lifecycle::ensure_session_control_access;
use crate::daemon::plugins::remote_desktop::session_recovery::RemoteDesktopRecoverySnapshot;
use crate::daemon::plugins::remote_desktop::target::RemoteDesktopTargetKind;
use crate::daemon::plugins::remote_desktop::target_focus::{
    RemoteAppTargetFocusError, TargetFocusFailureReason,
};

#[derive(Debug, Clone, Copy)]
struct ExpectedTargetState {
    consent_epoch: u64,
    binding_epoch: u64,
    target_identity_epoch: u64,
    target_geometry_revision: u64,
    target_focus_epoch: u64,
}

impl ExpectedTargetState {
    fn parse(args: &Value) -> anyhow::Result<Self> {
        Ok(Self {
            consent_epoch: required_epoch(args, "expected_consent_epoch")?,
            binding_epoch: required_epoch(args, "expected_binding_epoch")?,
            target_identity_epoch: required_epoch(args, "expected_target_identity_epoch")?,
            target_geometry_revision: required_epoch(args, "expected_target_geometry_revision")?,
            target_focus_epoch: required_epoch(args, "expected_target_focus_epoch")?,
        })
    }

    fn validate_before_host_mutation(self, session: &RemoteDesktopSession) -> anyhow::Result<()> {
        self.validate_stable_epochs(session)?;
        let snapshot = session.target_snapshot();
        if snapshot.target_focus_epoch() != self.target_focus_epoch {
            return Err(stale_target_error(
                "expected_target_focus_epoch does not match committed target focus epoch",
            )
            .into());
        }
        if snapshot.input_blocked_reason() != Some("target_blurred")
            || snapshot.focused() != Some(false)
        {
            return Err(stale_target_error(
                "target focus mutation requires the current exact target_blurred state",
            )
            .into());
        }
        Ok(())
    }

    fn validate_before_commit(self, session: &RemoteDesktopSession) -> anyhow::Result<()> {
        self.validate_stable_epochs(session)?;
        let snapshot = session.target_snapshot();
        let focus_is_request_commit = snapshot.target_focus_epoch() == self.target_focus_epoch;
        let focus_was_committed_by_monitor = snapshot.focused() == Some(true)
            && snapshot.target_focus_epoch() == self.target_focus_epoch.saturating_add(1);
        if !focus_is_request_commit && !focus_was_committed_by_monitor {
            return Err(stale_target_error(
                "target focus epoch changed independently while focus request was in flight",
            )
            .into());
        }
        Ok(())
    }

    fn validate_stable_epochs(self, session: &RemoteDesktopSession) -> anyhow::Result<()> {
        let snapshot = session.target_snapshot();
        let matches = session.consent_state().consent_epoch() == self.consent_epoch
            && snapshot.binding_epoch() == self.binding_epoch
            && snapshot.target_identity_epoch() == self.target_identity_epoch
            && snapshot.target_geometry_revision() == self.target_geometry_revision;
        if !matches {
            return Err(stale_target_error(
                "consent, binding, identity, or geometry epoch changed",
            )
            .into());
        }
        Ok(())
    }
}

/// Handle `remote_desktop.focus_target`.
pub(in crate::daemon::plugins::remote_desktop) fn handle(
    plugin: Arc<RemoteDesktopPlugin>,
    env: EnvelopeContext,
    args: Value,
) -> anyhow::Result<Value> {
    let session_id = require_str(&args, "session_id", ABILITY_FOCUS_TARGET)?.to_string();
    let expected = ExpectedTargetState::parse(&args)?;
    let sessions = plugin.session_store();
    let operation_lock = sessions.target_operation_lock(&session_id);
    let _operation = match operation_lock.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };

    let (binding, snapshot, reservation) =
        sessions.with_sessions(|sessions| -> anyhow::Result<_> {
            let session = sessions.get_mut(&session_id).ok_or_else(|| {
                crate::daemon::plugins::remote_desktop::errors::RemoteDesktopError::SessionNotFound {
                    ability: ABILITY_FOCUS_TARGET,
                    session_id: session_id.clone(),
                }
            })?;
            ensure_session_control_access(&plugin, ABILITY_FOCUS_TARGET, &env, &args, session)?;
            ensure_focus_authorized(session)?;
            expected.validate_before_host_mutation(session)?;
            let reservation = session.reserve_target_operation();
            Ok((
                session.target_binding().clone(),
                session.target_snapshot().clone(),
                reservation,
            ))
        })?;

    let proof = plugin
        .target_focus_controller()
        .focus_exact_target(&binding, &snapshot)?;

    let (previous_target_focus_epoch, target_focus_epoch, recovery_snapshot, view) =
        sessions.with_sessions(|sessions| -> anyhow::Result<_> {
            let session = sessions.get_mut(&session_id).ok_or_else(|| {
                crate::daemon::plugins::remote_desktop::errors::RemoteDesktopError::SessionNotFound {
                    ability: ABILITY_FOCUS_TARGET,
                    session_id: session_id.clone(),
                }
            })?;
            // The first phase is the complete authority/liveness admission
            // point. Once the admitted host focus effect has succeeded, a
            // newly elapsed wall-clock lease cannot retroactively turn that
            // effect into a failed invocation. The operation gate still
            // excludes lifecycle and target transitions, while the checks
            // below prove that this exact reservation and target generation
            // remain the commit owner.
            if !session.target_coherence_matches(&reservation) {
                return Err(stale_target_error(
                    "target coherence generation changed while focus request was in flight",
                )
                .into());
            }
            expected.validate_before_commit(session)?;
            let epochs = session.record_authorized_target_focus(
                proof.observed_at_ms(),
                proof.platform_backend(),
            );
            let recovery_snapshot = RemoteDesktopRecoverySnapshot::from_session(session)?;
            Ok((epochs.0, epochs.1, recovery_snapshot, plugin.session_view(session)))
        })?;
    drop(_operation);
    plugin.persist_recovery_snapshot(&recovery_snapshot)?;

    let target = view.get("target_tracking").cloned().unwrap_or(Value::Null);
    Ok(json!({
        "session_id": session_id,
        "subject_ura": binding.subject_ura(),
        "result": "focused",
        "focused": true,
        "consent_epoch": expected.consent_epoch,
        "binding_id": binding.binding_id(),
        "binding_epoch": expected.binding_epoch,
        "target_identity_epoch": expected.target_identity_epoch,
        "target_geometry_revision": expected.target_geometry_revision,
        "previous_target_focus_epoch": previous_target_focus_epoch,
        "target_focus_epoch": target_focus_epoch,
        "observed_at_ms": proof.observed_at_ms(),
        "platform_backend": proof.platform_backend(),
        "target_tracking": target,
        "session": view,
    }))
}

fn ensure_focus_authorized(session: &RemoteDesktopSession) -> anyhow::Result<()> {
    if session.mode() != "interactive"
        || !session.consent_state().permits_media_input()
        || !session.consent().permits_remote_focus()
    {
        return Err(RemoteAppTargetFocusError::new(
            TargetFocusFailureReason::RemoteFocusNotConsented,
            "interactive input and explicit remote-focus consent are required",
        )
        .into());
    }
    if session.target_binding().target_kind() == RemoteDesktopTargetKind::Display {
        return Err(RemoteAppTargetFocusError::new(
            TargetFocusFailureReason::TargetFocusUnsupported,
            "display targets use display-global input and cannot be focused as one target",
        )
        .into());
    }
    Ok(())
}

fn required_epoch(args: &Value, field: &'static str) -> anyhow::Result<u64> {
    args.get(field)
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)
        .ok_or_else(|| {
            crate::daemon::plugins::remote_desktop::errors::RemoteDesktopError::InvalidArgument {
                ability: ABILITY_FOCUS_TARGET,
                detail: format!("{field} must be a non-zero integer"),
            }
            .into()
        })
}

fn stale_target_error(detail: impl Into<String>) -> RemoteAppTargetFocusError {
    RemoteAppTargetFocusError::new(TargetFocusFailureReason::TargetFocusStale, detail)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    use super::*;
    use crate::daemon::ability::builtins::resources::media::screen_snapshot::SyntheticScreenBackend;
    use crate::daemon::persistence::resources::{ResourceBinding, ResourceEntry, ResourceType};
    use crate::daemon::plugins::remote_desktop::errors::RemoteDesktopError;
    use crate::daemon::plugins::remote_desktop::input::RemoteDesktopInputPolicy;
    use crate::daemon::plugins::remote_desktop::request::RemoteDesktopVideoConstraints;
    use crate::daemon::plugins::remote_desktop::session::RemoteDesktopSessionInit;
    use crate::daemon::plugins::remote_desktop::session_consent::RemoteDesktopConsentGrant;
    use crate::daemon::plugins::remote_desktop::session_creation::RemoteAppTargetBindingVerifier;
    use crate::daemon::plugins::remote_desktop::session_store::RemoteDesktopSessionStore;
    use crate::daemon::plugins::remote_desktop::target::{
        RemoteAppTargetBinding, ResourceEntryTargetResolver,
    };
    use crate::daemon::plugins::remote_desktop::target_focus::{
        RemoteAppTargetFocusController, RemoteAppTargetFocusProof,
    };
    use crate::daemon::plugins::remote_desktop::target_tracking::{
        TargetObservation, TargetTrackerSnapshot,
    };
    use crate::daemon::plugins::remote_desktop::test_support::{
        env_for, live_remote_target_metadata, reset_store, test_lock, test_runtime_limits,
        TestRemoteAppTargetBindingVerifier,
    };

    #[derive(Default)]
    struct RecordingFocusController {
        calls: AtomicUsize,
    }

    impl RemoteAppTargetFocusController for RecordingFocusController {
        fn focus_exact_target(
            &self,
            _binding: &RemoteAppTargetBinding,
            _snapshot: &TargetTrackerSnapshot,
        ) -> Result<RemoteAppTargetFocusProof, RemoteAppTargetFocusError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(RemoteAppTargetFocusProof::for_test("test_exact_focus", 200))
        }
    }

    #[derive(Default)]
    struct LeaseExpiringFocusController {
        calls: AtomicUsize,
        session: Mutex<Option<(Arc<RemoteDesktopSessionStore>, String)>>,
    }

    impl LeaseExpiringFocusController {
        fn expire_during_focus(
            &self,
            sessions: Arc<RemoteDesktopSessionStore>,
            session_id: impl Into<String>,
        ) {
            *self.session.lock().expect("focus test session lock") =
                Some((sessions, session_id.into()));
        }
    }

    impl RemoteAppTargetFocusController for LeaseExpiringFocusController {
        fn focus_exact_target(
            &self,
            _binding: &RemoteAppTargetBinding,
            _snapshot: &TargetTrackerSnapshot,
        ) -> Result<RemoteAppTargetFocusProof, RemoteAppTargetFocusError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let (sessions, session_id) = self
                .session
                .lock()
                .expect("focus test session lock")
                .clone()
                .expect("focus test session installed");
            sessions.with_sessions(|sessions| {
                sessions
                    .get_mut(&session_id)
                    .expect("focus test session exists")
                    .set_lease_expires_at_for_test(
                        crate::daemon::plugins::remote_desktop::session::now_ms().saturating_sub(1),
                    );
            });
            Ok(RemoteAppTargetFocusProof::for_test(
                "test_focus_crossed_lease_deadline",
                201,
            ))
        }
    }

    fn test_plugin_with_controller(
        focus: Arc<dyn RemoteAppTargetFocusController>,
    ) -> Arc<RemoteDesktopPlugin> {
        RemoteDesktopPlugin::with_target_focus_controller_for_test(
            Arc::new(SyntheticScreenBackend),
            Arc::new(TestRemoteAppTargetBindingVerifier) as Arc<dyn RemoteAppTargetBindingVerifier>,
            focus,
            test_runtime_limits().into(),
        )
    }

    fn test_plugin(focus: Arc<RecordingFocusController>) -> Arc<RemoteDesktopPlugin> {
        test_plugin_with_controller(focus)
    }

    fn test_window_binding(subject: &str) -> RemoteAppTargetBinding {
        let entry = ResourceEntry {
            resource_ura: subject.to_string(),
            owner_agent: "easynet:///r/acme/agent/device.01DEV.media".to_string(),
            kind: ResourceType::Window,
            binding: ResourceBinding::LocalDevice,
            hardware_id: "focus-window".to_string(),
            display_name: "Focus Test Window".to_string(),
            metadata: live_remote_target_metadata(json!({
                "backend": "xcap",
                "window_id": 42,
                "pid": 4242,
                "app_name": "Focus Test",
                "title": "Focus Test Window",
                "x": 10,
                "y": 20,
                "width": 640,
                "height": 480,
            })),
            first_seen_at: "2026-08-25T00:00:00Z".to_string(),
        };
        let mut binding = ResourceEntryTargetResolver
            .resolve_for_session_with_input_consent(
                "remote_desktop.create_session",
                &entry,
                "interactive",
                1,
                true,
            )
            .expect("focus window binding resolves");
        let proof = TestRemoteAppTargetBindingVerifier
            .verify_for_session("remote_desktop.create_session", &binding)
            .expect("focus window binding verifies");
        binding
            .commit_capture_proof("remote_desktop.create_session", proof)
            .expect("focus window proof commits");
        binding
    }

    fn create_window_session(
        plugin: &Arc<RemoteDesktopPlugin>,
        subject: &str,
        session_id: &str,
        remote_focus: bool,
    ) -> Value {
        let env = env_for(subject);
        let session = RemoteDesktopSession::new(RemoteDesktopSessionInit {
            session_id: session_id.to_string(),
            session_token: "focus-token".to_string(),
            creator_caller_ura: env.caller().to_string(),
            consent: RemoteDesktopConsentGrant::from_envelope_with_grants_for_test(
                &env,
                true,
                remote_focus,
            ),
            target_binding: test_window_binding(subject),
            mode: "interactive".to_string(),
            lease_ttl_ms: 5_000,
            transport_preferences: vec!["webrtc".to_string()],
            video: RemoteDesktopVideoConstraints::default(),
            input_policy: RemoteDesktopInputPolicy::new(true, true),
        });
        let created = plugin.session_view_with_token(&session);
        plugin.session_store().with_sessions(|sessions| {
            sessions.insert(session_id.to_string(), session);
        });
        created
    }

    fn focus_args(created: &Value) -> Value {
        let tracking = &created["target_tracking"];
        json!({
            "session_id": created["session_id"],
            "session_token": created["session_token"],
            "expected_consent_epoch": created["consent"]["consent_epoch"],
            "expected_binding_epoch": tracking["binding_epoch"],
            "expected_target_identity_epoch": tracking["target_identity_epoch"],
            "expected_target_geometry_revision": tracking["target_geometry_revision"],
            "expected_target_focus_epoch": tracking["target_focus_epoch"],
        })
    }

    #[test]
    fn focus_target_commits_verified_focus_epoch_before_input_can_resume() {
        let _lock = test_lock();
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let focus = Arc::new(RecordingFocusController::default());
        let plugin = test_plugin(Arc::clone(&focus));
        reset_store(&plugin);
        let ura = "easynet:///r/acme/resource/device.01DEV/streams/window.focus-success";
        let created = create_window_session(&plugin, &ura, "rd-focus-success", true);
        plugin.session_store().with_sessions(|sessions| {
            sessions
                .get_mut("rd-focus-success")
                .expect("focus session")
                .record_target_observation(TargetObservation::FocusChanged {
                    focused: false,
                    observed_at_ms: 100,
                });
        });
        let mut current = plugin.session_store().with_sessions(|sessions| {
            plugin.session_view(sessions.get("rd-focus-success").expect("focus session"))
        });
        current["session_token"] = created["session_token"].clone();

        let response = handle(Arc::clone(&plugin), env_for(&ura), focus_args(&current))
            .expect("authorized exact focus succeeds");

        assert_eq!(focus.calls.load(Ordering::SeqCst), 1);
        assert_eq!(response["focused"], json!(true));
        assert_eq!(response["platform_backend"], json!("test_exact_focus"));
        assert_eq!(
            response["session"]["target_tracking"]["input_enabled"],
            json!(true),
            "focus response must expose input-ready target tracking: {response}"
        );
        assert!(
            response["target_focus_epoch"].as_u64().unwrap()
                > response["previous_target_focus_epoch"].as_u64().unwrap()
        );
        assert!(response["session"]["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event["event_type"] == json!("TARGET_FOCUS_APPLIED")));
    }

    #[test]
    fn admitted_focus_crossing_lease_deadline_commits_exactly_once() {
        let _lock = test_lock();
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let focus = Arc::new(LeaseExpiringFocusController::default());
        let plugin = test_plugin_with_controller(focus.clone());
        reset_store(&plugin);
        let ura = "easynet:///r/acme/resource/device.01DEV/streams/window.focus-crossed-lease";
        let session_id = "rd-focus-crossed-lease";
        let created = create_window_session(&plugin, ura, session_id, true);
        plugin.session_store().with_sessions(|sessions| {
            sessions
                .get_mut(session_id)
                .expect("focus session")
                .record_target_observation(TargetObservation::FocusChanged {
                    focused: false,
                    observed_at_ms: 100,
                });
        });
        let mut current = plugin.session_store().with_sessions(|sessions| {
            plugin.session_view(sessions.get(session_id).expect("focus session"))
        });
        current["session_token"] = created["session_token"].clone();
        focus.expire_during_focus(plugin.session_store(), session_id);

        let response = handle(Arc::clone(&plugin), env_for(ura), focus_args(&current))
            .expect("a focus effect admitted before lease expiry must commit");

        assert_eq!(focus.calls.load(Ordering::SeqCst), 1);
        assert_eq!(response["focused"], json!(true));
        assert_eq!(
            response["platform_backend"],
            json!("test_focus_crossed_lease_deadline")
        );
        plugin.session_store().with_sessions(|sessions| {
            let session = sessions.get(session_id).expect("focus session remains");
            assert!(
                session.is_expired_at(crate::daemon::plugins::remote_desktop::session::now_ms())
            );
            assert_eq!(
                session.target_snapshot().target_focus_epoch(),
                response["target_focus_epoch"]
                    .as_u64()
                    .expect("focus response epoch")
            );
        });
    }

    #[test]
    fn focus_target_expired_before_admission_has_no_host_effect() {
        let _lock = test_lock();
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let focus = Arc::new(RecordingFocusController::default());
        let plugin = test_plugin(Arc::clone(&focus));
        reset_store(&plugin);
        let ura = "easynet:///r/acme/resource/device.01DEV/streams/window.focus-expired";
        let session_id = "rd-focus-expired";
        let created = create_window_session(&plugin, ura, session_id, true);
        plugin.session_store().with_sessions(|sessions| {
            let session = sessions.get_mut(session_id).expect("focus session");
            session.record_target_observation(TargetObservation::FocusChanged {
                focused: false,
                observed_at_ms: 100,
            });
            session.set_lease_expires_at_for_test(
                crate::daemon::plugins::remote_desktop::session::now_ms().saturating_sub(1),
            );
        });
        let mut current = plugin.session_store().with_sessions(|sessions| {
            plugin.session_view(sessions.get(session_id).expect("focus session"))
        });
        current["session_token"] = created["session_token"].clone();

        let error = handle(Arc::clone(&plugin), env_for(ura), focus_args(&current))
            .expect_err("an already-expired focus request must fail admission");

        assert!(matches!(
            error.downcast_ref::<RemoteDesktopError>(),
            Some(RemoteDesktopError::SessionExpired { .. })
        ));
        assert_eq!(focus.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn focus_target_requires_separate_remote_focus_consent_before_host_mutation() {
        let _lock = test_lock();
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let focus = Arc::new(RecordingFocusController::default());
        let plugin = test_plugin(Arc::clone(&focus));
        reset_store(&plugin);
        let ura = "easynet:///r/acme/resource/device.01DEV/streams/window.focus-denied";
        let created = create_window_session(&plugin, &ura, "rd-focus-denied", false);

        let error = handle(Arc::clone(&plugin), env_for(&ura), focus_args(&created))
            .expect_err("input control alone must not authorize target focus");

        let focus_error = error
            .downcast_ref::<RemoteAppTargetFocusError>()
            .expect("focus denial remains typed");
        assert_eq!(
            focus_error.reason(),
            TargetFocusFailureReason::RemoteFocusNotConsented
        );
        assert_eq!(focus.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn focus_target_rejects_every_stale_expected_epoch_before_host_mutation() {
        let _lock = test_lock();
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let focus = Arc::new(RecordingFocusController::default());
        let plugin = test_plugin(Arc::clone(&focus));
        reset_store(&plugin);
        let ura = "easynet:///r/acme/resource/device.01DEV/streams/window.focus-stale";
        let created = create_window_session(&plugin, &ura, "rd-focus-stale", true);

        for field in [
            "expected_consent_epoch",
            "expected_binding_epoch",
            "expected_target_identity_epoch",
            "expected_target_geometry_revision",
            "expected_target_focus_epoch",
        ] {
            let mut args = focus_args(&created);
            let stale = args[field]
                .as_u64()
                .expect("expected epoch is an integer")
                .saturating_add(1);
            args[field] = json!(stale);

            let error = handle(Arc::clone(&plugin), env_for(&ura), args)
                .expect_err("stale focus epoch must fail before host mutation");
            let focus_error = error
                .downcast_ref::<RemoteAppTargetFocusError>()
                .expect("stale focus failure remains typed");
            assert_eq!(
                focus_error.reason(),
                TargetFocusFailureReason::TargetFocusStale,
                "unexpected failure for {field}"
            );
        }
        assert_eq!(
            focus.calls.load(Ordering::SeqCst),
            0,
            "no stale request may reach the host focus controller"
        );
    }

    #[test]
    fn focus_target_rejects_non_blurred_target_state_before_host_mutation() {
        let _lock = test_lock();
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let focus = Arc::new(RecordingFocusController::default());
        let plugin = test_plugin(Arc::clone(&focus));
        reset_store(&plugin);
        let ura = "easynet:///r/acme/resource/device.01DEV/streams/window.focus-not-blurred";
        let created = create_window_session(&plugin, &ura, "rd-focus-not-blurred", true);

        let error = handle(Arc::clone(&plugin), env_for(&ura), focus_args(&created))
            .expect_err("focus must not mutate a target outside current target_blurred state");

        let focus_error = error
            .downcast_ref::<RemoteAppTargetFocusError>()
            .expect("non-blurred focus failure remains typed");
        assert_eq!(
            focus_error.reason(),
            TargetFocusFailureReason::TargetFocusStale
        );
        assert_eq!(focus.calls.load(Ordering::SeqCst), 0);
    }
}
