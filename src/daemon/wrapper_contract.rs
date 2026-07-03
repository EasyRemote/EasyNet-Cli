// EasyNet CLI — Convenience Wrapper shared contract
// =================================================
//
// File: src/daemon/wrapper_contract.rs
// Description: Shared daemon SDK contract for Convenience Wrapper record
//              projections.
//
// Protocol Responsibility
// -----------------------
// Own SDK wrapper DTO semantics for file, terminal, remote desktop, browser,
// and media session records. This module does not start sessions, parse product
// HTTP/WebSocket requests, execute abilities, or replace Runtime Core
// Invocation/stream/bidi paths.
//
// Implementation Approach
// -----------------------
// Validate explicit daemon/resource facts, preserve owner URAs, and normalize
// records into schema-backed DTOs with profile metadata. Session record
// projection is centralized in a small value object so each wrapper family uses
// the same identity and lifecycle rules.
//
// Usage Contract
// --------------
// Callers pass object-shaped JSON with explicit identity and state fields.
// Missing owner/session/file facts and invalid owner URAs are rejected. Product
// facades may adapt HTTP/UI/session protocols above this boundary, but the SDK
// record shape remains shared across languages.
//
// Architectural Position
// ----------------------
// EasyNet-Cli daemon SDK Convenience Wrapper profile. Execution helpers built
// later must lower through Runtime Core complete Invocation, StreamHandle, or
// BidiSession objects rather than becoming a one-method-per-ability transport.

use std::fmt;

use serde_json::{json, Map, Value};

use crate::daemon::sdk_contract::{
    object, optional_string_field, required_string, validate_ura, SdkContractError,
};

const WRAPPERS_PROFILE: &str = "wrappers";

pub(crate) fn project_file_record(input: &Value) -> Result<Value, WrapperError> {
    let obj = object(input, "FileRecord")?;
    let file_ref = required_string(obj, "file_ref")?;
    let owner_ura = required_string(obj, "owner_ura")?;
    validate_ura(owner_ura, "owner_ura")?;
    let content_type = required_string(obj, "content_type")?;
    let size_bytes = optional_u64_field(obj, "size_bytes")?;
    let content_hash = optional_string_field(obj, "content_hash")?;
    let mut metadata = typed_metadata(obj)?;
    stamp_metadata(&mut metadata, "wrappers.file_record");
    Ok(json!({
        "profile": WRAPPERS_PROFILE,
        "kind": "file_record",
        "file_ref": file_ref,
        "owner_ura": owner_ura,
        "content_type": content_type,
        "size_bytes": size_bytes,
        "content_hash": content_hash,
        "metadata": metadata,
    }))
}

pub(crate) fn project_terminal_session(input: &Value) -> Result<Value, WrapperError> {
    let obj = object(input, "TerminalSessionRecord")?;
    WrapperSessionProjection::new(obj, "terminal_session", "terminal_ref")?
        .session_json("wrappers.terminal_session")
}

pub(crate) fn project_remote_desktop_session(input: &Value) -> Result<Value, WrapperError> {
    let obj = object(input, "RemoteDesktopSessionRecord")?;
    WrapperSessionProjection::new(obj, "remote_desktop_session", "display_ref")?
        .session_json("wrappers.remote_desktop_session")
}

pub(crate) fn project_browser_session(input: &Value) -> Result<Value, WrapperError> {
    let obj = object(input, "BrowserSessionRecord")?;
    WrapperSessionProjection::new(obj, "browser_session", "browser_ref")?
        .session_json("wrappers.browser_session")
}

pub(crate) fn project_media_session(input: &Value) -> Result<Value, WrapperError> {
    let obj = object(input, "MediaSessionRecord")?;
    let media_kind = required_string(obj, "media_kind")?.to_string();
    WrapperSessionProjection::new(obj, "media_session", "stream_ref")?
        .with_extra("media_kind", media_kind)
        .session_json("wrappers.media_session")
}

#[derive(Debug, Clone)]
struct WrapperSessionProjection {
    kind: &'static str,
    session_id: String,
    owner_ura: String,
    state: String,
    ref_key: &'static str,
    ref_value: Option<String>,
    metadata: Map<String, Value>,
    extra: Map<String, Value>,
}

impl WrapperSessionProjection {
    fn new(
        obj: &Map<String, Value>,
        kind: &'static str,
        ref_key: &'static str,
    ) -> Result<Self, WrapperError> {
        let session_id = required_string(obj, "session_id")?.to_string();
        let owner_ura = required_string(obj, "owner_ura")?.to_string();
        validate_ura(&owner_ura, "owner_ura")?;
        let state = required_string(obj, "state")?.to_string();
        Ok(Self {
            kind,
            session_id,
            owner_ura,
            state,
            ref_key,
            ref_value: optional_string_field(obj, ref_key)?,
            metadata: typed_metadata(obj)?,
            extra: Map::new(),
        })
    }

    fn with_extra(mut self, key: &'static str, value: String) -> Self {
        self.extra.insert(key.to_string(), Value::String(value));
        self
    }

    fn session_json(mut self, source: &'static str) -> Result<Value, WrapperError> {
        stamp_metadata(&mut self.metadata, source);
        let mut record = Map::new();
        record.insert(
            "profile".to_string(),
            Value::String(WRAPPERS_PROFILE.to_string()),
        );
        record.insert("kind".to_string(), Value::String(self.kind.to_string()));
        record.insert("session_id".to_string(), Value::String(self.session_id));
        record.insert("owner_ura".to_string(), Value::String(self.owner_ura));
        record.insert("state".to_string(), Value::String(self.state));
        record.insert(
            self.ref_key.to_string(),
            self.ref_value.map(Value::String).unwrap_or(Value::Null),
        );
        for (key, value) in self.extra {
            record.insert(key, value);
        }
        record.insert("metadata".to_string(), Value::Object(self.metadata));
        Ok(Value::Object(record))
    }
}

fn stamp_metadata(metadata: &mut Map<String, Value>, source: &'static str) {
    metadata.insert(
        "profile".to_string(),
        Value::String(WRAPPERS_PROFILE.to_string()),
    );
    metadata.insert("source".to_string(), Value::String(source.to_string()));
}

fn typed_metadata(obj: &Map<String, Value>) -> Result<Map<String, Value>, WrapperError> {
    match obj.get("metadata") {
        None | Some(Value::Null) => Ok(Map::new()),
        Some(Value::Object(metadata)) => Ok(metadata.clone()),
        Some(_) => Err(WrapperError::InvalidField(
            "metadata",
            "must be an object".to_string(),
        )),
    }
}

fn optional_u64_field(
    obj: &Map<String, Value>,
    key: &'static str,
) -> Result<Option<u64>, WrapperError> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(value)) => value.as_u64().map(Some).ok_or_else(|| {
            WrapperError::InvalidField(key, "must be an unsigned integer".to_string())
        }),
        Some(_) => Err(WrapperError::InvalidField(
            key,
            "must be an unsigned integer".to_string(),
        )),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WrapperError {
    MissingField(&'static str),
    InvalidField(&'static str, String),
    Contract(String),
}

impl fmt::Display for WrapperError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WrapperError::MissingField(field) => write!(f, "missing required field {field}"),
            WrapperError::InvalidField(field, message) => {
                write!(f, "invalid field {field}: {message}")
            }
            WrapperError::Contract(message) => f.write_str(message),
        }
    }
}

impl From<SdkContractError> for WrapperError {
    fn from(value: SdkContractError) -> Self {
        match value {
            SdkContractError::MissingField(field) => Self::MissingField(field),
            SdkContractError::InvalidField(field, message) => Self::InvalidField(field, message),
            SdkContractError::Contract(message) => Self::Contract(message),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_file_record_preserves_resource_facts() {
        let file = project_file_record(&json!({
            "file_ref": "easynet:///r/example/resource/alice.files/report.txt",
            "owner_ura": "easynet:///r/example/agent/alice.sdk",
            "content_type": "text/plain",
            "size_bytes": 42,
            "content_hash": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        }))
        .unwrap();

        assert_eq!(file["profile"], WRAPPERS_PROFILE);
        assert_eq!(file["kind"], "file_record");
        assert_eq!(file["size_bytes"], 42);
        assert_eq!(file["metadata"]["source"], "wrappers.file_record");
    }

    #[test]
    fn project_file_record_rejects_invalid_owner_ura() {
        let err = project_file_record(&json!({
            "file_ref": "local-file-1",
            "owner_ura": "not-a-ura",
            "content_type": "text/plain"
        }))
        .unwrap_err();

        assert!(err.to_string().contains("owner_ura"));
    }

    #[test]
    fn project_terminal_session_requires_explicit_state() {
        let err = project_terminal_session(&json!({
            "session_id": "term-1",
            "owner_ura": "easynet:///r/example/agent/alice.sdk"
        }))
        .unwrap_err();

        assert!(err.to_string().contains("state"));
    }

    #[test]
    fn project_remote_desktop_session_projects_display_ref() {
        let session = project_remote_desktop_session(&json!({
            "session_id": "rdp-1",
            "owner_ura": "easynet:///r/example/agent/alice.sdk",
            "state": "active",
            "display_ref": "display-main"
        }))
        .unwrap();

        assert_eq!(session["kind"], "remote_desktop_session");
        assert_eq!(session["display_ref"], "display-main");
        assert_eq!(
            session["metadata"]["source"],
            "wrappers.remote_desktop_session"
        );
    }

    #[test]
    fn project_browser_session_projects_nullable_ref() {
        let session = project_browser_session(&json!({
            "session_id": "browser-1",
            "owner_ura": "easynet:///r/example/agent/alice.sdk",
            "state": "starting"
        }))
        .unwrap();

        assert_eq!(session["kind"], "browser_session");
        assert!(session["browser_ref"].is_null());
    }

    #[test]
    fn project_media_session_requires_media_kind() {
        let err = project_media_session(&json!({
            "session_id": "media-1",
            "owner_ura": "easynet:///r/example/agent/alice.sdk",
            "state": "active"
        }))
        .unwrap_err();

        assert!(err.to_string().contains("media_kind"));
    }

    #[test]
    fn project_media_session_projects_kind_and_stream_ref() {
        let session = project_media_session(&json!({
            "session_id": "media-1",
            "owner_ura": "easynet:///r/example/agent/alice.sdk",
            "state": "active",
            "media_kind": "voice",
            "stream_ref": "stream-voice-1"
        }))
        .unwrap();

        assert_eq!(session["kind"], "media_session");
        assert_eq!(session["media_kind"], "voice");
        assert_eq!(session["stream_ref"], "stream-voice-1");
    }
}
