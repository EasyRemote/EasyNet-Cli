// EasyNet CLI — remote desktop input plane
// ========================================
//
// File: plugins/remote-desktop/src/input.rs
// Description: Device-local input frames carried over the WebRTC data channel.
//
// Boundary:
// - The remote desktop plugin owns its session and input-policy contract.
// - EasyNet-Cli owns OS-local input injection; Axon owns generic admission.
// - EasyNet/Hub must never relay high-frequency pointer or keyboard events
//   through Invocation once a direct media/control channel is negotiated.

use std::sync::Arc;

use serde::Deserialize;
use serde_json::{json, Value};
use webrtc::data_channel::{DataChannel, DataChannelEvent};

use crate::daemon::persistence::resources::{ResourceEntry, ResourceType};
use crate::daemon::plugins::remote_desktop::session_store::RemoteDesktopSessionStore;

pub const INPUT_DATA_CHANNEL_LABEL: &str = "easynet.remote_desktop.input.v1";
pub const MAX_INPUT_FRAME_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteDesktopInputKind {
    Pointer,
    Key,
    Clipboard,
    FileDrop,
}

impl RemoteDesktopInputKind {
    pub fn as_policy_key(self) -> &'static str {
        match self {
            Self::Pointer => "pointer",
            Self::Key => "key",
            Self::Clipboard => "clipboard",
            Self::FileDrop => "file_drop",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum RemoteDesktopInputFrame {
    #[serde(rename = "pointer")]
    Pointer(PointerInputFrame),
    #[serde(rename = "key")]
    Key(KeyInputFrame),
    #[serde(rename = "clipboard")]
    Clipboard(ClipboardInputFrame),
    #[serde(rename = "file_drop")]
    FileDrop(FileDropInputFrame),
}

impl RemoteDesktopInputFrame {
    pub fn kind(&self) -> RemoteDesktopInputKind {
        match self {
            Self::Pointer(_) => RemoteDesktopInputKind::Pointer,
            Self::Key(_) => RemoteDesktopInputKind::Key,
            Self::Clipboard(_) => RemoteDesktopInputKind::Clipboard,
            Self::FileDrop(_) => RemoteDesktopInputKind::FileDrop,
        }
    }

    pub fn action(&self) -> &str {
        match self {
            Self::Pointer(frame) => frame.action.as_str(),
            Self::Key(frame) => frame.action.as_str(),
            Self::Clipboard(_) => "write",
            Self::FileDrop(_) => "drop",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct PointerInputFrame {
    pub action: String,
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
    #[serde(default)]
    pub normalized_x: Option<f64>,
    #[serde(default)]
    pub normalized_y: Option<f64>,
    #[serde(default)]
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub target_width: Option<f64>,
    #[serde(default)]
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub target_height: Option<f64>,
    #[serde(default)]
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub button: Option<u8>,
    #[serde(default)]
    pub delta_x: Option<f64>,
    #[serde(default)]
    pub delta_y: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct KeyInputFrame {
    pub action: String,
    #[serde(default)]
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub key: String,
    #[serde(default)]
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub code: String,
    #[serde(default)]
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub repeat: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ClipboardInputFrame {
    #[serde(default)]
    pub text: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FileDropInputFrame {
    #[serde(default)]
    pub files: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct InputApplyOutcome {
    pub applied: bool,
    pub reason: Option<&'static str>,
}

impl InputApplyOutcome {
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    fn applied() -> Self {
        Self {
            applied: true,
            reason: None,
        }
    }

    fn rejected(reason: &'static str) -> Self {
        Self {
            applied: false,
            reason: Some(reason),
        }
    }
}

pub fn parse_input_frame(text: &str) -> anyhow::Result<RemoteDesktopInputFrame> {
    if text.len() > MAX_INPUT_FRAME_BYTES {
        anyhow::bail!("remote desktop input frame exceeds {MAX_INPUT_FRAME_BYTES} bytes")
    }
    let frame: RemoteDesktopInputFrame = serde_json::from_str(text)?;
    validate_input_frame(&frame)?;
    Ok(frame)
}

pub(in crate::daemon::plugins::remote_desktop) fn apply_input_frame_with_policy(
    input_policy: &Value,
    frame: &RemoteDesktopInputFrame,
) -> InputApplyOutcome {
    match frame {
        RemoteDesktopInputFrame::Pointer(frame) => {
            apply_pointer_frame(frame, pointer_target_from_policy(input_policy))
        }
        RemoteDesktopInputFrame::Key(frame) => apply_key_frame(frame),
        RemoteDesktopInputFrame::Clipboard(_) => {
            InputApplyOutcome::rejected("clipboard_injection_not_enabled")
        }
        RemoteDesktopInputFrame::FileDrop(_) => {
            InputApplyOutcome::rejected("file_drop_not_enabled")
        }
    }
}

pub(in crate::daemon::plugins::remote_desktop) fn input_policy_for_entry(
    mut input_policy: Value,
    entry: &ResourceEntry,
) -> Value {
    let Some(target) = pointer_target_for_entry(entry) else {
        return input_policy;
    };
    let Some(map) = input_policy.as_object_mut() else {
        return input_policy;
    };
    map.insert(
        "pointer_target".to_string(),
        json!({
            "subject_type": entry.kind.as_str(),
            "hardware_id": entry.hardware_id.as_str(),
            "origin_x": target.origin_x,
            "origin_y": target.origin_y,
            "width": target.width,
            "height": target.height,
        }),
    );
    input_policy
}

pub fn input_injection_available() -> bool {
    platform::input_injection_available()
}

pub fn request_input_injection_permission() -> bool {
    platform::request_input_injection_permission()
}

/// Run the direct WebRTC input data-channel loop for one remote desktop
/// session.
///
/// Invariant 1: malformed frames are recorded as session diagnostics and do
/// not panic the channel task.
/// Invariant 2: every accepted frame passes through the session input policy
/// before local OS injection is attempted.
/// Invariant 3: counters are monotonic within a channel lifetime and are
/// emitted on close.
pub(in crate::daemon::plugins::remote_desktop) async fn run_remote_desktop_input_channel(
    sessions: Arc<RemoteDesktopSessionStore>,
    session_id: String,
    input_policy: Value,
    data_channel: Arc<dyn DataChannel>,
) {
    let mut accepted_count = 0_u64;
    let mut rejected_count = 0_u64;
    while let Some(event) = data_channel.poll().await {
        match event {
            DataChannelEvent::OnOpen => record_input_channel_event(
                &sessions,
                &session_id,
                "INPUT_CHANNEL_OPENED",
                json!({
                    "label": INPUT_DATA_CHANNEL_LABEL,
                    "input_injection_available": input_injection_available(),
                }),
            ),
            DataChannelEvent::OnClose | DataChannelEvent::OnClosing => {
                record_input_channel_event(
                    &sessions,
                    &session_id,
                    "INPUT_CHANNEL_CLOSED",
                    json!({
                        "accepted_count": accepted_count,
                        "rejected_count": rejected_count,
                    }),
                );
                break;
            }
            DataChannelEvent::OnError => {
                record_input_channel_event(
                    &sessions,
                    &session_id,
                    "INPUT_CHANNEL_ERROR",
                    json!({ "reason": "data_channel_error" }),
                );
            }
            DataChannelEvent::OnMessage(message) => {
                if !message.is_string {
                    rejected_count = rejected_count.saturating_add(1);
                    record_input_channel_event(
                        &sessions,
                        &session_id,
                        "INPUT_FRAME_REJECTED",
                        json!({
                            "reason": "binary_input_frame_rejected",
                            "rejected_count": rejected_count,
                        }),
                    );
                    continue;
                }
                let Ok(text) = String::from_utf8(message.data.to_vec()) else {
                    rejected_count = rejected_count.saturating_add(1);
                    record_input_channel_event(
                        &sessions,
                        &session_id,
                        "INPUT_FRAME_REJECTED",
                        json!({
                            "reason": "invalid_utf8",
                            "rejected_count": rejected_count,
                        }),
                    );
                    continue;
                };
                let frame = match parse_input_frame(&text) {
                    Ok(frame) => frame,
                    Err(err) => {
                        rejected_count = rejected_count.saturating_add(1);
                        record_input_channel_event(
                            &sessions,
                            &session_id,
                            "INPUT_FRAME_REJECTED",
                            json!({
                                "reason": "invalid_input_frame",
                                "message": err.to_string(),
                                "rejected_count": rejected_count,
                            }),
                        );
                        continue;
                    }
                };
                let kind = frame.kind().as_policy_key();
                if !input_policy_allows(&input_policy, kind) {
                    rejected_count = rejected_count.saturating_add(1);
                    record_input_channel_event(
                        &sessions,
                        &session_id,
                        "INPUT_FRAME_REJECTED",
                        json!({
                            "reason": "input_policy_denied",
                            "kind": kind,
                            "action": frame.action(),
                            "rejected_count": rejected_count,
                        }),
                    );
                    continue;
                }
                let outcome = apply_input_frame_with_policy(&input_policy, &frame);
                if outcome.applied {
                    accepted_count = accepted_count.saturating_add(1);
                    if accepted_count == 1 || accepted_count.is_multiple_of(120) {
                        record_input_channel_event(
                            &sessions,
                            &session_id,
                            "INPUT_FRAME_APPLIED",
                            json!({
                                "kind": kind,
                                "action": frame.action(),
                                "accepted_count": accepted_count,
                                "rejected_count": rejected_count,
                            }),
                        );
                    }
                } else {
                    rejected_count = rejected_count.saturating_add(1);
                    record_input_channel_event(
                        &sessions,
                        &session_id,
                        "INPUT_FRAME_REJECTED",
                        json!({
                            "reason": outcome.reason.unwrap_or("input_injection_failed"),
                            "kind": kind,
                            "action": frame.action(),
                            "rejected_count": rejected_count,
                        }),
                    );
                }
            }
            DataChannelEvent::OnBufferedAmountLow | DataChannelEvent::OnBufferedAmountHigh => {}
        }
    }
}

pub(in crate::daemon::plugins::remote_desktop) fn input_policy_allows(
    policy: &Value,
    frame_type: &str,
) -> bool {
    let key = match frame_type {
        "key" => "keyboard_enabled",
        "pointer" => "pointer_enabled",
        "clipboard" => "clipboard_enabled",
        "file_drop" => "file_drop_enabled",
        _ => return false,
    };
    policy.get(key).and_then(Value::as_bool).unwrap_or(false)
}

pub(in crate::daemon::plugins::remote_desktop) fn record_input_channel_event(
    sessions: &RemoteDesktopSessionStore,
    session_id: &str,
    event_type: &str,
    payload: Value,
) {
    let mut s = sessions.lock();
    let Some(session) = s.get_mut(session_id) else {
        return;
    };
    if session.is_terminal() {
        return;
    }
    session.record_input_channel_event(event_type, payload);
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct PointerTargetGeometry {
    origin_x: f64,
    origin_y: f64,
    width: Option<f64>,
    height: Option<f64>,
}

#[cfg(any(target_os = "macos", test))]
#[derive(Debug, Clone, Copy, PartialEq)]
struct MappedPointerPoint {
    x: f64,
    y: f64,
}

fn pointer_target_for_entry(entry: &ResourceEntry) -> Option<PointerTargetGeometry> {
    match entry.kind {
        ResourceType::Display => {
            let origin_x = metadata_f64(entry, "x").unwrap_or(0.0);
            let origin_y = metadata_f64(entry, "y").unwrap_or(0.0);
            Some(PointerTargetGeometry {
                origin_x,
                origin_y,
                width: metadata_f64(entry, "width"),
                height: metadata_f64(entry, "height"),
            })
        }
        ResourceType::Window => Some(PointerTargetGeometry {
            origin_x: metadata_f64(entry, "x")?,
            origin_y: metadata_f64(entry, "y")?,
            width: metadata_f64(entry, "width"),
            height: metadata_f64(entry, "height"),
        }),
        ResourceType::Application => Some(PointerTargetGeometry {
            origin_x: metadata_f64(entry, "primary_x").or_else(|| metadata_f64(entry, "x"))?,
            origin_y: metadata_f64(entry, "primary_y").or_else(|| metadata_f64(entry, "y"))?,
            width: metadata_f64(entry, "primary_width").or_else(|| metadata_f64(entry, "width")),
            height: metadata_f64(entry, "primary_height").or_else(|| metadata_f64(entry, "height")),
        }),
        _ => None,
    }
}

fn pointer_target_from_policy(policy: &Value) -> Option<PointerTargetGeometry> {
    let target = policy.get("pointer_target")?;
    Some(PointerTargetGeometry {
        origin_x: value_f64(target.get("origin_x")?)?,
        origin_y: value_f64(target.get("origin_y")?)?,
        width: target.get("width").and_then(value_f64),
        height: target.get("height").and_then(value_f64),
    })
}

fn metadata_f64(entry: &ResourceEntry, key: &str) -> Option<f64> {
    entry.metadata.get(key).and_then(value_f64)
}

fn value_f64(value: &Value) -> Option<f64> {
    value.as_f64().filter(|value| value.is_finite())
}

#[cfg(any(target_os = "macos", test))]
fn map_pointer_point(
    frame: &PointerInputFrame,
    target: Option<PointerTargetGeometry>,
) -> MappedPointerPoint {
    let Some(target) = target else {
        return MappedPointerPoint {
            x: frame.x.max(0.0),
            y: frame.y.max(0.0),
        };
    };
    let local_x = map_axis(
        frame.x,
        frame.normalized_x,
        frame.target_width,
        target.width,
    );
    let local_y = map_axis(
        frame.y,
        frame.normalized_y,
        frame.target_height,
        target.height,
    );
    MappedPointerPoint {
        x: target.origin_x + local_x,
        y: target.origin_y + local_y,
    }
}

#[cfg(any(target_os = "macos", test))]
fn map_axis(
    raw: f64,
    normalized: Option<f64>,
    client_span: Option<f64>,
    source_span: Option<f64>,
) -> f64 {
    if let Some(normalized) = normalized {
        if let Some(span) = source_span.or(client_span) {
            return normalized * span.max(1.0);
        }
    }
    if let (Some(client_span), Some(source_span)) = (client_span, source_span) {
        if client_span.is_finite() && source_span.is_finite() && client_span > 0.0 {
            return raw * source_span / client_span;
        }
    }
    raw
}

fn validate_input_frame(frame: &RemoteDesktopInputFrame) -> anyhow::Result<()> {
    match frame {
        RemoteDesktopInputFrame::Pointer(pointer) => {
            match pointer.action.as_str() {
                "move" | "down" | "up" | "wheel" => {}
                other => anyhow::bail!("unsupported pointer action {other:?}"),
            }
            if !pointer.x.is_finite() || !pointer.y.is_finite() {
                anyhow::bail!("pointer coordinates must be finite")
            }
            if pointer
                .normalized_x
                .is_some_and(|value| !value.is_finite() || !(0.0..=1.0).contains(&value))
                || pointer
                    .normalized_y
                    .is_some_and(|value| !value.is_finite() || !(0.0..=1.0).contains(&value))
            {
                anyhow::bail!("normalized pointer coordinates must be in [0,1]")
            }
            if pointer.delta_x.is_some_and(|value| !value.is_finite())
                || pointer.delta_y.is_some_and(|value| !value.is_finite())
            {
                anyhow::bail!("pointer wheel deltas must be finite")
            }
        }
        RemoteDesktopInputFrame::Key(key) => match key.action.as_str() {
            "down" | "up" => {}
            other => anyhow::bail!("unsupported key action {other:?}"),
        },
        RemoteDesktopInputFrame::Clipboard(clipboard) => {
            if clipboard.text.len() > MAX_INPUT_FRAME_BYTES {
                anyhow::bail!("clipboard input frame is too large")
            }
        }
        RemoteDesktopInputFrame::FileDrop(file_drop) => {
            if file_drop.files.len() > 64 {
                anyhow::bail!("file drop frame contains too many paths")
            }
        }
    }
    Ok(())
}

fn apply_pointer_frame(
    frame: &PointerInputFrame,
    target: Option<PointerTargetGeometry>,
) -> InputApplyOutcome {
    platform::apply_pointer_frame(frame, target)
}

fn apply_key_frame(frame: &KeyInputFrame) -> InputApplyOutcome {
    platform::apply_key_frame(frame)
}

#[cfg(target_os = "macos")]
mod platform {
    use std::ffi::c_void;

    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    use objc2::Message;
    use objc2_core_foundation::CFString;
    use objc2_foundation::{NSMutableDictionary, NSNumber};

    use super::{
        map_pointer_point, InputApplyOutcome, KeyInputFrame, PointerInputFrame,
        PointerTargetGeometry,
    };

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGPoint {
        x: f64,
        y: f64,
    }

    type CGEventRef = *mut c_void;
    type CGEventSourceRef = *mut c_void;

    const K_CG_HID_EVENT_TAP: u32 = 0;
    const K_CG_EVENT_LEFT_MOUSE_DOWN: u32 = 1;
    const K_CG_EVENT_LEFT_MOUSE_UP: u32 = 2;
    const K_CG_EVENT_RIGHT_MOUSE_DOWN: u32 = 3;
    const K_CG_EVENT_RIGHT_MOUSE_UP: u32 = 4;
    const K_CG_EVENT_MOUSE_MOVED: u32 = 5;
    const K_CG_EVENT_OTHER_MOUSE_DOWN: u32 = 25;
    const K_CG_EVENT_OTHER_MOUSE_UP: u32 = 26;
    const K_CG_SCROLL_EVENT_UNIT_PIXEL: u32 = 0;

    const K_CG_MOUSE_BUTTON_LEFT: u32 = 0;
    const K_CG_MOUSE_BUTTON_RIGHT: u32 = 1;
    const K_CG_MOUSE_BUTTON_CENTER: u32 = 2;

    const ACCESSIBILITY_DENIED_REASON: &str = "accessibility_permission_denied";

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn CGEventCreateMouseEvent(
            source: CGEventSourceRef,
            mouse_type: u32,
            mouse_cursor_position: CGPoint,
            mouse_button: u32,
        ) -> CGEventRef;
        fn CGEventCreateKeyboardEvent(
            source: CGEventSourceRef,
            virtual_key: u16,
            key_down: bool,
        ) -> CGEventRef;
        fn CGEventCreateScrollWheelEvent(
            source: CGEventSourceRef,
            units: u32,
            wheel_count: u32,
            wheel1: i32,
            wheel2: i32,
        ) -> CGEventRef;
        fn CGEventPost(tap: u32, event: CGEventRef);
        fn CFRelease(cf: *const c_void);
        fn AXIsProcessTrusted() -> bool;
        fn AXIsProcessTrustedWithOptions(options: *const c_void) -> bool;
        static kAXTrustedCheckOptionPrompt: &'static CFString;
    }

    pub(super) fn input_injection_available() -> bool {
        unsafe { AXIsProcessTrusted() }
    }

    pub(super) fn request_input_injection_permission() -> bool {
        request_accessibility_prompt();
        input_injection_available()
    }

    pub(super) fn apply_pointer_frame(
        frame: &PointerInputFrame,
        target: Option<PointerTargetGeometry>,
    ) -> InputApplyOutcome {
        if !input_injection_available() {
            request_accessibility_prompt();
            return InputApplyOutcome::rejected(ACCESSIBILITY_DENIED_REASON);
        }
        if frame.action == "wheel" {
            return apply_wheel_frame(frame);
        }
        let (event_type, button) = match (frame.action.as_str(), frame.button.unwrap_or(0)) {
            ("move", _) => (K_CG_EVENT_MOUSE_MOVED, K_CG_MOUSE_BUTTON_LEFT),
            ("down", 0) => (K_CG_EVENT_LEFT_MOUSE_DOWN, K_CG_MOUSE_BUTTON_LEFT),
            ("up", 0) => (K_CG_EVENT_LEFT_MOUSE_UP, K_CG_MOUSE_BUTTON_LEFT),
            ("down", 1) => (K_CG_EVENT_CENTER_MOUSE_DOWN, K_CG_MOUSE_BUTTON_CENTER),
            ("up", 1) => (K_CG_EVENT_CENTER_MOUSE_UP, K_CG_MOUSE_BUTTON_CENTER),
            ("down", 2) => (K_CG_EVENT_RIGHT_MOUSE_DOWN, K_CG_MOUSE_BUTTON_RIGHT),
            ("up", 2) => (K_CG_EVENT_RIGHT_MOUSE_UP, K_CG_MOUSE_BUTTON_RIGHT),
            ("down", _) => (K_CG_EVENT_OTHER_MOUSE_DOWN, K_CG_MOUSE_BUTTON_CENTER),
            ("up", _) => (K_CG_EVENT_OTHER_MOUSE_UP, K_CG_MOUSE_BUTTON_CENTER),
            _ => return InputApplyOutcome::rejected("unsupported_pointer_action"),
        };
        let point = mapped_point(frame, target);
        unsafe {
            let event = CGEventCreateMouseEvent(std::ptr::null_mut(), event_type, point, button);
            if event.is_null() {
                return InputApplyOutcome::rejected("cg_event_create_failed");
            }
            CGEventPost(K_CG_HID_EVENT_TAP, event);
            CFRelease(event.cast_const());
        }
        InputApplyOutcome::applied()
    }

    pub(super) fn apply_key_frame(frame: &KeyInputFrame) -> InputApplyOutcome {
        if !input_injection_available() {
            request_accessibility_prompt();
            return InputApplyOutcome::rejected(ACCESSIBILITY_DENIED_REASON);
        }
        if frame.repeat {
            return InputApplyOutcome::applied();
        }
        let Some(keycode) =
            keycode_from_dom_code(&frame.code).or_else(|| keycode_from_key(&frame.key))
        else {
            return InputApplyOutcome::rejected("unsupported_key");
        };
        let key_down = match frame.action.as_str() {
            "down" => true,
            "up" => false,
            _ => return InputApplyOutcome::rejected("unsupported_key_action"),
        };
        unsafe {
            let event = CGEventCreateKeyboardEvent(std::ptr::null_mut(), keycode, key_down);
            if event.is_null() {
                return InputApplyOutcome::rejected("cg_event_create_failed");
            }
            CGEventPost(K_CG_HID_EVENT_TAP, event);
            CFRelease(event.cast_const());
        }
        InputApplyOutcome::applied()
    }

    fn apply_wheel_frame(frame: &PointerInputFrame) -> InputApplyOutcome {
        let vertical = frame
            .delta_y
            .map(|value| (-value).round() as i32)
            .unwrap_or(0);
        let horizontal = frame
            .delta_x
            .map(|value| (-value).round() as i32)
            .unwrap_or(0);
        unsafe {
            let event = CGEventCreateScrollWheelEvent(
                std::ptr::null_mut(),
                K_CG_SCROLL_EVENT_UNIT_PIXEL,
                2,
                vertical,
                horizontal,
            );
            if event.is_null() {
                return InputApplyOutcome::rejected("cg_event_create_failed");
            }
            CGEventPost(K_CG_HID_EVENT_TAP, event);
            CFRelease(event.cast_const());
        }
        InputApplyOutcome::applied()
    }

    fn request_accessibility_prompt() {
        let prompt = NSNumber::new_bool(true);
        let key = unsafe { cfstring_as_object(kAXTrustedCheckOptionPrompt) };
        let options = NSMutableDictionary::<AnyObject, AnyObject>::new();
        unsafe {
            let _: () = msg_send![&*options, setObject: &*prompt, forKey: key];
        }
        let _ = unsafe { AXIsProcessTrustedWithOptions((&*options as *const _) as *const c_void) };
    }

    unsafe fn cfstring_as_object<T: Message + ?Sized>(value: &'static T) -> &'static AnyObject {
        unsafe { &*(value as *const _ as *const AnyObject) }
    }

    const K_CG_EVENT_CENTER_MOUSE_DOWN: u32 = K_CG_EVENT_OTHER_MOUSE_DOWN;
    const K_CG_EVENT_CENTER_MOUSE_UP: u32 = K_CG_EVENT_OTHER_MOUSE_UP;

    fn mapped_point(frame: &PointerInputFrame, target: Option<PointerTargetGeometry>) -> CGPoint {
        let point = map_pointer_point(frame, target);
        CGPoint {
            x: point.x,
            y: point.y,
        }
    }

    fn keycode_from_key(key: &str) -> Option<u16> {
        match key {
            " " => Some(49),
            _ => None,
        }
    }

    fn keycode_from_dom_code(code: &str) -> Option<u16> {
        match code {
            "KeyA" => Some(0),
            "KeyS" => Some(1),
            "KeyD" => Some(2),
            "KeyF" => Some(3),
            "KeyH" => Some(4),
            "KeyG" => Some(5),
            "KeyZ" => Some(6),
            "KeyX" => Some(7),
            "KeyC" => Some(8),
            "KeyV" => Some(9),
            "KeyB" => Some(11),
            "KeyQ" => Some(12),
            "KeyW" => Some(13),
            "KeyE" => Some(14),
            "KeyR" => Some(15),
            "KeyY" => Some(16),
            "KeyT" => Some(17),
            "Digit1" => Some(18),
            "Digit2" => Some(19),
            "Digit3" => Some(20),
            "Digit4" => Some(21),
            "Digit6" => Some(22),
            "Digit5" => Some(23),
            "Equal" => Some(24),
            "Digit9" => Some(25),
            "Digit7" => Some(26),
            "Minus" => Some(27),
            "Digit8" => Some(28),
            "Digit0" => Some(29),
            "BracketRight" => Some(30),
            "KeyO" => Some(31),
            "KeyU" => Some(32),
            "BracketLeft" => Some(33),
            "KeyI" => Some(34),
            "KeyP" => Some(35),
            "Enter" => Some(36),
            "KeyL" => Some(37),
            "KeyJ" => Some(38),
            "Quote" => Some(39),
            "KeyK" => Some(40),
            "Semicolon" => Some(41),
            "Backslash" => Some(42),
            "Comma" => Some(43),
            "Slash" => Some(44),
            "KeyN" => Some(45),
            "KeyM" => Some(46),
            "Period" => Some(47),
            "Tab" => Some(48),
            "Space" => Some(49),
            "Backquote" => Some(50),
            "Backspace" => Some(51),
            "Escape" => Some(53),
            "MetaLeft" | "MetaRight" => Some(55),
            "ShiftLeft" => Some(56),
            "CapsLock" => Some(57),
            "AltLeft" | "AltRight" => Some(58),
            "ControlLeft" | "ControlRight" => Some(59),
            "ShiftRight" => Some(60),
            "ArrowLeft" => Some(123),
            "ArrowRight" => Some(124),
            "ArrowDown" => Some(125),
            "ArrowUp" => Some(126),
            _ => None,
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::{InputApplyOutcome, KeyInputFrame, PointerInputFrame, PointerTargetGeometry};

    pub(super) fn input_injection_available() -> bool {
        false
    }

    pub(super) fn request_input_injection_permission() -> bool {
        false
    }

    pub(super) fn apply_pointer_frame(
        _frame: &PointerInputFrame,
        _target: Option<PointerTargetGeometry>,
    ) -> InputApplyOutcome {
        InputApplyOutcome::rejected("platform_input_injection_unavailable")
    }

    pub(super) fn apply_key_frame(_frame: &KeyInputFrame) -> InputApplyOutcome {
        InputApplyOutcome::rejected("platform_input_injection_unavailable")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::persistence::resources::{ResourceBinding, ResourceEntry};

    #[test]
    fn parses_pointer_input_frame() {
        let frame = parse_input_frame(
            r#"{"type":"pointer","action":"move","x":10,"y":20,"normalized_x":0.5,"normalized_y":0.25}"#,
        )
        .unwrap();
        assert_eq!(frame.kind(), RemoteDesktopInputKind::Pointer);
    }

    #[test]
    fn rejects_out_of_range_normalized_pointer() {
        let err = parse_input_frame(
            r#"{"type":"pointer","action":"move","x":10,"y":20,"normalized_x":1.5}"#,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("normalized pointer coordinates"));
    }

    #[test]
    fn parses_key_input_frame() {
        let frame = parse_input_frame(r#"{"type":"key","action":"down","code":"KeyA"}"#).unwrap();
        assert_eq!(frame.kind(), RemoteDesktopInputKind::Key);
    }

    #[test]
    fn maps_window_relative_pointer_to_global_screen_point() {
        let entry = ResourceEntry {
            resource_ura: "easynet:///r/acme/resource/window.test".into(),
            owner_agent: "easynet:///r/acme/device/dev-1".into(),
            kind: ResourceType::Window,
            binding: ResourceBinding::LocalDevice,
            hardware_id: "window:macos:cgwindow:10:42".into(),
            display_name: "Cursor".into(),
            metadata: json!({
                "x": 100,
                "y": 200,
                "width": 800,
                "height": 600,
            }),
            first_seen_at: "2026-06-01T00:00:00Z".into(),
        };
        let policy = input_policy_for_entry(json!({"pointer_enabled": true}), &entry);
        let frame = match parse_input_frame(
            r#"{"type":"pointer","action":"move","x":0,"y":0,"normalized_x":0.5,"normalized_y":0.25}"#,
        )
        .unwrap()
        {
            RemoteDesktopInputFrame::Pointer(frame) => frame,
            _ => unreachable!(),
        };

        let point = map_pointer_point(&frame, pointer_target_from_policy(&policy));

        assert_eq!(point, MappedPointerPoint { x: 500.0, y: 350.0 });
    }

    #[test]
    fn maps_application_pointer_through_primary_window_bounds() {
        let entry = ResourceEntry {
            resource_ura: "easynet:///r/acme/resource/application.test".into(),
            owner_agent: "easynet:///r/acme/device/dev-1".into(),
            kind: ResourceType::Application,
            binding: ResourceBinding::LocalDevice,
            hardware_id: "application:macos:cgwindow:Cursor".into(),
            display_name: "Cursor".into(),
            metadata: json!({
                "primary_x": 300,
                "primary_y": 400,
                "primary_width": 1000,
                "primary_height": 500,
            }),
            first_seen_at: "2026-06-01T00:00:00Z".into(),
        };
        let policy = input_policy_for_entry(json!({"pointer_enabled": true}), &entry);
        let frame = match parse_input_frame(
            r#"{"type":"pointer","action":"down","x":250,"y":100,"target_width":500,"target_height":250}"#,
        )
        .unwrap()
        {
            RemoteDesktopInputFrame::Pointer(frame) => frame,
            _ => unreachable!(),
        };

        let point = map_pointer_point(&frame, pointer_target_from_policy(&policy));

        assert_eq!(point, MappedPointerPoint { x: 800.0, y: 600.0 });
    }
}
