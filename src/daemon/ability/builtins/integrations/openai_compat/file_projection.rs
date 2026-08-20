// EasyNet CLI — OpenAI-compatible file projection
// =================================================
//
// File: src/daemon/ability/builtins/integrations/openai_compat/file_projection.rs
// Description: OpenAI file DTO projection owned by the OpenAI adapter ability.

use std::path::Path;
use std::time::UNIX_EPOCH;

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

const PROFILE: &str = "compatibility";

#[derive(Debug, thiserror::Error)]
pub(super) enum FileProjectionError {
    #[error("missing required field {0}")]
    MissingField(&'static str),
    #[error("invalid field {0}: {1}")]
    InvalidField(&'static str, String),
    #[error("{0}")]
    Io(String),
}

pub(super) fn project_file_upload(input: &Value) -> Result<Value, FileProjectionError> {
    let obj = object(input, "OpenAIFileUpload")?;
    let purpose = required_string(obj, "purpose")?;
    let facts = if let Some(path) = optional_string(obj, "path")? {
        FileFacts::from_local_path(obj, &path, purpose)?
    } else {
        FileFacts::from_wire(obj, Some(purpose))?
    };
    facts.into_json("upload")
}

pub(super) fn project_file(input: &Value) -> Result<Value, FileProjectionError> {
    let obj = object(input, "OpenAIFile")?;
    FileFacts::from_wire(obj, None)?.into_json("file")
}

pub(super) fn project_file_delete_result(input: &Value) -> Result<Value, FileProjectionError> {
    let obj = object(input, "OpenAIFileDeleteResult")?;
    let id = file_id(obj)?;
    let deleted = obj
        .get("deleted")
        .and_then(Value::as_bool)
        .ok_or(FileProjectionError::MissingField("deleted"))?;
    let mut metadata = metadata(obj)?;
    metadata.insert("profile".to_string(), Value::String(PROFILE.to_string()));
    metadata.insert(
        "source".to_string(),
        Value::String("compatibility.file_delete".to_string()),
    );
    Ok(json!({
        "profile": PROFILE,
        "kind": "file_delete_result",
        "id": id,
        "object": "file",
        "deleted": deleted,
        "metadata": metadata,
    }))
}

#[derive(Debug)]
struct FileFacts {
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

impl FileFacts {
    fn from_local_path(
        obj: &Map<String, Value>,
        path: &str,
        purpose: &str,
    ) -> Result<Self, FileProjectionError> {
        let path = local_file(path)?;
        let bytes = std::fs::read(path).map_err(|error| {
            FileProjectionError::Io(format!(
                "read local upload file {}: {error}",
                path.display()
            ))
        })?;
        let content_hash = content_hash(&bytes);
        let filesystem_metadata = std::fs::metadata(path).map_err(|error| {
            FileProjectionError::Io(format!(
                "stat local upload file {}: {error}",
                path.display()
            ))
        })?;
        let filename = optional_string(obj, "filename")?.unwrap_or_else(|| {
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("upload.bin")
                .to_string()
        });
        let owner_ura = validated_owner(obj)?;
        let created_at = optional_u64(obj, "created_at")?.unwrap_or_else(|| {
            filesystem_metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_secs())
                .unwrap_or(0)
        });
        let mut projected_metadata = metadata(obj)?;
        projected_metadata.insert(
            "local_path".to_string(),
            Value::String(path.display().to_string()),
        );
        projected_metadata.insert(
            "content_hash".to_string(),
            Value::String(content_hash.clone()),
        );
        Ok(Self {
            id: optional_string(obj, "id")?
                .or(optional_string(obj, "file_id")?)
                .unwrap_or_else(|| stable_file_id(&content_hash)),
            filename,
            purpose: purpose.to_string(),
            bytes: filesystem_metadata.len(),
            created_at,
            status: optional_string(obj, "status")?.unwrap_or_else(|| "processed".to_string()),
            content_hash: Some(content_hash),
            file_ref: optional_string(obj, "file_ref")?.or(optional_string(obj, "resource_ref")?),
            owner_ura,
            content_type: optional_string(obj, "content_type")?,
            metadata: projected_metadata,
        })
    }

    fn from_wire(
        obj: &Map<String, Value>,
        purpose_override: Option<&str>,
    ) -> Result<Self, FileProjectionError> {
        let id = file_id(obj)?;
        let filename = optional_string(obj, "filename")?.unwrap_or_else(|| id.clone());
        let purpose = purpose_override
            .map(str::to_string)
            .or(optional_string(obj, "purpose")?)
            .unwrap_or_else(|| "assistants".to_string());
        let bytes = optional_u64(obj, "bytes")?
            .or(optional_u64(obj, "size_bytes")?)
            .unwrap_or(0);
        let created_at = optional_u64(obj, "created_at")?
            .or(optional_u64(obj, "created")?)
            .unwrap_or(0);
        let owner_ura = validated_owner(obj)?;
        let content_hash = optional_string(obj, "content_hash")?;
        let file_ref = optional_string(obj, "file_ref")?
            .or(optional_string(obj, "resource_ref")?)
            .or(optional_string(obj, "resource_ura")?);
        let mut projected_metadata = metadata(obj)?;
        if let Some(value) = content_hash.as_ref() {
            projected_metadata.insert("content_hash".to_string(), Value::String(value.clone()));
        }
        if let Some(value) = file_ref.as_ref() {
            projected_metadata.insert("file_ref".to_string(), Value::String(value.clone()));
        }
        Ok(Self {
            id,
            filename,
            purpose,
            bytes,
            created_at,
            status: optional_string(obj, "status")?.unwrap_or_else(|| "processed".to_string()),
            content_hash,
            file_ref,
            owner_ura,
            content_type: optional_string(obj, "content_type")?,
            metadata: projected_metadata,
        })
    }

    fn into_json(mut self, source: &'static str) -> Result<Value, FileProjectionError> {
        if self.id.trim().is_empty() {
            return Err(FileProjectionError::MissingField("id"));
        }
        self.metadata
            .insert("profile".to_string(), Value::String(PROFILE.to_string()));
        self.metadata.insert(
            "source".to_string(),
            Value::String(format!("compatibility.{source}")),
        );
        insert_optional_metadata(&mut self.metadata, "owner_ura", self.owner_ura);
        insert_optional_metadata(&mut self.metadata, "content_type", self.content_type);
        insert_optional_metadata(&mut self.metadata, "content_hash", self.content_hash);
        insert_optional_metadata(&mut self.metadata, "file_ref", self.file_ref);
        Ok(json!({
            "profile": PROFILE,
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

fn object<'a>(
    value: &'a Value,
    field: &'static str,
) -> Result<&'a Map<String, Value>, FileProjectionError> {
    value
        .as_object()
        .ok_or_else(|| FileProjectionError::InvalidField(field, "must be an object".to_string()))
}

fn required_string<'a>(
    obj: &'a Map<String, Value>,
    field: &'static str,
) -> Result<&'a str, FileProjectionError> {
    obj.get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or(FileProjectionError::MissingField(field))
}

fn optional_string(
    obj: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<String>, FileProjectionError> {
    match obj.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => {
            let value = value.trim();
            Ok((!value.is_empty()).then(|| value.to_string()))
        }
        Some(_) => Err(FileProjectionError::InvalidField(
            field,
            "must be a string".to_string(),
        )),
    }
}

fn optional_u64(
    obj: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<u64>, FileProjectionError> {
    match obj.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(number)) => number.as_u64().map(Some).ok_or_else(|| {
            FileProjectionError::InvalidField(field, "must be an unsigned integer".to_string())
        }),
        Some(Value::String(value)) => value.trim().parse::<u64>().map(Some).map_err(|_| {
            FileProjectionError::InvalidField(field, "must be an unsigned integer".to_string())
        }),
        Some(_) => Err(FileProjectionError::InvalidField(
            field,
            "must be an unsigned integer".to_string(),
        )),
    }
}

fn validated_owner(obj: &Map<String, Value>) -> Result<Option<String>, FileProjectionError> {
    let owner_ura = optional_string(obj, "owner_ura")?;
    if let Some(value) = owner_ura.as_deref() {
        crate::core::ura::parse_ura(value)
            .map_err(|error| FileProjectionError::InvalidField("owner_ura", error.to_string()))?;
    }
    Ok(owner_ura)
}

fn file_id(obj: &Map<String, Value>) -> Result<String, FileProjectionError> {
    if let Some(id) = optional_string(obj, "id")?.or(optional_string(obj, "file_id")?) {
        return Ok(id);
    }
    let stable_ref = optional_string(obj, "file_ref")?
        .or(optional_string(obj, "resource_ref")?)
        .or(optional_string(obj, "resource_ura")?)
        .or(optional_string(obj, "content_hash")?)
        .ok_or(FileProjectionError::MissingField("id"))?;
    Ok(stable_file_id(&stable_ref))
}

fn metadata(obj: &Map<String, Value>) -> Result<Map<String, Value>, FileProjectionError> {
    match obj.get("metadata") {
        None | Some(Value::Null) => Ok(Map::new()),
        Some(Value::Object(metadata)) => Ok(metadata.clone()),
        Some(_) => Err(FileProjectionError::InvalidField(
            "metadata",
            "must be an object".to_string(),
        )),
    }
}

fn local_file(raw: &str) -> Result<&Path, FileProjectionError> {
    let path = Path::new(raw);
    if !path.is_absolute() {
        return Err(FileProjectionError::InvalidField(
            "path",
            "must be an absolute path".to_string(),
        ));
    }
    if !path.is_file() {
        return Err(FileProjectionError::InvalidField(
            "path",
            "must be an existing file".to_string(),
        ));
    }
    Ok(path)
}

fn content_hash(bytes: &[u8]) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
}

fn stable_file_id(raw: &str) -> String {
    let digest = hex::encode(Sha256::digest(raw.as_bytes()));
    format!("file-{}", &digest[..24])
}

fn insert_optional_metadata(
    metadata: &mut Map<String, Value>,
    field: &'static str,
    value: Option<String>,
) {
    if let Some(value) = value {
        metadata.insert(field.to_string(), Value::String(value));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_projection_preserves_openai_shape_and_resource_ref() {
        let file = project_file(&json!({
            "id": "abc",
            "file_ref": "easynet:///r/example/resource/blob/abc",
            "filename": "input.txt",
            "bytes": 12,
        }))
        .unwrap();

        assert_eq!(file["object"], "file");
        assert_eq!(file["id"], "abc");
        assert_eq!(
            file["metadata"]["file_ref"],
            "easynet:///r/example/resource/blob/abc"
        );
    }

    #[test]
    fn delete_projection_requires_explicit_deleted_fact() {
        let error = project_file_delete_result(&json!({"id": "abc"})).unwrap_err();
        assert!(matches!(
            error,
            FileProjectionError::MissingField("deleted")
        ));
    }
}
