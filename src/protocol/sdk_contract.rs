// EasyNet CLI — daemon SDK shared contract
// =========================================
//
// File: src/protocol/sdk_contract.rs
// Description: Shared JSON DTO helpers for daemon SDK profile carriers.
//
// Protocol Responsibility
// -----------------------
// Own profile-neutral validation and construction for SDK objects that carry
// complete daemon Invocation tuples. This module does not execute abilities,
// sign envelopes, verify receipts, or define profile-specific DTO semantics.
//
// Implementation Approach
// -----------------------
// Centralize the common carrier rules that every SDK profile must obey:
// object-shaped inputs, explicit tuple fields, valid URAs, descriptor version
// checks, nonce bounds, and daemon system-ability DescriptorRef construction.
//
// Usage Contract
// --------------
// Profile contracts call `build_system_invocation` after they have built their
// profile-owned `args` payload. Callers must supply caller, callee, subject,
// descriptor version, nonce, and causal context explicitly.
//
// Architectural Position
// ----------------------
// EasyNet-Cli SDK protocol projection layer. Axon remains the protocol authority for
// canonical Invocation and Receipt semantics; this module only assembles the
// binding-facing JSON carrier used by EasyNet-Cli profiles.

use std::fmt;

use base64::Engine as _;
use easynet_axon::invocation::canonical_ability_descriptor_ref;
use serde_json::{json, Map, Value};

use crate::core::ura;

pub(crate) fn build_system_invocation(
    obj: &Map<String, Value>,
    profile: &'static str,
    system_ability: &str,
    args: Value,
) -> Result<Value, SdkContractError> {
    let caller_ura = required_string(obj, "caller_ura")?;
    validate_ura(caller_ura, "caller_ura")?;
    let callee_ura = required_string(obj, "callee_ura")?;
    validate_ura(callee_ura, "callee_ura")?;
    let subject_ura = required_string(obj, "subject_ura")?;
    validate_ura(subject_ura, "subject_ura")?;
    let descriptor_version = required_string(obj, "descriptor_version")?;
    validate_descriptor_version(descriptor_version)?;
    let descriptor_ref = system_descriptor_ref(callee_ura, system_ability, descriptor_version)?;
    let nonce_base64 = required_string(obj, "nonce_base64")?;
    validate_nonce(nonce_base64)?;
    let causal_context = obj
        .get("causal_context")
        .ok_or(SdkContractError::MissingField("causal_context"))?;
    if !causal_context.is_object() {
        return Err(SdkContractError::InvalidField(
            "causal_context",
            "must be an object".to_string(),
        ));
    }
    let mut metadata = typed_object_or_default(obj, "metadata", json!({}))?;
    metadata["profile"] = Value::String(profile.to_string());
    metadata["system_ability"] = Value::String(system_ability.to_string());
    metadata["carrier_owner"] = Value::String("daemon_sdk".to_string());

    Ok(json!({
        "caller_ura": caller_ura,
        "callee_ura": callee_ura,
        "descriptor_ref": descriptor_ref,
        "subject_ura": subject_ura,
        "nonce_base64": nonce_base64,
        "causal_context": causal_context,
        "args": args,
        "content_type": "application/json",
        "metadata": metadata,
    }))
}

pub(crate) fn system_descriptor_ref(
    callee_ura: &str,
    system_ability: &str,
    descriptor_version: &str,
) -> Result<String, SdkContractError> {
    let ability_ura = ura::owner_ability_ura(callee_ura, system_ability).ok_or_else(|| {
        SdkContractError::InvalidField(
            "callee_ura",
            format!("cannot derive system ability URA for {system_ability:?}"),
        )
    })?;
    canonical_ability_descriptor_ref(&format!("{ability_ura}@{descriptor_version}"))
        .map_err(|err| SdkContractError::InvalidField("descriptor_ref", err.to_string()))
}

pub(crate) fn object<'a>(
    value: &'a Value,
    name: &'static str,
) -> Result<&'a Map<String, Value>, SdkContractError> {
    value.as_object().ok_or(SdkContractError::InvalidField(
        name,
        "must be an object".to_string(),
    ))
}

pub(crate) fn required_string<'a>(
    obj: &'a Map<String, Value>,
    key: &'static str,
) -> Result<&'a str, SdkContractError> {
    obj.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or(SdkContractError::MissingField(key))
}

pub(crate) fn optional_string(obj: &Map<String, Value>, key: &'static str) -> Option<String> {
    obj.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(crate) fn optional_string_field(
    obj: &Map<String, Value>,
    key: &'static str,
) -> Result<Option<String>, SdkContractError> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(raw)) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                Ok(None)
            } else {
                Ok(Some(trimmed.to_string()))
            }
        }
        Some(_) => Err(SdkContractError::InvalidField(
            key,
            "must be a string".to_string(),
        )),
    }
}

pub(crate) fn first_optional_string_field(
    obj: &Map<String, Value>,
    primary: &'static str,
    fallback: &'static str,
) -> Result<Option<String>, SdkContractError> {
    optional_string_field(obj, primary)?.map_or_else(
        || optional_string_field(obj, fallback),
        |value| Ok(Some(value)),
    )
}

pub(crate) fn optional_bool_field(
    obj: &Map<String, Value>,
    key: &'static str,
) -> Result<Option<bool>, SdkContractError> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err(SdkContractError::InvalidField(
            key,
            "must be boolean".to_string(),
        )),
    }
}

pub(crate) fn optional_string_array_field(
    obj: &Map<String, Value>,
    key: &'static str,
) -> Result<Option<Vec<String>>, SdkContractError> {
    let Some(value) = obj.get(key).filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let values = value.as_array().ok_or_else(|| {
        SdkContractError::InvalidField(key, "must be an array of strings".to_string())
    })?;
    values
        .iter()
        .map(|value| {
            value.as_str().map(str::to_string).ok_or_else(|| {
                SdkContractError::InvalidField(key, "must be an array of strings".to_string())
            })
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

pub(crate) fn typed_object_or_default(
    obj: &Map<String, Value>,
    key: &'static str,
    default: Value,
) -> Result<Value, SdkContractError> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(value @ Value::Object(_)) => Ok(value.clone()),
        Some(_) => Err(SdkContractError::InvalidField(
            key,
            "must be an object or null".to_string(),
        )),
    }
}

pub(crate) fn validate_ura(raw: &str, field: &'static str) -> Result<(), SdkContractError> {
    ura::parse_ura(raw)
        .map(|_| ())
        .map_err(|err| SdkContractError::InvalidField(field, err.to_string()))
}

pub(crate) fn validate_descriptor_version(raw: &str) -> Result<(), SdkContractError> {
    if crate::daemon::ability::manifest::is_valid_descriptor_version(raw) {
        return Ok(());
    }
    Err(SdkContractError::InvalidField(
        "descriptor_version",
        "must be MAJOR.MINOR.PATCH numeric form".to_string(),
    ))
}

pub(crate) fn validate_nonce(nonce_base64: &str) -> Result<(), SdkContractError> {
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(nonce_base64)
        .map_err(|err| SdkContractError::InvalidField("nonce_base64", err.to_string()))?;
    if decoded.len() != 16 {
        return Err(SdkContractError::InvalidField(
            "nonce_base64",
            format!("must decode to exactly 16 bytes, got {}", decoded.len()),
        ));
    }
    if decoded.iter().all(|byte| *byte == 0) {
        return Err(SdkContractError::InvalidField(
            "nonce_base64",
            "must not be all-zero".to_string(),
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SdkContractError {
    MissingField(&'static str),
    InvalidField(&'static str, String),
    Contract(String),
}

impl fmt::Display for SdkContractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SdkContractError::MissingField(field) => write!(f, "missing required field {field}"),
            SdkContractError::InvalidField(field, message) => {
                write!(f, "invalid field {field}: {message}")
            }
            SdkContractError::Contract(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for SdkContractError {}
