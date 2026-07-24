//! EasyNet provider-scoped declarative exec plugin helper.
//!
//! This crate owns the JSON frame details used between `easynet-daemon` and a
//! process-backed declarative exec plugin. It is intentionally not part of a
//! canonical runtime SDK root: plugin sidecar execution is an EasyNet-Cli
//! provider boundary. Plugin authors implement handlers over
//! [`SidecarInvocation`] instead of hand-writing stdin/stdout protocol frames.

use std::fmt;
use std::io::{self, BufRead, Write};

use serde::Serialize;
use serde_json::{Map, Value};

/// Handler-facing view of one daemon-admitted sidecar invocation.
#[derive(Debug, Clone, PartialEq)]
pub struct SidecarInvocation {
    pub call_id: String,
    pub caller_ura: String,
    pub callee_ura: String,
    pub ability_ura: String,
    pub subject_ura: String,
    pub invocation_nonce: Vec<u8>,
    pub causal_context: Value,
    pub args: Map<String, Value>,
    pub frame_type: String,
}

impl SidecarInvocation {
    /// Project a daemon sidecar frame into a typed invocation.
    pub fn from_frame(frame: Value) -> Result<Self, SidecarProtocolError> {
        let mut object = expect_object(frame, "sidecar request frame")?;
        reject_unknown_request_fields(&object)?;
        let frame_type = take_required_string(&mut object, "type")?;
        if frame_type != "invoke" {
            return Err(SidecarProtocolError::new(format!(
                "exec sidecar expected invoke frame, got {frame_type:?}"
            )));
        }
        let call_id = take_required_string(&mut object, "call_id")?;
        let invocation = object.remove("invocation").ok_or_else(|| {
            SidecarProtocolError::new("sidecar frame field \"invocation\" must be an object")
        })?;
        let mut invocation = expect_object(invocation, "invocation")?;
        reject_legacy_tuple_aliases(&invocation)?;
        reject_unknown_invocation_fields(&invocation)?;
        let nonce = take_required_nonce(&mut invocation, "invocation_nonce")?;
        let causal_context =
            Value::Object(take_required_object(&mut invocation, "causal_context")?);
        let args = take_required_object(&mut invocation, "args")?;
        Ok(Self {
            call_id,
            caller_ura: take_required_string(&mut invocation, "caller_ura")?,
            callee_ura: take_required_string(&mut invocation, "callee_ura")?,
            ability_ura: take_required_string(&mut invocation, "ability_ura")?,
            subject_ura: take_required_string(&mut invocation, "subject_ura")?,
            invocation_nonce: nonce,
            causal_context,
            args,
            frame_type,
        })
    }
}

/// Malformed daemon/plugin sidecar frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidecarProtocolError {
    message: String,
}

impl SidecarProtocolError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for SidecarProtocolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for SidecarProtocolError {}

/// Run one declarative exec plugin invocation using process stdin/stdout.
pub fn serve_exec_plugin<H, R, E>(handler: H) -> io::Result<()>
where
    H: FnOnce(SidecarInvocation) -> Result<R, E>,
    R: Serialize,
    E: fmt::Display,
{
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let stdout = io::stdout();
    let mut output = stdout.lock();
    serve_exec_plugin_io(&mut input, &mut output, handler)
}

/// Run one declarative exec plugin invocation over explicit streams.
pub fn serve_exec_plugin_io<I, O, H, R, E>(
    input: &mut I,
    output: &mut O,
    handler: H,
) -> io::Result<()>
where
    I: BufRead,
    O: Write,
    H: FnOnce(SidecarInvocation) -> Result<R, E>,
    R: Serialize,
    E: fmt::Display,
{
    let mut call_id = String::new();
    match read_invocation(input) {
        Ok(invocation) => {
            call_id.clone_from(&invocation.call_id);
            match handler(invocation) {
                Ok(value) => write_response(
                    output,
                    &ResponseFrame {
                        frame_type: "result",
                        call_id: &call_id,
                        value: Some(serde_json::to_value(value).map_err(io::Error::other)?),
                        message: None,
                    },
                ),
                Err(error) => write_response(
                    output,
                    &ResponseFrame {
                        frame_type: "error",
                        call_id: &call_id,
                        value: None,
                        message: Some(error.to_string()),
                    },
                ),
            }
        }
        Err(error) => write_response(
            output,
            &ResponseFrame {
                frame_type: "error",
                call_id: &call_id,
                value: None,
                message: Some(error.to_string()),
            },
        ),
    }
}

fn read_invocation(input: &mut impl BufRead) -> Result<SidecarInvocation, SidecarProtocolError> {
    let mut line = String::new();
    let bytes = input.read_line(&mut line).map_err(|error| {
        SidecarProtocolError::new(format!("read sidecar request frame: {error}"))
    })?;
    if bytes == 0 {
        return Err(SidecarProtocolError::new("missing sidecar request frame"));
    }
    let decoded = serde_json::from_str::<Value>(&line).map_err(|error| {
        SidecarProtocolError::new(format!("invalid sidecar request JSON: {error}"))
    })?;
    SidecarInvocation::from_frame(decoded)
}

#[derive(Serialize)]
struct ResponseFrame<'a> {
    #[serde(rename = "type")]
    frame_type: &'a str,
    call_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

fn write_response(output: &mut impl Write, frame: &ResponseFrame<'_>) -> io::Result<()> {
    serde_json::to_writer(&mut *output, frame).map_err(io::Error::other)?;
    output.write_all(b"\n")?;
    output.flush()
}

fn expect_object(value: Value, field: &str) -> Result<Map<String, Value>, SidecarProtocolError> {
    match value {
        Value::Object(object) => Ok(object),
        _ => Err(SidecarProtocolError::new(format!(
            "sidecar frame field {field:?} must be an object"
        ))),
    }
}

fn reject_legacy_tuple_aliases(object: &Map<String, Value>) -> Result<(), SidecarProtocolError> {
    for (legacy, canonical) in [
        ("caller", "caller_ura"),
        ("callee", "callee_ura"),
        ("ability", "ability_ura"),
        ("subject", "subject_ura"),
    ] {
        if object.contains_key(legacy) {
            return Err(SidecarProtocolError::new(format!(
                "sidecar frame field {legacy:?} is retired; use {canonical:?}"
            )));
        }
    }
    Ok(())
}

fn reject_unknown_invocation_fields(
    object: &Map<String, Value>,
) -> Result<(), SidecarProtocolError> {
    for field in object.keys() {
        if !matches!(
            field.as_str(),
            "caller_ura"
                | "callee_ura"
                | "ability_ura"
                | "subject_ura"
                | "invocation_nonce"
                | "causal_context"
                | "args"
        ) {
            return Err(SidecarProtocolError::new(format!(
                "sidecar frame field {field:?} is not part of the canonical invocation frame"
            )));
        }
    }
    Ok(())
}

fn reject_unknown_request_fields(object: &Map<String, Value>) -> Result<(), SidecarProtocolError> {
    for field in object.keys() {
        if !matches!(field.as_str(), "type" | "call_id" | "invocation") {
            return Err(SidecarProtocolError::new(format!(
                "sidecar request frame field {field:?} is not part of the canonical request frame"
            )));
        }
    }
    Ok(())
}

fn take_required_object(
    object: &mut Map<String, Value>,
    field: &str,
) -> Result<Map<String, Value>, SidecarProtocolError> {
    match object.remove(field) {
        Some(value) => expect_object(value, field),
        None => Err(SidecarProtocolError::new(format!(
            "sidecar frame field {field:?} must be an object"
        ))),
    }
}

fn take_required_string(
    object: &mut Map<String, Value>,
    field: &str,
) -> Result<String, SidecarProtocolError> {
    match object.remove(field) {
        Some(Value::String(value)) if !value.is_empty() => Ok(value),
        _ => Err(SidecarProtocolError::new(format!(
            "sidecar frame field {field:?} must be a string"
        ))),
    }
}

fn take_required_nonce(
    object: &mut Map<String, Value>,
    field: &str,
) -> Result<Vec<u8>, SidecarProtocolError> {
    let Some(Value::Array(items)) = object.remove(field) else {
        return Err(SidecarProtocolError::new(format!(
            "sidecar frame field {field:?} must be a byte array"
        )));
    };
    if items.is_empty() {
        return Err(SidecarProtocolError::new(format!(
            "sidecar frame field {field:?} must be a byte array"
        )));
    }
    let mut nonce = Vec::with_capacity(items.len());
    for item in items {
        let Some(value) = item.as_u64() else {
            return Err(SidecarProtocolError::new(format!(
                "sidecar frame field {field:?} must contain bytes"
            )));
        };
        if value > u8::MAX as u64 {
            return Err(SidecarProtocolError::new(format!(
                "sidecar frame field {field:?} must contain bytes"
            )));
        }
        nonce.push(value as u8);
    }
    Ok(nonce)
}
