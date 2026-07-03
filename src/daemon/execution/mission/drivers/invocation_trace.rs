// EasyNet CLI — Driver Invocation Trace Metadata
// ==============================================
//
// File: src/daemon/execution/mission/drivers/invocation_trace.rs
// Description: shared parser for EasyNet-owned invocation trace
//              metadata that LLM runtimes surface through tool
//              result text. This module is deliberately scoped to
//              drivers: it translates observed CLI/MCP stream
//              payloads into dispatch observability fields, and is
//              not an Axon protocol parser.
//
// Boundary: Axon owns the signed Invocation and receipt semantics;
//           EasyNet-Cli daemon owns the MCP projection and product
//           observability envelope. The trace object parsed here is
//           an observability echo, not a source of protocol truth.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use serde_json::Value;

use crate::daemon::execution::mission::dispatch::ToolCall;

/// Reserved payload key used by EasyNet MCP projection to attach
/// daemon invocation identity to an otherwise ordinary tool result.
///
/// This is intentionally narrower than the `x-easynet` descriptor
/// metadata key used in MCP tool specs. Tool results are arbitrary
/// ability payloads, so a generic `x-easynet` field could collide
/// with business data. The `*-invocation` suffix names the exact
/// contract this parser consumes.
pub(crate) const TRACE_METADATA_KEY: &str = "x-easynet-invocation";

/// Server name used by Codex MCP events for the daemon-hosted
/// EasyNet MCP provider.
pub(crate) const EASYNET_MCP_SERVER: &str = "easynet";

/// Invocation identity fields extracted from an EasyNet MCP result.
///
/// Invariant 1: every field is optional because runtimes and daemon
/// paths may surface only a partial observability echo.
/// Invariant 2: this value must only be applied to tool calls that
/// were already identified as EasyNet MCP calls by the driver; a
/// random third-party tool returning the same key is not trusted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct InvocationTraceMetadata {
    pub(crate) ability: Option<String>,
    pub(crate) mcp_tool_name: Option<String>,
    pub(crate) request_id: Option<String>,
    pub(crate) ability_ura: Option<String>,
    pub(crate) invocation_ura: Option<String>,
    pub(crate) caller_ura: Option<String>,
    pub(crate) callee_ura: Option<String>,
    pub(crate) subject_ura: Option<String>,
}

/// Parse EasyNet invocation trace metadata from the text forms
/// emitted by Claude Code and Codex tool-result streams. Accepts:
///
/// - a JSON object containing the trace metadata key;
/// - a JSON string whose contents are that object;
/// - an MCP content envelope containing text parts.
pub(crate) fn parse_invocation_trace_metadata(text: &str) -> Option<InvocationTraceMetadata> {
    let value = parse_tool_result_json(text)?;
    let meta = value.get(TRACE_METADATA_KEY)?.as_object()?;
    Some(InvocationTraceMetadata {
        ability: json_string(meta.get("ability")),
        mcp_tool_name: json_string(meta.get("mcp_tool")),
        request_id: json_string(meta.get("request_id")),
        ability_ura: json_string(meta.get("ability_ura")),
        invocation_ura: json_string(meta.get("invocation_ura")),
        caller_ura: json_string(meta.get("caller_ura")),
        callee_ura: json_string(meta.get("callee_ura")),
        subject_ura: json_string(meta.get("subject_ura")),
    })
}

/// Convert tool result text into JSON when possible, preserving the
/// original string when the result is plain text.
pub(crate) fn text_to_json_value(text: &str) -> Value {
    serde_json::from_str::<Value>(text).unwrap_or_else(|_| Value::String(text.to_string()))
}

/// Apply EasyNet trace metadata to the matching MCP tool call.
///
/// The merge is intentionally gated on an existing EasyNet MCP marker
/// (`mcp_tool_name.is_some()`). Third-party tool output can contain a
/// spoofed `x-easynet-invocation` object, but it cannot cause a random
/// non-EasyNet tool call to gain invocation URAs.
pub(crate) fn apply_tool_result_meta(
    calls: &mut [ToolCall],
    tool_use_id: Option<&str>,
    meta: InvocationTraceMetadata,
) {
    let target = if let Some(id) = tool_use_id {
        calls
            .iter_mut()
            .rev()
            .find(|call| call.tool_use_id.as_deref() == Some(id) && call.mcp_tool_name.is_some())
    } else {
        calls.iter_mut().rev().find(|call| {
            call.invocation_ura.is_none()
                && call.mcp_tool_name.as_deref().is_some_and(|name| {
                    meta.mcp_tool_name.as_deref() == Some(name)
                        || meta.ability.as_deref() == Some(name)
                })
        })
    };
    let Some(call) = target else {
        return;
    };
    if let Some(ability) = meta.ability {
        call.ability = ability;
    }
    if meta.mcp_tool_name.is_some() {
        call.mcp_tool_name = meta.mcp_tool_name;
    }
    call.request_id = meta.request_id.or(call.request_id.take());
    call.ability_ura = meta.ability_ura.or(call.ability_ura.take());
    call.invocation_ura = meta.invocation_ura.or(call.invocation_ura.take());
    call.caller_ura = meta.caller_ura.or(call.caller_ura.take());
    call.callee_ura = meta.callee_ura.or(call.callee_ura.take());
    call.subject_ura = meta.subject_ura.or(call.subject_ura.take());
}

fn parse_tool_result_json(text: &str) -> Option<Value> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let outer = serde_json::from_str::<Value>(trimmed).ok()?;
    if outer.get(TRACE_METADATA_KEY).is_some() {
        return Some(outer);
    }
    if let Some(inner) = outer.as_str() {
        return serde_json::from_str::<Value>(inner).ok();
    }
    let inner = outer
        .get("content")
        .and_then(Value::as_array)?
        .iter()
        .find_map(|part| part.get("text").and_then(Value::as_str))?;
    serde_json::from_str::<Value>(inner).ok()
}

fn json_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_direct_and_nested_trace_metadata() {
        let direct = parse_invocation_trace_metadata(
        r#"{"ok":true,"x-easynet-invocation":{"ability":"demo.weather","mcp_tool":"demo_weather"}}"#,
        )
        .expect("direct metadata");
        assert_eq!(direct.ability.as_deref(), Some("demo.weather"));
        assert_eq!(direct.mcp_tool_name.as_deref(), Some("demo_weather"));

        let nested = parse_invocation_trace_metadata(
            r#"{"content":[{"type":"text","text":"{\"x-easynet-invocation\":{\"request_id\":\"req-1\"}}"}]}"#,
        )
        .expect("nested metadata");
        assert_eq!(nested.request_id.as_deref(), Some("req-1"));
    }
}
