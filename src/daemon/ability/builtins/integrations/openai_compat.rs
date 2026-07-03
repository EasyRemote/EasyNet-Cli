// EasyNet CLI — OpenAI Compatibility adapter ability
// ====================================================
//
// File: src/daemon/ability/builtins/integrations/openai_compat.rs
// Description: device-local abilities `openai.chat_completions`
//              and `openai.list_models` that project EasyNet
//              chat-base abilities through the OpenAI streaming
//              completion wire shape (RFC-006-C v0.1).
//
// Conformance: INV-1 (Adapter Purity), INV-2 (Capability-URA Key),
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

use crate::daemon::ability::builtins::governance::api_key;
use crate::daemon::ability::builtins::resources::pages::PagesIdentity;
use crate::daemon::ability::dispatch::{AxonAbilityCatalog, LocalRpcHandler};
use crate::daemon::invocation::routing::target::{CallMode, InvocationTarget, TargetScope};
use crate::support::platform::process_singleton::ProcessSingleton;

/// Process-wide handle to the live ability registry. The inner
/// `Arc<OnceLock<Arc<AxonAbilityCatalog>>>` is the seam
/// `build_registry_with_services` uses to backfill the registry
/// after the `AxonAbilityCatalog` assembly completes.
///
/// `ProcessSingleton::last_writer_wins()` because the in-process
/// test binary shares this static across thousands of tests with
/// overlapping `build_registry()` invocations; a `Once`-mode handle
/// would silently let the first test pin the registry for every
/// other test. Production sets it exactly once at boot. See
/// `support::process_singleton` for the mode-choice rationale.
static DISPATCH_HANDLE: ProcessSingleton<OnceLock<Arc<AxonAbilityCatalog>>> =
    ProcessSingleton::last_writer_wins();

/// Process-wide identity for OpenAI-compat URA projection. Same
/// rationale as `DISPATCH_HANDLE`: production sets once at boot,
/// but the in-process test binary needs last-writer-wins so a
/// `set_identity({user: Some("alice"), …})` from
/// `ensure_openai_http_registry` can override a default written
/// earlier by `build_registry()` in another test.
static OPENAI_IDENTITY: ProcessSingleton<OpenAICompatIdentity> =
    ProcessSingleton::last_writer_wins();

#[derive(Debug, Clone)]
struct OpenAICompatIdentity {
    user: Option<String>,
    realm: String,
}

/// Stable OpenAI-compat execution context.
///
/// Production installs this context process-wide at daemon boot via
/// `set_dispatch_handle`/`set_identity`; tests that exercise the HTTP
/// boundary can carry an explicit copy in the request extensions so
/// unrelated in-process registry construction cannot swap the backing
/// handle mid-request.
#[derive(Debug, Clone)]
pub(crate) struct OpenAICompatRuntime {
    dispatch_handle: Arc<OnceLock<Arc<AxonAbilityCatalog>>>,
    identity: Option<OpenAICompatIdentity>,
}

impl OpenAICompatRuntime {
    #[cfg(test)]
    pub(crate) fn from_pages_identity(
        dispatch_handle: Arc<OnceLock<Arc<AxonAbilityCatalog>>>,
        identity: PagesIdentity,
    ) -> Self {
        Self {
            dispatch_handle,
            identity: Some(OpenAICompatIdentity::from_pages_identity(identity)),
        }
    }

    fn current() -> Option<Self> {
        Some(Self {
            dispatch_handle: current_dispatch_handle()?,
            identity: current_identity(),
        })
    }

    pub(crate) fn handle_chat_completions(&self, args: Value) -> anyhow::Result<Value> {
        handle_chat_completions_with_handle(&self.dispatch_handle, args)
    }

    pub(crate) fn handle_list_models(&self, args: Value) -> anyhow::Result<Value> {
        handle_list_models_with_context(&self.dispatch_handle, self.identity.as_ref(), args)
    }
}

impl OpenAICompatIdentity {
    fn from_pages_identity(identity: PagesIdentity) -> Self {
        Self {
            user: identity.user,
            realm: identity
                .realm
                .unwrap_or_else(|| crate::core::ura::REALM_EASYNET.to_string()),
        }
    }
}

pub(crate) fn set_dispatch_handle(handle: Arc<OnceLock<Arc<AxonAbilityCatalog>>>) {
    DISPATCH_HANDLE.set(handle);
}

fn current_dispatch_handle() -> Option<Arc<OnceLock<Arc<AxonAbilityCatalog>>>> {
    DISPATCH_HANDLE.get()
}

pub(crate) fn set_identity(identity: PagesIdentity) {
    OPENAI_IDENTITY.set(Arc::new(OpenAICompatIdentity::from_pages_identity(
        identity,
    )));
}

fn current_identity() -> Option<OpenAICompatIdentity> {
    OPENAI_IDENTITY.get().map(|arc| (*arc).clone())
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedOpenAIModelAbility {
    local_dispatch_key: String,
    owner_ura: String,
}

/// Resolve an OpenAI model id to the local dispatch key and owner URA
/// for an agent-owned chat ability.
///
/// Required shape: canonical ability URA
///   `easynet:///r/<realm>/ability/<user>.<agent>.chat`
fn resolve_model_to_ability_and_owner(model: &str) -> anyhow::Result<ResolvedOpenAIModelAbility> {
    let parsed = crate::core::ura::parse_ura(model)
        .map_err(|e| anyhow::anyhow!("model must be a valid canonical Ability URA: {e}"))?;
    if parsed.kind != crate::core::ura::URAKind::Ability {
        anyhow::bail!("model must be a canonical Ability URA");
    }
    let Some(ability) = parsed.ability() else {
        anyhow::bail!("model Ability URA has no typed ability owner");
    };
    let crate::core::ura::AbilityOwner::Agent { user_id, agent_id } = ability.owner else {
        anyhow::bail!("model must point to an agent-owned chat Ability URA");
    };
    if crate::core::ura::ability_name_from_parts(&parsed).as_deref() != Some("chat") {
        anyhow::bail!("model must point to the canonical agent chat Ability URA");
    }
    let owner_ura = crate::core::ura::agent_ura(&parsed.realm, &user_id, &agent_id);
    Ok(ResolvedOpenAIModelAbility {
        local_dispatch_key: crate::core::ura::local_dispatch_ability_key(&owner_ura, "chat"),
        owner_ura,
    })
}

/// Resolve an OpenAI model id to the local dispatch key for an
/// agent-owned chat ability.
fn resolve_model_to_ability(model: &str) -> anyhow::Result<String> {
    resolve_model_to_ability_and_owner(model).map(|resolved| resolved.local_dispatch_key)
}

/// Validate the public OpenAI-compatible `model` identifier.
///
/// The OpenAI wire field stays named `model` for client compatibility,
/// but EasyNet's value is a canonical agent-owned chat Ability URA,
/// not a provider nickname and not a daemon-local registry key.
pub(crate) fn validate_chat_model_id(model: &str) -> anyhow::Result<()> {
    resolve_model_to_ability(model).map(|_| ())
}

/// `openai.chat_completions`
///
/// args (one of two shapes accepted):
///   1. Direct OpenAI body:
///      `{ "model": "...", "messages": [...], "temperature": ..., ... }`
///   2. Wrapped (when called from the HTTP listener):
///      `{ "request": <openai body>, "auth_token": "<bearer>" }`
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
    let runtime =
        OpenAICompatRuntime::current().ok_or_else(|| anyhow::anyhow!("dispatch handle not set"))?;
    runtime.handle_chat_completions(args)
}

fn handle_chat_completions_with_handle(
    dispatch_handle: &Arc<OnceLock<Arc<AxonAbilityCatalog>>>,
    args: Value,
) -> anyhow::Result<Value> {
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
        let (ura, _id_prefix) =
            api_key::resolve_token(tok).map_err(|e| anyhow::anyhow!("auth failed: {e}"))?;
        Some(ura)
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
    // and inline-fetch any `easynet:///r/.../resource/...` URA.
    // Replaces the URA with a `data:<mime>;base64,<...>` form so
    // the chat-base ability handles bytes, not protocol-aware URA
    // resolution. RFC-006-C extension: agent inputs may carry
    // EasyNet resource references; the adapter is the single
    // place that turns them into bytes.
    deref_easynet_uras_in_messages(&mut messages, dispatch_handle);

    let target = resolve_model_to_ability_and_owner(&model_str)?;
    if !is_chat_base(&target.local_dispatch_key) {
        anyhow::bail!(
            "model '{model_str}' resolves to '{}' which is not chat-base",
            target.local_dispatch_key
        );
    }

    // INV-1: forward via standard registry, no own dispatcher.
    let registry = registry_from_handle(dispatch_handle, "dispatch")?;

    let (prompt, system) = flatten_messages(&messages);
    let mut ability_args = json!({ "prompt": prompt });
    if let Some(s) = system {
        ability_args["system"] = json!(s);
    }

    let invocation_target = InvocationTarget {
        scope: TargetScope::Local,
        ability: target.local_dispatch_key.clone(),
        normalized_args: ability_args,
        call_mode: CallMode::Rpc,
        subject: Some(target.owner_ura.clone()),
        causal_context: None,
    };
    let dispatch_result = registry
        .invoke_rpc_target_json(invocation_target)
        .map_err(|e| {
            anyhow::anyhow!(
                "chat-base ability `{}` failed: {e}",
                target.local_dispatch_key
            )
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

/// `openai.list_models` — return list of chat-base abilities
/// available on this daemon, projected as OpenAI-shape models.
pub fn handle_list_models(_args: Value) -> anyhow::Result<Value> {
    let runtime =
        OpenAICompatRuntime::current().ok_or_else(|| anyhow::anyhow!("dispatch handle not set"))?;
    runtime.handle_list_models(serde_json::json!({}))
}

fn handle_list_models_with_context(
    dispatch_handle: &Arc<OnceLock<Arc<AxonAbilityCatalog>>>,
    identity: Option<&OpenAICompatIdentity>,
    _args: Value,
) -> anyhow::Result<Value> {
    let registry = registry_from_handle(dispatch_handle, "dispatch")?;

    let mut models: Vec<Value> = Vec::new();
    for name in registry.list_rpc_names() {
        if !is_chat_base(&name) {
            continue;
        }
        if let Some(model_id) = project_model_id_with_identity(registry.as_ref(), &name, identity) {
            models.push(json!({
                "id":       model_id,
                "object":   "model",
                "created":  0,
                "owned_by": "easynet",
                "ability":  name,
            }));
        }
    }

    Ok(json!({ "object": "list", "data": models }))
}

fn registry_from_handle(
    dispatch_handle: &Arc<OnceLock<Arc<AxonAbilityCatalog>>>,
    context: &str,
) -> anyhow::Result<Arc<AxonAbilityCatalog>> {
    dispatch_handle
        .get()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("{context} handle empty"))
}

pub fn register(reg: &mut AxonAbilityCatalog) {
    use crate::daemon::ability::dispatch::OwnerKind;
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
        "openai.chat_completions",
        OwnerKind::Device,
        Arc::new(handle_chat_completions) as LocalRpcHandler,
    );
    reg.register_rpc_with_owner(
        "openai.list_models",
        OwnerKind::Device,
        Arc::new(handle_list_models) as LocalRpcHandler,
    );
}

fn project_model_id_with_identity(
    registry: &AxonAbilityCatalog,
    ability_name: &str,
    identity: Option<&OpenAICompatIdentity>,
) -> Option<String> {
    let identity = identity?;
    // SPEC §9.1.A Step 5: agent ownership comes from the control-plane
    // record, not the legacy `owner` side table.
    let Some(crate::daemon::ability::dispatch::OwnerKind::Agent(agent_id)) =
        registry.control_plane_owner(ability_name)
    else {
        return None;
    };
    let user = identity.user.as_deref()?;
    let owner_ura = crate::core::ura::agent_ura(&identity.realm, user, &agent_id);
    let public_name = crate::core::ura::owner_local_ability_name(&owner_ura, ability_name);
    crate::core::ura::owner_ability_ura(&owner_ura, &public_name)
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
// bytes, not URAs — it doesn't need protocol awareness.

fn deref_easynet_uras_in_messages(
    messages: &mut [Value],
    dispatch_handle: &Arc<OnceLock<Arc<AxonAbilityCatalog>>>,
) {
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
                            if crate::core::ura::parse_ura(s).is_ok() {
                                if let Ok(data_url) = deref_to_data_url(s, dispatch_handle) {
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
                            if crate::core::ura::parse_ura(s).is_ok() {
                                if let Ok(data_url) = deref_to_data_url(s, dispatch_handle) {
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

/// Resolve an `easynet:///r/<realm>/resource/<owner>/<path>` URA
/// through the local ability dispatcher and return a
/// `data:<mime>;base64,<...>` URL.
fn deref_to_data_url(
    ura: &str,
    dispatch_handle: &Arc<OnceLock<Arc<AxonAbilityCatalog>>>,
) -> anyhow::Result<String> {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;

    let parsed = crate::core::ura::parse_ura(ura)
        .map_err(|e| anyhow::anyhow!("deref `{ura}`: parse: {e}"))?;
    if !matches!(parsed.kind, crate::core::ura::URAKind::Resource) {
        anyhow::bail!("deref `{ura}`: not a resource URA");
    }
    // Owner segment is `<userID>.<owner-tail>` for v4.1.5 dot-id
    // resources (pages: `<u>.<project>`; files: `<u>.files`). Pick
    // the ability dispatch shape based on the owner-tail.
    let id_part = parsed
        .resource_owner_id()
        .ok_or_else(|| anyhow::anyhow!("deref `{ura}`: missing resource owner"))?;
    let path = parsed.resource_path().unwrap_or_default().to_string();
    let (ability, args) = match id_part.split_once('.') {
        Some((_user, "files")) => (
            format!("{id_part}.get"),
            json!({ "ura": ura, "path": path }),
        ),
        Some((_user, project)) => {
            // Pages-shape: `<user>.<project>.page.fetch` with
            // `path` arg.
            let owner_user = id_part.split('.').next().unwrap_or("");
            let _ = owner_user;
            let mut pf_path = path.to_string();
            if !pf_path.starts_with('/') {
                pf_path = format!("/{pf_path}");
            }
            (
                format!("{id_part}.page.fetch"),
                json!({ "path": pf_path, "project_id": project }),
            )
        }
        None => anyhow::bail!("deref `{ura}`: owner segment lacks dot"),
    };
    let registry = registry_from_handle(dispatch_handle, "deref")?;
    let resp = registry
        .invoke_rpc_json(&ability, args)
        .map_err(|e| anyhow::anyhow!("deref `{ura}`: {ability} failed: {e}"))?;
    let bytes_b64 = resp
        .get("bytes_b64")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("deref `{ura}`: response missing bytes_b64"))?;
    let mime = resp
        .get("content_type")
        .and_then(Value::as_str)
        .unwrap_or("application/octet-stream");
    // Re-encode through base64 to canonicalize (handler already
    // returns standard b64 padded; this is belt-and-braces).
    let raw = STANDARD
        .decode(bytes_b64.as_bytes())
        .map_err(|e| anyhow::anyhow!("deref `{ura}`: b64 decode: {e}"))?;
    let canon = STANDARD.encode(raw);
    Ok(format!("data:{mime};base64,{canon}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::ability::dispatch::OwnerKind;

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
    fn resolve_model_to_ability_rejects_bare_agent_name() {
        let err = resolve_model_to_ability("codex").expect_err("bare agent names are not models");
        assert!(
            err.to_string()
                .contains("model must be a valid canonical Ability URA"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn resolve_model_to_ability_rejects_local_chat_registry_key() {
        let err =
            resolve_model_to_ability("codex.chat").expect_err("local dispatch keys are not models");
        assert!(
            err.to_string()
                .contains("model must be a valid canonical Ability URA"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn resolve_model_to_ability_rejects_non_agent_ability_ura() {
        let err =
            resolve_model_to_ability("easynet:///r/easynet.run/ability/device.01HUB.e2e.run.shell")
                .expect_err("device-owned abilities cannot be OpenAI models");
        assert!(
            err.to_string()
                .contains("model must point to an agent-owned chat Ability URA"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn resolve_model_to_ability_rejects_non_chat_agent_ability_ura() {
        let err = resolve_model_to_ability("easynet:///r/easynet.run/ability/alice.codex.plan")
            .expect_err("non-chat agent abilities cannot be OpenAI models");
        assert!(
            err.to_string()
                .contains("model must point to the canonical agent chat Ability URA"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn project_model_id_prefers_canonical_ability_ura_for_agent_owned_chat() {
        // Hold the env lock: catalog registration consults HOME-rooted
        // authority/runtime state, so a concurrent HOME-mutating test must
        // not race it (passes isolated, flakes only under parallelism).
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let mut reg = AxonAbilityCatalog::new();
        reg.register_rpc_with_owner("codex.chat", OwnerKind::Agent("codex".into()), ok_handler());
        let identity = OpenAICompatIdentity {
            user: Some("alice".into()),
            realm: "easynet.run".into(),
        };
        let got = project_model_id_with_identity(&reg, "codex.chat", Some(&identity));
        assert_eq!(
            got.as_deref(),
            Some("easynet:///r/easynet.run/ability/alice.codex.chat")
        );
    }

    #[test]
    fn project_model_id_drops_chat_key_when_identity_is_missing() {
        let mut reg = AxonAbilityCatalog::new();
        reg.register_rpc_with_owner("codex.chat", OwnerKind::Agent("codex".into()), ok_handler());

        assert_eq!(
            project_model_id_with_identity(&reg, "codex.chat", None),
            None
        );
    }

    #[test]
    fn project_model_id_drops_chat_key_when_user_is_missing() {
        let mut reg = AxonAbilityCatalog::new();
        reg.register_rpc_with_owner("codex.chat", OwnerKind::Agent("codex".into()), ok_handler());
        let identity = OpenAICompatIdentity {
            user: None,
            realm: "easynet.run".into(),
        };

        assert_eq!(
            project_model_id_with_identity(&reg, "codex.chat", Some(&identity)),
            None
        );
    }

    #[test]
    fn project_model_id_drops_non_agent_chat_owner() {
        let mut reg = AxonAbilityCatalog::new();
        reg.register_rpc_with_owner("device.chat", OwnerKind::Device, ok_handler());
        let identity = OpenAICompatIdentity {
            user: Some("alice".into()),
            realm: "easynet.run".into(),
        };

        assert_eq!(
            project_model_id_with_identity(&reg, "device.chat", Some(&identity)),
            None
        );
    }
}
