// EasyNet CLI — remote desktop InvokeBidi transport
// =================================================
//
// File: plugins/remote-desktop/src/invoke_bidi.rs
// Description: Invocation-scoped Bidi worker for diagnostic remote desktop attach.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::sync::{mpsc, watch};

use crate::daemon::ability::builtins::resources::media::screen_snapshot::{
    EncodedFrame, ScreenCaptureOptions, ScreenSnapshotBackend,
};
use crate::daemon::ability::dispatch::BidiOutputFrame;
use crate::daemon::plugins::remote_desktop::constants::{
    ABILITY_ATTACH_SESSION, REASON_PREVIEW_CAPTURE_FAILED, REASON_PREVIEW_CLIENT_CLOSED,
    REASON_RESOURCE_UNAVAILABLE, TRANSPORT_INVOKE_BIDI,
};
use crate::daemon::plugins::remote_desktop::input::{
    apply_input_frame_with_policy, input_policy_for_binding, input_policy_reject_reason,
    parse_input_frame, unsupported_input_channel_reason,
};
use crate::daemon::plugins::remote_desktop::media::encode::{
    BuiltinH264StreamTerminal, BuiltinH264TerminalCallback, spawn_builtin_h264_stream,
};
use crate::daemon::plugins::remote_desktop::request::AttachEncoding;
use crate::daemon::plugins::remote_desktop::session_store::RemoteDesktopSessionStore;
use crate::daemon::plugins::remote_desktop::target::{
    DiagnosticCaptureSubject, RemoteAppTargetBinding,
};
use crate::daemon::plugins::remote_desktop::transport::BidiTerminalGuard;

pub(in crate::daemon::plugins::remote_desktop) struct BidiCaptureWorkerConfig {
    pub(in crate::daemon::plugins::remote_desktop) session_store: Arc<RemoteDesktopSessionStore>,
    pub(in crate::daemon::plugins::remote_desktop) session_id: String,
    pub(in crate::daemon::plugins::remote_desktop) backend: Arc<dyn ScreenSnapshotBackend>,
    pub(in crate::daemon::plugins::remote_desktop) target_binding: RemoteAppTargetBinding,
    pub(in crate::daemon::plugins::remote_desktop) options: ScreenCaptureOptions,
    pub(in crate::daemon::plugins::remote_desktop) encoding: AttachEncoding,
    pub(in crate::daemon::plugins::remote_desktop) input_policy: Value,
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
    let input_policy = input_policy_for_binding(config.input_policy, &config.target_binding);
    let capture_subject = config.target_binding.diagnostic_capture_subject().clone();
    let target_binding = config.target_binding;
    spawn_bidi_control_loop(
        config.from_client,
        config.to_client.clone(),
        input_policy,
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
        capture_subject,
        options: config.options,
        encoding: config.encoding,
        latest_frame: latest_frame_tx,
        control_to_client: config.to_client,
        stop_rx: config.stop_rx,
        max_frame_queue_depth: config.max_frame_queue_depth,
    });
}

pub(in crate::daemon::plugins::remote_desktop) fn handle_bidi_input_frame(
    input_policy: &Value,
    frame: Value,
) -> Value {
    let text = match serde_json::to_string(&frame) {
        Ok(text) => text,
        Err(err) => {
            return json!({
                "type": "warn",
                "code": "invalid_input_frame",
                "message": err.to_string(),
            });
        }
    };
    let frame = match parse_input_frame(&text) {
        Ok(frame) => frame,
        Err(err) => {
            return json!({
                "type": "warn",
                "code": "invalid_input_frame",
                "message": err.to_string(),
            });
        }
    };
    let kind = frame.kind().as_policy_key();
    if let Some(reason) = unsupported_input_channel_reason(kind) {
        return json!({
            "type": "warn",
            "code": reason,
            "input_type": kind,
            "action": frame.action(),
            "message": "clipboard and file-drop frames require dedicated remote desktop abilities",
        });
    }
    if let Some(reason) = input_policy_reject_reason(input_policy, kind) {
        let code = if reason == "input_policy_denied" {
            "input_disabled"
        } else {
            reason
        };
        return json!({
            "type": "warn",
            "code": code,
            "input_type": kind,
            "action": frame.action(),
            "message": "interactive input is disabled by this remote desktop session policy",
        });
    }
    let outcome = apply_input_frame_with_policy(input_policy, &frame);
    if outcome.applied {
        json!({
            "type": "input_applied",
            "input_type": kind,
            "action": frame.action(),
        })
    } else {
        json!({
            "type": "warn",
            "code": outcome.reason.unwrap_or("input_injection_failed"),
            "input_type": kind,
            "action": frame.action(),
        })
    }
}

async fn capture_bidi_frame(
    backend: Arc<dyn ScreenSnapshotBackend>,
    capture_subject: DiagnosticCaptureSubject,
    options: ScreenCaptureOptions,
) -> anyhow::Result<EncodedFrame> {
    let entry = capture_subject.to_backend_resource_entry();
    tokio::task::spawn_blocking(move || backend.capture_jpeg(&entry, &options))
        .await
        .map_err(|err| anyhow::anyhow!("{ABILITY_ATTACH_SESSION}: capture task failed: {err}"))?
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
    mut from_client: mpsc::Receiver<Value>,
    to_client: mpsc::Sender<BidiOutputFrame>,
    input_policy: Value,
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
                        .send(BidiOutputFrame::json(handle_bidi_input_frame(
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
    capture_subject: DiagnosticCaptureSubject,
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
        capture_subject,
        options,
        encoding,
        latest_frame,
        control_to_client,
        mut stop_rx,
        max_frame_queue_depth,
    } = config;
    let terminal_guard = BidiTerminalGuard::new();
    if encoding == AttachEncoding::AnnexBH264
        && spawn_builtin_h264_stream(
            target_binding,
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
                capture_subject.clone(),
                options.clone(),
            )
            .await;
            match capture {
                Ok(frame) => {
                    let frame = build_bidi_frames(seq, capture_subject.hardware_id(), frame);
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
    session_store.with_sessions(|sessions| {
        let init = crate::daemon::plugins::remote_desktop::test_support::test_session_init(
            session_id,
            "easynet:///r/acme/resource/display.01",
            vec![TRANSPORT_INVOKE_BIDI.to_string()],
        );
        let mut session =
            crate::daemon::plugins::remote_desktop::session::RemoteDesktopSession::new(init);
        session.attach_preview_transport(stop_tx);
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
            json!({"type": "pointer", "action": "move", "x": 10, "y": 20}),
        );

        assert_eq!(response["type"], json!("warn"));
        assert_eq!(response["code"], json!("input_disabled"));
        assert_eq!(response["input_type"], json!("pointer"));
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
