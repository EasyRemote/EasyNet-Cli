// EasyNet CLI — remote desktop InvokeBidi transport
// =================================================
//
// File: plugins/remote-desktop/src/invoke_bidi.rs
// Description: Invocation-scoped Bidi worker for diagnostic remote desktop attach.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::sync::{mpsc, watch};

use crate::daemon::ability::builtins::resources::media::screen_snapshot::{
    EncodedFrame, ScreenCaptureOptions, ScreenSnapshotBackend,
};
use crate::daemon::ability::dispatch::BidiOutputFrame;
use crate::daemon::plugins::remote_desktop::constants::{
    ABILITY_ATTACH_SESSION, REASON_PREVIEW_CAPTURE_FAILED, REASON_PREVIEW_CLIENT_CLOSED,
    REASON_RESOURCE_UNAVAILABLE, TRANSPORT_INVOKE_BIDI,
};
#[cfg(test)]
use crate::daemon::plugins::remote_desktop::input::RemoteDesktopInputPolicy;
use crate::daemon::plugins::remote_desktop::input::{
    apply_input_frame_with_effective_policy, current_session_effective_input_policy,
    parse_input_frame, unsupported_input_channel_reason, EffectiveRemoteDesktopInputPolicy,
    InputTransportGuard, RemoteDesktopInputFrame,
};
use crate::daemon::plugins::remote_desktop::media::encode::{
    spawn_builtin_h264_stream, BuiltinH264StreamTerminal, BuiltinH264TerminalCallback,
};
use crate::daemon::plugins::remote_desktop::request::AttachEncoding;
use crate::daemon::plugins::remote_desktop::session_store::RemoteDesktopSessionStore;
use crate::daemon::plugins::remote_desktop::target::{
    RemoteAppTargetBinding, RemoteDesktopTargetKind,
};
use crate::daemon::plugins::remote_desktop::transport::BidiTerminalGuard;

pub(in crate::daemon::plugins::remote_desktop) struct BidiCaptureWorkerConfig {
    pub(in crate::daemon::plugins::remote_desktop) session_store: Arc<RemoteDesktopSessionStore>,
    pub(in crate::daemon::plugins::remote_desktop) session_id: String,
    pub(in crate::daemon::plugins::remote_desktop) backend: Arc<dyn ScreenSnapshotBackend>,
    pub(in crate::daemon::plugins::remote_desktop) target_binding: RemoteAppTargetBinding,
    pub(in crate::daemon::plugins::remote_desktop) options: ScreenCaptureOptions,
    pub(in crate::daemon::plugins::remote_desktop) encoding: AttachEncoding,
    pub(in crate::daemon::plugins::remote_desktop) input_policy: EffectiveRemoteDesktopInputPolicy,
    pub(in crate::daemon::plugins::remote_desktop) from_client: mpsc::Receiver<Value>,
    pub(in crate::daemon::plugins::remote_desktop) to_client: mpsc::Sender<BidiOutputFrame>,
    pub(in crate::daemon::plugins::remote_desktop) stop_tx: watch::Sender<bool>,
    pub(in crate::daemon::plugins::remote_desktop) stop_rx: watch::Receiver<bool>,
    pub(in crate::daemon::plugins::remote_desktop) max_frame_queue_depth: usize,
}

pub(in crate::daemon::plugins::remote_desktop) fn spawn_bidi_capture_worker(
    config: BidiCaptureWorkerConfig,
) {
    let (latest_frame_tx, latest_frame_rx) = watch::channel::<Option<Vec<BidiOutputFrame>>>(None);
    let target_binding = config.target_binding;
    spawn_bidi_control_loop(
        Arc::clone(&config.session_store),
        config.session_id.clone(),
        config.from_client,
        config.to_client.clone(),
        config.input_policy,
        config.stop_tx,
    );
    spawn_latest_frame_forwarder(
        latest_frame_rx,
        config.to_client.clone(),
        config.stop_rx.clone(),
    );
    spawn_bidi_frame_loop(BidiFrameLoopConfig {
        session_store: config.session_store,
        session_id: config.session_id,
        backend: config.backend,
        target_binding,
        options: config.options,
        encoding: config.encoding,
        latest_frame: latest_frame_tx,
        control_to_client: config.to_client,
        stop_rx: config.stop_rx,
        max_frame_queue_depth: config.max_frame_queue_depth,
    });
}

#[cfg(test)]
pub(in crate::daemon::plugins::remote_desktop) fn handle_bidi_input_frame(
    input_policy: &Value,
    frame: Value,
) -> Value {
    let frame = match parse_bidi_input_frame(frame) {
        Ok(frame) => frame,
        Err(err) => return err,
    };
    handle_parsed_bidi_input_frame(input_policy, &frame)
}

fn parse_bidi_input_frame(frame: Value) -> Result<RemoteDesktopInputFrame, Value> {
    if let Some(frame_type) = frame.get("type").and_then(Value::as_str) {
        if let Some(reason) = unsupported_input_channel_reason(frame_type) {
            return Err(json!({
                "type": "warn",
                "code": reason,
                "input_type": frame_type,
                "message": "clipboard and file-drop frames require dedicated remote desktop abilities",
            }));
        }
    }
    let text = match serde_json::to_string(&frame) {
        Ok(text) => text,
        Err(err) => {
            return Err(json!({
                "type": "warn",
                "code": "invalid_input_frame",
                "message": err.to_string(),
            }));
        }
    };
    match parse_input_frame(&text) {
        Ok(frame) => Ok(frame),
        Err(err) => Err(json!({
            "type": "warn",
            "code": "invalid_input_frame",
            "message": err.to_string(),
        })),
    }
}

#[cfg(test)]
fn handle_parsed_bidi_input_frame(input_policy: &Value, frame: &RemoteDesktopInputFrame) -> Value {
    let effective_policy = EffectiveRemoteDesktopInputPolicy::from_test_value(input_policy.clone());
    handle_parsed_bidi_input_frame_with_policy(&effective_policy, frame)
}

fn handle_parsed_bidi_input_frame_with_policy(
    input_policy: &EffectiveRemoteDesktopInputPolicy,
    frame: &RemoteDesktopInputFrame,
) -> Value {
    let kind = frame.kind().as_policy_key();
    let outcome = apply_input_frame_with_effective_policy(input_policy, frame);
    if outcome.applied {
        return attach_input_frame_telemetry(
            json!({
                "type": "input_applied",
                "input_type": kind,
                "action": frame.action(),
            }),
            frame,
        );
    }
    let reason = outcome.reason.unwrap_or("input_injection_failed");
    let code = if reason == "input_policy_denied" {
        "input_disabled"
    } else {
        reason
    };
    let message = if unsupported_input_channel_reason(kind) == Some(reason) {
        "clipboard and file-drop frames require dedicated remote desktop abilities"
    } else {
        "interactive input is disabled by this remote desktop session policy"
    };
    attach_input_frame_telemetry(
        json!({
            "type": "warn",
            "code": code,
            "input_type": kind,
            "action": frame.action(),
            "message": message,
        }),
        frame,
    )
}

pub(in crate::daemon::plugins::remote_desktop) fn handle_bidi_input_frame_for_session(
    session_store: &RemoteDesktopSessionStore,
    session_id: &str,
    input_policy: &EffectiveRemoteDesktopInputPolicy,
    frame: Value,
) -> Value {
    let frame = match parse_bidi_input_frame(frame) {
        Ok(frame) => frame,
        Err(err) => return err,
    };
    let kind = frame.kind().as_policy_key();
    let Some(effective_input_policy) = current_session_effective_input_policy(
        session_store,
        session_id,
        InputTransportGuard::DiagnosticPreview,
        input_policy,
    ) else {
        return attach_input_frame_telemetry(
            json!({
                "type": "warn",
                "code": "target_input_not_ready",
                "input_type": kind,
                "action": frame.action(),
                "message": "interactive input is disabled because the target is not ready for this diagnostic preview session",
            }),
            &frame,
        );
    };
    handle_parsed_bidi_input_frame_with_policy(&effective_input_policy, &frame)
}

fn attach_input_frame_telemetry(mut payload: Value, frame: &RemoteDesktopInputFrame) -> Value {
    let Some(map) = payload.as_object_mut() else {
        return payload;
    };
    if let Some(client_sent_at_ms) = frame.client_sent_at_ms() {
        map.insert("client_sent_at_ms".to_string(), json!(client_sent_at_ms));
    }
    if let Some(client_sequence) = frame.client_sequence() {
        map.insert("client_sequence".to_string(), json!(client_sequence));
    }
    payload
}

async fn capture_bidi_frame(
    backend: Arc<dyn ScreenSnapshotBackend>,
    target_binding: RemoteAppTargetBinding,
    options: ScreenCaptureOptions,
) -> anyhow::Result<EncodedFrame> {
    tokio::task::spawn_blocking(move || {
        capture_binding_diagnostic_jpeg(backend, &target_binding, &options)
    })
    .await
    .map_err(|err| anyhow::anyhow!("{ABILITY_ATTACH_SESSION}: capture task failed: {err}"))?
}

fn capture_binding_diagnostic_jpeg(
    backend: Arc<dyn ScreenSnapshotBackend>,
    target_binding: &RemoteAppTargetBinding,
    options: &ScreenCaptureOptions,
) -> anyhow::Result<EncodedFrame> {
    match target_binding.target_kind() {
        RemoteDesktopTargetKind::Display => {
            let entry = target_binding
                .diagnostic_capture_subject()
                .to_backend_resource_entry();
            backend.capture_jpeg(&entry, options)
        }
        RemoteDesktopTargetKind::Window | RemoteDesktopTargetKind::Application => {
            capture_native_binding_diagnostic_jpeg(target_binding, options)
        }
    }
}

#[cfg(target_os = "macos")]
fn capture_native_binding_diagnostic_jpeg(
    target_binding: &RemoteAppTargetBinding,
    options: &ScreenCaptureOptions,
) -> anyhow::Result<EncodedFrame> {
    crate::daemon::plugins::remote_desktop::screencapturekit_capture::capture_jpeg_for_binding(
        ABILITY_ATTACH_SESSION,
        target_binding,
        options,
    )
}

#[cfg(not(target_os = "macos"))]
fn capture_native_binding_diagnostic_jpeg(
    target_binding: &RemoteAppTargetBinding,
    _options: &ScreenCaptureOptions,
) -> anyhow::Result<EncodedFrame> {
    Err(crate::daemon::plugins::remote_desktop::target::RemoteAppTargetError::new(
        ABILITY_ATTACH_SESSION,
        crate::daemon::plugins::remote_desktop::target::TargetResolutionError::CaptureBackendUnavailable,
        format!(
            "diagnostic InvokeBidi preview for {} requires a binding-backed native capture adapter; display fallback is forbidden",
            target_binding.target_kind().as_str()
        ),
    )
    .into())
}

fn build_bidi_frames(seq: u64, hardware_id: &str, frame: EncodedFrame) -> Vec<BidiOutputFrame> {
    let byte_size = frame.jpeg_bytes.len();
    let metadata = json!({
        "type": "frame",
        "transport": TRANSPORT_INVOKE_BIDI,
        "seq": seq,
        "content_type": "image/jpeg",
        "encoding": "binary",
        "width": frame.width,
        "height": frame.height,
        "byte_size": byte_size,
        "captured_at": chrono::Utc::now().to_rfc3339(),
        "hardware_id": hardware_id,
    });
    vec![
        BidiOutputFrame::json(metadata),
        BidiOutputFrame::binary(frame.jpeg_bytes, "image/jpeg"),
    ]
}

fn spawn_bidi_control_loop(
    session_store: Arc<RemoteDesktopSessionStore>,
    session_id: String,
    mut from_client: mpsc::Receiver<Value>,
    to_client: mpsc::Sender<BidiOutputFrame>,
    input_policy: EffectiveRemoteDesktopInputPolicy,
    stop_tx: watch::Sender<bool>,
) {
    tokio::spawn(async move {
        while let Some(frame) = from_client.recv().await {
            let frame_type = match bidi_control_frame_type(&frame) {
                Ok(frame_type) => frame_type,
                Err(warn) => {
                    let _ = to_client.send(BidiOutputFrame::json(warn)).await;
                    continue;
                }
            };
            match frame_type {
                "close" => {
                    let _ = stop_tx.send(true);
                    break;
                }
                "ping" => {
                    let _ = to_client
                        .send(BidiOutputFrame::json(json!({
                            "type": "pong",
                            "at_ms": crate::daemon::plugins::remote_desktop::session::now_ms(),
                        })))
                        .await;
                }
                "key" | "pointer" | "clipboard" | "file_drop" => {
                    let _ = to_client
                        .send(BidiOutputFrame::json(handle_bidi_input_frame_for_session(
                            &session_store,
                            &session_id,
                            &input_policy,
                            frame,
                        )))
                        .await;
                }
                other => {
                    let _ = to_client
                        .send(BidiOutputFrame::json(json!({
                            "type": "warn",
                            "code": "unknown_frame",
                            "message": format!("unknown remote desktop bidi frame type {other:?}"),
                        })))
                        .await;
                }
            }
        }
        let _ = stop_tx.send(true);
    });
}

fn bidi_control_frame_type(frame: &Value) -> Result<&str, Value> {
    match frame.get("type") {
        Some(Value::String(frame_type)) if !frame_type.trim().is_empty() => Ok(frame_type),
        Some(Value::String(_)) => Err(json!({
            "type": "warn",
            "code": "invalid_frame",
            "message": "remote desktop bidi frame type must be a non-empty string",
        })),
        Some(_) => Err(json!({
            "type": "warn",
            "code": "invalid_frame",
            "message": "remote desktop bidi frame type must be a string",
        })),
        None => Err(json!({
            "type": "warn",
            "code": "invalid_frame",
            "message": "remote desktop bidi frame type is required",
        })),
    }
}

struct BidiFrameLoopConfig {
    session_store: Arc<RemoteDesktopSessionStore>,
    session_id: String,
    backend: Arc<dyn ScreenSnapshotBackend>,
    target_binding: RemoteAppTargetBinding,
    options: ScreenCaptureOptions,
    encoding: AttachEncoding,
    latest_frame: watch::Sender<Option<Vec<BidiOutputFrame>>>,
    control_to_client: mpsc::Sender<BidiOutputFrame>,
    stop_rx: watch::Receiver<bool>,
    max_frame_queue_depth: usize,
}

fn spawn_bidi_frame_loop(config: BidiFrameLoopConfig) {
    let BidiFrameLoopConfig {
        session_store,
        session_id,
        backend,
        target_binding,
        options,
        encoding,
        latest_frame,
        control_to_client,
        mut stop_rx,
        max_frame_queue_depth,
    } = config;
    let terminal_guard = BidiTerminalGuard::new();
    let hardware_id = target_binding
        .diagnostic_capture_subject()
        .hardware_id()
        .to_string();
    if encoding == AttachEncoding::AnnexBH264
        && spawn_builtin_h264_stream(
            target_binding.clone(),
            options.clone(),
            max_frame_queue_depth,
            control_to_client.clone(),
            stop_rx.clone(),
            terminal_guard.clone(),
            h264_terminal_callback(Arc::clone(&session_store), session_id.clone()),
        )
    {
        return;
    }
    tokio::spawn(async move {
        let interval = Duration::from_secs_f64(1.0 / options.fps as f64);
        let mut seq = 0_u64;
        let _ = control_to_client
            .send(BidiOutputFrame::json(json!({
                "type": "transport",
                "transport": TRANSPORT_INVOKE_BIDI,
                "state": "connected",
                "encoding": "metadata_json_plus_binary",
                "message": "Diagnostic transport sends metadata JSON followed by raw binary frame chunks; production remote desktop still requires WebRTC/hardware video encoding.",
            })))
            .await;
        loop {
            if *stop_rx.borrow() {
                break;
            }
            let started = Instant::now();
            let capture = capture_bidi_frame(
                Arc::clone(&backend),
                target_binding.clone(),
                options.clone(),
            )
            .await;
            match capture {
                Ok(frame) => {
                    let frame = build_bidi_frames(seq, &hardware_id, frame);
                    seq = seq.saturating_add(1);
                    if latest_frame.send(Some(frame)).is_err() {
                        break;
                    }
                }
                Err(err) => {
                    let message = err.to_string();
                    session_store.mark_preview_transport_failed(
                        &session_id,
                        REASON_PREVIEW_CAPTURE_FAILED,
                        message.clone(),
                    );
                    terminal_guard
                        .send_error(&control_to_client, REASON_RESOURCE_UNAVAILABLE, message)
                        .await;
                    break;
                }
            }
            let remaining = interval.checked_sub(started.elapsed());
            tokio::select! {
                _ = stop_rx.changed() => {
                    if *stop_rx.borrow() {
                        break;
                    }
                }
                _ = tokio::time::sleep(remaining.unwrap_or_default()) => {}
            }
        }
        session_store
            .detach_preview_transport_from_worker(&session_id, REASON_PREVIEW_CLIENT_CLOSED);
        terminal_guard
            .send_closed(&control_to_client, REASON_PREVIEW_CLIENT_CLOSED)
            .await;
    });
}

fn h264_terminal_callback(
    session_store: Arc<RemoteDesktopSessionStore>,
    session_id: String,
) -> BuiltinH264TerminalCallback {
    Arc::new(move |terminal| match terminal {
        BuiltinH264StreamTerminal::Closed(reason) => {
            session_store.detach_preview_transport_from_worker(&session_id, reason);
        }
        BuiltinH264StreamTerminal::Failed { reason, message } => {
            session_store.mark_preview_transport_failed(&session_id, reason, message);
        }
    })
}

#[cfg(test)]
fn apply_h264_terminal_for_test(
    session_store: Arc<RemoteDesktopSessionStore>,
    session_id: String,
    terminal: BuiltinH264StreamTerminal,
) {
    h264_terminal_callback(session_store, session_id)(terminal);
}

#[cfg(test)]
fn install_h264_preview_session_for_test(
    session_store: &RemoteDesktopSessionStore,
    session_id: &str,
) {
    let (stop_tx, _stop_rx) = watch::channel(false);
    let init = crate::daemon::plugins::remote_desktop::test_support::test_session_init(
        session_id,
        "easynet:///r/acme/resource/display.01",
        vec![TRANSPORT_INVOKE_BIDI.to_string()],
    );
    let mut session =
        crate::daemon::plugins::remote_desktop::session::RemoteDesktopSession::new(init);
    session.attach_preview_transport(stop_tx);
    session_store.with_sessions(|sessions| {
        sessions.insert(session_id.to_string(), session);
    });
}

fn spawn_latest_frame_forwarder(
    mut latest_frame: watch::Receiver<Option<Vec<BidiOutputFrame>>>,
    to_client: mpsc::Sender<BidiOutputFrame>,
    mut stop_rx: watch::Receiver<bool>,
) {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                changed = stop_rx.changed() => {
                    if changed.is_err() || *stop_rx.borrow() {
                        break;
                    }
                }
                changed = latest_frame.changed() => {
                    if changed.is_err() {
                        break;
                    }
                }
            }
            let Some(frames) = latest_frame.borrow_and_update().clone() else {
                continue;
            };
            for frame in frames {
                if to_client.send(frame).await.is_err() {
                    return;
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use tokio::sync::broadcast;

    use crate::daemon::persistence::resources::{ResourceBinding, ResourceEntry, ResourceType};
    use crate::daemon::plugins::remote_desktop::target::ResourceEntryTargetResolver;

    #[derive(Debug)]
    struct CountingScreenBackend {
        calls: Arc<AtomicUsize>,
    }

    impl ScreenSnapshotBackend for CountingScreenBackend {
        fn capture_jpeg(
            &self,
            _entry: &ResourceEntry,
            _options: &ScreenCaptureOptions,
        ) -> anyhow::Result<EncodedFrame> {
            self.calls.fetch_add(1, Ordering::SeqCst);
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
            anyhow::bail!("stream not used by diagnostic frame source tests")
        }
    }

    fn display_binding_for_test() -> RemoteAppTargetBinding {
        let entry = ResourceEntry {
            resource_ura: "easynet:///r/acme/resource/device.01DEV/streams/display.test"
                .to_string(),
            owner_agent: "easynet:///r/acme/agent/device.01DEV.media".to_string(),
            kind: ResourceType::Display,
            binding: ResourceBinding::LocalDevice,
            hardware_id: "display:test".to_string(),
            display_name: "Test Display".to_string(),
            metadata: json!({"primary_display": true, "backend": "xcap"}),
            first_seen_at: "2026-01-01T00:00:00Z".to_string(),
        };
        ResourceEntryTargetResolver
            .resolve_for_session(ABILITY_ATTACH_SESSION, &entry, "view_only", 1)
            .expect("display target binding resolves")
    }

    #[cfg(not(target_os = "macos"))]
    fn window_binding_for_test() -> RemoteAppTargetBinding {
        let entry = ResourceEntry {
            resource_ura: "easynet:///r/acme/resource/device.01DEV/streams/window.test".to_string(),
            owner_agent: "easynet:///r/acme/agent/device.01DEV.media".to_string(),
            kind: ResourceType::Window,
            binding: ResourceBinding::LocalDevice,
            hardware_id: "window:macos:cgwindow:10:42".to_string(),
            display_name: "Test Window".to_string(),
            metadata:
                crate::daemon::plugins::remote_desktop::test_support::live_remote_target_metadata(
                    json!({
                        "window_id": 42,
                        "pid": 10,
                        "x": 0,
                        "y": 0,
                        "width": 800,
                        "height": 600,
                    }),
                ),
            first_seen_at: "2026-01-01T00:00:00Z".to_string(),
        };
        ResourceEntryTargetResolver
            .resolve_for_session(ABILITY_ATTACH_SESSION, &entry, "view_only", 1)
            .expect("window target binding resolves")
    }

    #[test]
    fn diagnostic_jpeg_display_capture_uses_explicit_display_backend_adapter() {
        let calls = Arc::new(AtomicUsize::new(0));
        let frame = capture_binding_diagnostic_jpeg(
            Arc::new(CountingScreenBackend {
                calls: Arc::clone(&calls),
            }),
            &display_binding_for_test(),
            &ScreenCaptureOptions::default(),
        )
        .expect("display diagnostic capture uses backend adapter");

        assert_eq!(frame.width, 1);
        assert_eq!(frame.height, 1);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn diagnostic_jpeg_window_capture_does_not_use_resource_entry_backend() {
        let calls = Arc::new(AtomicUsize::new(0));
        let err = capture_binding_diagnostic_jpeg(
            Arc::new(CountingScreenBackend {
                calls: Arc::clone(&calls),
            }),
            &window_binding_for_test(),
            &ScreenCaptureOptions::default(),
        )
        .expect_err("non-macOS window diagnostic capture must fail closed without fallback");

        assert!(err.to_string().contains("capture_backend_unavailable"));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "window diagnostic capture must not route through ResourceEntry backend"
        );
    }

    #[test]
    fn h264_terminal_failure_marks_diagnostic_preview_failed() {
        let session_store = Arc::new(RemoteDesktopSessionStore::new());
        install_h264_preview_session_for_test(&session_store, "rd-h264-failed");

        apply_h264_terminal_for_test(
            Arc::clone(&session_store),
            "rd-h264-failed".to_string(),
            BuiltinH264StreamTerminal::Failed {
                reason: REASON_PREVIEW_CAPTURE_FAILED,
                message: "encoder stopped".to_string(),
            },
        );

        session_store.with_sessions(|sessions| {
            let session = sessions.get("rd-h264-failed").unwrap();
            assert!(!session.preview_attached());
            assert_eq!(session.end_reason(), None);
            assert!(session.events().iter().any(|event| {
                event["event_type"] == json!("DIAGNOSTIC_PREVIEW_FAILED")
                    && event["payload"]["reason"] == json!(REASON_PREVIEW_CAPTURE_FAILED)
            }));
        });
    }

    #[test]
    fn h264_terminal_close_detaches_preview_session() {
        let session_store = Arc::new(RemoteDesktopSessionStore::new());
        install_h264_preview_session_for_test(&session_store, "rd-h264-closed");

        apply_h264_terminal_for_test(
            Arc::clone(&session_store),
            "rd-h264-closed".to_string(),
            BuiltinH264StreamTerminal::Closed(REASON_PREVIEW_CLIENT_CLOSED),
        );

        session_store.with_sessions(|sessions| {
            let session = sessions.get("rd-h264-closed").unwrap();
            assert!(!session.preview_attached());
            assert_eq!(session.end_reason(), None);
        });
    }

    #[test]
    fn diagnostic_bidi_input_uses_real_input_parser_not_placeholder_warning() {
        let response = handle_bidi_input_frame(
            &json!({"clipboard_enabled": true}),
            json!({"type": "clipboard", "text": "hello"}),
        );

        assert_eq!(response["type"], json!("warn"));
        assert_eq!(response["code"], json!("clipboard_input_unsupported"));
        assert_ne!(response["code"], json!("input_not_wired"));
    }

    #[test]
    fn diagnostic_bidi_input_respects_session_policy() {
        let response = handle_bidi_input_frame(
            &json!({"pointer_enabled": false}),
            json!({
                "type": "pointer",
                "action": "move",
                "x": 10,
                "y": 20,
                "sent_at_ms": 1_787_331_000_123_u64,
                "client_sequence": 9_u64,
            }),
        );

        assert_eq!(response["type"], json!("warn"));
        assert_eq!(response["code"], json!("input_disabled"));
        assert_eq!(response["input_type"], json!("pointer"));
        assert_eq!(response["client_sent_at_ms"], json!(1_787_331_000_123_u64));
        assert_eq!(response["client_sequence"], json!(9_u64));
    }

    #[test]
    fn diagnostic_bidi_view_only_input_reports_scope_unsupported() {
        let response = handle_bidi_input_frame(
            &json!({
                "input_scope": "view_only",
                "pointer_enabled": false,
            }),
            json!({"type": "pointer", "action": "move", "x": 10, "y": 20}),
        );

        assert_eq!(response["type"], json!("warn"));
        assert_eq!(response["code"], json!("input_scope_unsupported"));
        assert_eq!(response["input_type"], json!("pointer"));
    }

    #[test]
    fn diagnostic_bidi_view_only_target_loss_preserves_scope_unsupported() {
        let session_store = Arc::new(RemoteDesktopSessionStore::new());
        install_h264_preview_session_for_test(&session_store, "rd-bidi-target-lost");
        session_store.with_sessions(|sessions| {
            let session = sessions.get_mut("rd-bidi-target-lost").unwrap();
            assert!(session
                .record_target_observation(
                    crate::daemon::plugins::remote_desktop::target_tracking::TargetObservation::Lost {
                        reason: crate::daemon::plugins::remote_desktop::target::TargetResolutionError::TargetNotFound,
                        detail: "first lost probe".into(),
                        observed_at_ms: 1,
                    },
                )
                .is_none());
            assert!(session
                .record_target_observation(
                    crate::daemon::plugins::remote_desktop::target_tracking::TargetObservation::Lost {
                        reason: crate::daemon::plugins::remote_desktop::target::TargetResolutionError::TargetNotFound,
                        detail: "debounced lost probe".into(),
                        observed_at_ms: 1_001,
                    },
                )
                .is_none());
            assert!(
                session
                    .record_target_observation(
                        crate::daemon::plugins::remote_desktop::target_tracking::TargetObservation::Lost {
                            reason: crate::daemon::plugins::remote_desktop::target::TargetResolutionError::TargetNotFound,
                            detail: "committed lost probe".into(),
                            observed_at_ms: 1_002,
                        },
                    )
                    .is_none(),
                "diagnostic preview has no production media epoch to stop"
            );
            assert_eq!(
                session.state(),
                crate::daemon::plugins::remote_desktop::session::RemoteDesktopState::Suspended
            );
            assert_eq!(
                session.target_tracking_state()["input_enabled"],
                json!(false)
            );
        });

        let input_policy = EffectiveRemoteDesktopInputPolicy::from_test_value(json!({
            "input_scope": "display_global",
            "pointer_enabled": true,
        }));
        let response = handle_bidi_input_frame_for_session(
            &session_store,
            "rd-bidi-target-lost",
            &input_policy,
            json!({
                "type": "pointer",
                "action": "move",
                "x": 10,
                "y": 20,
                "sent_at_ms": 1_787_331_000_456_u64,
                "client_sequence": 10_u64,
            }),
        );

        assert_eq!(response["type"], json!("warn"));
        assert_eq!(response["code"], json!("input_scope_unsupported"));
        assert_eq!(response["input_type"], json!("pointer"));
        assert_eq!(response["client_sent_at_ms"], json!(1_787_331_000_456_u64));
        assert_eq!(response["client_sequence"], json!(10_u64));
    }

    #[test]
    fn diagnostic_bidi_input_rechecks_session_target_snapshot() {
        let entry = ResourceEntry {
            resource_ura: "easynet:///r/acme/resource/device.01/streams/display.interactive"
                .to_string(),
            owner_agent: "easynet:///r/acme/agent/device.01DEV.media".to_string(),
            kind: ResourceType::Display,
            binding: ResourceBinding::LocalDevice,
            hardware_id: "display:macos:cgdisplay:1".to_string(),
            display_name: "Interactive Display".to_string(),
            metadata: json!({
                "display_id": 1,
                "x": 0,
                "y": 0,
                "width": 1920,
                "height": 1080,
            }),
            first_seen_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let target_binding = ResourceEntryTargetResolver
            .resolve_for_session_with_input_consent(
                ABILITY_ATTACH_SESSION,
                &entry,
                "interactive",
                1,
                true,
            )
            .expect("interactive display binding resolves with input consent");
        let requested_policy = RemoteDesktopInputPolicy::new(true, true);
        let input_policy =
            EffectiveRemoteDesktopInputPolicy::for_binding(&requested_policy, &target_binding);
        assert_eq!(input_policy.input_scope().as_str(), "display_global");

        let session_store = Arc::new(RemoteDesktopSessionStore::new());
        let mut init = crate::daemon::plugins::remote_desktop::test_support::test_session_init(
            "rd-bidi-display-target-lost",
            &entry.resource_ura,
            vec![TRANSPORT_INVOKE_BIDI.to_string()],
        );
        init.mode = "interactive".to_string();
        init.target_binding = target_binding;
        init.input_policy = requested_policy;
        let (stop_tx, _stop_rx) = watch::channel(false);
        let mut session =
            crate::daemon::plugins::remote_desktop::session::RemoteDesktopSession::new(init);
        session.attach_preview_transport(stop_tx);
        session_store.with_sessions(|sessions| {
            sessions.insert("rd-bidi-display-target-lost".to_string(), session);
        });
        session_store.with_sessions(|sessions| {
            let session = sessions.get_mut("rd-bidi-display-target-lost").unwrap();
            assert!(session
                .record_target_observation(
                    crate::daemon::plugins::remote_desktop::target_tracking::TargetObservation::Lost {
                        reason: crate::daemon::plugins::remote_desktop::target::TargetResolutionError::TargetNotFound,
                        detail: "display disconnected".into(),
                        observed_at_ms: 1_002,
                    },
                )
                .is_none());
        });

        let response = handle_bidi_input_frame_for_session(
            &session_store,
            "rd-bidi-display-target-lost",
            &input_policy,
            json!({
                "type": "pointer",
                "action": "move",
                "x": 10,
                "y": 20,
                "sent_at_ms": 1_787_331_000_789_u64,
                "client_sequence": 11_u64,
            }),
        );

        assert_eq!(response["type"], json!("warn"));
        assert_eq!(response["code"], json!("target_input_not_ready"));
        assert_eq!(response["input_type"], json!("pointer"));
        assert_eq!(response["client_sent_at_ms"], json!(1_787_331_000_789_u64));
        assert_eq!(response["client_sequence"], json!(11_u64));
    }

    #[test]
    fn diagnostic_bidi_control_frame_type_fails_closed() {
        for (frame, expected) in [
            (json!({}), "type is required"),
            (json!({"type": 7}), "type must be a string"),
            (json!({"type": ""}), "type must be a non-empty string"),
        ] {
            let response = bidi_control_frame_type(&frame)
                .expect_err("malformed control frame type must fail closed");
            assert_eq!(response["type"], json!("warn"));
            assert_eq!(response["code"], json!("invalid_frame"));
            assert!(
                response["message"]
                    .as_str()
                    .is_some_and(|message| message.contains(expected)),
                "expected {expected:?}; got {response}"
            );
        }
    }
}
