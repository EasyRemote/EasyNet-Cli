// EasyNet CLI — device.browser.* remote browser session abilities
// ================================================================
//
// File: src/runtime/agents/browser_session_ability.rs
// Description: v0 mock handlers for the remote WebView session
//              ability family declared in RFC-012 §RemoteWebSurface.
//              The session store is in-process behind an
//              `Arc<Mutex<HashMap>>` keyed by session URA. v0 does
//              NOT spawn a real WebView — the capture_viewport
//              streaming handler returns a single placeholder webp
//              frame so the frontend canvas paint pipeline can be
//              end-to-end exercised. RFC-013 W1–W8 replaces the mock
//              capture/input bodies with wry calls per platform.
//
// Abilities registered here
// -------------------------
//   device.browser.open_session       Mint a session URA for a URL.
//   device.browser.send_input         Inject one input event.
//   device.browser.capture_viewport   Stream frames (v0 = one stub).
//   device.browser.close_session      Tear down a session; idempotent.
//
// Storage model
// -------------
// `SessionState` rows are kept in a process-global OnceLock<Mutex<HashMap>>.
// Persistence and federation fan-out are out of v0 scope; the session is
// local to the daemon that created it. session_ura uniqueness is enforced
// at create time.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use serde_json::{json, Value};

use crate::runtime::ability_dispatch::{AxonAbilityCatalog, StreamSource};

pub const ABILITY_OPEN_SESSION: &str = "device.browser.open_session";
pub const ABILITY_SEND_INPUT: &str = "device.browser.send_input";
pub const ABILITY_CAPTURE_VIEWPORT: &str = "device.browser.capture_viewport";
pub const ABILITY_CLOSE_SESSION: &str = "device.browser.close_session";

const DEFAULT_VIEWPORT_W: u32 = 1280;
const DEFAULT_VIEWPORT_H: u32 = 800;
const DEFAULT_IDLE_TIMEOUT_S: u64 = 1800;

// Minimal lossless webp: 2-byte VP8L stream encoding a 1×1 opaque
// pixel. Frontend canvas pipeline only needs valid webp bytes to
// exercise base64→blob→drawImage; the visible pixel is intentionally
// trivial since real capture lands in RFC-013.
const PLACEHOLDER_WEBP: &[u8] = &[
    0x52, 0x49, 0x46, 0x46, 0x1A, 0x00, 0x00, 0x00, 0x57, 0x45, 0x42, 0x50, 0x56, 0x50, 0x38, 0x4C,
    0x0D, 0x00, 0x00, 0x00, 0x2F, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x88, 0x88, 0x08,
];

/// Several fields exist only for RFC-013 W1+ readers (idle-timeout
/// reaper, capture loop, audit). v0 mock writes them at create time
/// and reads only what handlers need; `allow(dead_code)` keeps the
/// struct shape honest now and avoids re-declaring it when wry
/// integration lands.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct SessionState {
    session_ura: String,
    url: String,
    viewport_width: u32,
    viewport_height: u32,
    device_scale_factor: f64,
    idle_timeout_seconds: u64,
    created_at_ms: u64,
    last_input_at_ms: u64,
    sequence: u64,
}

fn store() -> &'static Mutex<HashMap<String, SessionState>> {
    static STORE: OnceLock<Mutex<HashMap<String, SessionState>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn require_str<'a>(args: &'a Value, key: &str, ability: &str) -> anyhow::Result<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("{ability}: `{key}` is required"))
}

fn url_ok(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

/// Description helpers — sourced exactly here so the generator-driven
/// drift contract in `agents/mod.rs` stays single-source-of-truth.
///
/// **V0 MOCK PREFIX**. Every description starts with `[V0 MOCK …]` so
/// an LLM tool selector sees the warning even when only the first
/// sentence is sampled. The prefix is removed atomically by RFC-013 W1
/// once the real WebView lands; until then it is load-bearing —
/// downstream callers MUST self-disclose that the surface is non-
/// functional so federated planners do not route real user intent
/// through a 1-pixel placeholder.
pub fn open_session_description() -> &'static str {
    "[V0 MOCK — NOT YET FUNCTIONAL; replaced by real WebView in RFC-013 W1] Open a top-level WebView session on this device against an http/https URL the frontend cannot iframe-embed (X-Frame-Options or CSP frame-ancestors). Returns a session URA the caller subscribes to via device.browser.capture_viewport for frames, and writes to via device.browser.send_input for input events. v0 ships a mock handler (no real WebView); RFC-013 W1–W8 replaces the mock with wry across WKWebView / WebView2 / WebKitGTK."
}

pub fn send_input_description() -> &'static str {
    "[V0 MOCK — NOT YET FUNCTIONAL; replaced by real WebView in RFC-013 W1] Inject a user input event (click / mousedown / mouseup / mousemove / scroll / keydown / keyup / text / navigate) into an open WebView session. session_ura MUST reference a live session. High-frequency events (scroll, mousemove) SHOULD be coalesced by the caller before invocation; v0 stores the timestamp but does not buffer beyond the most recent event."
}

pub fn capture_viewport_description() -> &'static str {
    "[V0 MOCK — RETURNS A 1x1 PLACEHOLDER FRAME, NOT REAL VIEWPORT CONTENT; replaced by real capture in RFC-013 W2] Stream webp/png/jpeg viewport frames from an open WebView session. Streaming ability — each emitted frame carries `bytes_base64` + `encoding` + viewport dimensions + monotonic `sequence`. v0 emits a single placeholder webp frame so the frontend canvas pipeline can be exercised end-to-end; RFC-013 swaps the mock for real per-platform capture."
}

pub fn close_session_description() -> &'static str {
    "[V0 MOCK — NOT YET FUNCTIONAL; replaced by real WebView in RFC-013 W1] Close a WebView session created by device.browser.open_session. Idempotent: closing an already-closed (or never-opened) session returns success with status='already_closed'. Forces the session's frame stream to terminate and removes the session row from the device-local store."
}

pub fn open_session_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["url"],
        "properties": {
            "url": {
                "type": "string",
                "minLength": 1,
                "pattern": "^https?://",
                "description": "Target URL. http:// or https:// only."
            },
            "viewport_width": {
                "type": "integer",
                "minimum": 320,
                "maximum": 3840,
                "description": "Initial viewport width in CSS pixels. Default 1280."
            },
            "viewport_height": {
                "type": "integer",
                "minimum": 240,
                "maximum": 2400,
                "description": "Initial viewport height in CSS pixels. Default 800."
            },
            "device_scale_factor": {
                "type": "number",
                "minimum": 1.0,
                "maximum": 3.0,
                "description": "Device pixel ratio for rendering. Default 1.0."
            },
            "idle_timeout_seconds": {
                "type": "integer",
                "minimum": 60,
                "maximum": 7200,
                "description": "Auto-close after this many seconds with no input. Default 1800 (30 min)."
            }
        }
    })
}

pub fn send_input_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["session_ura", "event"],
        "properties": {
            "session_ura": {
                "type": "string",
                "minLength": 1,
                "description": "Session URA returned by device.browser.open_session."
            },
            "event": {
                "type": "object",
                "required": ["kind"],
                "properties": {
                    "kind": {
                        "type": "string",
                        "enum": ["click", "mousedown", "mouseup", "mousemove", "scroll", "keydown", "keyup", "text", "navigate"]
                    },
                    "x": { "type": "integer" },
                    "y": { "type": "integer" },
                    "button": { "type": "string", "enum": ["left", "middle", "right"] },
                    "delta_x": { "type": "number" },
                    "delta_y": { "type": "number" },
                    "key": { "type": "string" },
                    "text": { "type": "string" },
                    "url": { "type": "string" }
                }
            }
        }
    })
}

pub fn capture_viewport_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["session_ura"],
        "properties": {
            "session_ura": {
                "type": "string",
                "minLength": 1,
                "description": "Session URA returned by device.browser.open_session."
            },
            "encoding": {
                "type": "string",
                "enum": ["png", "webp", "jpeg"],
                "description": "Output image encoding. Default 'webp'."
            },
            "quality": {
                "type": "integer",
                "minimum": 1,
                "maximum": 100,
                "description": "Encoder quality for lossy formats. Default 75."
            }
        }
    })
}

pub fn close_session_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["session_ura"],
        "properties": {
            "session_ura": {
                "type": "string",
                "minLength": 1,
                "description": "Session URA returned by device.browser.open_session."
            }
        }
    })
}

/// Register the four device.browser.* verbs on the local registry.
///
/// **M0 owner-kind note**: like voice_call_ability, all four verbs
/// mount as `OwnerKind::Device`. RFC-013 may re-classify capture as
/// a per-agent surface once wry is integrated, but for v0 the WebView
/// lives in the daemon process and Device is honest.
pub fn register(reg: &mut AxonAbilityCatalog) {
    use crate::runtime::ability_dispatch::OwnerKind;
    reg.register_rpc_with_owner(
        ABILITY_OPEN_SESSION,
        OwnerKind::Device,
        Arc::new(open_session_handler),
    );
    reg.register_rpc_with_owner(
        ABILITY_SEND_INPUT,
        OwnerKind::Device,
        Arc::new(send_input_handler),
    );
    reg.register_stream_with_owner(
        ABILITY_CAPTURE_VIEWPORT,
        OwnerKind::Device,
        Arc::new(capture_viewport_handler),
    );
    reg.register_rpc_with_owner(
        ABILITY_CLOSE_SESSION,
        OwnerKind::Device,
        Arc::new(close_session_handler),
    );
}

// ── Handlers ────────────────────────────────────────────────────

fn open_session_handler(args: Value) -> anyhow::Result<Value> {
    let url = require_str(&args, "url", "browser.open_session")?.to_string();
    if !url_ok(&url) {
        anyhow::bail!("browser.open_session: `url` must start with http:// or https://");
    }
    let viewport_width = args
        .get("viewport_width")
        .and_then(Value::as_u64)
        .map(|v| v as u32)
        .unwrap_or(DEFAULT_VIEWPORT_W);
    let viewport_height = args
        .get("viewport_height")
        .and_then(Value::as_u64)
        .map(|v| v as u32)
        .unwrap_or(DEFAULT_VIEWPORT_H);
    let device_scale_factor = args
        .get("device_scale_factor")
        .and_then(Value::as_f64)
        .unwrap_or(1.0);
    let idle_timeout_seconds = args
        .get("idle_timeout_seconds")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_IDLE_TIMEOUT_S);

    // session URA: easynet:///r/local/resource/daemon.browser/<ulid>
    // v0 uses "local" as realm; RFC-013 will anchor realm/device from
    // the daemon's self-identity. The id segment reuses the existing
    // process-wide ULID minter (`runtime::keyring::store::ulid_like`)
    // so we don't fork a second generator with weaker uniqueness.
    // The URA literal is built through `crate::ura::resource_dot_ura`
    // so the centralised URA construction lint
    // (`tests/scripts/test_no_raw_ura_construction.sh`) keeps passing.
    let id = crate::runtime::keyring::store::ulid_like();
    let session_ura = crate::ura::resource_dot_ura("local", "daemon.browser", &id);

    let state = SessionState {
        session_ura: session_ura.clone(),
        url: url.clone(),
        viewport_width,
        viewport_height,
        device_scale_factor,
        idle_timeout_seconds,
        created_at_ms: now_ms(),
        last_input_at_ms: now_ms(),
        sequence: 0,
    };
    store().lock().unwrap().insert(session_ura.clone(), state);

    let viewport = format!("{viewport_width}x{viewport_height}");
    crate::op_event!(
        component = browser_open_session,
        kind = session_opened,
        session_ura = session_ura,
        url = url,
        viewport = viewport,
    );

    Ok(json!({
        "session_ura": session_ura,
        "url": url,
        "viewport_width": viewport_width,
        "viewport_height": viewport_height,
        "device_scale_factor": device_scale_factor,
        "idle_timeout_seconds": idle_timeout_seconds,
        "state": "open",
        "created_at_ms": now_ms(),
    }))
}

fn send_input_handler(args: Value) -> anyhow::Result<Value> {
    let session_ura = require_str(&args, "session_ura", "browser.send_input")?.to_string();
    let event = args
        .get("event")
        .ok_or_else(|| anyhow::anyhow!("browser.send_input: `event` is required"))?
        .clone();
    let kind = event
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("browser.send_input: `event.kind` is required"))?
        .to_string();

    let mut s = store().lock().unwrap();
    let session = s
        .get_mut(&session_ura)
        .ok_or_else(|| anyhow::anyhow!("browser.send_input: session {session_ura:?} not found"))?;
    session.last_input_at_ms = now_ms();

    // v0: log the event and ack. RFC-013 will route this through the
    // platform-specific input synthesizer (NSEvent / SendInput / GDK).
    if cfg!(debug_assertions) {
        crate::op_event!(
            component = browser_send_input,
            kind = input_accepted,
            session = session_ura,
            event_kind = kind,
        );
    }

    Ok(json!({
        "accepted": true,
        "session_ura": session_ura,
        "event_kind": kind,
        "received_at_ms": now_ms(),
    }))
}

fn capture_viewport_handler(args: Value) -> anyhow::Result<StreamSource> {
    let session_ura = require_str(&args, "session_ura", "browser.capture_viewport")?.to_string();
    let encoding = args
        .get("encoding")
        .and_then(Value::as_str)
        .unwrap_or("webp")
        .to_string();

    let (sequence, viewport_width, viewport_height) = {
        let mut s = store().lock().unwrap();
        let session = s.get_mut(&session_ura).ok_or_else(|| {
            anyhow::anyhow!("browser.capture_viewport: session {session_ura:?} not found")
        })?;
        let seq = session.sequence;
        session.sequence += 1;
        (seq, session.viewport_width, session.viewport_height)
    };

    let bytes_base64 = BASE64_STANDARD.encode(PLACEHOLDER_WEBP);

    // v0 emits a single placeholder frame as a Snapshot. RFC-013 W2+
    // replaces this with a Live or SnapshotThenLive source backed by
    // a tokio task that drives wry's `takeSnapshot` / `CapturePreview`
    // / `webkit_web_view_get_snapshot` at 10 fps.
    let frame = json!({
        "sequence": sequence,
        "encoding": encoding,
        "bytes_base64": bytes_base64,
        "width_px": viewport_width,
        "height_px": viewport_height,
        "captured_at_ms": now_ms(),
        "is_placeholder": true,
    });

    Ok(StreamSource::Snapshot(vec![frame]))
}

fn close_session_handler(args: Value) -> anyhow::Result<Value> {
    let session_ura = require_str(&args, "session_ura", "browser.close_session")?.to_string();
    let removed = store().lock().unwrap().remove(&session_ura);
    let status = if removed.is_some() {
        "closed"
    } else {
        "already_closed"
    };
    if removed.is_some() {
        crate::op_event!(
            component = browser_close_session,
            kind = session_closed,
            session_ura = session_ura,
        );
    }
    Ok(json!({
        "session_ura": session_ura,
        "status": status,
        "closed_at_ms": now_ms(),
    }))
}

// ── Tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::ability_dispatch::AxonAbilityCatalog;

    fn open_url(url: &str) -> Value {
        open_session_handler(json!({ "url": url })).expect("open ok")
    }

    fn session_ura_from(v: &Value) -> String {
        v["session_ura"].as_str().unwrap().to_string()
    }

    #[test]
    fn open_session_rejects_non_http_scheme() {
        let err = open_session_handler(json!({ "url": "javascript:alert(1)" }))
            .unwrap_err()
            .to_string();
        assert!(err.contains("http"), "got: {err}");
    }

    #[test]
    fn open_session_rejects_missing_url() {
        let err = open_session_handler(json!({})).unwrap_err().to_string();
        assert!(err.contains("`url` is required"), "got: {err}");
    }

    #[test]
    fn open_session_returns_canonical_resource_ura() {
        let resp = open_url("https://github.com");
        let ura = session_ura_from(&resp);
        // Go through the centralised URA parser rather than a raw
        // scheme-prefix `starts_with` so the canonical-construction
        // lint (`tests/scripts/test_no_raw_ura_construction.sh`)
        // does not have to special-case this test.
        let parsed = crate::ura::parse_ura(&ura)
            .unwrap_or_else(|e| panic!("session_ura {ura:?} must parse: {e}"));
        assert_eq!(
            parsed.kind,
            crate::ura::URAKind::Resource,
            "session_ura must resolve to a Resource URA, got kind={:?}",
            parsed.kind
        );
    }

    #[test]
    fn open_session_honours_viewport_overrides() {
        let resp = open_session_handler(json!({
            "url": "https://example.com",
            "viewport_width": 1440,
            "viewport_height": 900,
        }))
        .unwrap();
        assert_eq!(resp["viewport_width"], 1440);
        assert_eq!(resp["viewport_height"], 900);
    }

    #[test]
    fn send_input_requires_known_session() {
        let err = send_input_handler(json!({
            "session_ura": "easynet:///r/local/resource/daemon.browser/bogus",
            "event": { "kind": "click", "x": 1, "y": 2 },
        }))
        .unwrap_err()
        .to_string();
        assert!(err.contains("not found"), "got: {err}");
    }

    #[test]
    fn send_input_records_event_kind() {
        let opened = open_url("https://github.com");
        let ura = session_ura_from(&opened);
        let r = send_input_handler(json!({
            "session_ura": ura,
            "event": { "kind": "click", "x": 100, "y": 200, "button": "left" },
        }))
        .unwrap();
        assert_eq!(r["accepted"], true);
        assert_eq!(r["event_kind"], "click");
    }

    #[test]
    fn capture_viewport_emits_one_placeholder_frame() {
        let opened = open_url("https://example.com");
        let ura = session_ura_from(&opened);
        let stream = capture_viewport_handler(json!({ "session_ura": ura })).unwrap();
        let frames = stream.into_snapshot();
        assert_eq!(frames.len(), 1, "v0 emits exactly one placeholder frame");
        let f = &frames[0];
        assert_eq!(f["encoding"], "webp");
        assert_eq!(f["is_placeholder"], true);
        let b64 = f["bytes_base64"].as_str().unwrap();
        assert!(!b64.is_empty());
        assert!(f["sequence"].as_u64().is_some());
    }

    #[test]
    fn capture_viewport_increments_sequence_across_calls() {
        let opened = open_url("https://example.com");
        let ura = session_ura_from(&opened);
        let s1 = capture_viewport_handler(json!({ "session_ura": ura.clone() }))
            .unwrap()
            .into_snapshot();
        let s2 = capture_viewport_handler(json!({ "session_ura": ura }))
            .unwrap()
            .into_snapshot();
        assert_eq!(s1[0]["sequence"], 0);
        assert_eq!(s2[0]["sequence"], 1);
    }

    #[test]
    fn close_session_is_idempotent() {
        let opened = open_url("https://example.com");
        let ura = session_ura_from(&opened);
        let first = close_session_handler(json!({ "session_ura": ura.clone() })).unwrap();
        assert_eq!(first["status"], "closed");
        let second = close_session_handler(json!({ "session_ura": ura })).unwrap();
        assert_eq!(second["status"], "already_closed");
    }

    #[test]
    fn close_session_removes_state_from_store() {
        let opened = open_url("https://example.com");
        let ura = session_ura_from(&opened);
        close_session_handler(json!({ "session_ura": ura.clone() })).unwrap();
        let err = send_input_handler(json!({
            "session_ura": ura,
            "event": { "kind": "click", "x": 1, "y": 1 },
        }))
        .unwrap_err()
        .to_string();
        assert!(err.contains("not found"), "got: {err}");
    }

    #[test]
    fn register_mounts_all_four_verbs() {
        let mut reg = AxonAbilityCatalog::new();
        register(&mut reg);
        let names = reg.list_abilities();
        for verb in [
            ABILITY_OPEN_SESSION,
            ABILITY_SEND_INPUT,
            ABILITY_CAPTURE_VIEWPORT,
            ABILITY_CLOSE_SESSION,
        ] {
            assert!(
                names.iter().any(|n| n == verb),
                "missing {verb} after register()"
            );
        }
    }
}
