// EasyNet CLI — Compatibility shared contract
// ============================================
//
// File: src/daemon/compatibility_contract.rs
// Description: Shared daemon SDK contract for OpenAI-compatible carrier
//              construction and result projection.
//
// Protocol Responsibility
// -----------------------
// Own the EasyNet-Cli SDK Compatibility DTO projection that lowers
// OpenAI-shaped model/chat requests to governed daemon abilities. This module
// does not own HTTP auth, billing, rate limits, SSE fanout, model execution, or
// OpenAI as daemon protocol.
//
// Implementation Approach
// -----------------------
// Reuse the shared daemon SDK carrier builder for complete Invocation tuples
// and delegate chat-model identity validation to the daemon OpenAI adapter.
// Projection validates daemon-returned OpenAI-compatible envelopes into stable
// SDK DTOs without fabricating model ids, chunks, completion ids, or usage.
//
// Usage Contract
// --------------
// Carrier construction requires explicit Invocation tuple fields. Chat
// requests must carry canonical agent-owned chat Ability URAs as `model`.
// Streaming carriers make stream intent explicit before dispatch.
//
// Architectural Position
// ----------------------
// EasyNet-Cli daemon SDK Compatibility profile. Runtime Core remains the only
// submit/open path for returned Invocation carriers; backend/product HTTP
// compatibility routes remain above this SDK profile.

use serde_json::{json, Map, Value};

use crate::daemon::ability::builtins::integrations::openai_compat;
use crate::daemon::sdk_contract::{
    build_system_invocation, object, optional_bool_field, optional_string_field, required_string,
    SdkContractError,
};

const COMPATIBILITY_PROFILE: &str = "compatibility";
const ABILITY_OPENAI_LIST_MODELS: &str =
    crate::daemon::ability::names::integrations::OPENAI_LIST_MODELS;
const ABILITY_OPENAI_CHAT_COMPLETIONS: &str =
    crate::daemon::ability::names::integrations::OPENAI_CHAT_COMPLETIONS;

pub(crate) type CompatibilityError = SdkContractError;

pub(crate) fn build_list_models_invocation(request: &Value) -> Result<Value, CompatibilityError> {
    let obj = object(request, "CompatibilityListModelsRequest")?;
    let mut args = Map::new();
    if let Some(auth_token) = optional_string_field(obj, "auth_token")? {
        args.insert("auth_token".to_string(), Value::String(auth_token));
    }
    build_system_invocation(
        obj,
        COMPATIBILITY_PROFILE,
        ABILITY_OPENAI_LIST_MODELS,
        Value::Object(args),
    )
}

pub(crate) fn build_chat_completion_invocation(
    request: &Value,
) -> Result<Value, CompatibilityError> {
    let obj = object(request, "CompatibilityChatCompletionRequest")?;
    let openai_request = normalized_chat_request(obj, ChatCarrierMode::Unary)?;
    let args = compatibility_chat_args(obj, openai_request)?;
    build_system_invocation(
        obj,
        COMPATIBILITY_PROFILE,
        ABILITY_OPENAI_CHAT_COMPLETIONS,
        args,
    )
}

pub(crate) fn build_stream_chat_completion_invocation(
    request: &Value,
) -> Result<Value, CompatibilityError> {
    let obj = object(request, "CompatibilityStreamChatCompletionRequest")?;
    let openai_request = normalized_chat_request(obj, ChatCarrierMode::Stream)?;
    let args = compatibility_chat_args(obj, openai_request)?;
    build_system_invocation(
        obj,
        COMPATIBILITY_PROFILE,
        ABILITY_OPENAI_CHAT_COMPLETIONS,
        args,
    )
}

pub(crate) fn project_model_page(input: &Value) -> Result<Value, CompatibilityError> {
    let obj = object(input, "CompatibilityModelPageInput")?;
    require_const_string(obj, "object", "list")?;
    let data = obj
        .get("data")
        .and_then(Value::as_array)
        .ok_or(CompatibilityError::MissingField("data"))?;
    let mut items = Vec::with_capacity(data.len());
    for entry in data {
        items.push(project_model_record(entry)?);
    }
    Ok(json!({
        "profile": COMPATIBILITY_PROFILE,
        "kind": "model_page",
        "object": "list",
        "data": items,
        "next_cursor": Value::Null,
        "metadata": {
            "profile": COMPATIBILITY_PROFILE,
            "source": "openai.list_models",
            "count": data.len(),
        },
    }))
}

pub(crate) fn project_chat_completion(input: &Value) -> Result<Value, CompatibilityError> {
    let obj = object(input, "CompatibilityChatCompletionInput")?;
    require_const_string(obj, "object", "chat.completion")?;
    let id = required_string(obj, "id")?;
    let model = required_string(obj, "model")?;
    validate_chat_model(model, "model")?;
    let created = required_u64(obj, "created")?;
    let choices = required_non_empty_array(obj, "choices")?.clone();
    let usage = obj.get("usage").filter(|value| !value.is_null()).cloned();

    Ok(json!({
        "profile": COMPATIBILITY_PROFILE,
        "kind": "chat_completion",
        "id": id,
        "object": "chat.completion",
        "created": created,
        "model": model,
        "choices": choices,
        "usage": usage.unwrap_or(Value::Null),
        "metadata": {
            "profile": COMPATIBILITY_PROFILE,
            "source": "openai.chat_completions",
            "easynet_user_ura": obj.get("easynet_user_ura").cloned().unwrap_or(Value::Null),
        },
    }))
}

pub(crate) fn project_chat_stream(input: &Value) -> Result<Value, CompatibilityError> {
    let obj = object(input, "CompatibilityChatStreamInput")?;
    let stream = optional_bool_field(obj, "stream")?.unwrap_or(false);
    if !stream {
        return Err(CompatibilityError::InvalidField(
            "stream",
            "must be true for a chat completion stream".to_string(),
        ));
    }
    let chunks = required_non_empty_array(obj, "chunks")?;
    let done_sentinel = required_string(obj, "done_sentinel")?;
    let mut items = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        items.push(project_stream_chunk(chunk)?);
    }
    Ok(json!({
        "profile": COMPATIBILITY_PROFILE,
        "kind": "chat_completion_stream",
        "stream": true,
        "items": items,
        "done_sentinel": done_sentinel,
        "metadata": {
            "profile": COMPATIBILITY_PROFILE,
            "source": "openai.chat_completions",
            "easynet_user_ura": obj.get("easynet_user_ura").cloned().unwrap_or(Value::Null),
        },
    }))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChatCarrierMode {
    Unary,
    Stream,
}

fn normalized_chat_request(
    obj: &Map<String, Value>,
    mode: ChatCarrierMode,
) -> Result<Value, CompatibilityError> {
    let request = obj
        .get("request")
        .ok_or(CompatibilityError::MissingField("request"))?;
    let request_obj = object(request, "request")?;
    validate_chat_request(request_obj)?;
    match (mode, optional_bool_field(request_obj, "stream")?) {
        (ChatCarrierMode::Unary, Some(true)) => Err(CompatibilityError::InvalidField(
            "request.stream",
            "use stream_chat_completions for streaming requests".to_string(),
        )),
        (ChatCarrierMode::Stream, Some(false)) => Err(CompatibilityError::InvalidField(
            "request.stream",
            "must not be false for stream_chat_completions".to_string(),
        )),
        (ChatCarrierMode::Stream, _) => {
            let mut request = request.clone();
            request
                .as_object_mut()
                .expect("request was validated as object")
                .insert("stream".to_string(), Value::Bool(true));
            Ok(request)
        }
        (ChatCarrierMode::Unary, _) => Ok(request.clone()),
    }
}

fn validate_chat_request(obj: &Map<String, Value>) -> Result<(), CompatibilityError> {
    let model = required_string(obj, "model")?;
    validate_chat_model(model, "request.model")?;
    let messages = obj
        .get("messages")
        .and_then(Value::as_array)
        .ok_or(CompatibilityError::MissingField("request.messages"))?;
    if messages.is_empty() {
        return Err(CompatibilityError::InvalidField(
            "request.messages",
            "must contain at least one message".to_string(),
        ));
    }
    Ok(())
}

fn compatibility_chat_args(
    obj: &Map<String, Value>,
    openai_request: Value,
) -> Result<Value, CompatibilityError> {
    let mut args = Map::new();
    args.insert("request".to_string(), openai_request);
    if let Some(auth_token) = optional_string_field(obj, "auth_token")? {
        args.insert("auth_token".to_string(), Value::String(auth_token));
    }
    Ok(Value::Object(args))
}

fn project_model_record(input: &Value) -> Result<Value, CompatibilityError> {
    let obj = object(input, "CompatibilityModelRecordInput")?;
    require_const_string(obj, "object", "model")?;
    let id = required_string(obj, "id")?;
    validate_chat_model(id, "id")?;
    let created = optional_u64_field(obj, "created")?;
    let owned_by = optional_string_field(obj, "owned_by")?;
    let daemon_ability = optional_string_field(obj, "ability")?;
    Ok(json!({
        "profile": COMPATIBILITY_PROFILE,
        "kind": "model",
        "id": id,
        "object": "model",
        "created": created,
        "owned_by": owned_by,
        "ability_ref": id,
        "metadata": {
            "profile": COMPATIBILITY_PROFILE,
            "source": "openai.list_models",
            "daemon_ability": daemon_ability,
            "raw_model": input,
        },
    }))
}

fn project_stream_chunk(input: &Value) -> Result<Value, CompatibilityError> {
    let obj = object(input, "CompatibilityStreamChunkInput")?;
    require_const_string(obj, "object", "chat.completion.chunk")?;
    let id = required_string(obj, "id")?;
    let model = required_string(obj, "model")?;
    validate_chat_model(model, "model")?;
    let created = required_u64(obj, "created")?;
    let choices = required_non_empty_array(obj, "choices")?.clone();
    let usage = obj.get("usage").filter(|value| !value.is_null()).cloned();
    Ok(json!({
        "profile": COMPATIBILITY_PROFILE,
        "kind": "chat_completion_chunk",
        "id": id,
        "object": "chat.completion.chunk",
        "created": created,
        "model": model,
        "choices": choices,
        "usage": usage.unwrap_or(Value::Null),
        "metadata": {
            "profile": COMPATIBILITY_PROFILE,
            "source": "openai.chat_completions",
        },
    }))
}

fn validate_chat_model(raw: &str, field: &'static str) -> Result<(), CompatibilityError> {
    openai_compat::validate_chat_model_id(raw)
        .map_err(|err| CompatibilityError::InvalidField(field, err.to_string()))
}

fn require_const_string(
    obj: &Map<String, Value>,
    field: &'static str,
    expected: &'static str,
) -> Result<(), CompatibilityError> {
    let actual = required_string(obj, field)?;
    if actual != expected {
        return Err(CompatibilityError::InvalidField(
            field,
            format!("must be {expected:?}"),
        ));
    }
    Ok(())
}

fn required_non_empty_array<'a>(
    obj: &'a Map<String, Value>,
    field: &'static str,
) -> Result<&'a Vec<Value>, CompatibilityError> {
    let values = obj
        .get(field)
        .and_then(Value::as_array)
        .ok_or(CompatibilityError::MissingField(field))?;
    if values.is_empty() {
        return Err(CompatibilityError::InvalidField(
            field,
            "must not be empty".to_string(),
        ));
    }
    Ok(values)
}

fn required_u64(obj: &Map<String, Value>, field: &'static str) -> Result<u64, CompatibilityError> {
    match obj.get(field) {
        Some(Value::Number(number)) => number.as_u64().ok_or_else(|| {
            CompatibilityError::InvalidField(field, "must be an unsigned integer".to_string())
        }),
        Some(Value::String(raw)) => raw.trim().parse::<u64>().map_err(|_| {
            CompatibilityError::InvalidField(field, "must be an unsigned integer".to_string())
        }),
        Some(_) => Err(CompatibilityError::InvalidField(
            field,
            "must be an unsigned integer".to_string(),
        )),
        None => Err(CompatibilityError::MissingField(field)),
    }
}

fn optional_u64_field(
    obj: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<u64>, CompatibilityError> {
    match obj.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => required_u64(obj, field).map(Some),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model_id() -> &'static str {
        "easynet:///r/example/ability/alice.codex.chat"
    }

    fn base_request(extra: Value) -> Value {
        let mut request = json!({
            "caller_ura": "easynet:///r/example/agent/alice.sdk",
            "callee_ura": "easynet:///r/example/device/dev-a",
            "subject_ura": "easynet:///r/example/device/dev-a",
            "descriptor_version": "1.0.0",
            "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
            "causal_context": {"form": "none"},
            "metadata": {"request_id": "compat-1"}
        });
        let Value::Object(extra) = extra else {
            return request;
        };
        let obj = request.as_object_mut().unwrap();
        for (key, value) in extra {
            obj.insert(key, value);
        }
        request
    }

    #[test]
    fn build_list_models_invocation_preserves_complete_tuple() {
        let request = base_request(json!({
            "auth_token": "tok_123"
        }));

        let carrier = build_list_models_invocation(&request).unwrap();

        assert_eq!(carrier["metadata"]["profile"], COMPATIBILITY_PROFILE);
        assert_eq!(carrier["metadata"]["system_ability"], "openai.list_models");
        assert_eq!(carrier["args"]["auth_token"], "tok_123");
        assert_eq!(
            carrier["descriptor_ref"],
            "easynet:///r/example/ability/device.dev-a.openai.list_models@1.0.0"
        );
    }

    #[test]
    fn build_chat_completion_rejects_provider_nickname_model() {
        let request = base_request(json!({
            "request": {
                "model": "gpt-5",
                "messages": [{"role": "user", "content": "hello"}]
            }
        }));

        let err = build_chat_completion_invocation(&request).unwrap_err();

        assert!(err.to_string().contains("canonical Ability URA"));
    }

    #[test]
    fn build_chat_completion_rejects_stream_true_for_unary_carrier() {
        let request = base_request(json!({
            "request": {
                "model": model_id(),
                "stream": true,
                "messages": [{"role": "user", "content": "hello"}]
            }
        }));

        let err = build_chat_completion_invocation(&request).unwrap_err();

        assert!(err.to_string().contains("stream_chat_completions"));
    }

    #[test]
    fn build_stream_chat_completion_sets_stream_true() {
        let request = base_request(json!({
            "request": {
                "model": model_id(),
                "messages": [{"role": "user", "content": "hello"}]
            }
        }));

        let carrier = build_stream_chat_completion_invocation(&request).unwrap();

        assert_eq!(
            carrier["metadata"]["system_ability"],
            "openai.chat_completions"
        );
        assert_eq!(carrier["args"]["request"]["stream"], true);
    }

    #[test]
    fn project_model_page_validates_model_ids() {
        let input = json!({
            "object": "list",
            "data": [{
                "id": model_id(),
                "object": "model",
                "created": 0,
                "owned_by": "easynet",
                "ability": "codex.chat"
            }]
        });

        let page = project_model_page(&input).unwrap();

        assert_eq!(page["kind"], "model_page");
        assert_eq!(page["data"][0]["ability_ref"], model_id());
        assert_eq!(page["data"][0]["metadata"]["daemon_ability"], "codex.chat");
    }

    #[test]
    fn project_model_page_rejects_malformed_created() {
        let input = json!({
            "object": "list",
            "data": [{
                "id": model_id(),
                "object": "model",
                "created": {"seconds": 0}
            }]
        });

        let err = project_model_page(&input).unwrap_err();

        assert!(err.to_string().contains("unsigned integer"));
    }

    #[test]
    fn project_chat_completion_preserves_choices_and_usage() {
        let input = json!({
            "id": "chatcmpl-1",
            "object": "chat.completion",
            "created": 1,
            "model": model_id(),
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "ok"},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2},
            "easynet_user_ura": "easynet:///r/example/user/alice"
        });

        let completion = project_chat_completion(&input).unwrap();

        assert_eq!(completion["kind"], "chat_completion");
        assert_eq!(completion["choices"][0]["message"]["content"], "ok");
        assert_eq!(
            completion["metadata"]["easynet_user_ura"],
            "easynet:///r/example/user/alice"
        );
    }

    #[test]
    fn project_chat_stream_projects_chunks_and_done_sentinel() {
        let input = json!({
            "stream": true,
            "chunks": [{
                "id": "chatcmpl-1",
                "object": "chat.completion.chunk",
                "created": 1,
                "model": model_id(),
                "choices": [{
                    "index": 0,
                    "delta": {"role": "assistant"},
                    "finish_reason": null
                }]
            }],
            "done_sentinel": "[DONE]"
        });

        let stream = project_chat_stream(&input).unwrap();

        assert_eq!(stream["kind"], "chat_completion_stream");
        assert_eq!(stream["items"][0]["kind"], "chat_completion_chunk");
        assert_eq!(stream["done_sentinel"], "[DONE]");
    }
}
