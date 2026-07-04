// EasyNet CLI — Compatibility shared contract
// ============================================
//
// File: src/daemon/compatibility_contract.rs
// Description: Shared daemon SDK contract for OpenAI-compatible carrier
//              construction and result/file DTO projection.
//
// Protocol Responsibility
// -----------------------
// Own the EasyNet-Cli SDK Compatibility DTO projection that lowers
// OpenAI-shaped model/chat requests to governed daemon abilities and projects
// file-wrapper facts into compatibility DTOs. This module does not own HTTP
// auth, billing, rate limits, SSE fanout, model execution, file storage, or
// OpenAI as daemon protocol.
//
// Implementation Approach
// -----------------------
// Reuse the shared daemon SDK carrier builder for complete Invocation tuples
// and delegate chat-model identity validation to the daemon OpenAI adapter.
// Projection validates daemon-returned OpenAI-compatible envelopes and
// SDK file/resource facts into stable DTOs without fabricating model ids,
// chunks, completion ids, file ids, or usage.
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

use std::path::Path;
use std::time::UNIX_EPOCH;

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::daemon::ability::builtins::integrations::openai_compat;
use crate::daemon::sdk_contract::{
    build_system_invocation, object, optional_bool_field, optional_string_field, required_string,
    validate_ura, SdkContractError,
};

const COMPATIBILITY_PROFILE: &str = "compatibility";
const ABILITY_OPENAI_LIST_MODELS: &str =
    crate::daemon::ability::names::integrations::OPENAI_LIST_MODELS;
const ABILITY_OPENAI_CHAT_COMPLETIONS: &str =
    crate::daemon::ability::names::integrations::OPENAI_CHAT_COMPLETIONS;
const ABILITY_OPENAI_FILES_UPLOAD: &str = "openai.files.upload";
const ABILITY_OPENAI_FILES_RETRIEVE: &str = "openai.files.retrieve";
const ABILITY_OPENAI_FILES_DELETE: &str = "openai.files.delete";

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

pub(crate) fn build_file_upload_invocation(request: &Value) -> Result<Value, CompatibilityError> {
    let obj = object(request, "CompatibilityFileUploadRequest")?;
    let purpose = required_string(obj, "purpose")?;
    let mut args = Map::new();
    args.insert("purpose".to_string(), Value::String(purpose.to_string()));
    insert_file_ref_arg(obj, &mut args)?;
    insert_auth_arg(obj, &mut args)?;
    build_system_invocation(
        obj,
        COMPATIBILITY_PROFILE,
        ABILITY_OPENAI_FILES_UPLOAD,
        Value::Object(args),
    )
}

pub(crate) fn build_file_retrieve_invocation(request: &Value) -> Result<Value, CompatibilityError> {
    let obj = object(request, "CompatibilityFileRequest")?;
    let mut args = Map::new();
    args.insert(
        "file_id".to_string(),
        Value::String(compatibility_file_id(obj)?),
    );
    insert_auth_arg(obj, &mut args)?;
    build_system_invocation(
        obj,
        COMPATIBILITY_PROFILE,
        ABILITY_OPENAI_FILES_RETRIEVE,
        Value::Object(args),
    )
}

pub(crate) fn build_file_delete_invocation(request: &Value) -> Result<Value, CompatibilityError> {
    let obj = object(request, "CompatibilityFileDeleteRequest")?;
    let deleted = obj
        .get("deleted")
        .and_then(Value::as_bool)
        .ok_or(CompatibilityError::MissingField("deleted"))?;
    if !deleted {
        return Err(CompatibilityError::InvalidField(
            "deleted",
            "must be true for file delete carriers".to_string(),
        ));
    }
    let mut args = Map::new();
    args.insert(
        "file_id".to_string(),
        Value::String(compatibility_file_id(obj)?),
    );
    args.insert("deleted".to_string(), Value::Bool(true));
    insert_auth_arg(obj, &mut args)?;
    build_system_invocation(
        obj,
        COMPATIBILITY_PROFILE,
        ABILITY_OPENAI_FILES_DELETE,
        Value::Object(args),
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

pub(crate) fn project_file_upload(input: &Value) -> Result<Value, CompatibilityError> {
    let obj = object(input, "CompatibilityFileUploadRequest")?;
    let purpose = required_string(obj, "purpose")?;
    let facts = if let Some(path) = optional_string_field(obj, "path")? {
        CompatibilityFileFacts::from_local_path(obj, &path, purpose)?
    } else {
        CompatibilityFileFacts::from_file_facts(obj, Some(purpose))?
    };
    facts.file_json("upload")
}

pub(crate) fn project_file(input: &Value) -> Result<Value, CompatibilityError> {
    let obj = object(input, "CompatibilityFileInput")?;
    CompatibilityFileFacts::from_file_facts(obj, None)?.file_json("file")
}

pub(crate) fn project_file_delete_result(input: &Value) -> Result<Value, CompatibilityError> {
    let obj = object(input, "CompatibilityFileDeleteInput")?;
    let id = compatibility_file_id(obj)?;
    let deleted = obj
        .get("deleted")
        .and_then(Value::as_bool)
        .ok_or(CompatibilityError::MissingField("deleted"))?;
    let mut metadata = typed_metadata(obj)?;
    metadata.insert(
        "profile".to_string(),
        Value::String(COMPATIBILITY_PROFILE.to_string()),
    );
    metadata.insert(
        "source".to_string(),
        Value::String("compatibility.file_delete".to_string()),
    );
    Ok(json!({
        "profile": COMPATIBILITY_PROFILE,
        "kind": "file_delete_result",
        "id": id,
        "object": "file",
        "deleted": deleted,
        "metadata": metadata,
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
    insert_auth_arg(obj, &mut args)?;
    Ok(Value::Object(args))
}

fn insert_auth_arg(
    obj: &Map<String, Value>,
    args: &mut Map<String, Value>,
) -> Result<(), CompatibilityError> {
    if let Some(auth_token) = optional_string_field(obj, "auth_token")? {
        args.insert("auth_token".to_string(), Value::String(auth_token));
    }
    Ok(())
}

fn insert_file_ref_arg(
    obj: &Map<String, Value>,
    args: &mut Map<String, Value>,
) -> Result<(), CompatibilityError> {
    if let Some(file_ref) = optional_string_field(obj, "file_ref")? {
        args.insert("file_ref".to_string(), Value::String(file_ref));
        return Ok(());
    }
    if let Some(resource_ref) = optional_string_field(obj, "resource_ref")? {
        args.insert("file_ref".to_string(), Value::String(resource_ref));
        return Ok(());
    }
    if let Some(resource_ura) = optional_string_field(obj, "resource_ura")? {
        args.insert("file_ref".to_string(), Value::String(resource_ura));
        return Ok(());
    }
    let file_id = compatibility_file_id(obj)?;
    args.insert("file_id".to_string(), Value::String(file_id));
    Ok(())
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

#[derive(Debug, Clone)]
struct CompatibilityFileFacts {
    id: String,
    filename: String,
    purpose: String,
    bytes: u64,
    created_at: u64,
    status: String,
    content_hash: Option<String>,
    file_ref: Option<String>,
    owner_ura: Option<String>,
    content_type: Option<String>,
    metadata: Map<String, Value>,
}

impl CompatibilityFileFacts {
    fn from_local_path(
        obj: &Map<String, Value>,
        path: &str,
        purpose: &str,
    ) -> Result<Self, CompatibilityError> {
        let path = validate_local_file_path(path)?;
        let bytes = std::fs::read(path).map_err(|err| {
            CompatibilityError::Contract(format!(
                "read local upload file {}: {err}",
                path.display()
            ))
        })?;
        let content_hash = sha256_content_hash(&bytes);
        let metadata = std::fs::metadata(path).map_err(|err| {
            CompatibilityError::Contract(format!(
                "stat local upload file {}: {err}",
                path.display()
            ))
        })?;
        let filename = optional_string_field(obj, "filename")?.unwrap_or_else(|| {
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("upload.bin")
                .to_string()
        });
        let owner_ura = optional_string_field(obj, "owner_ura")?;
        if let Some(owner_ura) = owner_ura.as_deref() {
            validate_ura(owner_ura, "owner_ura")?;
        }
        let created_at = optional_u64_field(obj, "created_at")?.unwrap_or_else(|| {
            metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_secs())
                .unwrap_or(0)
        });
        let mut dto_metadata = typed_metadata(obj)?;
        dto_metadata.insert(
            "local_path".to_string(),
            Value::String(path.display().to_string()),
        );
        dto_metadata.insert(
            "content_hash".to_string(),
            Value::String(content_hash.clone()),
        );
        Ok(Self {
            id: optional_string_field(obj, "id")?
                .or_else(|| optional_string_field(obj, "file_id").ok().flatten())
                .unwrap_or_else(|| file_id_from_stable_ref(&content_hash)),
            filename,
            purpose: purpose.to_string(),
            bytes: metadata.len(),
            created_at,
            status: optional_string_field(obj, "status")?
                .unwrap_or_else(|| "processed".to_string()),
            content_hash: Some(content_hash),
            file_ref: optional_string_field(obj, "file_ref")?
                .or_else(|| optional_string_field(obj, "resource_ref").ok().flatten()),
            owner_ura,
            content_type: optional_string_field(obj, "content_type")?,
            metadata: dto_metadata,
        })
    }

    fn from_file_facts(
        obj: &Map<String, Value>,
        purpose_override: Option<&str>,
    ) -> Result<Self, CompatibilityError> {
        let id = compatibility_file_id(obj)?;
        let filename = optional_string_field(obj, "filename")?.unwrap_or_else(|| id.clone());
        let purpose = purpose_override
            .map(str::to_string)
            .or_else(|| optional_string_field(obj, "purpose").ok().flatten())
            .unwrap_or_else(|| "assistants".to_string());
        let bytes = optional_u64_field(obj, "bytes")?
            .or_else(|| optional_u64_field(obj, "size_bytes").ok().flatten())
            .unwrap_or(0);
        let created_at = optional_u64_field(obj, "created_at")?
            .or_else(|| optional_u64_field(obj, "created").ok().flatten())
            .unwrap_or(0);
        let owner_ura = optional_string_field(obj, "owner_ura")?;
        if let Some(owner_ura) = owner_ura.as_deref() {
            validate_ura(owner_ura, "owner_ura")?;
        }
        let content_hash = optional_string_field(obj, "content_hash")?;
        let file_ref = optional_string_field(obj, "file_ref")?
            .or_else(|| optional_string_field(obj, "resource_ref").ok().flatten())
            .or_else(|| optional_string_field(obj, "resource_ura").ok().flatten());
        let mut metadata = typed_metadata(obj)?;
        if let Some(hash) = content_hash.as_deref() {
            metadata.insert("content_hash".to_string(), Value::String(hash.to_string()));
        }
        if let Some(file_ref) = file_ref.as_deref() {
            metadata.insert("file_ref".to_string(), Value::String(file_ref.to_string()));
        }
        Ok(Self {
            id,
            filename,
            purpose,
            bytes,
            created_at,
            status: optional_string_field(obj, "status")?
                .unwrap_or_else(|| "processed".to_string()),
            content_hash,
            file_ref,
            owner_ura,
            content_type: optional_string_field(obj, "content_type")?,
            metadata,
        })
    }

    fn file_json(mut self, source: &'static str) -> Result<Value, CompatibilityError> {
        if self.id.trim().is_empty() {
            return Err(CompatibilityError::MissingField("id"));
        }
        self.metadata.insert(
            "profile".to_string(),
            Value::String(COMPATIBILITY_PROFILE.to_string()),
        );
        self.metadata.insert(
            "source".to_string(),
            Value::String(format!("compatibility.{source}")),
        );
        if let Some(owner_ura) = self.owner_ura.as_deref() {
            self.metadata.insert(
                "owner_ura".to_string(),
                Value::String(owner_ura.to_string()),
            );
        }
        if let Some(content_type) = self.content_type.as_deref() {
            self.metadata.insert(
                "content_type".to_string(),
                Value::String(content_type.to_string()),
            );
        }
        if let Some(content_hash) = self.content_hash.as_deref() {
            self.metadata.insert(
                "content_hash".to_string(),
                Value::String(content_hash.to_string()),
            );
        }
        if let Some(file_ref) = self.file_ref.as_deref() {
            self.metadata
                .insert("file_ref".to_string(), Value::String(file_ref.to_string()));
        }
        Ok(json!({
            "profile": COMPATIBILITY_PROFILE,
            "kind": "file",
            "id": self.id,
            "object": "file",
            "bytes": self.bytes,
            "created_at": self.created_at,
            "filename": self.filename,
            "purpose": self.purpose,
            "status": self.status,
            "metadata": self.metadata,
        }))
    }
}

fn validate_local_file_path(raw: &str) -> Result<&Path, CompatibilityError> {
    let path = Path::new(raw);
    if !path.is_absolute() {
        return Err(CompatibilityError::InvalidField(
            "path",
            "must be an absolute path".to_string(),
        ));
    }
    if !path.is_file() {
        return Err(CompatibilityError::InvalidField(
            "path",
            "must be an existing file".to_string(),
        ));
    }
    Ok(path)
}

fn sha256_content_hash(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!("sha256:{}", hex::encode(digest))
}

fn file_id_from_stable_ref(raw: &str) -> String {
    let digest = Sha256::digest(raw.as_bytes());
    let hex = hex::encode(digest);
    format!("file-{}", &hex[..24])
}

fn compatibility_file_id(obj: &Map<String, Value>) -> Result<String, CompatibilityError> {
    if let Some(id) = optional_string_field(obj, "id")?
        .or_else(|| optional_string_field(obj, "file_id").ok().flatten())
    {
        return Ok(id);
    }
    let stable_ref = optional_string_field(obj, "file_ref")?
        .or_else(|| optional_string_field(obj, "resource_ref").ok().flatten())
        .or_else(|| optional_string_field(obj, "resource_ura").ok().flatten())
        .or_else(|| optional_string_field(obj, "content_hash").ok().flatten())
        .ok_or(CompatibilityError::MissingField("id"))?;
    Ok(file_id_from_stable_ref(&stable_ref))
}

fn typed_metadata(obj: &Map<String, Value>) -> Result<Map<String, Value>, CompatibilityError> {
    match obj.get("metadata") {
        None | Some(Value::Null) => Ok(Map::new()),
        Some(Value::Object(metadata)) => Ok(metadata.clone()),
        Some(_) => Err(CompatibilityError::InvalidField(
            "metadata",
            "must be an object".to_string(),
        )),
    }
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
    fn build_file_carriers_preserve_minimal_daemon_args() {
        let upload = build_file_upload_invocation(&base_request(json!({
            "auth_token": "tok_123",
            "purpose": "batch",
            "file_ref": "easynet:///r/example/resource/alice.files/prompt.jsonl",
            "id": "file-easynet-docs-1"
        })))
        .unwrap();

        assert_eq!(
            upload["descriptor_ref"],
            "easynet:///r/example/ability/device.dev-a.openai.files.upload@1.0.0"
        );
        assert_eq!(upload["metadata"]["system_ability"], "openai.files.upload");
        assert_eq!(
            upload["args"]["file_ref"],
            "easynet:///r/example/resource/alice.files/prompt.jsonl"
        );
        assert_eq!(upload["args"]["purpose"], "batch");
        assert_eq!(upload["args"]["auth_token"], "tok_123");
        assert!(upload["args"].get("id").is_none());

        let retrieve = build_file_retrieve_invocation(&base_request(json!({
            "id": "file-easynet-docs-1"
        })))
        .unwrap();
        assert_eq!(
            retrieve["metadata"]["system_ability"],
            "openai.files.retrieve"
        );
        assert_eq!(retrieve["args"]["file_id"], "file-easynet-docs-1");

        let delete = build_file_delete_invocation(&base_request(json!({
            "id": "file-easynet-docs-1",
            "deleted": true
        })))
        .unwrap();
        assert_eq!(delete["metadata"]["system_ability"], "openai.files.delete");
        assert_eq!(delete["args"]["file_id"], "file-easynet-docs-1");
        assert_eq!(delete["args"]["deleted"], true);
    }

    #[test]
    fn build_file_delete_rejects_false_delete_intent() {
        let err = build_file_delete_invocation(&base_request(json!({
            "id": "file-easynet-docs-1",
            "deleted": false
        })))
        .unwrap_err();

        assert!(err.to_string().contains("must be true"));
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

    #[test]
    fn project_file_upload_reads_local_file_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("prompt.jsonl");
        std::fs::write(&path, b"{\"prompt\":\"hello\"}\n").unwrap();

        let file = project_file_upload(&json!({
            "path": path.display().to_string(),
            "purpose": "batch",
            "owner_ura": "easynet:///r/example/agent/alice.sdk",
            "content_type": "application/jsonl"
        }))
        .unwrap();

        assert_eq!(file["kind"], "file");
        assert_eq!(file["object"], "file");
        assert_eq!(file["bytes"], 19);
        assert_eq!(file["filename"], "prompt.jsonl");
        assert_eq!(file["purpose"], "batch");
        assert!(file["id"].as_str().unwrap().starts_with("file-"));
        assert!(file["metadata"]["content_hash"]
            .as_str()
            .unwrap()
            .starts_with("sha256:"));
    }

    #[test]
    fn project_file_upload_rejects_relative_path() {
        let err = project_file_upload(&json!({
            "path": "prompt.jsonl",
            "purpose": "batch"
        }))
        .unwrap_err();

        assert!(err.to_string().contains("absolute path"));
    }

    #[test]
    fn project_file_projects_explicit_file_record() {
        let file = project_file(&json!({
            "file_ref": "easynet:///r/example/resource/alice.files/prompt.jsonl",
            "owner_ura": "easynet:///r/example/agent/alice.sdk",
            "filename": "prompt.jsonl",
            "purpose": "assistants",
            "size_bytes": 19,
            "content_hash": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "content_type": "application/jsonl"
        }))
        .unwrap();

        assert_eq!(file["kind"], "file");
        assert!(file["id"].as_str().unwrap().starts_with("file-"));
        assert_eq!(file["bytes"], 19);
        assert_eq!(
            file["metadata"]["file_ref"],
            "easynet:///r/example/resource/alice.files/prompt.jsonl"
        );
    }

    #[test]
    fn project_file_delete_result_requires_explicit_deleted_flag() {
        let err = project_file_delete_result(&json!({"id": "file-123"})).unwrap_err();

        assert!(err.to_string().contains("deleted"));
    }

    #[test]
    fn project_file_delete_result_projects_ack() {
        let result = project_file_delete_result(&json!({
            "id": "file-123",
            "deleted": true
        }))
        .unwrap();

        assert_eq!(result["kind"], "file_delete_result");
        assert_eq!(result["object"], "file");
        assert_eq!(result["deleted"], true);
    }
}
