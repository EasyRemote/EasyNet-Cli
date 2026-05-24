// EasyNet CLI — Streamable HTTP / SSE parsers
// ===========================================
//
// File: src/runtime/execution/mcp_client/http/sse.rs
//
// Server-Sent Events parsing for the MCP 2025-06-18 Streamable HTTP
// transport. Two surfaces:
//
//   * [`parse_sse_body`] — used by the POST response path, where a
//     terminal JSON-RPC response is REQUIRED and intervening
//     `notifications/*` frames are captured for sink routing.
//
//   * [`parse_one_sse_event`] + [`find_event_terminator`] — used by
//     the GET listener channel, which never sees JSON-RPC responses
//     (those flow back through POST); only `notifications/*` frames
//     are surfaced, plus the SSE `retry:` hint.
//
// Both share the spec-conformant SSE wire shape (LF/CRLF agnostic,
// `data:`/`id:`/`event:`/`retry:` fields, `:`-prefixed comments) and
// the JSON-RPC discriminator (`{id, result|error}` vs `{method}`
// without `id`).
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use anyhow::{anyhow, Context};
use serde_json::Value;

use crate::runtime::execution::mcp_client::ObservedNotification;

#[derive(Debug)]
pub(super) struct SseParseResult {
    /// JSON-RPC response body (bytes). The LAST `data:` event whose
    /// payload looks like a JSON-RPC response (`id` + `result`/`error`).
    /// MCP spec REQUIRES a stream to terminate with such a frame; if
    /// none was seen the parse fails.
    pub(super) response: Vec<u8>,
    /// Every JSON-RPC notification (`{jsonrpc, method, params}` with
    /// no `id`) seen in the stream, in arrival order. Empty when the
    /// upstream did not emit progress.
    pub(super) notifications: Vec<ObservedNotification>,
    /// Latest `id:` field observed across the parsed stream, if any.
    /// Threaded back into `HttpConnection.last_event_id` so a
    /// subsequent reconnect (POST or GET listener) can replay it
    /// via the `Last-Event-Id` header per spec §"Resumability
    /// and Retries".
    pub(super) last_event_id: Option<String>,
}

/// Parse an SSE-encoded MCP response body. Splits intervening
/// `notifications/*` frames from the final JSON-RPC response.
///
/// SSE wire format per [HTML Living Standard §Server-sent events]:
///   * Each event is a block of `field: value\n` lines.
///   * Events are separated by a blank line (`\n\n`).
///   * `data:` lines within one event are joined by `\n`.
///   * Comment lines start with `:` and are ignored.
///   * `id:`, `event:`, `retry:` fields exist but MCP does not use
///     them for response framing; we accept and ignore them so a
///     spec-compliant server that emits them does not break parsing.
///
/// Notification routing: every JSON object with a `"method"` field
/// and no `"id"` is captured as an `ObservedNotification`. Per MCP
/// 2025-06-18 §"Streamable HTTP" the server MAY interleave
/// `notifications/progress`, `notifications/tools/list_changed`,
/// `notifications/message`, etc. before the final response frame —
/// they are first-class output of this parser rather than being
/// silently dropped.
pub(super) fn parse_sse_body(body: &[u8]) -> anyhow::Result<SseParseResult> {
    let text = std::str::from_utf8(body).context("SSE body is not valid UTF-8")?;
    let mut last_response: Option<Vec<u8>> = None;
    let mut notifications: Vec<ObservedNotification> = Vec::new();
    // Per HTML living spec §"event stream model" the `id:` field is
    // **stream-level**: once an event with an id arrives, the user
    // agent's "last event id" is that value for every subsequent
    // reconnect attempt, even if later events have no id of their
    // own. We mirror that — record the latest id we see, regardless
    // of frame kind. Round-2 `Last-Event-Id` replay reads this.
    let mut last_event_id: Option<String> = None;

    // Split on blank-line separators (LF/CRLF agnostic). Normalise
    // CRLF → LF first to keep the splitter simple. The SSE spec says
    // a blank line is a line containing only the line terminator.
    let normalised = text.replace("\r\n", "\n");
    for block in normalised.split("\n\n") {
        let mut data_chunks: Vec<&str> = Vec::new();
        for line in block.lines() {
            // Comments — `:` followed by anything (including nothing,
            // which is a heartbeat). Per spec, ignore.
            if line.starts_with(':') {
                continue;
            }
            // SSE field syntax: `field`, `field: value`, or
            // `field:value`. We consume `data` for payloads and
            // `id` for resumption; `event:` and `retry:` are still
            // spec-legal but MCP-irrelevant here. (`retry:` is
            // honoured by the listener loop, not the parser.)
            let (field, value) = match line.split_once(':') {
                Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
                None => (line, ""),
            };
            if field == "data" {
                data_chunks.push(value);
            } else if field == "id" {
                // Per spec, an empty id resets to "no last event id".
                last_event_id = if value.is_empty() {
                    None
                } else {
                    Some(value.to_string())
                };
            }
        }
        if data_chunks.is_empty() {
            continue;
        }
        let payload = data_chunks.join("\n");
        // Per SSE, non-JSON `data:` lines are spec-legal but MCP
        // never produces them — skip silently.
        let parsed: Value = match serde_json::from_str(&payload) {
            Ok(v) => v,
            Err(_) => continue,
        };

        // JSON-RPC response: has `id` AND (`result` or `error`).
        if parsed.get("id").is_some()
            && (parsed.get("result").is_some() || parsed.get("error").is_some())
        {
            last_response = Some(payload.into_bytes());
            continue;
        }

        // JSON-RPC notification: has `method` AND no `id`.
        // The `params` field is optional in JSON-RPC; when absent we
        // surface a JSON `null` so the sink contract is uniform.
        if parsed.get("id").is_none() {
            if let Some(method) = parsed.get("method").and_then(Value::as_str) {
                notifications.push(ObservedNotification {
                    method: method.to_string(),
                    params: parsed.get("params").cloned().unwrap_or(Value::Null),
                });
            }
        }
    }

    let response = last_response.ok_or_else(|| {
        anyhow!(
            "SSE body had no JSON-RPC response frame (body len = {}, notifications observed = {})",
            body.len(),
            notifications.len()
        )
    })?;
    Ok(SseParseResult {
        response,
        notifications,
        last_event_id,
    })
}

/// Locate the byte boundary between two SSE events inside a streaming
/// receive buffer. Handles both `\n\n` and `\r\n\r\n` terminators.
///
/// Returns (event_body_end, terminator_len) — `buffer[..idx]` is
/// the event body and the next `terminator_len` bytes are the
/// separator to be discarded. Looks for `\r\n\r\n` first so the
/// CRLF form isn't truncated to a bare `\n\n` match.
pub(super) fn find_event_terminator(buf: &[u8]) -> Option<(usize, usize)> {
    let lf_lf = buf.windows(2).position(|w| w == b"\n\n");
    let crlf_crlf = buf.windows(4).position(|w| w == b"\r\n\r\n");
    match (lf_lf, crlf_crlf) {
        (Some(a), Some(b)) if b <= a => Some((b, 4)),
        (Some(a), _) => Some((a, 2)),
        (None, Some(b)) => Some((b, 4)),
        (None, None) => None,
    }
}

#[derive(Debug, Default)]
pub(super) struct ParsedSseEvent {
    pub(super) notifications: Vec<ObservedNotification>,
    pub(super) id: Option<String>,
    pub(super) retry_ms: Option<u64>,
}

/// Parse one SSE event's bytes into the listener's view. Stricter
/// than `parse_sse_body` — listener events are never JSON-RPC
/// responses (those return through POST), so anything with an
/// `id` JSON field is silently dropped. Only `notifications/*`
/// frames flow to the sink.
pub(super) fn parse_one_sse_event(event_bytes: &[u8]) -> anyhow::Result<ParsedSseEvent> {
    let text = std::str::from_utf8(event_bytes).context("SSE event not valid UTF-8")?;
    let normalised = text.replace("\r\n", "\n");
    let mut parsed = ParsedSseEvent::default();
    let mut data_chunks: Vec<&str> = Vec::new();
    for line in normalised.lines() {
        if line.starts_with(':') {
            continue;
        }
        let (field, value) = match line.split_once(':') {
            Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
            None => (line, ""),
        };
        match field {
            "data" => data_chunks.push(value),
            "id" => {
                parsed.id = if value.is_empty() {
                    None
                } else {
                    Some(value.to_string())
                };
            }
            "retry" => {
                if let Ok(ms) = value.parse::<u64>() {
                    parsed.retry_ms = Some(ms);
                }
            }
            _ => {}
        }
    }
    if data_chunks.is_empty() {
        return Ok(parsed);
    }
    let payload = data_chunks.join("\n");
    let value: Value = match serde_json::from_str(&payload) {
        Ok(v) => v,
        Err(_) => return Ok(parsed),
    };
    if value.get("id").is_none() {
        if let Some(method) = value.get("method").and_then(Value::as_str) {
            parsed.notifications.push(ObservedNotification {
                method: method.to_string(),
                params: value.get("params").cloned().unwrap_or(Value::Null),
            });
        }
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sse_body_picks_last_response_and_captures_notifications() {
        let body = b"event: progress\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\",\"params\":{\"progress\":0.5}}\n\n\
                     data: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"ok\":true}}\n\n";
        let parsed = parse_sse_body(body).expect("parse");
        let resp: Value = serde_json::from_slice(&parsed.response).unwrap();
        assert_eq!(resp["id"], 1);
        assert_eq!(resp["result"]["ok"], true);
        assert_eq!(parsed.notifications.len(), 1);
        assert_eq!(parsed.notifications[0].method, "notifications/progress");
    }

    #[test]
    fn parse_sse_body_captures_every_notification_in_order() {
        let body = b"data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\",\"params\":{\"progress\":0.25}}\n\n\
                     data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\",\"params\":{\"progress\":0.75}}\n\n\
                     data: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n\n";
        let parsed = parse_sse_body(body).expect("parse");
        assert_eq!(parsed.notifications.len(), 2);
        assert_eq!(parsed.notifications[0].params["progress"], 0.25);
        assert_eq!(parsed.notifications[1].params["progress"], 0.75);
    }

    #[test]
    fn parse_sse_body_handles_multiline_data_events() {
        let body = b"data: {\"jsonrpc\":\"2.0\",\n\
                     data: \"id\":1,\n\
                     data: \"result\":{\"line\":\"joined\"}\n\
                     data: }\n\n";
        let parsed = parse_sse_body(body).expect("parse");
        let resp: Value = serde_json::from_slice(&parsed.response).unwrap();
        assert_eq!(resp["result"]["line"], "joined");
    }

    #[test]
    fn parse_sse_body_handles_crlf_line_endings() {
        let body = b"data: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"crlf\":true}}\r\n\r\n";
        let parsed = parse_sse_body(body).expect("parse");
        let resp: Value = serde_json::from_slice(&parsed.response).unwrap();
        assert_eq!(resp["result"]["crlf"], true);
    }

    #[test]
    fn parse_sse_body_ignores_comments_and_non_data_fields() {
        let body = b": this is a heartbeat\n\
                     event: progress\n\
                     id: 42\n\
                     data: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n\n";
        let parsed = parse_sse_body(body).expect("parse");
        let resp: Value = serde_json::from_slice(&parsed.response).unwrap();
        assert_eq!(resp["id"], 1);
        // The id: line is recorded as last_event_id.
        assert_eq!(parsed.last_event_id.as_deref(), Some("42"));
    }

    #[test]
    fn parse_sse_body_errors_when_only_notifications_seen() {
        let body = b"data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\",\"params\":{}}\n\n";
        let err = parse_sse_body(body).expect_err("must require terminal response");
        assert!(format!("{err}").contains("no JSON-RPC response"));
    }

    #[test]
    fn parse_sse_body_skips_non_json_data_events() {
        // Spec-legal non-JSON `data:` line — MCP never emits these,
        // we silently skip rather than bailing.
        let body = b"data: hello world\n\n\
                     data: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n\n";
        let parsed = parse_sse_body(body).expect("parse");
        let resp: Value = serde_json::from_slice(&parsed.response).unwrap();
        assert_eq!(resp["id"], 1);
    }

    #[test]
    fn parse_sse_body_ignores_data_event_with_neither_id_nor_method() {
        // A JSON object that has neither `id` (response) nor
        // `method` (notification) is dropped — not a notification,
        // not a response.
        let body = b"data: {\"jsonrpc\":\"2.0\",\"orphan\":true}\n\n\
                     data: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n\n";
        let parsed = parse_sse_body(body).expect("parse");
        let resp: Value = serde_json::from_slice(&parsed.response).unwrap();
        assert_eq!(resp["id"], 1);
        assert!(parsed.notifications.is_empty());
    }

    #[test]
    fn parse_sse_body_resets_last_event_id_on_empty_id() {
        // SSE spec: an empty `id:` resets the stream's last event id.
        let body = b"id: 10\n\
                     data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\",\"params\":{}}\n\n\
                     id:\n\
                     data: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n\n";
        let parsed = parse_sse_body(body).expect("parse");
        assert!(parsed.last_event_id.is_none(), "empty id: must reset");
    }

    #[test]
    fn find_event_terminator_handles_both_line_endings() {
        assert_eq!(find_event_terminator(b"abc\n\nrest"), Some((3, 2)));
        assert_eq!(find_event_terminator(b"abc\r\n\r\nrest"), Some((3, 4)));
        assert_eq!(find_event_terminator(b"no terminator"), None);
        // When both forms could match, CRLF (4 bytes) wins if it
        // starts at or before the LF/LF position.
        assert_eq!(find_event_terminator(b"a\r\n\r\nb\n\nc"), Some((1, 4)));
    }

    #[test]
    fn parse_one_sse_event_captures_retry_hint() {
        let event = b"retry: 1500\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/tools/list_changed\",\"params\":{}}";
        let parsed = parse_one_sse_event(event).expect("parse");
        assert_eq!(parsed.retry_ms, Some(1500));
        assert_eq!(parsed.notifications.len(), 1);
        assert_eq!(
            parsed.notifications[0].method,
            "notifications/tools/list_changed"
        );
    }

    #[test]
    fn parse_one_sse_event_drops_json_rpc_responses_silently() {
        // Listener channel is for notifications only — a stray
        // response on the GET path is not the parser's problem to
        // surface, it just doesn't get routed to the sink.
        let event = b"data: {\"jsonrpc\":\"2.0\",\"id\":99,\"result\":{}}";
        let parsed = parse_one_sse_event(event).expect("parse");
        assert!(parsed.notifications.is_empty());
    }
}
