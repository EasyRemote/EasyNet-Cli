// EasyNet CLI — remote desktop attach handler
// ===========================================

use std::sync::Arc;

use serde_json::Value;
use tokio::sync::{mpsc, watch};

use crate::daemon::ability::dispatch::{
    bidi_input_channel, BidiOutputFrame, BidiSource, EnvelopeContext, BIDI_CHANNEL_BOUND,
};
use crate::daemon::plugins::remote_desktop::constants::ABILITY_ATTACH_SESSION;
use crate::daemon::plugins::remote_desktop::errors::RemoteDesktopError;
use crate::daemon::plugins::remote_desktop::input::EffectiveRemoteDesktopInputPolicy;
use crate::daemon::plugins::remote_desktop::invoke_bidi::{
    spawn_bidi_capture_worker, BidiCaptureWorkerConfig,
};
use crate::daemon::plugins::remote_desktop::request::require_str;
use crate::daemon::plugins::remote_desktop::request::{
    parse_attach_capture_options, parse_attach_encoding,
};
use crate::daemon::plugins::remote_desktop::runtime::RemoteDesktopPlugin;
use crate::daemon::plugins::remote_desktop::session_lifecycle::ensure_session_access;

/// Handle `remote_desktop.attach`.
pub(in crate::daemon::plugins::remote_desktop) fn handle(
    plugin: Arc<RemoteDesktopPlugin>,
    env: EnvelopeContext,
    args: Value,
) -> anyhow::Result<BidiSource> {
    let session_id = require_str(&args, "session_id", ABILITY_ATTACH_SESSION)?.to_string();
    let target_snapshot_executor = plugin.target_snapshot_executor();
    let session_store = plugin.session_store();
    let (completion_tx, completion_rx) = std::sync::mpsc::channel();
    let (
        target_binding,
        options,
        input_policy,
        encoding,
        preview_epoch,
        stop_tx,
        stop_rx,
        old_stop,
    ) = session_store.with_target_operation_session(
        &session_id,
        |session| -> anyhow::Result<_> {
            let session = session.ok_or_else(|| RemoteDesktopError::SessionNotFound {
                ability: ABILITY_ATTACH_SESSION,
                session_id: session_id.clone(),
            })?;
            ensure_session_access(&plugin, ABILITY_ATTACH_SESSION, &env, &args, session)?;
            let target_binding = session.target_binding().clone();
            let options = parse_attach_capture_options(&args, session)?;
            let encoding = parse_attach_encoding(&args)?;
            let input_policy = EffectiveRemoteDesktopInputPolicy::for_binding(
                session.input_policy(),
                &target_binding,
                Arc::clone(&target_snapshot_executor),
            );
            let (stop_tx, stop_rx) = watch::channel(false);
            let (preview_epoch, old_stop) = session
                .attach_preview_transport(stop_tx.clone())
                .expect("live admitted session accepts diagnostic preview");
            Ok((
                target_binding,
                options,
                input_policy,
                encoding,
                preview_epoch,
                stop_tx,
                stop_rx,
                old_stop,
            ))
        },
    )?;
    plugin.transport_manager().activate_preview(
        session_id.clone(),
        preview_epoch,
        stop_tx.clone(),
        completion_rx,
    );
    if let Some(old_stop) = old_stop {
        let _ = old_stop.send(true);
    }

    let (xport_to_handler_tx, xport_to_handler_rx) = bidi_input_channel(BIDI_CHANNEL_BOUND);
    let (xport_from_handler_tx, xport_from_handler_rx) =
        mpsc::channel::<BidiOutputFrame>(plugin.config().max_frame_queue());
    spawn_bidi_capture_worker(BidiCaptureWorkerConfig {
        session_store: plugin.session_store(),
        session_id: session_id.clone(),
        preview_epoch,
        backend: plugin.screen_backend(),
        target_binding,
        options,
        encoding,
        input_policy,
        from_client: xport_to_handler_rx,
        to_client: xport_from_handler_tx,
        stop_tx: stop_tx.clone(),
        stop_rx,
        max_frame_queue_depth: plugin.config().max_frame_queue(),
        completion_tx,
    });
    Ok(BidiSource {
        to_client: xport_to_handler_tx,
        from_client: xport_from_handler_rx,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{mpsc as std_mpsc, Mutex};
    use std::time::{Duration, Instant};

    use serde_json::json;
    use tokio::sync::broadcast;

    use crate::daemon::ability::builtins::resources::media::screen_snapshot::{
        EncodedFrame, ScreenCaptureOptions, ScreenSnapshotBackend, SyntheticScreenBackend,
    };
    use crate::daemon::ability::dispatch::AxonAbilityCatalog;
    use crate::daemon::invocation::routing::target::{CallMode, InvocationTarget, TargetScope};
    use crate::daemon::persistence::resources::{
        self, ResourceBinding, ResourceEntry, ResourceType, ResourcesFile,
    };
    use crate::daemon::plugins::remote_desktop::constants::{
        ABILITY_ATTACH_SESSION, ABILITY_CREATE_SESSION, ABILITY_GRANT_CONSENT,
        REASON_PREVIEW_CAPTURE_FAILED, REASON_PREVIEW_CLIENT_CLOSED, TRANSPORT_INVOKE_BIDI,
    };
    use crate::daemon::plugins::remote_desktop::input::RemoteDesktopInputPolicy;
    use crate::daemon::plugins::remote_desktop::request::RemoteDesktopVideoConstraints;
    use crate::daemon::plugins::remote_desktop::runtime::RemoteDesktopPlugin;
    use crate::daemon::plugins::remote_desktop::session::{
        RemoteDesktopSession, RemoteDesktopSessionInit, RemoteDesktopState,
    };
    use crate::daemon::plugins::remote_desktop::session_consent::RemoteDesktopConsentGrant;
    use crate::daemon::plugins::remote_desktop::target::ResourceEntryTargetResolver;
    use crate::daemon::plugins::remote_desktop::test_support::live_remote_target_metadata;
    use crate::daemon::plugins::remote_desktop::test_support::{
        env_for, reset_store, seed_display, test_catalog_with_screen_backend,
        test_consent_causal_context, test_lock, test_plugin, test_runtime_limits,
        TestRemoteAppTargetBindingVerifier,
    };

    #[derive(Debug)]
    struct FailingScreenBackend;

    #[derive(Debug)]
    struct BlockingScreenBackend {
        started: Mutex<Option<std_mpsc::Sender<()>>>,
        release: Mutex<std_mpsc::Receiver<()>>,
    }

    impl ScreenSnapshotBackend for BlockingScreenBackend {
        fn capture_jpeg(
            &self,
            _entry: &ResourceEntry,
            _options: &ScreenCaptureOptions,
        ) -> anyhow::Result<EncodedFrame> {
            if let Some(started) = self
                .started
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take()
            {
                let _ = started.send(());
            }
            self.release
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .recv_timeout(Duration::from_secs(2))
                .map_err(|error| anyhow::anyhow!("release blocking capture: {error}"))?;
            Ok(EncodedFrame {
                jpeg_bytes: vec![0xff, 0xd8, 0xff, 0xd9],
                width: 1,
                height: 1,
            })
        }

        fn open_stream(
            &self,
            _entry: ResourceEntry,
            _options: ScreenCaptureOptions,
        ) -> anyhow::Result<broadcast::Receiver<Value>> {
            anyhow::bail!("stream not used by blocking preview test")
        }
    }

    impl ScreenSnapshotBackend for FailingScreenBackend {
        fn capture_jpeg(
            &self,
            _entry: &ResourceEntry,
            _options: &ScreenCaptureOptions,
        ) -> anyhow::Result<EncodedFrame> {
            anyhow::bail!("synthetic capture failure")
        }

        fn open_stream(
            &self,
            _entry: ResourceEntry,
            _options: ScreenCaptureOptions,
        ) -> anyhow::Result<broadcast::Receiver<Value>> {
            anyhow::bail!("synthetic stream failure")
        }
    }

    fn grant_consent_ticket(dispatcher: &AxonAbilityCatalog, subject: &str) -> String {
        let granted = dispatcher
            .execute_rpc(InvocationTarget {
                scope: TargetScope::Local,
                ability: ABILITY_GRANT_CONSENT.to_string(),
                normalized_args: json!({"intent": "remote_desktop_session"}),
                call_mode: CallMode::Rpc,
                subject: crate::daemon::invocation::routing::target::InvocationSubject::explicit(
                    subject.to_string(),
                ),
                causal_context:
                    crate::daemon::invocation::routing::target::InvocationCausalContext::explicit(
                        test_consent_causal_context(),
                    ),
                request_metadata: std::collections::HashMap::new(),
            })
            .expect("grant_consent issues a local test ticket");
        granted["consent_ticket"]
            .as_str()
            .expect("grant_consent returns consent_ticket")
            .to_string()
    }

    fn bidi_open_failure(result: anyhow::Result<BidiSource>) -> String {
        match result {
            Err(error) => error.to_string(),
            Ok(mut source) => tokio::runtime::Runtime::new()
                .expect("bidi failure observer runtime")
                .block_on(async move {
                    let frame =
                        tokio::time::timeout(Duration::from_secs(2), source.from_client.recv())
                            .await
                            .expect("rejected bidi invocation reaches terminal outcome")
                            .expect("rejected bidi invocation emits terminal error")
                            .into_json_value()
                            .expect("bidi terminal failure is JSON");
                    assert_eq!(frame["type"], json!("error"));
                    frame.to_string()
                }),
        }
    }

    #[test]
    fn attach_bidi_emits_synthetic_frame_and_closes_on_request() {
        let _lock = test_lock();
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let _g = crate::cli::commands::test_support::HomeGuard::new();
            let mut file = ResourcesFile::default();
            let ura = seed_display(&mut file, "remote-desktop-bidi-display");
            resources::save(&file).unwrap();

            let reg = test_catalog_with_screen_backend(Arc::new(SyntheticScreenBackend));
            let dispatcher = Arc::new(reg);
            let create_target = InvocationTarget {
                scope: TargetScope::Local,
                ability: ABILITY_CREATE_SESSION.to_string(),
                normalized_args: json!({
                    "session_id": "rd-bidi-test",
                    "consent_ticket": grant_consent_ticket(&dispatcher, &ura),
                    "mode": "view_only",
                    "lease_ttl_ms": 5000,
                    "video": { "max_width": 320, "max_height": 180, "max_fps": 144 },
                }),
                call_mode: CallMode::Rpc,
                subject: crate::daemon::invocation::routing::target::InvocationSubject::explicit(
                    ura.clone(),
                ),
                causal_context:
                    crate::daemon::invocation::routing::target::InvocationCausalContext::explicit(
                        test_consent_causal_context(),
                    ),
                request_metadata: std::collections::HashMap::new(),
            };
            let created = dispatcher.execute_rpc(create_target).unwrap();
            let token = created["session_token"]
                .as_str()
                .expect("create_session returns session_token")
                .to_string();

            let attach_target = InvocationTarget {
                scope: TargetScope::Local,
                ability: ABILITY_ATTACH_SESSION.to_string(),
                normalized_args: json!({
                    "session_id": "rd-bidi-test",
                    "session_token": token,
                    "fps": 144,
                    "resolution": "320x180"
                }),
                call_mode: CallMode::Bidi,
                subject: crate::daemon::invocation::routing::target::InvocationSubject::explicit(
                    ura,
                ),
                causal_context:
                    crate::daemon::invocation::routing::target::InvocationCausalContext::explicit(
                        test_consent_causal_context(),
                    ),
                request_metadata: std::collections::HashMap::new(),
            };
            let mut bidi = dispatcher.execute_bidi(attach_target).unwrap();
            let first = tokio::time::timeout(Duration::from_secs(2), bidi.from_client.recv())
                .await
                .unwrap()
                .unwrap()
                .into_json_value()
                .unwrap();
            assert_eq!(first["type"], json!("transport"));
            let frame = tokio::time::timeout(Duration::from_secs(2), bidi.from_client.recv())
                .await
                .unwrap()
                .unwrap()
                .into_json_value()
                .unwrap();
            assert_eq!(frame["type"], json!("frame"));
            assert_eq!(frame["transport"], json!(TRANSPORT_INVOKE_BIDI));
            assert_eq!(frame["encoding"], json!("binary"));
            assert_eq!(frame["width"], json!(320));
            assert_eq!(frame["height"], json!(180));
            let raw_frame = tokio::time::timeout(Duration::from_secs(2), bidi.from_client.recv())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(raw_frame.content_type, "image/jpeg");
            assert!(raw_frame.payload.len() > 100);

            bidi.to_client.send(json!({"type": "close"})).await.unwrap();
            let closed = tokio::time::timeout(Duration::from_secs(2), bidi.from_client.recv())
                .await
                .unwrap()
                .unwrap()
                .into_json_value()
                .unwrap();
            assert_eq!(closed["type"], json!("closed"));
        });
    }

    #[test]
    fn attach_bidi_client_close_detaches_preview_transport_state() {
        let _lock = test_lock();
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let _g = crate::cli::commands::test_support::HomeGuard::new();
            let plugin = test_plugin();
            let mut file = ResourcesFile::default();
            let ura = seed_display(&mut file, "remote-desktop-close-detach-display");
            resources::save(&file).unwrap();
            let created =
                crate::daemon::plugins::remote_desktop::test_support::create_test_session(
                    Arc::clone(&plugin),
                    env_for(&ura),
                    json!({
                        "session_id": "rd-close-detach",
                        "mode": "view_only",
                        "lease_ttl_ms": 5000,
                        "video": { "max_width": 320, "max_height": 180, "max_fps": 144 },
                    }),
                )
                .unwrap();
            let token = created["session_token"].as_str().unwrap().to_string();
            let mut bidi = handle(
                Arc::clone(&plugin),
                env_for(&ura),
                json!({
                    "session_id": "rd-close-detach",
                    "session_token": token,
                    "fps": 144,
                    "resolution": "320x180",
                }),
            )
            .unwrap();
            bidi.to_client.send(json!({"type": "close"})).await.unwrap();
            let closed = read_until_closed(&mut bidi.from_client).await;
            assert_eq!(closed["type"], json!("closed"));
            assert_eq!(closed["reason"], json!(REASON_PREVIEW_CLIENT_CLOSED));

            plugin.session_store().with_sessions(|sessions| {
                let session = sessions.get("rd-close-detach").unwrap();
                assert!(!session.preview_attached());
                assert_eq!(session.state(), RemoteDesktopState::Negotiating);
                assert!(session.events().iter().any(|event| {
                    event["event_type"] == json!("TRANSPORT_DETACHED")
                        && event["payload"]["reason"] == json!(REASON_PREVIEW_CLIENT_CLOSED)
                }));
            });
        });
    }

    async fn read_until_closed(rx: &mut tokio::sync::mpsc::Receiver<BidiOutputFrame>) -> Value {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            let frame = tokio::time::timeout(remaining, rx.recv())
                .await
                .expect("closed frame before timeout")
                .expect("bidi output channel open");
            if let Ok(value) = frame.into_json_value() {
                if value["type"] == json!("closed") {
                    return value;
                }
            }
        }
    }

    fn insert_window_session(plugin: &RemoteDesktopPlugin, session_id: &str, subject_ura: &str) {
        let env = env_for(subject_ura);
        let entry = ResourceEntry {
            resource_ura: subject_ura.to_string(),
            owner_agent: "easynet:///r/acme/agent/device.01DEV.media".to_string(),
            kind: ResourceType::Window,
            binding: ResourceBinding::LocalDevice,
            hardware_id: "window:macos:cgwindow:10:42".to_string(),
            display_name: "Test Window".to_string(),
            metadata: live_remote_target_metadata(json!({
                "window_id": 42,
                "pid": 10,
                "x": 0,
                "y": 0,
                "width": 800,
                "height": 600,
            })),
            first_seen_at: "2026-06-01T00:00:00Z".to_string(),
        };
        let target_binding = ResourceEntryTargetResolver
            .resolve_for_session(ABILITY_CREATE_SESSION, &entry, "view_only", 1)
            .expect("window target binding resolves for attach test");
        let session = RemoteDesktopSession::new(RemoteDesktopSessionInit {
            session_id: session_id.to_string(),
            session_token: "token".to_string(),
            creator_caller_ura: env.caller().to_string(),
            consent: RemoteDesktopConsentGrant::from_envelope_for_test(&env),
            target_binding,
            mode: "view_only".to_string(),
            lease_ttl_ms: 5_000,
            transport_preferences: vec![TRANSPORT_INVOKE_BIDI.to_string()],
            video: RemoteDesktopVideoConstraints::default(),
            input_policy: RemoteDesktopInputPolicy::default(),
        });
        plugin.session_store().with_sessions(|sessions| {
            sessions.insert(session_id.to_string(), session);
        });
    }

    #[test]
    fn attach_bidi_accepts_window_binding_before_frame_source_selection() {
        let _lock = test_lock();
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let plugin = test_plugin();
            let subject_ura = "easynet:///r/acme/resource/device.01DEV/streams/window.test";
            insert_window_session(&plugin, "rd-window-preview-accepted", subject_ura);

            let bidi = handle(
                Arc::clone(&plugin),
                env_for(subject_ura),
                json!({
                    "session_id": "rd-window-preview-accepted",
                    "session_token": "token",
                }),
            )
            .expect("attach handler must accept a session-owned window binding");

            let _ = bidi.to_client.send(json!({"type": "close"})).await;
        });
    }

    #[test]
    fn attach_bidi_capture_failure_marks_session_failed() {
        let _lock = test_lock();
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let _g = crate::cli::commands::test_support::HomeGuard::new();
            let plugin = RemoteDesktopPlugin::with_target_binding_verifier(
                Arc::new(FailingScreenBackend),
                Arc::new(TestRemoteAppTargetBindingVerifier),
                test_runtime_limits().into(),
            );
            let mut file = ResourcesFile::default();
            let ura = seed_display(&mut file, "remote-desktop-capture-failure-display");
            resources::save(&file).unwrap();
            let created =
                crate::daemon::plugins::remote_desktop::test_support::create_test_session(
                    Arc::clone(&plugin),
                    env_for(&ura),
                    json!({
                        "session_id": "rd-capture-failed",
                        "mode": "view_only",
                        "lease_ttl_ms": 5000,
                        "video": { "max_width": 320, "max_height": 180, "max_fps": 144 },
                    }),
                )
                .unwrap();
            let token = created["session_token"].as_str().unwrap().to_string();
            let mut bidi = handle(
                Arc::clone(&plugin),
                env_for(&ura),
                json!({
                    "session_id": "rd-capture-failed",
                    "session_token": token,
                    "fps": 144,
                    "resolution": "320x180",
                }),
            )
            .unwrap();
            let transport = tokio::time::timeout(Duration::from_secs(2), bidi.from_client.recv())
                .await
                .unwrap()
                .unwrap()
                .into_json_value()
                .unwrap();
            assert_eq!(transport["type"], json!("transport"));
            let error = tokio::time::timeout(Duration::from_secs(2), bidi.from_client.recv())
                .await
                .unwrap()
                .unwrap()
                .into_json_value()
                .unwrap();
            assert_eq!(error["type"], json!("error"));
            match tokio::time::timeout(Duration::from_millis(100), bidi.from_client.recv()).await {
                Err(_) | Ok(None) => {}
                Ok(Some(frame)) => panic!(
                    "terminal error must not be followed by another frame: {:?}",
                    frame.into_json_value()
                ),
            }

            plugin.session_store().with_sessions(|sessions| {
                let session = sessions.get("rd-capture-failed").unwrap();
                assert!(!session.preview_attached());
                assert_eq!(session.state(), RemoteDesktopState::Negotiating);
                assert_eq!(session.end_reason(), None);
                assert!(session.events().iter().any(|event| {
                    event["event_type"] == json!("DIAGNOSTIC_PREVIEW_FAILED")
                        && event["payload"]["transport_kind"] == json!(TRANSPORT_INVOKE_BIDI)
                        && event["payload"]["reason"] == json!(REASON_PREVIEW_CAPTURE_FAILED)
                }));
            });
        });
    }

    #[test]
    fn attach_reserves_preview_ownership_before_worker_can_race_session_close() {
        let _lock = test_lock();
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let _g = crate::cli::commands::test_support::HomeGuard::new();
            let (started_tx, started_rx) = std_mpsc::channel();
            let (release_tx, release_rx) = std_mpsc::channel();
            let plugin = RemoteDesktopPlugin::with_target_binding_verifier(
                Arc::new(BlockingScreenBackend {
                    started: Mutex::new(Some(started_tx)),
                    release: Mutex::new(release_rx),
                }),
                Arc::new(TestRemoteAppTargetBindingVerifier),
                test_runtime_limits().into(),
            );
            reset_store(&plugin);
            let mut file = ResourcesFile::default();
            let ura = seed_display(&mut file, "remote-desktop-attach-close-race-display");
            resources::save(&file).unwrap();
            let env = env_for(&ura);
            let created =
                crate::daemon::plugins::remote_desktop::test_support::create_test_session(
                    Arc::clone(&plugin),
                    env.clone(),
                    json!({
                        "session_id": "rd-attach-close-race",
                        "mode": "view_only",
                        "lease_ttl_ms": 5000,
                    }),
                )
                .expect("test session creates");
            let token = created["session_token"]
                .as_str()
                .expect("create_session returns token")
                .to_string();
            let _bidi = handle(
                Arc::clone(&plugin),
                env.clone(),
                json!({
                    "session_id": "rd-attach-close-race",
                    "session_token": token.clone(),
                }),
            )
            .expect("attach starts owned preview");
            started_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("preview capture worker starts");

            let closing_plugin = Arc::clone(&plugin);
            let close = std::thread::spawn(move || {
                crate::daemon::plugins::remote_desktop::handlers::end_session::handle(
                    closing_plugin,
                    env,
                    json!({
                        "session_id": "rd-attach-close-race",
                        "session_token": token,
                        "reason": "test_attach_close_race",
                    }),
                )
            });
            let deadline = Instant::now() + Duration::from_secs(1);
            loop {
                let closing = plugin.session_store().with_sessions(|sessions| {
                    sessions
                        .get("rd-attach-close-race")
                        .is_some_and(|session| session.is_terminating() && !session.is_terminal())
                });
                if closing {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "end_session did not enter Closing while preview capture was blocked"
                );
                std::thread::yield_now();
            }

            release_tx
                .send(())
                .expect("blocked preview capture is released");
            let ended = close
                .join()
                .expect("end_session thread exits")
                .expect("end_session completes after owned worker exits");
            assert_eq!(ended["state"], json!("closed"));
            assert_eq!(ended["end_reason"], json!("test_attach_close_race"));
        });
    }

    #[test]
    fn attach_bidi_rejects_subject_mismatch() {
        let _lock = test_lock();
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let ura = seed_display(&mut file, "remote-desktop-bidi-display");
        let other_ura = seed_display(&mut file, "remote-desktop-bidi-other-display");
        resources::save(&file).unwrap();

        let reg = test_catalog_with_screen_backend(Arc::new(SyntheticScreenBackend));
        let dispatcher = Arc::new(reg);
        let created = dispatcher
            .execute_rpc(InvocationTarget {
                scope: TargetScope::Local,
                ability: ABILITY_CREATE_SESSION.to_string(),
                normalized_args: json!({
                    "session_id": "rd-bidi-subject-mismatch",
                    "consent_ticket": grant_consent_ticket(&dispatcher, &ura),
                    "mode": "view_only",
                    "lease_ttl_ms": 5000,
                    "video": { "max_width": 320, "max_height": 180, "max_fps": 144 },
                }),
                call_mode: CallMode::Rpc,
                subject: crate::daemon::invocation::routing::target::InvocationSubject::explicit(
                    ura,
                ),
                causal_context:
                    crate::daemon::invocation::routing::target::InvocationCausalContext::explicit(
                        test_consent_causal_context(),
                    ),
                request_metadata: std::collections::HashMap::new(),
            })
            .unwrap();
        let token = created["session_token"].as_str().unwrap().to_string();

        let failure = bidi_open_failure(dispatcher.execute_bidi(InvocationTarget {
            scope: TargetScope::Local,
            ability: ABILITY_ATTACH_SESSION.to_string(),
            normalized_args: json!({
                "session_id": "rd-bidi-subject-mismatch",
                "session_token": token,
            }),
            call_mode: CallMode::Bidi,
            subject: crate::daemon::invocation::routing::target::InvocationSubject::explicit(
                other_ura,
            ),
            causal_context:
                crate::daemon::invocation::routing::target::InvocationCausalContext::explicit(
                    test_consent_causal_context(),
                ),
            request_metadata: std::collections::HashMap::new(),
        }));
        assert!(failure.contains("does not match session subject"));
    }

    #[test]
    fn attach_bidi_rejects_daemon_derived_subject() {
        let _lock = test_lock();
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let ura = seed_display(&mut file, "remote-desktop-bidi-missing-subject-display");
        resources::save(&file).unwrap();

        let reg = test_catalog_with_screen_backend(Arc::new(SyntheticScreenBackend));
        let dispatcher = Arc::new(reg);
        let created = dispatcher
            .execute_rpc(InvocationTarget {
                scope: TargetScope::Local,
                ability: ABILITY_CREATE_SESSION.to_string(),
                normalized_args: json!({
                    "session_id": "rd-bidi-missing-subject",
                    "consent_ticket": grant_consent_ticket(&dispatcher, &ura),
                    "mode": "view_only",
                    "lease_ttl_ms": 5000,
                    "video": { "max_width": 320, "max_height": 180, "max_fps": 144 },
                }),
                call_mode: CallMode::Rpc,
                subject: crate::daemon::invocation::routing::target::InvocationSubject::explicit(
                    ura,
                ),
                causal_context:
                    crate::daemon::invocation::routing::target::InvocationCausalContext::explicit(
                        test_consent_causal_context(),
                    ),
                request_metadata: std::collections::HashMap::new(),
            })
            .unwrap();
        let token = created["session_token"].as_str().unwrap().to_string();

        let failure = bidi_open_failure(dispatcher.execute_bidi(InvocationTarget {
                scope: TargetScope::Local,
                ability: ABILITY_ATTACH_SESSION.to_string(),
                normalized_args: json!({
                    "session_id": "rd-bidi-missing-subject",
                    "session_token": token,
                }),
                call_mode: CallMode::Bidi,
                subject: crate::daemon::invocation::routing::target::InvocationSubject::daemon_system_derived(),
                causal_context: crate::daemon::invocation::routing::target::InvocationCausalContext::explicit(test_consent_causal_context()),
                request_metadata: std::collections::HashMap::new(),
            }));
        assert!(failure.contains("does not match session subject"));
    }
}
