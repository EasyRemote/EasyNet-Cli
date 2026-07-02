// EasyNet CLI — remote desktop attach handler
// ===========================================

use std::sync::Arc;

use serde_json::Value;
use tokio::sync::{mpsc, watch};

use crate::plugins::remote_desktop::constants::{ABILITY_ATTACH_SESSION, REASON_SESSION_NOT_FOUND};
use crate::plugins::remote_desktop::invoke_bidi::{
    spawn_bidi_capture_worker, BidiCaptureWorkerConfig,
};
use crate::plugins::remote_desktop::request::require_str;
use crate::plugins::remote_desktop::request::{
    parse_attach_capture_options, parse_attach_encoding,
};
use crate::plugins::remote_desktop::resource::resolve_screen_resource;
use crate::plugins::remote_desktop::runtime::RemoteDesktopPlugin;
use crate::plugins::remote_desktop::session_lifecycle::ensure_session_access;
use crate::runtime::ability_dispatch::{
    BidiOutputFrame, BidiSource, EnvelopeContext, BIDI_CHANNEL_BOUND,
};

/// Handle `remote_desktop.attach`.
pub(in crate::plugins::builtin::remote_desktop) fn handle(
    plugin: Arc<RemoteDesktopPlugin>,
    env: EnvelopeContext,
    args: Value,
) -> anyhow::Result<BidiSource> {
    let session_id = require_str(&args, "session_id", ABILITY_ATTACH_SESSION)?.to_string();
    let (entry, options, input_policy, encoding, stop_tx, stop_rx) = {
        plugin
            .session_store()
            .with_sessions(|sessions| -> anyhow::Result<_> {
                let session = sessions.get_mut(&session_id).ok_or_else(|| {
                    anyhow::anyhow!(
                        "{ABILITY_ATTACH_SESSION}: session {session_id:?} not found; reason={REASON_SESSION_NOT_FOUND}"
                    )
                })?;
                ensure_session_access(&plugin, ABILITY_ATTACH_SESSION, &env, &args, session)?;
                let entry = resolve_screen_resource(ABILITY_ATTACH_SESSION, session.subject_ura())?;
                let options = parse_attach_capture_options(&args, session)?;
                let encoding = parse_attach_encoding(&args)?;
                let input_policy = session.input_policy().to_value();
                let (stop_tx, stop_rx) = watch::channel(false);
                if let Some(old_stop) = session.attach_preview_transport(stop_tx.clone()) {
                    let _ = old_stop.send(true);
                }
                Ok((entry, options, input_policy, encoding, stop_tx, stop_rx))
            })?
    };

    let (xport_to_handler_tx, xport_to_handler_rx) = mpsc::channel::<Value>(BIDI_CHANNEL_BOUND);
    let (xport_from_handler_tx, xport_from_handler_rx) =
        mpsc::channel::<BidiOutputFrame>(plugin.config().max_frame_queue());
    spawn_bidi_capture_worker(BidiCaptureWorkerConfig {
        session_store: plugin.session_store(),
        session_id,
        backend: plugin.screen_backend(),
        entry,
        options,
        encoding,
        input_policy,
        from_client: xport_to_handler_rx,
        to_client: xport_from_handler_tx,
        stop_tx,
        stop_rx,
        max_frame_queue_depth: plugin.config().max_frame_queue(),
    });
    Ok(BidiSource {
        to_client: xport_to_handler_tx,
        from_client: xport_from_handler_rx,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use serde_json::json;
    use tokio::sync::broadcast;

    use crate::daemon::ability::builtins::resources::media::screen_snapshot::{
        EncodedFrame, ScreenCaptureOptions, ScreenSnapshotBackend, SyntheticScreenBackend,
    };
    use crate::daemon::plugins::{
        DaemonPluginBinder, PluginContributionBuilder, PluginContributionSet, PluginKind,
        PluginRequirementSet,
    };
    use crate::persistence::resources::{self, ResourceEntry, ResourcesFile};
    use crate::plugins::remote_desktop::constants::{
        ABILITY_ATTACH_SESSION, ABILITY_CREATE_SESSION, REASON_PREVIEW_CAPTURE_FAILED,
        REASON_PREVIEW_CLIENT_CLOSED, TRANSPORT_INVOKE_BIDI,
    };
    use crate::plugins::remote_desktop::runtime::RemoteDesktopPlugin;
    use crate::plugins::remote_desktop::session::RemoteDesktopState;
    use crate::plugins::remote_desktop::test_support::{
        env_for, seed_display, test_consent_causal_context, test_lock, test_runtime_limits,
    };
    use crate::runtime::ability_dispatch::AxonAbilityCatalog;
    use crate::runtime::invocation_target::{CallMode, InvocationTarget, TargetScope};

    #[derive(Debug)]
    struct FailingScreenBackend;

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

    fn registry_with_screen_backend(backend: Arc<dyn ScreenSnapshotBackend>) -> AxonAbilityCatalog {
        let limits = test_runtime_limits();
        let mut builder = PluginContributionBuilder::new(
            "easynet.remote_desktop",
            "0.1.0",
            PluginKind::Builtin,
            limits,
            PluginRequirementSet::default(),
            Vec::new(),
        );
        crate::plugins::remote_desktop::registration::contribute_with_screen_backend(
            &mut builder,
            backend,
            limits,
        )
        .expect("remote desktop contribution");
        let contribution = builder
            .finish()
            .expect("remote desktop contribution finish");
        let contributions = PluginContributionSet::new(vec![contribution]);
        let mut reg = AxonAbilityCatalog::new();
        DaemonPluginBinder::static_catalog(&mut reg)
            .bind_set(&contributions)
            .expect("bind remote desktop contribution");
        reg
    }

    #[test]
    fn attach_bidi_emits_synthetic_frame_and_closes_on_request() {
        let _lock = test_lock();
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let _g = crate::cli::test_support::HomeGuard::new();
            let mut file = ResourcesFile::default();
            let ura = seed_display(&mut file, "remote-desktop-bidi-display");
            resources::save(&file).unwrap();

            let reg = registry_with_screen_backend(Arc::new(SyntheticScreenBackend));
            let dispatcher = Arc::new(reg);
            let create_target = InvocationTarget {
                scope: TargetScope::Local,
                ability: ABILITY_CREATE_SESSION.to_string(),
                normalized_args: json!({
                    "session_id": "rd-bidi-test",
                    "mode": "view_only",
                    "lease_ttl_ms": 5000,
                    "video": { "max_width": 320, "max_height": 180, "max_fps": 144 },
                }),
                call_mode: CallMode::Rpc,
                subject: Some(ura.clone()),
                causal_context: Some(test_consent_causal_context()),
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
                subject: Some(ura),
                causal_context: Some(test_consent_causal_context()),
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
            let _g = crate::cli::test_support::HomeGuard::new();
            let plugin = RemoteDesktopPlugin::new(
                Arc::new(SyntheticScreenBackend),
                test_runtime_limits().into(),
            );
            let mut file = ResourcesFile::default();
            let ura = seed_display(&mut file, "remote-desktop-close-detach-display");
            resources::save(&file).unwrap();
            let created = crate::plugins::remote_desktop::handlers::create_session::handle(
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

    #[test]
    fn attach_bidi_capture_failure_marks_session_failed() {
        let _lock = test_lock();
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let _g = crate::cli::test_support::HomeGuard::new();
            let plugin = RemoteDesktopPlugin::new(
                Arc::new(FailingScreenBackend),
                test_runtime_limits().into(),
            );
            let mut file = ResourcesFile::default();
            let ura = seed_display(&mut file, "remote-desktop-capture-failure-display");
            resources::save(&file).unwrap();
            let created = crate::plugins::remote_desktop::handlers::create_session::handle(
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
            assert!(
                tokio::time::timeout(Duration::from_millis(100), bidi.from_client.recv())
                    .await
                    .is_err(),
                "terminal error must not be followed by a duplicate closed frame"
            );

            plugin.session_store().with_sessions(|sessions| {
                let session = sessions.get("rd-capture-failed").unwrap();
                assert!(!session.preview_attached());
                assert_eq!(session.state(), RemoteDesktopState::Failed);
                assert_eq!(session.end_reason(), Some(REASON_PREVIEW_CAPTURE_FAILED));
                assert!(session.events().iter().any(|event| {
                    event["event_type"] == json!("SESSION_FAILED")
                        && event["payload"]["transport_kind"] == json!(TRANSPORT_INVOKE_BIDI)
                        && event["payload"]["reason"] == json!(REASON_PREVIEW_CAPTURE_FAILED)
                }));
            });
        });
    }

    #[test]
    fn attach_bidi_rejects_subject_mismatch() {
        let _lock = test_lock();
        let _g = crate::cli::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let ura = seed_display(&mut file, "remote-desktop-bidi-display");
        let other_ura = seed_display(&mut file, "remote-desktop-bidi-other-display");
        resources::save(&file).unwrap();

        let reg = registry_with_screen_backend(Arc::new(SyntheticScreenBackend));
        let dispatcher = Arc::new(reg);
        let created = dispatcher
            .execute_rpc(InvocationTarget {
                scope: TargetScope::Local,
                ability: ABILITY_CREATE_SESSION.to_string(),
                normalized_args: json!({
                    "session_id": "rd-bidi-subject-mismatch",
                    "mode": "view_only",
                    "lease_ttl_ms": 5000,
                    "video": { "max_width": 320, "max_height": 180, "max_fps": 144 },
                }),
                call_mode: CallMode::Rpc,
                subject: Some(ura),
                causal_context: Some(test_consent_causal_context()),
            })
            .unwrap();
        let token = created["session_token"].as_str().unwrap().to_string();

        let err = dispatcher
            .execute_bidi(InvocationTarget {
                scope: TargetScope::Local,
                ability: ABILITY_ATTACH_SESSION.to_string(),
                normalized_args: json!({
                    "session_id": "rd-bidi-subject-mismatch",
                    "session_token": token,
                }),
                call_mode: CallMode::Bidi,
                subject: Some(other_ura),
                causal_context: Some(test_consent_causal_context()),
            })
            .unwrap_err();
        assert!(err.to_string().contains("does not match session subject"));
    }

    #[test]
    fn attach_bidi_requires_resource_subject() {
        let _lock = test_lock();
        let _g = crate::cli::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let ura = seed_display(&mut file, "remote-desktop-bidi-missing-subject-display");
        resources::save(&file).unwrap();

        let reg = registry_with_screen_backend(Arc::new(SyntheticScreenBackend));
        let dispatcher = Arc::new(reg);
        let created = dispatcher
            .execute_rpc(InvocationTarget {
                scope: TargetScope::Local,
                ability: ABILITY_CREATE_SESSION.to_string(),
                normalized_args: json!({
                    "session_id": "rd-bidi-missing-subject",
                    "mode": "view_only",
                    "lease_ttl_ms": 5000,
                    "video": { "max_width": 320, "max_height": 180, "max_fps": 144 },
                }),
                call_mode: CallMode::Rpc,
                subject: Some(ura),
                causal_context: Some(test_consent_causal_context()),
            })
            .unwrap();
        let token = created["session_token"].as_str().unwrap().to_string();

        let err = dispatcher
            .execute_bidi(InvocationTarget {
                scope: TargetScope::Local,
                ability: ABILITY_ATTACH_SESSION.to_string(),
                normalized_args: json!({
                    "session_id": "rd-bidi-missing-subject",
                    "session_token": token,
                }),
                call_mode: CallMode::Bidi,
                subject: None,
                causal_context: Some(test_consent_causal_context()),
            })
            .unwrap_err();
        assert!(err.to_string().contains("envelope subject is required"));
    }
}
