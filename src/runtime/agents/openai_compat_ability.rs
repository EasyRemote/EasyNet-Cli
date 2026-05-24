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

use std::sync::{Arc, OnceLock, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::runtime::ability_dispatch::{LocalAbilityRegistry, LocalRpcHandler};
use crate::runtime::agents::api_key_ability;

/// Process-wide handle to the live ability registry. The outer
/// `RwLock` is what makes this last-writer-wins: in production the
/// daemon's boot path sets it exactly once, but the test binary
/// shares the static across thousands of tests with overlapping
/// `build_registry()` invocations, and a `OnceLock` here would
/// silently let whichever test ran first pin the handle for every
/// other test in the same process. The inner `Arc<OnceLock<...>>`
/// is the seam `build_registry_with_services` uses to backfill the
/// registry after the LocalAbilityRegistry assembly completes.
///
/// **Production invariant**: write once at boot, read many times
/// thereafter. Multiple writes are allowed (so tests can re-bind
/// the handle), but observing a stale read is not a contract — the
/// dispatch path always re-acquires through `current_handle()`.
static DISPATCH_HANDLE: RwLock<Option<Arc<OnceLock<Arc<LocalAbilityRegistry>>>>> =
    RwLock::new(None);
/// Process-wide identity for OpenAI-compat URA projection. Same
/// rationale as `DISPATCH_HANDLE`: production sets once at boot,
/// but the in-process test binary needs last-writer-wins so a
/// `set_identity({user: Some("alice"), …})` from
/// `ensure_openai_http_registry` can override a default written
/// earlier by `build_registry()` in another test.
static OPENAI_IDENTITY: RwLock<Option<OpenAICompatIdentity>> = RwLock::new(None);

#[derive(Debug, Clone)]
struct OpenAICompatIdentity {
    user: Option<String>,
    realm: String,
}

pub(crate) fn set_dispatch_handle(handle: Arc<OnceLock<Arc<LocalAbilityRegistry>>>) {
    *DISPATCH_HANDLE.write().expect("DISPATCH_HANDLE poisoned") = Some(handle);
}

fn current_dispatch_handle() -> Option<Arc<OnceLock<Arc<LocalAbilityRegistry>>>> {
    DISPATCH_HANDLE
        .read()
        .expect("DISPATCH_HANDLE poisoned")
        .as_ref()
        .cloned()
}

pub(crate) fn set_identity(identity: crate::runtime::agents::PagesIdentity) {
    *OPENAI_IDENTITY.write().expect("OPENAI_IDENTITY poisoned") = Some(OpenAICompatIdentity {
        user: identity.user,
        realm: identity
            .realm
            .unwrap_or_else(|| crate::ura::REALM_EASYNET.to_string()),
    });
}

fn current_identity() -> Option<OpenAICompatIdentity> {
    OPENAI_IDENTITY
        .read()
        .expect("OPENAI_IDENTITY poisoned")
        .clone()
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

/// Resolve an OpenAI model id to a local chat-base ability name.
///
/// Preferred shape: canonical ability URA
///   `easynet:///r/<realm>/ability/<user>.<agent>.chat`
///
/// Backward-compat shape:
///   `<agent>` or `<agent>.chat`
fn resolve_model_to_ability(model: &str) -> anyhow::Result<String> {
    if model.starts_with("easynet:///") {
        let parsed = crate::ura::parse_ura(model)
            .map_err(|e| anyhow::anyhow!("model must be a valid ability URA: {e}"))?;
        if parsed.kind != crate::ura::URAKind::Ability {
            anyhow::bail!("model must be an ability URA");
        }
        if crate::ura::ability_name_from_parts(&parsed).as_deref() != Some("chat") {
            anyhow::bail!("model must point to the canonical agent chat ability URA");
        }
        return Ok(crate::ura::agent_scoped_registry_ability(
            &crate::ura::agent_ura(&parsed.realm, &parsed.user_id, &parsed.agent_id),
            "chat",
        ));
    }
    if model.ends_with(".chat") {
        return Ok(model.to_string());
    }
    Ok(format!("{model}.chat"))
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
    let user_ura = if let Some(tok) = auth_token.as_deref() {
        let (uri, _id_prefix) =
            api_key_ability::resolve_token(tok).map_err(|e| anyhow::anyhow!("auth failed: {e}"))?;
        Some(uri)
    } else {
        None
    };

    let model_str = request
        .get("model")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("missing required field: model"))?
        .to_string();
    let mut messages = request
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if messages.is_empty() {
        anyhow::bail!("messages array is empty");
    }
    // Multimodal URA-deref: walk every message's content blocks
    // and inline-fetch any `easynet:///r/.../resource/...` URI.
    // Replaces the URI with a `data:<mime>;base64,<...>` form so
    // the chat-base ability handles bytes, not protocol-aware URI
    // resolution. RFC-006-C extension: agent inputs may carry
    // EasyNet resource references; the adapter is the single
    // place that turns them into bytes.
    deref_easynet_uris_in_messages(&mut messages);

    let target_ability = resolve_model_to_ability(&model_str)?;
    if !is_chat_base(&target_ability) {
        anyhow::bail!("model '{model_str}' resolves to '{target_ability}' which is not chat-base");
    }

    // INV-1: forward via standard registry, no own dispatcher.
    let handle = current_dispatch_handle()
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

    let dispatch_result = handler(ability_args)
        .map_err(|e| anyhow::anyhow!("chat-base ability `{target_ability}` failed: {e}"))?;

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
            "easynet_user_ura": user_ura,
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
        "easynet_user_ura": user_ura,
    }))
}

/// `01HUB.openai.list_models` — return list of chat-base abilities
/// available on this daemon, projected as OpenAI-shape models.
pub fn handle_list_models(_args: Value) -> anyhow::Result<Value> {
    let handle = current_dispatch_handle()
        .ok_or_else(|| anyhow::anyhow!("dispatch handle not set"))?;
    let registry = handle
        .get()
        .ok_or_else(|| anyhow::anyhow!("dispatch handle empty"))?;

    let mut models: Vec<Value> = Vec::new();
    for name in registry.list_rpc_names() {
        if !is_chat_base(&name) {
            continue;
        }
        let model_id = project_model_id(registry.as_ref(), &name);
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
    use crate::runtime::ability_dispatch::OwnerKind;
    // RFC-006-C v0.1 — DEVICE-local OpenAI protocol shim. The
    // device daemon serves OpenAI's `/v1/chat/completions` and
    // `/v1/models` HTTP surface against locally-hosted chat-base
    // abilities (`<agent>.chat`). Owner is `Device` because the
    // handler runs on the host and only sees host-local chat-base
    // abilities — there is no hub round-trip in the call path.
    //
    // What `hub.openai.*` means is up to whichever hub chose to
    // advertise it (federation.resolve include_abilities=true
    // surfaces it to clients on demand). Device-side never
    // pre-registers a `hub.*` name on behalf of the hub: that
    // would let the device daemon lie about what the hub offers.
    reg.register_rpc_with_owner(
        "device.openai.chat_completions",
        OwnerKind::Device,
        Arc::new(handle_chat_completions) as LocalRpcHandler,
    );
    reg.register_rpc_with_owner(
        "device.openai.list_models",
        OwnerKind::Device,
        Arc::new(handle_list_models) as LocalRpcHandler,
    );
}

fn project_model_id(registry: &LocalAbilityRegistry, ability_name: &str) -> String {
    project_model_id_with_identity(registry, ability_name, current_identity().as_ref())
}

fn project_model_id_with_identity(
    registry: &LocalAbilityRegistry,
    ability_name: &str,
    identity: Option<&OpenAICompatIdentity>,
) -> String {
    let Some(identity) = identity else {
        return ability_name.to_string();
    };
    let Some(crate::runtime::ability_dispatch::OwnerKind::Agent(agent_id)) =
        registry.lookup_owner(ability_name)
    else {
        return ability_name.to_string();
    };
    let Some(user) = identity.user.as_deref() else {
        return ability_name.to_string();
    };
    let owner_ura = crate::ura::agent_ura(&identity.realm, user, &agent_id);
    let public_name = crate::ura::public_ability_name_for_owner(&owner_ura, ability_name);
    crate::ura::ability_ura(&identity.realm, user, &agent_id, &public_name)
}

// ─── EasyNet URA dereference for multimodal message content ──────
//
// OpenAI multimodal request shape:
//
//   { "role": "user",
//     "content": [
//       { "type": "text", "text": "..." },
//       { "type": "image_url",
//         "image_url": { "url": "easynet:///r/.../resource/..." } },
//       { "type": "input_image",
//         "image_url": { "url": "easynet:///..." } },
//       { "type": "file",
//         "file": { "url": "easynet:///..." } },
//     ]
//   }
//
// For every block whose `*.url` field starts `easynet:///`, the
// adapter dispatches `<owner>.files.get` (for `<u>.files/<sha>`
// shapes) or `<owner>.<project>.page.fetch` (for pages-shape
// resources) and replaces the URL with a `data:<mime>;base64,<...>`
// form before forwarding to the chat-base ability. The agent gets
// bytes, not URIs — it doesn't need protocol awareness.

fn deref_easynet_uris_in_messages(messages: &mut Vec<Value>) {
    for msg in messages.iter_mut() {
        let Some(content) = msg.get_mut("content") else {
            continue;
        };
        let Some(arr) = content.as_array_mut() else {
            continue;
        };
        for block in arr.iter_mut() {
            // Image blocks: `image_url.url` or `input_image.image_url.url`.
            for nested_key in &["image_url", "input_image"] {
                if let Some(nested) = block.get_mut(*nested_key) {
                    if let Some(url) = nested.get_mut("url") {
                        if let Some(s) = url.as_str() {
                            if crate::ura::parse_ura(s).is_ok() {
                                if let Ok(data_url) = deref_to_data_url(s) {
                                    *url = Value::String(data_url);
                                }
                            }
                        }
                    }
                }
            }
            // File blocks: `file.url` or `file.file_url`.
            if let Some(file) = block.get_mut("file") {
                for url_key in &["url", "file_url"] {
                    if let Some(url) = file.get_mut(*url_key) {
                        if let Some(s) = url.as_str() {
                            if crate::ura::parse_ura(s).is_ok() {
                                if let Ok(data_url) = deref_to_data_url(s) {
                                    *url = Value::String(data_url);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Resolve an `easynet:///r/<realm>/resource/<owner>/<path>` URI
/// through the local ability dispatcher and return a
/// `data:<mime>;base64,<...>` URL.
fn deref_to_data_url(uri: &str) -> anyhow::Result<String> {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;

    let parsed =
        crate::ura::parse_ura(uri).map_err(|e| anyhow::anyhow!("deref `{uri}`: parse: {e}"))?;
    if !matches!(parsed.kind, crate::ura::URAKind::Resource) {
        anyhow::bail!("deref `{uri}`: not a resource URA");
    }
    // Owner segment is `<userID>.<owner-tail>` for v4.1.5 dot-id
    // resources (pages: `<u>.<project>`; files: `<u>.files`). Pick
    // the ability dispatch shape based on the owner-tail.
    let id_part = &parsed.user_id;
    let path = &parsed.path;
    let (ability, args) = match id_part.split_once('.') {
        Some((_user, "files")) => (
            format!("{id_part}.get"),
            json!({ "uri": uri, "path": path }),
        ),
        Some((_user, project)) => {
            // Pages-shape: `<user>.<project>.page.fetch` with
            // `path` arg.
            let owner_user = id_part.split('.').next().unwrap_or("");
            let _ = owner_user;
            let mut pf_path = path.clone();
            if !pf_path.starts_with('/') {
                pf_path = format!("/{pf_path}");
            }
            (
                format!("{id_part}.page.fetch"),
                json!({ "path": pf_path, "project_id": project }),
            )
        }
        None => anyhow::bail!("deref `{uri}`: owner segment lacks dot"),
    };
    let handle = current_dispatch_handle()
        .ok_or_else(|| anyhow::anyhow!("deref: dispatch handle not set"))?;
    let registry = handle
        .get()
        .ok_or_else(|| anyhow::anyhow!("deref: dispatch handle empty"))?;
    let handler = registry
        .get_rpc(&ability)
        .cloned()
        .or_else(|| registry.resolve_rpc(&ability))
        .ok_or_else(|| anyhow::anyhow!("deref `{uri}`: ability `{ability}` not registered"))?;
    let resp =
        handler(args).map_err(|e| anyhow::anyhow!("deref `{uri}`: {ability} failed: {e}"))?;
    let bytes_b64 = resp
        .get("bytes_b64")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("deref `{uri}`: response missing bytes_b64"))?;
    let mime = resp
        .get("content_type")
        .and_then(Value::as_str)
        .unwrap_or("application/octet-stream");
    // Re-encode through base64 to canonicalize (handler already
    // returns standard b64 padded; this is belt-and-braces).
    let raw = STANDARD
        .decode(bytes_b64.as_bytes())
        .map_err(|e| anyhow::anyhow!("deref `{uri}`: b64 decode: {e}"))?;
    let canon = STANDARD.encode(raw);
    Ok(format!("data:{mime};base64,{canon}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::ability_dispatch::OwnerKind;

    fn ok_handler() -> LocalRpcHandler {
        Arc::new(|_| Ok(json!({"reply":"ok"})))
    }

    #[test]
    fn resolve_model_to_ability_accepts_canonical_chat_ability_ura() {
        let got = resolve_model_to_ability("easynet:///r/easynet.run/ability/alice.codex.chat")
            .expect("canonical URA must resolve");
        assert_eq!(got, "codex.chat");
    }

    #[test]
    fn project_model_id_prefers_canonical_ability_ura_for_agent_owned_chat() {
        let mut reg = LocalAbilityRegistry::new();
        reg.register_rpc_with_owner("codex.chat", OwnerKind::Agent("codex".into()), ok_handler());
        let identity = OpenAICompatIdentity {
            user: Some("alice".into()),
            realm: "easynet.run".into(),
        };
        let got = project_model_id_with_identity(&reg, "codex.chat", Some(&identity));
        assert_eq!(got, "easynet:///r/easynet.run/ability/alice.codex.chat");
    }
}
