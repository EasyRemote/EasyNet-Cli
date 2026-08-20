//! Browser ability descriptor source.
//! ==================================
//!
//! File: plugins/browser/src/schema.rs
//! Description: Public ability descriptions and strict invocation schemas.
//!
//! Protocol Responsibility:
//! - Declare the governed browser surface consumed by Axon descriptor policy.
//!
//! Implementation Approach:
//! - Keep descriptions and JSON schemas as compiled functions projected into
//!   checked-in provider-backed descriptors.
//!
//! Usage Contract:
//! - Registration is the only consumer; checked-in TOML must match exactly.
//!
//! Architectural Position:
//! - Browser plugin public contract metadata.

use serde_json::{json, Value};

use super::constants::{
    MAX_BROWSER_OPTION_BYTES, MAX_INPUT_TEXT_BYTES, MAX_KEY_BYTES, MAX_SELECTOR_BYTES,
    MAX_URL_BYTES,
};

pub fn open_session_description() -> &'static str {
    "Open a real Chrome/Chromium page target governed by the EasyNet browser plugin. Headed mode is the default so a human and an agent can operate the same page. The plugin launches Chrome with an isolated non-default profile or connects to an explicit loopback CDP endpoint, negotiates the running protocol version, and returns a resource URA used as the subject of subsequent browser invocations."
}

pub fn show_session_description() -> &'static str {
    "Read the bounded lifecycle and protocol readiness projection for a browser session. The invocation subject MUST be the session resource URA and the caller MUST be the session creator. CDP endpoints, filesystem profile paths, cookies, and credentials are never returned."
}

pub fn send_input_description() -> &'static str {
    "Apply one human-scale navigation, pointer, keyboard, text, or form event to the real page target through Chrome DevTools Protocol. The plugin brings the target to the foreground before input so human-visible focus and agent focus stay aligned. The invocation subject MUST be the browser session resource URA."
}

pub fn capture_viewport_description() -> &'static str {
    "Emit a bounded finite sequence of real viewport frames from Page.startScreencast. Frames are acknowledged to Chrome before Axon downstream backpressure, and Page.stopScreencast is issued on every terminal path. This is an observation stream; interactive automation should use browser.attach_session."
}

pub fn attach_session_description() -> &'static str {
    "Attach one bounded Axon InvokeBidi channel to the browser session. Agent CDP commands may be sent individually or in batches of up to 32 that amortize Axon framing; commands share a bounded concurrent lane and may respond out of order by correlation id, while high-level input remains arrival-ordered. Responses, target events, ping, and detach control all travel as JSON frames inside the canonical Axon session. The plugin injects the bound CDP session id and rejects caller-supplied session routing or browser-wide destructive methods."
}

pub fn capture_page_description() -> &'static str {
    "Capture one structural snapshot of the session's current page as a self-contained MHTML document (Page.captureSnapshot). This is the structural observation plane: it returns the page's semantic state for agent reasoning, indexing, and archival — not pixels (browser.capture_viewport) and not live control (browser.attach_session). The invocation subject MUST be the session resource URA and the caller MUST be the session creator. Snapshots above the bounded size limit fail rather than truncate."
}

pub fn close_session_description() -> &'static str {
    "Idempotently close a browser session. The target is closed, pending CDP calls are drained, an owned Chrome process is terminated, and session/profile capacity is released. A caller-supplied existing browser process is never terminated. The invocation subject MUST be the browser session resource URA."
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
                "maxLength": MAX_URL_BYTES,
                "pattern": "^https?://",
                "description": "Initial page URL. http:// and https:// only."
            },
            "headless": {
                "type": "boolean",
                "description": "Launch without a visible window. Defaults to false."
            },
            "cdp_endpoint": {
                "type": "string",
                "minLength": 1,
                "maxLength": MAX_BROWSER_OPTION_BYTES,
                "description": "Existing loopback CDP HTTP or browser WebSocket endpoint."
            },
            "executable_path": {
                "type": "string",
                "minLength": 1,
                "maxLength": MAX_BROWSER_OPTION_BYTES,
                "description": "Explicit local Chrome/Chromium executable path."
            },
            "profile": {
                "type": "string",
                "pattern": "^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$",
                "description": "Named persistent profile under the EasyNet browser state root."
            },
            "viewport_width": {
                "type": "integer",
                "minimum": 320,
                "maximum": 3840
            },
            "viewport_height": {
                "type": "integer",
                "minimum": 240,
                "maximum": 2400
            },
            "idle_timeout_seconds": {
                "type": "integer",
                "minimum": 60,
                "maximum": 7200
            }
        }
    })
}

pub fn show_session_input_schema() -> Value {
    empty_object_schema()
}

pub fn send_input_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["event"],
        "properties": {
            "event": input_event_schema()
        }
    })
}

pub fn capture_viewport_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "format": {
                "type": "string",
                "enum": ["jpeg", "png"]
            },
            "quality": {
                "type": "integer",
                "minimum": 0,
                "maximum": 100
            },
            "max_width": {
                "type": "integer",
                "minimum": 1,
                "maximum": 7680
            },
            "max_height": {
                "type": "integer",
                "minimum": 1,
                "maximum": 4320
            },
            "max_frames": {
                "type": "integer",
                "minimum": 1,
                "maximum": 300
            },
            "timeout_seconds": {
                "type": "integer",
                "minimum": 1,
                "maximum": 120
            }
        }
    })
}

pub fn attach_session_input_schema() -> Value {
    empty_object_schema()
}

pub fn capture_page_input_schema() -> Value {
    empty_object_schema()
}

pub fn close_session_input_schema() -> Value {
    empty_object_schema()
}

fn empty_object_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false
    })
}

fn input_event_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["kind"],
        "properties": {
            "kind": {
                "type": "string",
                "enum": ["navigate", "click", "mouse", "scroll", "keydown", "keyup", "text", "fill"]
            },
            "url": {"type": "string", "maxLength": MAX_URL_BYTES, "pattern": "^https?://"},
            "selector": {"type": "string", "minLength": 1, "maxLength": MAX_SELECTOR_BYTES},
            "value": {"type": "string", "maxLength": MAX_INPUT_TEXT_BYTES},
            "text": {"type": "string", "maxLength": MAX_INPUT_TEXT_BYTES},
            "x": {"type": "number"},
            "y": {"type": "number"},
            "delta_x": {"type": "number"},
            "delta_y": {"type": "number"},
            "button": {"type": "string", "enum": ["none", "left", "middle", "right"]},
            "action": {"type": "string", "enum": ["move", "down", "up"]},
            "key": {"type": "string", "minLength": 1, "maxLength": MAX_KEY_BYTES}
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_schema_defaults_to_governed_fields_only() {
        let schema = open_session_input_schema();
        let properties = schema["properties"].as_object().expect("properties");
        assert!(properties.contains_key("url"));
        assert!(properties.contains_key("cdp_endpoint"));
        assert!(!properties.contains_key("session_ura"));
    }

    #[test]
    fn attach_schema_does_not_accept_cdp_session_routing() {
        assert_eq!(attach_session_input_schema()["additionalProperties"], false);
    }

    #[test]
    fn public_string_fields_have_explicit_semantic_bounds() {
        let open = open_session_input_schema();
        assert_eq!(open["properties"]["url"]["maxLength"], MAX_URL_BYTES);
        assert_eq!(
            open["properties"]["cdp_endpoint"]["maxLength"],
            MAX_BROWSER_OPTION_BYTES
        );

        let input = send_input_input_schema();
        let event = &input["properties"]["event"]["properties"];
        assert_eq!(event["selector"]["maxLength"], MAX_SELECTOR_BYTES);
        assert_eq!(event["text"]["maxLength"], MAX_INPUT_TEXT_BYTES);
        assert_eq!(event["key"]["maxLength"], MAX_KEY_BYTES);
    }
}
