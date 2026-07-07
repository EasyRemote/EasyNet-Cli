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
use crate::daemon::persistence::resources::ResourceEntry;
use crate::daemon::plugins::remote_desktop::constants::{
    ABILITY_ATTACH_SESSION, REASON_PREVIEW_CAPTURE_FAILED, REASON_PREVIEW_CLIENT_CLOSED,
    REASON_RESOURCE_UNAVAILABLE, TRANSPORT_INVOKE_BIDI,
};
use crate::daemon::plugins::remote_desktop::input::{
    apply_input_frame_with_policy, input_policy_allows, input_policy_for_entry, parse_input_frame,
};
use crate::daemon::plugins::remote_desktop::media::encode::{
    spawn_builtin_h264_stream, BuiltinH264StreamTerminal, BuiltinH264TerminalCallback,
};
use crate::daemon::plugins::remote_desktop::request::AttachEncoding;
use crate::daemon::plugins::remote_desktop::session_store::RemoteDesktopSessionStore;
use crate::daemon::plugins::remote_desktop::transport::BidiTerminalGuard;

pub(in crate::daemon::plugins::remote_desktop) struct BidiCaptureWorkerConfig {
    pub(in crate::daemon::plugins::remote_desktop) session_store: Arc<RemoteDesktopSessionStore>,
    pub(in crate::daemon::plugins::remote_desktop) session_id: String,
    pub(in crate::daemon::plugins::remote_desktop) backend: Arc<dyn ScreenSnapshotBackend>,
    pub(in crate::daemon::plugins::remote_desktop) entry: ResourceEntry,
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
    let input_policy = input_policy_for_entry(config.input_policy, &config.entry);
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
        entry: config.entry,
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
    if !input_policy_allows(input_policy, kind) {
        return json!({
            "type": "warn",
            "code": "input_disabled",
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
    entry: ResourceEntry,
    options: ScreenCaptureOptions,
) -> anyhow::Result<EncodedFrame> {
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
            match frame.get("type").and_then(Value::as_str).unwrap_or("") {
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

struct BidiFrameLoopConfig {
    session_store: Arc<RemoteDesktopSessionStore>,
    session_id: String,
    backend: Arc<dyn ScreenSnapshotBackend>,
    entry: ResourceEntry,
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
        entry,
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
            entry.clone(),
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
                "message": "Diagnostic fallback sends metadata JSON followed by raw binary frame chunks; production remote desktop still requires WebRTC/hardware video encoding.",
            })))
            .await;
        loop {
            if *stop_rx.borrow() {
                break;
            }
            let started = Instant::now();
            let capture =
                capture_bidi_frame(Arc::clone(&backend), entry.clone(), options.clone()).await;
            match capture {
                Ok(frame) => {
                    let frame = build_bidi_frames(seq, &entry.hardware_id, frame);
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
        let mut session = crate::daemon::plugins::remote_desktop::session::RemoteDesktopSession::new(
            crate::daemon::plugins::remote_desktop::session::RemoteDesktopSessionInit {
                session_id: session_id.to_string(),
                session_token: "token".to_string(),
                creator_caller_ura: Some("easynet:///r/acme/user/test-caller".to_string()),
                consent: crate::daemon::plugins::remote_desktop::session_consent::RemoteDesktopConsentGrant::from_envelope_for_test(
                    &crate::daemon::ability::dispatch::EnvelopeContext::for_test(
                        "easynet:///r/acme/user/test-caller",
                        "easynet:///r/acme/resource/display.01",
                    ),
                ),
                subject_ura: "easynet:///r/acme/resource/display.01".to_string(),
                subject_type: crate::daemon::persistence::resources::ResourceType::Display,
                subject_display_name: "Test Display".to_string(),
                mode: "view_only".to_string(),
                lease_ttl_ms: 5_000,
                transport_preferences: vec![TRANSPORT_INVOKE_BIDI.to_string()],
                video:
                    crate::daemon::plugins::remote_desktop::request::RemoteDesktopVideoConstraints::default(
                    ),
                input_policy:
                    crate::daemon::plugins::remote_desktop::request::RemoteDesktopInputPolicy::default(),
            },
        );
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
    fn h264_terminal_failure_marks_preview_session_failed() {
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
            assert_eq!(
                session.end_reason(),
                Some(REASON_PREVIEW_CAPTURE_FAILED),
                "H.264 worker failure must be projected into session terminal state"
            );
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
        assert_eq!(response["code"], json!("clipboard_injection_not_enabled"));
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
}
