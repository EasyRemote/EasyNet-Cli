// EasyNet CLI — OpenAI Compatibility adapter ability
// ====================================================
//
// File: src/runtime/agents/openai_compat_ability.rs
// Description: hub-rooted abilities `01HUB.openai.chat_completions`
//              and `01HUB.openai.list_models` that project EasyNet
//              chat-base abilities through the OpenAI streaming
//              completion wire shape (RFC-006-C v0.1).
//
// Conformance: INV-1 (Adapter Purity), INV-2 (Capability-URI Key),
//              INV-3 (Filter Determinism), INV-4 (Auth Receipt
//              Trail).
//
// v0.1 simplifications:
//   - Non-streaming response only (single JSON, not SSE chunks).
//     Streaming added in v0.2 once the listener wires to a
//     ResponseStream sink.
//   - Tool-call surface dropped (tool_use frames in dispatch are
//     captured in operational receipts, not surfaced to client).
//   - Reasoning frames dropped (same reason).
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::{Arc, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::runtime::ability_dispatch::{LocalAbilityRegistry, LocalRpcHandler};
use crate::runtime::agents::api_key_ability;

/// Process-wide handle to the live ability registry, set at boot.
/// chat_completions reaches through it to invoke target chat-base
/// abilities without IPC self-loop.
static DISPATCH_HANDLE: OnceLock<Arc<OnceLock<Arc<LocalAbilityRegistry>>>> = OnceLock::new();

pub(crate) fn set_dispatch_handle(handle: Arc<OnceLock<Arc<LocalAbilityRegistry>>>) {
    let _ = DISPATCH_HANDLE.set(handle);
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn random_id(prefix: &str) -> String {
    use rand::RngCore;
    let mut buf = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut buf);
    format!("{prefix}{}", hex::encode(buf))
}

/// Tell whether an ability name is chat-base. v0.1 convention:
/// the ability has the form `<owner>.chat` — owner is a single
/// dot-free segment, then `.chat`. Names with extra interior
/// dots (e.g. `<user>.<project>.api.chat`, the api manifest
/// surface from RFC-006-B) are excluded so a project that
/// happens to have an `api/chat.toml` does NOT shadow the
/// agent's `.chat`.
///
/// Future explicit interface markers can broaden this without
/// changing callers.
fn is_chat_base(name: &str) -> bool {
    name.ends_with(".chat")
        && name.split('.').count() == 2
        && !name.contains(".api.")
        && !name.contains(".page.")
        && !name.contains(".actions.")
}

/// Convert an OpenAI `messages` array into a single prompt string +
/// optional system message. EasyNet `<agent>.chat` takes
/// `{prompt, system}` which is the simplest mapping.
///
/// Strategy:
///   - the LAST `user` message becomes `prompt`
///   - all `system` messages concatenated become `system`
///   - earlier `user`/`assistant` history is folded into prompt
///     prefixed by role labels so the agent has context
fn flatten_messages(messages: &[Value]) -> (String, Option<String>) {
    let mut system_parts: Vec<String> = Vec::new();
    let mut history: Vec<(String, String)> = Vec::new();

    for m in messages {
        let role = m.get("role").and_then(Value::as_str).unwrap_or("");
        let content = match m.get("content") {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Array(parts)) => parts
                .iter()
                .filter_map(|p| p.get("text").and_then(Value::as_str).map(String::from))
                .collect::<Vec<_>>()
                .join("\n"),
            _ => String::new(),
        };
        if content.is_empty() {
            continue;
        }
        match role {
            "system" => system_parts.push(content),
            _ => history.push((role.to_string(), content)),
        }
    }

    let system = (!system_parts.is_empty()).then(|| system_parts.join("\n\n"));

    let prompt = if history.len() <= 1 {
        history.pop().map(|(_, c)| c).unwrap_or_default()
    } else {
        // Multi-turn: render as a transcript so the agent has
        // history. Last turn is the new user message.
        let mut buf = String::new();
        for (role, content) in history {
            buf.push_str(&format!("{role}: {content}\n\n"));
        }
        buf.trim_end().to_string()
    };

    (prompt, system)
}

/// Resolve a model name to a chat-base ability name on the local
/// registry. Strategy:
///   1. if `model` already ends in `.chat`, use verbatim
///   2. otherwise append `.chat` (e.g. "web-builder" → "web-builder.chat")
fn resolve_model_to_ability(model: &str) -> String {
    if model.ends_with(".chat") {
        model.to_string()
    } else {
        format!("{model}.chat")
    }
}

/// `01HUB.openai.chat_completions`
///
/// args (one of two shapes accepted):
///   1. Direct OpenAI body:
///        { "model": "...", "messages": [...], "temperature": ..., ... }
///   2. Wrapped (when called from the HTTP listener):
///        { "request": <openai body>, "auth_token": "<bearer>" }
///
/// returns OpenAI ChatCompletion shape (non-streaming):
///   {
///     "id": "chatcmpl-...",
///     "object": "chat.completion",
///     "created": <unix>,
///     "model": "<resolved>",
///     "choices": [{"index": 0, "message": {"role":"assistant","content":"..."},
///                  "finish_reason": "stop"}],
///     "usage": {"prompt_tokens":..., "completion_tokens":..., "total_tokens":...}
///   }
pub fn handle_chat_completions(args: Value) -> anyhow::Result<Value> {
    // Unwrap optional auth + request envelope.
    let (request, auth_token) = if args.get("request").is_some() {
        let req = args.get("request").cloned().unwrap_or(Value::Null);
        let token = args
            .get("auth_token")
            .and_then(Value::as_str)
            .map(String::from);
        (req, token)
    } else {
        (args.clone(), None)
    };

    // INV-2: resolve API key. If no token supplied, accept (for
    // in-process callers like easynet llm-api CLI invoking via
    // local IPC); only the HTTP boundary requires the token.
    let user_uri = if let Some(tok) = auth_token.as_deref() {
        let (uri, _id_prefix) = api_key_ability::resolve_token(tok)
            .map_err(|e| anyhow::anyhow!("auth failed: {e}"))?;
        Some(uri)
    } else {
        None
    };

    let model_str = request
        .get("model")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("missing required field: model"))?
        .to_string();
    let messages = request
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if messages.is_empty() {
        anyhow::bail!("messages array is empty");
    }

    let target_ability = resolve_model_to_ability(&model_str);
    if !is_chat_base(&target_ability) {
        anyhow::bail!("model '{model_str}' resolves to '{target_ability}' which is not chat-base");
    }

    // INV-1: forward via standard registry, no own dispatcher.
    let handle = DISPATCH_HANDLE
        .get()
        .ok_or_else(|| anyhow::anyhow!("dispatch handle not set"))?;
    let registry = handle
        .get()
        .ok_or_else(|| anyhow::anyhow!("dispatch handle empty"))?;

    let handler = registry
        .get_rpc(&target_ability)
        .cloned()
        .or_else(|| registry.resolve_rpc(&target_ability))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "ability `{target_ability}` not registered. \
                 Has the agent been added via `easynet agent add`?"
            )
        })?;

    let (prompt, system) = flatten_messages(&messages);
    let mut ability_args = json!({ "prompt": prompt });
    if let Some(s) = system {
        ability_args["system"] = json!(s);
    }

    let dispatch_result = handler(ability_args).map_err(|e| {
        anyhow::anyhow!("chat-base ability `{target_ability}` failed: {e}")
    })?;

    // Extract reply text from agent response. Different chat
    // abilities return slightly different shapes; v0.1 supports:
    //   - { reply: "text" }
    //   - { message: "text" }
    //   - { content: "text" }
    //   - any string at top level
    let reply_text = dispatch_result
        .get("reply")
        .or_else(|| dispatch_result.get("message"))
        .or_else(|| dispatch_result.get("content"))
        .and_then(Value::as_str)
        .map(String::from)
        .unwrap_or_else(|| {
            // fallback: stringify the whole response
            serde_json::to_string(&dispatch_result).unwrap_or_default()
        });

    // INV-3: deterministic projection. Approximate token counts
    // using char-based heuristic (same input → same numbers).
    let prompt_chars: usize = messages
        .iter()
        .filter_map(|m| m.get("content").and_then(Value::as_str))
        .map(str::len)
        .sum();
    let prompt_tokens = (prompt_chars / 4).max(1);
    let completion_tokens = (reply_text.len() / 4).max(1);

    let id = random_id("chatcmpl-");
    let created = now_secs();

    // INV-3 differs by stream flag. Unary returns one ChatCompletion
    // object; streaming returns a list of ChatCompletionChunk objects
    // plus a done sentinel. The chunking rule is fixed and stateless,
    // so determinism still holds: same reply text → same chunk list.
    let stream = request
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    if !stream {
        return Ok(json!({
            "id":      id,
            "object":  "chat.completion",
            "created": created,
            "model":   model_str,
            "choices": [{
                "index": 0,
                "message": {
                    "role":    "assistant",
                    "content": reply_text,
                },
                "finish_reason": "stop",
            }],
            "usage": {
                "prompt_tokens":     prompt_tokens,
                "completion_tokens": completion_tokens,
                "total_tokens":      prompt_tokens + completion_tokens,
            },
            "easynet_user_uri": user_uri,
        }));
    }

    // Streaming path. v0.1 simplification: the underlying chat
    // ability is unary, so we synthesise an OpenAI-shape stream by
    // chunking the full reply.
    //
    // Chunk size: 64 chars per chunk — small enough for perceptible
    // streaming on the receiving UI, large enough not to flood the
    // wire for long replies.
    const CHUNK_BYTES: usize = 64;
    let mut chunks: Vec<Value> = Vec::new();

    // First chunk: announce role.
    chunks.push(json!({
        "id":      id,
        "object":  "chat.completion.chunk",
        "created": created,
        "model":   model_str,
        "choices": [{
            "index": 0,
            "delta": { "role": "assistant" },
            "finish_reason": Value::Null,
        }],
    }));

    // Body chunks — char-by-char to stay UTF-8 safe; emit when
    // accumulator hits CHUNK_BYTES.
    let mut acc = String::with_capacity(CHUNK_BYTES + 8);
    for ch in reply_text.chars() {
        acc.push(ch);
        if acc.len() >= CHUNK_BYTES {
            chunks.push(json!({
                "id":      id,
                "object":  "chat.completion.chunk",
                "created": created,
                "model":   model_str,
                "choices": [{
                    "index": 0,
                    "delta": { "content": acc.clone() },
                    "finish_reason": Value::Null,
                }],
            }));
            acc.clear();
        }
    }
    if !acc.is_empty() {
        chunks.push(json!({
            "id":      id,
            "object":  "chat.completion.chunk",
            "created": created,
            "model":   model_str,
            "choices": [{
                "index": 0,
                "delta": { "content": acc },
                "finish_reason": Value::Null,
            }],
        }));
    }

    // Final chunk: finish_reason + usage.
    chunks.push(json!({
        "id":      id,
        "object":  "chat.completion.chunk",
        "created": created,
        "model":   model_str,
        "choices": [{
            "index": 0,
            "delta": {},
            "finish_reason": "stop",
        }],
        "usage": {
            "prompt_tokens":     prompt_tokens,
            "completion_tokens": completion_tokens,
            "total_tokens":      prompt_tokens + completion_tokens,
        },
    }));

    Ok(json!({
        "stream":          true,
        "chunks":          chunks,
        "done_sentinel":   "[DONE]",
        "easynet_user_uri": user_uri,
    }))
}

/// `01HUB.openai.list_models` — return list of chat-base abilities
/// available on this daemon, projected as OpenAI-shape models.
pub fn handle_list_models(_args: Value) -> anyhow::Result<Value> {
    let handle = DISPATCH_HANDLE
        .get()
        .ok_or_else(|| anyhow::anyhow!("dispatch handle not set"))?;
    let registry = handle
        .get()
        .ok_or_else(|| anyhow::anyhow!("dispatch handle empty"))?;

    let mut models: Vec<Value> = Vec::new();
    for name in registry.list_rpc_names() {
        if !is_chat_base(&name) {
            continue;
        }
        // model id is the ability's owner prefix
        let model_id = name
            .strip_suffix(".chat")
            .unwrap_or(&name)
            .to_string();
        models.push(json!({
            "id":       model_id,
            "object":   "model",
            "created":  0,
            "owned_by": "easynet",
            "ability":  name,
        }));
    }

    Ok(json!({ "object": "list", "data": models }))
}

pub fn register(reg: &mut LocalAbilityRegistry) {
    reg.register_rpc(
        "01HUB.openai.chat_completions",
        Arc::new(handle_chat_completions) as LocalRpcHandler,
    );
    reg.register_rpc(
        "01HUB.openai.list_models",
        Arc::new(handle_list_models) as LocalRpcHandler,
    );
}
