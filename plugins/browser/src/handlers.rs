//! Axon ability adapters for browser plugin operations.
//! =====================================================
//!
//! File: plugins/browser/src/handlers.rs
//! Description: RPC, finite-stream, and InvokeBidi application adapters.
//!
//! Protocol Responsibility:
//! - Consume Axon envelope context and carry CDP only as bounded application
//!   JSON inside canonical InvokeBidi frames.
//!
//! Implementation Approach:
//! - Use a bounded concurrent raw-command lane with shared permits, one FIFO
//!   input lane, bounded batches, and direct channel backpressure.
//!
//! Usage Contract:
//! - Session identity is resolved before entry; frame IDs correlate responses.
//!
//! Architectural Position:
//! - Browser plugin application boundary above session/CDP infrastructure.

use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Map, Value};
use tokio::sync::{broadcast, mpsc, Semaphore};
use tokio::task::JoinSet;

use crate::daemon::ability::dispatch::{
    BidiOutputFrame, BidiSource, EnvelopeContext, StreamSource, BIDI_CHANNEL_BOUND,
};

use super::cdp::{validate_agent_command, CdpEvent, CdpFailure};
use super::constants::*;
use super::errors::{BrowserError, BrowserResult};
use super::input::apply_input;
use super::runtime::BrowserRuntime;
use super::session::{BrowserSession, SessionActivityLease};

pub fn open_session(
    runtime: Arc<BrowserRuntime>,
    env: EnvelopeContext,
    args: Value,
) -> BrowserResult<Value> {
    runtime.open_session(&env, args)
}

pub fn show_session(
    runtime: Arc<BrowserRuntime>,
    env: EnvelopeContext,
    args: Value,
) -> BrowserResult<Value> {
    runtime.show_session(&env, args)
}

pub fn send_input(
    runtime: Arc<BrowserRuntime>,
    env: EnvelopeContext,
    args: Value,
) -> BrowserResult<Value> {
    let event = single_required_field(ABILITY_SEND_INPUT, args, "event")?;
    let session = runtime.require_session(ABILITY_SEND_INPUT, &env)?;
    crate::support::async_bridge::run_blocking(
        apply_input(session, ABILITY_SEND_INPUT, event),
        crate::support::async_bridge::SyncBridgeRuntimePolicy::BuildCurrentThreadTokio,
    )
}

pub fn capture_page(
    runtime: Arc<BrowserRuntime>,
    env: EnvelopeContext,
    args: Value,
) -> BrowserResult<Value> {
    let object = args.as_object();
    if object.is_some_and(|object| !object.is_empty()) {
        return Err(invalid(
            ABILITY_CAPTURE_PAGE,
            "capture_page takes no arguments",
        ));
    }
    let session = runtime.require_session(ABILITY_CAPTURE_PAGE, &env)?;
    crate::support::async_bridge::run_blocking(
        async move {
            let result = session
                .command("Page.captureSnapshot", Some(json!({"format": "mhtml"})))
                .await?;
            let content = result.get("data").and_then(Value::as_str).ok_or_else(|| {
                invalid(
                    ABILITY_CAPTURE_PAGE,
                    "Page.captureSnapshot returned no data",
                )
            })?;
            if content.len() > MAX_PAGE_SNAPSHOT_BYTES {
                return Err(invalid(
                    ABILITY_CAPTURE_PAGE,
                    format!(
                        "page snapshot is {} bytes, above the {} byte bound; narrow the page before capturing",
                        content.len(),
                        MAX_PAGE_SNAPSHOT_BYTES
                    ),
                ));
            }
            Ok(json!({
                "session_ura": session.session_ura(),
                "format": "mhtml",
                "content": content,
                "content_bytes": content.len(),
                "captured_at_ms": super::session::now_ms(),
            }))
        },
        crate::support::async_bridge::SyncBridgeRuntimePolicy::BuildCurrentThreadTokio,
    )
}

pub fn capture_viewport(
    runtime: Arc<BrowserRuntime>,
    env: EnvelopeContext,
    args: Value,
) -> BrowserResult<StreamSource> {
    let request = CaptureRequest::parse(args)?;
    let session = runtime.require_session(ABILITY_CAPTURE_VIEWPORT, &env)?;
    let lease = session.begin_capture()?;
    let mut events = session.client().subscribe();
    let (sender, receiver) = mpsc::channel(runtime.max_frame_queue());
    tokio::spawn(async move {
        run_capture(runtime, session, request, &mut events, sender, lease).await;
    });
    Ok(StreamSource::Finite(receiver))
}

pub fn attach_session(
    runtime: Arc<BrowserRuntime>,
    env: EnvelopeContext,
    args: Value,
) -> BrowserResult<BidiSource> {
    require_empty_args(ABILITY_ATTACH_SESSION, &args)?;
    let session = runtime.require_session(ABILITY_ATTACH_SESSION, &env)?;
    let lease = session.begin_attachment()?;
    let mut events = session.client().subscribe();
    let (to_handler, from_transport) = mpsc::channel::<Value>(BIDI_CHANNEL_BOUND);
    let (from_handler, to_transport) = mpsc::channel::<BidiOutputFrame>(BIDI_CHANNEL_BOUND);
    tokio::spawn(async move {
        run_attachment(
            runtime,
            session,
            from_transport,
            from_handler,
            &mut events,
            lease,
        )
        .await;
    });
    Ok(BidiSource {
        to_client: to_handler,
        from_client: to_transport,
    })
}

pub fn close_session(
    runtime: Arc<BrowserRuntime>,
    env: EnvelopeContext,
    args: Value,
) -> BrowserResult<Value> {
    runtime.close_session(&env, args)
}

async fn run_capture(
    runtime: Arc<BrowserRuntime>,
    session: Arc<BrowserSession>,
    request: CaptureRequest,
    events: &mut broadcast::Receiver<CdpEvent>,
    sender: mpsc::Sender<anyhow::Result<Value>>,
    _lease: SessionActivityLease,
) {
    let start = session
        .raw_command(
            "Page.startScreencast",
            Some(json!({
                "format": request.format,
                "quality": request.quality,
                "maxWidth": request.max_width,
                "maxHeight": request.max_height,
                "everyNthFrame": 1,
            })),
        )
        .await;
    if let Err(error) = start {
        if cdp_failure_requires_close(&error) {
            let _ = runtime
                .close_session_from_runtime(Arc::clone(&session), "capture_start_failed")
                .await;
        }
        let _ = sender.send(Err(anyhow::anyhow!(error.to_string()))).await;
        return;
    }

    let deadline = tokio::time::Instant::now() + Duration::from_secs(request.timeout_seconds);
    let mut emitted = 0_u64;
    let mut terminal_error = None;
    let mut close_reason = None;
    while emitted < request.max_frames {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            terminal_error =
                Some("browser.capture_viewport timed out waiting for a CDP frame".to_string());
            break;
        }
        let event = match tokio::time::timeout(remaining, events.recv()).await {
            Ok(Ok(event)) => event,
            Ok(Err(broadcast::error::RecvError::Lagged(skipped))) => {
                terminal_error = Some(format!(
                    "browser.capture_viewport CDP event queue lagged by {skipped} frame(s)"
                ));
                break;
            }
            Ok(Err(broadcast::error::RecvError::Closed)) => {
                terminal_error = Some("browser.capture_viewport CDP connection closed".to_string());
                close_reason = Some("cdp_connection_closed");
                break;
            }
            Err(_) => {
                terminal_error =
                    Some("browser.capture_viewport timed out waiting for a CDP frame".to_string());
                break;
            }
        };
        if target_detached(&session, &event) {
            terminal_error = Some("Chrome detached the governed page target".to_string());
            close_reason = Some("target_detached");
            break;
        }
        if !session.event_belongs_to_session(&event) || event.method != "Page.screencastFrame" {
            continue;
        }
        let frame_id = event.params.get("sessionId").and_then(Value::as_u64);
        if let Some(frame_id) = frame_id {
            if let Err(error) = session
                .raw_command(
                    "Page.screencastFrameAck",
                    Some(json!({"sessionId": frame_id})),
                )
                .await
            {
                terminal_error = Some(error.to_string());
                if cdp_failure_requires_close(&error) {
                    close_reason = Some("capture_ack_failed");
                }
                break;
            }
        }
        let data = event
            .params
            .get("data")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if data.is_empty() {
            continue;
        }
        emitted += 1;
        let frame = json!({
            "type": "browser.viewport_frame",
            "sequence": emitted,
            "content_type": if request.format == "png" { "image/png" } else { "image/jpeg" },
            "encoding": "base64",
            "data": data,
            "metadata": event.params.get("metadata").cloned().unwrap_or(Value::Null),
        });
        if sender.send(Ok(frame)).await.is_err() {
            break;
        }
    }
    if let Some(reason) = close_reason {
        let _ = runtime
            .close_session_from_runtime(Arc::clone(&session), reason)
            .await;
    } else {
        let _ = session.raw_command("Page.stopScreencast", None).await;
    }
    if let Some(error) = terminal_error {
        let _ = sender.send(Err(anyhow::anyhow!(error))).await;
    }
}

async fn run_attachment(
    runtime: Arc<BrowserRuntime>,
    session: Arc<BrowserSession>,
    mut inbound: mpsc::Receiver<Value>,
    outbound: mpsc::Sender<BidiOutputFrame>,
    events: &mut broadcast::Receiver<CdpEvent>,
    _lease: SessionActivityLease,
) {
    if send_json(
        &outbound,
        json!({
            "type": "browser.ready",
            "session": session.status(),
            "transport": "axon_invoke_bidi",
            "wire": "cdp_json_v1",
        }),
    )
    .await
    .is_err()
    {
        return;
    }

    let mut detached_sent = false;
    let mut concurrent_lane = JoinSet::new();
    let mut input_lane = JoinSet::new();
    let mut queued_input = VecDeque::new();
    let command_permits = Arc::new(Semaphore::new(ATTACH_OPERATION_BOUND));

    // Viewport mirror: protocol invariant — the viewport stream never
    // propagates consumer backpressure into the browser capture producer.
    // Chrome's screencast ack is Chrome-side flow control and is owned
    // HERE: every Page.screencastFrame is acked immediately, decoupled
    // from downstream delivery. Downstream gets latest-wins delivery: a
    // single latest-frame slot plus a dedicated sender task, so a slow
    // consumer skips stale frames instead of accumulating perceptual
    // latency (frame_101..103 pending -> only 103 matters).
    let latest_frame: Arc<std::sync::Mutex<Option<Value>>> = Arc::new(std::sync::Mutex::new(None));
    let frame_notify = Arc::new(tokio::sync::Notify::new());
    // Producer-side pacing (plugin QoS, not consumer backpressure): Chrome
    // keeps exactly one un-acked frame in flight, so the ack cadence IS the
    // capture rate. Acking instantly lets Chrome outrun the downstream
    // pipe; frames then queue FIFO inside bidi/hub/WS and the viewer sees
    // pipeline-depth-old frames — worse perceived latency than the old
    // consumer-driven ack. Pace acks to a target frame interval instead so
    // at most one frame is ever in flight end to end.
    const VIEWPORT_FRAME_INTERVAL: std::time::Duration = std::time::Duration::from_millis(66);
    let pending_ack: Arc<std::sync::Mutex<Option<Value>>> = Arc::new(std::sync::Mutex::new(None));
    let ack_notify = Arc::new(tokio::sync::Notify::new());
    let ack_pacer = {
        let pending_ack = Arc::clone(&pending_ack);
        let ack_notify = Arc::clone(&ack_notify);
        let session = Arc::clone(&session);
        tokio::spawn(async move {
            let mut last_ack = tokio::time::Instant::now() - VIEWPORT_FRAME_INTERVAL;
            loop {
                ack_notify.notified().await;
                loop {
                    let session_id = pending_ack
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .take();
                    let Some(session_id) = session_id else { break };
                    let elapsed = last_ack.elapsed();
                    if elapsed < VIEWPORT_FRAME_INTERVAL {
                        tokio::time::sleep(VIEWPORT_FRAME_INTERVAL - elapsed).await;
                    }
                    last_ack = tokio::time::Instant::now();
                    let _ = session
                        .command(
                            "Page.screencastFrameAck",
                            Some(json!({"sessionId": session_id})),
                        )
                        .await;
                }
            }
        })
    };
    let mirror_sender = {
        let latest_frame = Arc::clone(&latest_frame);
        let frame_notify = Arc::clone(&frame_notify);
        let outbound = outbound.clone();
        tokio::spawn(async move {
            loop {
                frame_notify.notified().await;
                loop {
                    let frame = latest_frame
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .take();
                    let Some(frame) = frame else { break };
                    if send_json(&outbound, frame).await.is_err() {
                        return;
                    }
                }
            }
        })
    };
    let mut render_sequence: u64 = 0;
    // CDP event fan-out is subscription-based. The interactive viewer
    // consumes only render frames and input acks; unconditionally
    // forwarding every Page/DOM/Runtime event flooded the downstream
    // callback queue on busy pages, and overflow shedding then dropped
    // fresh render frames while stale ones aged at the queue head — the
    // viewer experienced that as severe latency. Agents that want raw
    // events opt in per method (or "*") via a cdp.subscribe frame.
    let mut cdp_event_subscriptions: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    {
        // The plugin owns the screencast lifecycle: start immediately so
        // consumers only ever observe frames. Bounded capture size — every
        // frame crosses bidi as one payload, so size caps effective rate.
        let session = Arc::clone(&session);
        // Lock the layout viewport to the session's requested size so long
        // pages compute the correct scroll extent regardless of the actual
        // Chrome window size. deviceScaleFactor stays 1: Page.startScreencast
        // emits a 1x compositor preview bitmap and ignores DPR entirely
        // (empirically verified — a higher scale changes neither the frame
        // resolution nor its byte size), so raising it here would only
        // distort layout without improving clarity. Bound the screencast to
        // the CSS viewport; quality is the only clarity lever this transport
        // exposes.
        let vp = session.viewport();
        let css_width = (vp.width as f64).round().clamp(1.0, 3840.0) as u64;
        let css_height = (vp.height as f64).round().clamp(1.0, 2400.0) as u64;
        tokio::spawn(async move {
            let _ = session
                .command(
                    "Emulation.setDeviceMetricsOverride",
                    Some(json!({
                        "width": css_width,
                        "height": css_height,
                        "deviceScaleFactor": 1,
                        "mobile": false,
                    })),
                )
                .await;
            let _ = session
                .command(
                    "Page.startScreencast",
                    Some(json!({
                        "format": "jpeg",
                        "quality": 80,
                        "maxWidth": css_width,
                        "maxHeight": css_height,
                        "everyNthFrame": 1,
                    })),
                )
                .await;
        });
    }
    loop {
        tokio::select! {
            completed = concurrent_lane.join_next(), if !concurrent_lane.is_empty() => {
                let Some(completed) = completed else {
                    continue;
                };
                match emit_attachment_action(&session, &outbound, completed, "operation").await {
                    AttachmentDisposition::Continue => {}
                    AttachmentDisposition::Detached => {
                        detached_sent = true;
                        break;
                    }
                    AttachmentDisposition::Closed => break,
                }
            }
            completed = input_lane.join_next(), if !input_lane.is_empty() => {
                let Some(completed) = completed else {
                    continue;
                };
                match emit_attachment_action(&session, &outbound, completed, "input operation").await {
                    AttachmentDisposition::Continue => {}
                    AttachmentDisposition::Detached => {
                        detached_sent = true;
                        break;
                    }
                    AttachmentDisposition::Closed => break,
                }
                if let Some(frame) = queued_input.pop_front() {
                    let session = Arc::clone(&session);
                    let command_permits = Arc::clone(&command_permits);
                    input_lane.spawn(async move {
                        handle_attachment_frame(session, frame, command_permits).await
                    });
                }
            }
            frame = inbound.recv(), if attachment_operation_count(
                &concurrent_lane,
                &input_lane,
                &queued_input,
            ) < ATTACH_OPERATION_BOUND => {
                let Some(frame) = frame else {
                    break;
                };
                if let Some(reply) =
                    handle_cdp_subscription_frame(&frame, &mut cdp_event_subscriptions)
                {
                    if send_json(&outbound, reply).await.is_err() {
                        break;
                    }
                    continue;
                }
                if is_input_frame(&frame) {
                    if input_lane.is_empty() {
                        let session = Arc::clone(&session);
                        let command_permits = Arc::clone(&command_permits);
                        input_lane.spawn(async move {
                            handle_attachment_frame(session, frame, command_permits).await
                        });
                    } else if !coalesce_queued_input(&mut queued_input, &frame) {
                        queued_input.push_back(frame);
                    }
                } else {
                    let session = Arc::clone(&session);
                    let command_permits = Arc::clone(&command_permits);
                    concurrent_lane.spawn(async move {
                        handle_attachment_frame(session, frame, command_permits).await
                    });
                }
            }
            event = events.recv() => {
                match event {
                    Ok(event) if target_detached(&session, &event) => {
                        let _ = send_json(&outbound, json!({
                            "type": "browser.error",
                            "code": "target_detached",
                            "message": "Chrome detached the governed page target",
                        })).await;
                        let _ = runtime
                            .close_session_from_runtime(Arc::clone(&session), "target_detached")
                            .await;
                        break;
                    }
                    Ok(event)
                        if session.event_belongs_to_session(&event)
                            && event.method == "Page.screencastFrame" =>
                    {
                        let params = &event.params;
                        if let Some(session_id) = params.get("sessionId").cloned() {
                            *pending_ack
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner()) =
                                Some(session_id);
                            ack_notify.notify_one();
                        }
                        let data = params.get("data").and_then(Value::as_str).unwrap_or("");
                        if !data.is_empty() {
                            render_sequence += 1;
                            let metadata = params.get("metadata").cloned().unwrap_or(Value::Null);
                            let number = |key: &str, fallback: f64| {
                                metadata.get(key).and_then(Value::as_f64).unwrap_or(fallback)
                            };
                            let session_vp = session.viewport();
                            let frame = json!({
                                "type": "browser.render_frame",
                                "sequence": render_sequence,
                                "data": data,
                                "encoding": "base64",
                                "content_type": "image/jpeg",
                                "url": "",
                                "title": "",
                                "captured_at_ms": super::session::now_ms(),
                                "interactive": true,
                                "viewport": {
                                    "width_px": number("deviceWidth", session_vp.width as f64),
                                    "height_px": number("deviceHeight", session_vp.height as f64),
                                    // Chrome's screencast metadata reports CSS-px
                                    // device dimensions; the scale that ties them
                                    // to the captured image is the session DPR.
                                    "device_scale_factor": session_vp.device_scale_factor,
                                },
                                "scroll": {
                                    "x": number("scrollOffsetX", 0.0),
                                    "y": number("scrollOffsetY", 0.0),
                                    "max_x": 0,
                                    "max_y": 0,
                                },
                            });
                            *latest_frame
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(frame);
                            frame_notify.notify_one();
                        }
                    }
                    Ok(event) if session.event_belongs_to_session(&event) => {
                        if event.method == "Page.screencastVisibilityChanged"
                            && event
                                .params
                                .get("visible")
                                .and_then(Value::as_bool)
                                == Some(false)
                        {
                            session.clear_foreground();
                        }
                        if !(cdp_event_subscriptions.contains("*")
                            || cdp_event_subscriptions.contains(event.method.as_str()))
                        {
                            continue;
                        }
                        if send_json(&outbound, json!({
                            "type": "cdp.event",
                            "method": event.method,
                            "params": event.params,
                        })).await.is_err() {
                            break;
                        }
                    }
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        let _ = send_json(&outbound, json!({
                            "type": "browser.error",
                            "code": "cdp_event_backpressure",
                            "message": format!("CDP event queue lagged by {skipped} event(s)"),
                        })).await;
                        break;
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        let _ = runtime
                            .close_session_from_runtime(
                                Arc::clone(&session),
                                "cdp_connection_closed",
                            )
                            .await;
                        break;
                    }
                }
            }
        }
    }
    mirror_sender.abort();
    ack_pacer.abort();
    concurrent_lane.abort_all();
    input_lane.abort_all();
    if !detached_sent {
        let _ = send_json(&outbound, detached_frame(&session, "transport_closed")).await;
    }
}

fn attachment_operation_count(
    concurrent_lane: &JoinSet<AttachmentAction>,
    input_lane: &JoinSet<AttachmentAction>,
    queued_input: &VecDeque<Value>,
) -> usize {
    concurrent_lane.len() + input_lane.len() + queued_input.len()
}

/// Handle `cdp.subscribe` / `cdp.unsubscribe` frames in place. Returns the
/// reply to send when the frame was a subscription control frame; None lets
/// the frame continue down the normal lanes.
fn handle_cdp_subscription_frame(
    frame: &Value,
    subscriptions: &mut std::collections::HashSet<String>,
) -> Option<Value> {
    let object = frame.as_object()?;
    let frame_type = object.get("type").and_then(Value::as_str)?;
    let subscribe = match frame_type {
        "cdp.subscribe" => true,
        "cdp.unsubscribe" => false,
        _ => return None,
    };
    let id = object.get("id").cloned().unwrap_or(Value::Null);
    let Some(methods) = object.get("methods").and_then(Value::as_array) else {
        return Some(frame_error(
            None,
            "invalid_frame",
            "cdp.subscribe requires a `methods` array of CDP method names (or *)",
        ));
    };
    for method in methods {
        let Some(method) = method.as_str().map(str::trim).filter(|m| !m.is_empty()) else {
            continue;
        };
        if subscribe {
            subscriptions.insert(method.to_string());
        } else {
            subscriptions.remove(method);
        }
    }
    Some(json!({
        "type": "cdp.subscription",
        "id": id,
        "subscribed": subscriptions.iter().collect::<Vec<_>>(),
    }))
}

/// Collapse continuous pointer input into the queue tail so a serial input
/// lane cannot accumulate latency. The lane executes one frame at a time;
/// without coalescing, a move/scroll flood queues linearly and every later
/// click waits behind stale positions — perceived latency grows without
/// bound. Ordering stays exact because only the TAIL is ever merged:
///   new move  + tail move   -> replace the tail (latest position wins)
///   new scroll + tail scroll -> accumulate deltas into the tail
/// Discrete events (click, keys, navigate) are never merged and keep their
/// position relative to everything already queued.
fn coalesce_queued_input(queue: &mut VecDeque<Value>, frame: &Value) -> bool {
    fn pointer_class(frame: &Value) -> Option<&'static str> {
        let event = frame.get("event")?;
        match event.get("kind").and_then(Value::as_str)? {
            "scroll" => Some("scroll"),
            "mouse" if event.get("action").and_then(Value::as_str) == Some("move") => Some("move"),
            _ => None,
        }
    }
    let Some(class) = pointer_class(frame) else {
        return false;
    };
    let Some(tail) = queue.back_mut() else {
        return false;
    };
    if pointer_class(tail) != Some(class) {
        return false;
    }
    match class {
        "move" => {
            *tail = frame.clone();
        }
        "scroll" => {
            let delta = |value: &Value, key: &str| {
                value
                    .get("event")
                    .and_then(|event| event.get(key))
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0)
            };
            let dx = delta(tail, "delta_x") + delta(frame, "delta_x");
            let dy = delta(tail, "delta_y") + delta(frame, "delta_y");
            *tail = frame.clone();
            if let Some(event) = tail.get_mut("event").and_then(Value::as_object_mut) {
                event.insert("delta_x".into(), json!(dx));
                event.insert("delta_y".into(), json!(dy));
            }
        }
        _ => return false,
    }
    true
}

fn is_input_frame(frame: &Value) -> bool {
    frame.get("type").and_then(Value::as_str) == Some("input")
}

enum AttachmentAction {
    Reply(Value),
    Detach,
}

enum AttachmentDisposition {
    Continue,
    Detached,
    Closed,
}

async fn emit_attachment_action(
    session: &BrowserSession,
    outbound: &mpsc::Sender<BidiOutputFrame>,
    completed: Result<AttachmentAction, tokio::task::JoinError>,
    lane: &str,
) -> AttachmentDisposition {
    match completed {
        Ok(AttachmentAction::Reply(reply)) => {
            if send_json(outbound, reply).await.is_ok() {
                AttachmentDisposition::Continue
            } else {
                AttachmentDisposition::Closed
            }
        }
        Ok(AttachmentAction::Detach) => {
            let _ = send_json(outbound, detached_frame(session, "caller_detached")).await;
            AttachmentDisposition::Detached
        }
        Err(error) => {
            let _ = send_json(
                outbound,
                frame_error(
                    None,
                    "attachment_worker_failed",
                    &format!("attachment {lane} failed: {error}"),
                ),
            )
            .await;
            AttachmentDisposition::Closed
        }
    }
}

async fn handle_attachment_frame(
    session: Arc<BrowserSession>,
    frame: Value,
    command_permits: Arc<Semaphore>,
) -> AttachmentAction {
    let Some(object) = frame.as_object() else {
        return AttachmentAction::Reply(frame_error(
            None,
            "invalid_frame",
            "frame must be an object",
        ));
    };
    let frame_type = object
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match frame_type {
        "detach" => match validate_attachment_frame(object, &["type", "id"], false) {
            Ok(_) => AttachmentAction::Detach,
            Err(error) => AttachmentAction::Reply(frame_error(None, "invalid_frame", &error)),
        },
        "ping" => match validate_attachment_frame(object, &["type", "id"], false) {
            Ok(id) => AttachmentAction::Reply(
                json!({"type":"pong","id":id,"at_ms":super::session::now_ms()}),
            ),
            Err(error) => AttachmentAction::Reply(frame_error(None, "invalid_frame", &error)),
        },
        "input" => {
            let id = match validate_attachment_frame(object, &["type", "id", "event"], true) {
                Ok(id) => id,
                Err(error) => {
                    return AttachmentAction::Reply(frame_error(None, "invalid_frame", &error));
                }
            };
            let Some(event) = object.get("event").cloned() else {
                return AttachmentAction::Reply(frame_error(
                    Some(id),
                    "invalid_input",
                    "input frame requires event",
                ));
            };
            match apply_input(session, ABILITY_ATTACH_SESSION, event).await {
                Ok(result) => AttachmentAction::Reply(json!({
                    "type": "browser.input_ack",
                    "id": id,
                    "result": result,
                })),
                Err(error) => AttachmentAction::Reply(frame_error(
                    Some(id),
                    "input_failed",
                    &error.to_string(),
                )),
            }
        }
        "cdp.command" => {
            let command = match parse_agent_cdp_command(object, &["type", "id", "method", "params"])
            {
                Ok(command) => command,
                Err(error) => return AttachmentAction::Reply(error),
            };
            AttachmentAction::Reply(
                execute_agent_cdp_command(session, command_permits, command).await,
            )
        }
        "cdp.batch" => {
            let batch = match parse_agent_cdp_batch(object) {
                Ok(batch) => batch,
                Err(error) => return AttachmentAction::Reply(error),
            };
            let responses = futures::future::join_all(batch.commands.into_iter().map(|command| {
                execute_agent_cdp_command(
                    Arc::clone(&session),
                    Arc::clone(&command_permits),
                    command,
                )
            }))
            .await;
            AttachmentAction::Reply(json!({
                "type": "cdp.batch_response",
                "id": batch.id,
                "responses": responses,
            }))
        }
        _ => AttachmentAction::Reply(frame_error(
            object.get("id").cloned(),
            "unknown_frame",
            "supported frame types: cdp.command, cdp.batch, input, ping, detach",
        )),
    }
}

#[derive(Debug)]
struct AgentCdpCommand {
    id: Value,
    method: String,
    params: Option<Value>,
}

#[derive(Debug)]
struct AgentCdpBatch {
    id: Value,
    commands: Vec<AgentCdpCommand>,
}

fn parse_agent_cdp_batch(object: &Map<String, Value>) -> Result<AgentCdpBatch, Value> {
    let id = validate_attachment_frame(object, &["type", "id", "commands"], true)
        .map_err(|error| frame_error(None, "invalid_frame", &error))?;
    let commands = object
        .get("commands")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            frame_error(
                Some(id.clone()),
                "invalid_batch",
                "commands must be an array",
            )
        })?;
    if commands.is_empty() || commands.len() > ATTACH_BATCH_COMMAND_BOUND {
        return Err(frame_error(
            Some(id),
            "invalid_batch",
            &format!("commands must contain 1..={ATTACH_BATCH_COMMAND_BOUND} entries"),
        ));
    }

    let mut parsed = Vec::with_capacity(commands.len());
    let mut command_ids = HashSet::with_capacity(commands.len());
    for (index, command) in commands.iter().enumerate() {
        let Some(command) = command.as_object() else {
            return Err(frame_error(
                Some(id),
                "invalid_batch",
                &format!("commands[{index}] must be an object"),
            ));
        };
        let command =
            parse_agent_cdp_command(command, &["id", "method", "params"]).map_err(|error| {
                let detail = error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("invalid CDP command");
                frame_error(
                    Some(id.clone()),
                    "invalid_batch",
                    &format!("commands[{index}] invalid: {detail}"),
                )
            })?;
        let correlation_key = serde_json::to_string(&command.id)
            .expect("string/number correlation id serialization cannot fail");
        if !command_ids.insert(correlation_key) {
            return Err(frame_error(
                Some(id),
                "invalid_batch",
                &format!("commands[{index}] duplicates an earlier command id"),
            ));
        }
        parsed.push(command);
    }
    Ok(AgentCdpBatch {
        id,
        commands: parsed,
    })
}

fn parse_agent_cdp_command(
    object: &Map<String, Value>,
    allowed: &[&str],
) -> Result<AgentCdpCommand, Value> {
    let id = validate_attachment_frame(object, allowed, true)
        .map_err(|error| frame_error(None, "invalid_frame", &error))?;
    let method = object
        .get("method")
        .and_then(Value::as_str)
        .ok_or_else(|| frame_error(Some(id.clone()), "invalid_command", "method is required"))?
        .to_string();
    let params = match object.get("params") {
        None | Some(Value::Null) => None,
        Some(Value::Object(params)) => Some(Value::Object(params.clone())),
        Some(_) => {
            return Err(frame_error(
                Some(id),
                "invalid_command",
                "params must be an object",
            ));
        }
    };
    if let Err(detail) = validate_agent_command(&method, params.as_ref()) {
        let error = BrowserError::CdpPolicy {
            ability: ABILITY_ATTACH_SESSION,
            method: method.clone(),
        };
        return Err(frame_error(
            Some(id),
            REASON_CDP_POLICY,
            &format!("{error}: {detail}"),
        ));
    }
    Ok(AgentCdpCommand { id, method, params })
}

async fn execute_agent_cdp_command(
    session: Arc<BrowserSession>,
    command_permits: Arc<Semaphore>,
    command: AgentCdpCommand,
) -> Value {
    let _permit = command_permits
        .acquire_owned()
        .await
        .expect("attachment command semaphore is never closed");
    match session.raw_command(&command.method, command.params).await {
        Ok(result) => json!({
            "type": "cdp.response",
            "id": command.id,
            "result": result,
        }),
        Err(error) => cdp_error_response(command.id, &command.method, error),
    }
}

fn validate_attachment_frame(
    object: &Map<String, Value>,
    allowed: &[&str],
    id_required: bool,
) -> Result<Value, String> {
    let unknown = object
        .keys()
        .filter(|key| !allowed.contains(&key.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if !unknown.is_empty() {
        return Err(format!(
            "unsupported frame field(s): {}",
            unknown.join(", ")
        ));
    }
    match object.get("id") {
        Some(Value::String(id)) if id.is_empty() => {
            Err("frame id string must not be empty".to_string())
        }
        Some(Value::String(id)) if id.len() > MAX_CORRELATION_ID_BYTES => Err(format!(
            "frame id string exceeds {MAX_CORRELATION_ID_BYTES} bytes"
        )),
        Some(id @ (Value::String(_) | Value::Number(_))) => Ok(id.clone()),
        Some(_) => Err("frame id must be a string or number".to_string()),
        None if id_required => Err("frame id is required".to_string()),
        None => Ok(Value::Null),
    }
}

fn cdp_error_response(id: Value, method: &str, error: CdpFailure) -> Value {
    match error {
        CdpFailure::Protocol {
            code,
            message,
            data,
            ..
        } => json!({
            "type": "cdp.response",
            "id": id,
            "error": {"code": code, "message": message, "data": data, "method": method},
        }),
        other => json!({
            "type": "cdp.response",
            "id": id,
            "error": {"code": "transport_error", "message": other.to_string(), "method": method},
        }),
    }
}

fn target_detached(session: &BrowserSession, event: &CdpEvent) -> bool {
    event.method == "Target.detachedFromTarget"
        && event.params.get("sessionId").and_then(Value::as_str) == Some(session.cdp_session_id())
}

fn cdp_failure_requires_close(error: &CdpFailure) -> bool {
    matches!(
        error,
        CdpFailure::Closed | CdpFailure::Timeout { .. } | CdpFailure::Wire(_)
    )
}

fn detached_frame(session: &BrowserSession, reason: &str) -> Value {
    json!({
        "type": "browser.detached",
        "session_ura": session.session_ura(),
        "reason": reason,
    })
}

fn frame_error(id: Option<Value>, code: &str, message: &str) -> Value {
    json!({
        "type": "browser.error",
        "id": id.unwrap_or(Value::Null),
        "code": code,
        "message": message,
    })
}

async fn send_json(
    sender: &mpsc::Sender<BidiOutputFrame>,
    value: Value,
) -> Result<(), mpsc::error::SendError<BidiOutputFrame>> {
    sender.send(BidiOutputFrame::json(value)).await
}

fn single_required_field(ability: &'static str, args: Value, field: &str) -> BrowserResult<Value> {
    let object = args
        .as_object()
        .ok_or_else(|| invalid(ability, "args must be an object"))?;
    if object.len() != 1 || !object.contains_key(field) {
        return Err(invalid(
            ability,
            format!("args must contain exactly `{field}`"),
        ));
    }
    Ok(object.get(field).cloned().expect("required field"))
}

fn require_empty_args(ability: &'static str, args: &Value) -> BrowserResult<()> {
    match args.as_object() {
        Some(object) if object.is_empty() => Ok(()),
        Some(object) => Err(invalid(
            ability,
            format!(
                "unsupported argument field(s): {}",
                object.keys().cloned().collect::<Vec<_>>().join(", ")
            ),
        )),
        None => Err(invalid(ability, "args must be an object")),
    }
}

fn invalid(ability: &'static str, detail: impl Into<String>) -> BrowserError {
    BrowserError::InvalidArgument {
        ability,
        detail: detail.into(),
    }
}

#[derive(Clone, Debug)]
struct CaptureRequest {
    format: String,
    quality: u64,
    max_width: u64,
    max_height: u64,
    max_frames: u64,
    timeout_seconds: u64,
}

impl CaptureRequest {
    fn parse(args: Value) -> BrowserResult<Self> {
        let object = args
            .as_object()
            .ok_or_else(|| invalid(ABILITY_CAPTURE_VIEWPORT, "args must be an object"))?;
        let allowed = [
            "format",
            "quality",
            "max_width",
            "max_height",
            "max_frames",
            "timeout_seconds",
        ];
        let unknown = object
            .keys()
            .filter(|key| !allowed.contains(&key.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        if !unknown.is_empty() {
            return Err(invalid(
                ABILITY_CAPTURE_VIEWPORT,
                format!("unsupported argument field(s): {}", unknown.join(", ")),
            ));
        }
        let format = object
            .get("format")
            .and_then(Value::as_str)
            .unwrap_or("jpeg");
        if !matches!(format, "jpeg" | "png") {
            return Err(invalid(
                ABILITY_CAPTURE_VIEWPORT,
                "format must be jpeg or png",
            ));
        }
        Ok(Self {
            format: format.to_string(),
            quality: bounded_u64(object, "quality", 80, 0, 100)?,
            max_width: bounded_u64(object, "max_width", 1920, 1, 7680)?,
            max_height: bounded_u64(object, "max_height", 1080, 1, 4320)?,
            max_frames: bounded_u64(
                object,
                "max_frames",
                1,
                MIN_CAPTURE_FRAMES,
                MAX_CAPTURE_FRAMES,
            )?,
            timeout_seconds: bounded_u64(object, "timeout_seconds", 15, 1, 120)?,
        })
    }
}

fn bounded_u64(
    object: &Map<String, Value>,
    field: &str,
    default: u64,
    min: u64,
    max: u64,
) -> BrowserResult<u64> {
    let value = object
        .get(field)
        .map(|value| value.as_u64())
        .unwrap_or(Some(default))
        .ok_or_else(|| {
            invalid(
                ABILITY_CAPTURE_VIEWPORT,
                format!("`{field}` must be an integer"),
            )
        })?;
    if (min..=max).contains(&value) {
        Ok(value)
    } else {
        Err(invalid(
            ABILITY_CAPTURE_VIEWPORT,
            format!("`{field}` must be between {min} and {max}"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_request_is_finite_and_bounded() {
        let request = CaptureRequest::parse(json!({})).unwrap();
        assert_eq!(request.max_frames, 1);
        assert!(CaptureRequest::parse(json!({"max_frames": 301})).is_err());
    }

    #[test]
    fn cdp_error_preserves_protocol_code() {
        let response = cdp_error_response(
            json!("client-1"),
            "Page.navigate",
            CdpFailure::Protocol {
                method: "Page.navigate".to_string(),
                code: -32602,
                message: "bad url".to_string(),
                data: None,
            },
        );
        assert_eq!(response["error"]["code"], -32602);
        assert_eq!(response["id"], "client-1");
    }

    #[test]
    fn attachment_frames_require_correlatable_ids_and_exact_fields() {
        let command = json!({"type":"cdp.command","id":"c1","method":"Page.reload"});
        assert!(validate_attachment_frame(
            command.as_object().unwrap(),
            &["type", "id", "method", "params"],
            true,
        )
        .is_ok());

        let missing_id = json!({"type":"cdp.command","method":"Page.reload"});
        assert!(validate_attachment_frame(
            missing_id.as_object().unwrap(),
            &["type", "id", "method", "params"],
            true,
        )
        .is_err());

        let extra = json!({"type":"ping","sessionId":"escape"});
        assert!(
            validate_attachment_frame(extra.as_object().unwrap(), &["type", "id"], false,).is_err()
        );

        let oversized_id = json!({
            "type": "ping",
            "id": "x".repeat(MAX_CORRELATION_ID_BYTES + 1),
        });
        assert!(validate_attachment_frame(
            oversized_id.as_object().unwrap(),
            &["type", "id"],
            false,
        )
        .is_err());
    }

    #[test]
    fn cdp_batches_are_bounded_strict_and_uniquely_correlated() {
        let valid = json!({
            "type": "cdp.batch",
            "id": "batch-1",
            "commands": [
                {"id": 1, "method": "Runtime.evaluate", "params": {"expression": "1"}},
                {"id": "two", "method": "Page.reload"},
            ],
        });
        let parsed = parse_agent_cdp_batch(valid.as_object().unwrap()).expect("valid batch");
        assert_eq!(parsed.id, "batch-1");
        assert_eq!(parsed.commands.len(), 2);

        let duplicate = json!({
            "type": "cdp.batch",
            "id": "batch-2",
            "commands": [
                {"id": 1, "method": "Page.reload"},
                {"id": 1, "method": "Page.reload"},
            ],
        });
        assert_eq!(
            parse_agent_cdp_batch(duplicate.as_object().unwrap()).expect_err("duplicate ids fail")
                ["id"],
            "batch-2"
        );

        let too_many = json!({
            "type": "cdp.batch",
            "id": "batch-3",
            "commands": (0..=ATTACH_BATCH_COMMAND_BOUND)
                .map(|id| json!({"id": id, "method": "Page.reload"}))
                .collect::<Vec<_>>(),
        });
        assert_eq!(
            parse_agent_cdp_batch(too_many.as_object().unwrap()).expect_err("oversize batch fails")
                ["code"],
            "invalid_batch"
        );

        let routed = json!({
            "type": "cdp.batch",
            "id": "batch-4",
            "commands": [{
                "id": 1,
                "method": "Runtime.evaluate",
                "sessionId": "escape",
            }],
        });
        assert_eq!(
            parse_agent_cdp_batch(routed.as_object().unwrap()).expect_err("caller routing fails")
                ["id"],
            "batch-4"
        );
    }
}
