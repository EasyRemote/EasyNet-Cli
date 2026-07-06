// EasyNet CLI — Identity shared contract
// ======================================
//
// File: src/protocol/identity_contract.rs
// Description: Shared daemon SDK contract for Identity signing-key carriers.
//
// Protocol Responsibility
// -----------------------
// Own SDK-facing carrier construction for daemon identity key-management
// abilities. This module does not register keys, revoke keys, sign
// invocations, verify receipts, or reinterpret Axon identity grammar.
//
// Implementation Approach
// -----------------------
// Normalize SDK request DTOs into the existing daemon identity ability args
// and delegate complete Invocation tuple construction to `sdk_contract`.
// Inputs must carry caller, callee, subject, nonce, descriptor version, and
// causal context explicitly.
//
// Usage Contract
// --------------
// The carrier builder is strict: it accepts only ed25519 public key metadata
// and requires explicit `owner_ura` plus public key material where the daemon
// ability requires it. Legacy SDK fields such as `key_id` remain facade facts;
// they are not used to invent daemon trust-row identity.
//
// Architectural Position
// ----------------------
// EasyNet-Cli daemon SDK Identity profile. Axon remains the protocol authority
// for URA and Invocation semantics; daemon admission owns trust-write policy.

use base64::Engine as _;
use serde_json::{json, Map, Value};
use sha2::Digest as _;

use crate::protocol::sdk_contract::{
    build_system_invocation, object, optional_string_field, required_string,
    typed_object_or_default, validate_ura, SdkContractError,
};

const IDENTITY_PROFILE: &str = "directory_identity";
const ABILITY_REGISTER_PUBKEY: &str =
    crate::daemon::ability::names::federation::IDENTITY_REGISTER_PUBKEY;
const ABILITY_LIST_USER_PUBKEYS: &str =
    crate::daemon::ability::names::federation::IDENTITY_LIST_USER_PUBKEYS;
const ABILITY_REVOKE_USER_PUBKEY: &str =
    crate::daemon::ability::names::federation::IDENTITY_REVOKE_USER_PUBKEY;

pub(crate) type IdentitySdkError = SdkContractError;

pub(crate) fn build_register_signing_key_invocation(
    request: &Value,
) -> Result<Value, IdentitySdkError> {
    let obj = object(request, "SigningKeyRegistrationRequest")?;
    let owner_ura = required_string(obj, "owner_ura")?;
    validate_ura(owner_ura, "owner_ura")?;
    validate_algorithm(required_string(obj, "algorithm")?)?;
    let public_key_base64 = required_string(obj, "public_key_base64")?;
    validate_ed25519_public_key(public_key_base64)?;
    let role = normalized_role(
        optional_string_field(obj, "role")?
            .as_deref()
            .unwrap_or("user"),
    )?;
    let args = json!({
        "agent_ura": owner_ura,
        "public_key_b64": public_key_base64,
        "role": role,
    });
    build_system_invocation(obj, IDENTITY_PROFILE, ABILITY_REGISTER_PUBKEY, args)
}

pub(crate) fn build_list_signing_keys_invocation(
    request: &Value,
) -> Result<Value, IdentitySdkError> {
    let obj = object(request, "SigningKeyListRequest")?;
    let owner_ura = required_string(obj, "owner_ura")?;
    validate_ura(owner_ura, "owner_ura")?;
    let args = json!({
        "agent_ura": owner_ura,
    });
    build_system_invocation(obj, IDENTITY_PROFILE, ABILITY_LIST_USER_PUBKEYS, args)
}

pub(crate) fn build_revoke_signing_key_invocation(
    request: &Value,
) -> Result<Value, IdentitySdkError> {
    let obj = object(request, "SigningKeyRevokeRequest")?;
    let owner_ura = required_string(obj, "owner_ura")?;
    validate_ura(owner_ura, "owner_ura")?;
    let public_key_base64 = required_string(obj, "public_key_base64")?;
    validate_ed25519_public_key(public_key_base64)?;
    let args = json!({
        "agent_ura": owner_ura,
        "public_key_b64": public_key_base64,
    });
    build_system_invocation(obj, IDENTITY_PROFILE, ABILITY_REVOKE_USER_PUBKEY, args)
}

pub(crate) fn project_signing_key_record(input: &Value) -> Result<Value, IdentitySdkError> {
    let projection = SigningKeyProjectionInput::parse(input, "SigningKeyRecord")?;
    let result = object(&projection.result, "SigningKeyRegistrationResult")?;
    if !optional_bool(result, "ok")?.unwrap_or(false) {
        return Err(IdentitySdkError::InvalidField(
            "ok",
            "identity.register_pubkey did not acknowledge success".to_string(),
        ));
    }
    let owner_ura = required_string(&projection.request, "owner_ura")?;
    validate_ura(owner_ura, "owner_ura")?;
    let public_key_base64 = required_string(&projection.request, "public_key_base64")?;
    validate_ed25519_public_key(public_key_base64)?;
    let algorithm = optional_string_field(&projection.request, "algorithm")?
        .unwrap_or_else(|| "ed25519".to_string());
    validate_algorithm(&algorithm)?;
    let usage = signing_key_usage(&projection.request)?;
    let key_id = match optional_string_field(&projection.request, "key_id")? {
        Some(key_id) => key_id,
        None => public_key_key_id(public_key_base64)?,
    };
    let role = normalized_role(
        optional_string_field(&projection.request, "role")?
            .as_deref()
            .unwrap_or("user"),
    )?;
    Ok(signing_key_record_json(
        &key_id,
        owner_ura,
        &algorithm,
        public_key_base64,
        "active",
        usage,
        json!({
            "source": "identity.register_pubkey",
            "daemon_ack": result,
            "role": role,
        }),
        0,
        0,
    ))
}

pub(crate) fn project_signing_key_page(input: &Value) -> Result<Value, IdentitySdkError> {
    let projection = SigningKeyProjectionInput::parse(input, "SigningKeyPage")?;
    let result = object(&projection.result, "SigningKeyListResult")?;
    let owner_ura = optional_string_field(result, "agent_ura")?
        .or_else(|| {
            optional_string_field(&projection.request, "owner_ura")
                .ok()
                .flatten()
        })
        .ok_or(IdentitySdkError::MissingField("owner_ura"))?;
    validate_ura(&owner_ura, "owner_ura")?;
    let limit = optional_usize(&projection.request, "limit")?.unwrap_or(50);
    validate_page_limit(limit)?;
    let keys = result
        .get("keys")
        .and_then(Value::as_array)
        .ok_or(IdentitySdkError::MissingField("keys"))?;
    let mut items = Vec::with_capacity(keys.len().min(limit));
    for key in keys.iter().take(limit) {
        let key_obj = object(key, "SigningKeyListResult.keys[]")?;
        let public_key_base64 = required_string(key_obj, "public_key_b64")?;
        validate_ed25519_public_key(public_key_base64)?;
        let created_unix_ms = optional_u64(key_obj, "added_at_unix_ms")?.unwrap_or(0);
        let key_id = public_key_key_id(public_key_base64)?;
        items.push(signing_key_record_json(
            &key_id,
            &owner_ura,
            "ed25519",
            public_key_base64,
            "active",
            vec!["invocation.sign".to_string()],
            json!({
                "source": "identity.list_user_pubkeys",
                "rotation_epoch": optional_u64(result, "rotation_epoch")?.unwrap_or(0),
            }),
            created_unix_ms,
            0,
        ));
    }
    let has_more = keys.len() > limit;
    Ok(json!({
        "profile": IDENTITY_PROFILE,
        "items": items,
        "next_cursor": if has_more { Some(limit.to_string()) } else { None },
        "limit": limit,
        "metadata": {
            "source": "identity.list_user_pubkeys",
            "owner_ura": owner_ura,
            "total_available": keys.len(),
            "rotation_epoch": optional_u64(result, "rotation_epoch")?.unwrap_or(0),
            "revoked_key_count": optional_u64(result, "revoked_key_count")?.unwrap_or(0),
        },
    }))
}

pub(crate) fn project_signing_key_revoke_result(input: &Value) -> Result<Value, IdentitySdkError> {
    let projection = SigningKeyProjectionInput::parse(input, "SigningKeyRevokeResult")?;
    let result = object(&projection.result, "SigningKeyRevokeResult.result")?;
    if !optional_bool(result, "ok")?.unwrap_or(false) {
        return Err(IdentitySdkError::InvalidField(
            "ok",
            "identity.revoke_user_pubkey did not acknowledge success".to_string(),
        ));
    }
    let public_key_base64 = optional_string_field(&projection.request, "public_key_base64")?;
    if let Some(public_key_base64) = public_key_base64.as_deref() {
        validate_ed25519_public_key(public_key_base64)?;
    }
    let key_id = match optional_string_field(&projection.request, "key_id")? {
        Some(key_id) => key_id,
        None => public_key_base64
            .as_deref()
            .map(public_key_key_id)
            .transpose()?
            .ok_or(IdentitySdkError::MissingField("key_id"))?,
    };
    let removed = optional_bool(result, "removed")?.unwrap_or(true);
    Ok(json!({
        "profile": IDENTITY_PROFILE,
        "key_id": key_id,
        "revoked": true,
        "state": if removed { "revoked" } else { "not_found" },
        "metadata": {
            "source": "identity.revoke_user_pubkey",
            "removed": removed,
            "reason": optional_string_field(&projection.request, "reason")?,
        },
    }))
}

pub(crate) fn project_signer_handle(input: &Value) -> Result<Value, IdentitySdkError> {
    let projection = SigningKeyProjectionInput::parse(input, "SignerHandle")?;
    let request = projection.request;
    let owner_ura = required_string(&request, "owner_ura")?;
    validate_ura(owner_ura, "owner_ura")?;
    let requested_key_id = required_string(&request, "key_id")?;
    let usage =
        optional_string_field(&request, "usage")?.unwrap_or_else(|| "invocation.sign".to_string());
    if usage != "invocation.sign" {
        return Err(IdentitySdkError::InvalidField(
            "usage",
            "only invocation.sign signer handles are exposed by the daemon SDK".to_string(),
        ));
    }

    let result = object(&projection.result, "SignerHandle.result")?;
    let result_owner =
        optional_string_field(result, "agent_ura")?.unwrap_or_else(|| owner_ura.to_string());
    if result_owner != owner_ura {
        return Err(IdentitySdkError::InvalidField(
            "owner_ura",
            "daemon signer key inventory owner does not match request".to_string(),
        ));
    }
    let keys = result
        .get("keys")
        .and_then(Value::as_array)
        .ok_or(IdentitySdkError::MissingField("keys"))?;

    for key in keys {
        let key_obj = object(key, "SignerHandle.keys[]")?;
        let public_key_base64 = required_string(key_obj, "public_key_b64")?;
        validate_ed25519_public_key(public_key_base64)?;
        let derived_key_id = public_key_key_id(public_key_base64)?;
        let daemon_key_id = optional_string_field(key_obj, "key_id")?;
        let key_matches = daemon_key_id.as_deref() == Some(requested_key_id)
            || derived_key_id == requested_key_id;
        if !key_matches {
            continue;
        }
        let key_id = daemon_key_id.unwrap_or(derived_key_id);
        let key_state = match optional_string_field(key_obj, "state")? {
            Some(state) => state,
            None => {
                optional_string_field(key_obj, "status")?.unwrap_or_else(|| "active".to_string())
            }
        };
        if key_state != "active" {
            return Err(IdentitySdkError::InvalidField(
                "key_state",
                "signer key must be active in daemon identity inventory".to_string(),
            ));
        }
        let signer_id = format!("signer-{key_id}");
        let policy_ref = signer_policy_ref(owner_ura, &key_id, public_key_base64);
        return Ok(json!({
            "profile": IDENTITY_PROFILE,
            "signer_id": signer_id,
            "owner_ura": owner_ura,
            "key_id": key_id,
            "algorithm": "ed25519",
            "policy": {
                "mode": "local_daemon_signing",
                "usage": usage,
                "signer_id": signer_id,
                "policy_ref": policy_ref,
                "inventory_owner_ura": owner_ura,
                "key_state": key_state,
            },
            "metadata": {
                "source": "identity.list_user_pubkeys",
                "source_ability": "identity.list_user_pubkeys",
                "public_key_base64": public_key_base64,
                "rotation_epoch": optional_u64(result, "rotation_epoch")?.unwrap_or(0),
                "policy_ref": policy_ref,
            },
        }));
    }

    Err(IdentitySdkError::InvalidField(
        "key_id",
        "signer key was not present in daemon identity inventory".to_string(),
    ))
}

struct SigningKeyProjectionInput {
    request: Map<String, Value>,
    result: Value,
}

impl SigningKeyProjectionInput {
    fn parse(input: &Value, name: &'static str) -> Result<Self, IdentitySdkError> {
        let obj = object(input, name)?;
        let request = match typed_object_or_default(obj, "request", json!({}))? {
            Value::Object(request) => request,
            _ => unreachable!("typed_object_or_default returns object/default"),
        };
        let result = obj
            .get("result")
            .filter(|value| !value.is_null())
            .cloned()
            .unwrap_or_else(|| input.clone());
        Ok(Self { request, result })
    }
}

fn signing_key_record_json(
    key_id: &str,
    owner_ura: &str,
    algorithm: &str,
    public_key_base64: &str,
    state: &str,
    usage: Vec<String>,
    metadata: Value,
    created_unix_ms: u64,
    revoked_unix_ms: u64,
) -> Value {
    json!({
        "profile": IDENTITY_PROFILE,
        "key_id": key_id,
        "owner_ura": owner_ura,
        "algorithm": algorithm,
        "public_key_base64": public_key_base64,
        "state": state,
        "usage": usage,
        "created_unix_ms": created_unix_ms,
        "revoked_unix_ms": revoked_unix_ms,
        "metadata": metadata,
    })
}

fn signing_key_usage(obj: &Map<String, Value>) -> Result<Vec<String>, IdentitySdkError> {
    let Some(raw) = obj.get("usage").filter(|value| !value.is_null()) else {
        return Ok(vec!["invocation.sign".to_string()]);
    };
    let values = raw.as_array().ok_or_else(|| {
        IdentitySdkError::InvalidField("usage", "must be an array of strings".to_string())
    })?;
    if values.is_empty() {
        return Err(IdentitySdkError::InvalidField(
            "usage",
            "must not be empty".to_string(),
        ));
    }
    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .ok_or_else(|| {
                    IdentitySdkError::InvalidField(
                        "usage",
                        "must be an array of non-empty strings".to_string(),
                    )
                })
        })
        .collect()
}

fn public_key_key_id(public_key_base64: &str) -> Result<String, IdentitySdkError> {
    let decoded = decode_ed25519_public_key(public_key_base64)?;
    let digest = sha2::Sha256::digest(&decoded);
    Ok(format!("ed25519:{}", hex::encode(&digest[..16])))
}

pub(crate) fn signer_policy_ref(owner_ura: &str, key_id: &str, public_key_base64: &str) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(owner_ura.as_bytes());
    hasher.update(b"\0");
    hasher.update(key_id.as_bytes());
    hasher.update(b"\0");
    hasher.update(public_key_base64.as_bytes());
    let digest = hasher.finalize();
    format!("daemon-key-inventory:sha256:{}", hex::encode(&digest[..16]))
}

fn optional_bool(
    obj: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<bool>, IdentitySdkError> {
    match obj.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err(IdentitySdkError::InvalidField(
            field,
            "must be boolean".to_string(),
        )),
    }
}

fn optional_u64(
    obj: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<u64>, IdentitySdkError> {
    match obj.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(number)) => number
            .as_u64()
            .map(Some)
            .ok_or_else(|| IdentitySdkError::InvalidField(field, "must be unsigned".to_string())),
        Some(_) => Err(IdentitySdkError::InvalidField(
            field,
            "must be an unsigned integer".to_string(),
        )),
    }
}

fn optional_usize(
    obj: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<usize>, IdentitySdkError> {
    optional_u64(obj, field).map(|value| value.and_then(|value| usize::try_from(value).ok()))
}

fn validate_page_limit(limit: usize) -> Result<(), IdentitySdkError> {
    if (1..=500).contains(&limit) {
        return Ok(());
    }
    Err(IdentitySdkError::InvalidField(
        "limit",
        "must be between 1 and 500".to_string(),
    ))
}

fn validate_algorithm(raw: &str) -> Result<(), IdentitySdkError> {
    if raw.eq_ignore_ascii_case("ed25519") {
        return Ok(());
    }
    Err(IdentitySdkError::InvalidField(
        "algorithm",
        "only ed25519 public signing keys are accepted by daemon identity abilities".to_string(),
    ))
}

fn normalized_role(raw: &str) -> Result<&'static str, IdentitySdkError> {
    match raw {
        "device" => Ok("device"),
        "backend" => Ok("backend"),
        "hub" => Ok("hub"),
        "user" => Ok("user"),
        _ => Err(IdentitySdkError::InvalidField(
            "role",
            "must be one of device, backend, hub, or user".to_string(),
        )),
    }
}

fn validate_ed25519_public_key(raw: &str) -> Result<(), IdentitySdkError> {
    decode_ed25519_public_key(raw).map(|_| ())
}

fn decode_ed25519_public_key(raw: &str) -> Result<Vec<u8>, IdentitySdkError> {
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(raw)
        .map_err(|err| IdentitySdkError::InvalidField("public_key_base64", err.to_string()))?;
    if decoded.len() != 32 {
        return Err(IdentitySdkError::InvalidField(
            "public_key_base64",
            format!("must decode to exactly 32 bytes, got {}", decoded.len()),
        ));
    }
    Ok(decoded)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn public_key() -> &'static str {
        "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE="
    }

    fn base_request(extra: Value) -> Value {
        let mut request = json!({
            "caller_ura": "easynet:///r/example/agent/alice.sdk",
            "callee_ura": "easynet:///r/example/device/dev-a",
            "subject_ura": "easynet:///r/example/user/alice",
            "descriptor_version": "1.0.0",
            "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
            "causal_context": {"form": "none"},
            "metadata": {"request_id": "identity-1"}
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
    fn build_register_signing_key_invocation_targets_identity_pubkey_ability() {
        let request = base_request(json!({
            "owner_ura": "easynet:///r/example/user/alice",
            "key_id": "alice-key-1",
            "algorithm": "ed25519",
            "public_key_base64": public_key(),
            "usage": ["invocation"],
            "role": "user"
        }));

        let invocation = build_register_signing_key_invocation(&request).unwrap();

        assert_eq!(invocation["metadata"]["profile"], IDENTITY_PROFILE);
        assert_eq!(
            invocation["metadata"]["system_ability"],
            ABILITY_REGISTER_PUBKEY
        );
        assert_eq!(
            invocation["args"]["agent_ura"],
            "easynet:///r/example/user/alice"
        );
        assert_eq!(invocation["args"]["public_key_b64"], public_key());
        assert_eq!(invocation["args"]["role"], "user");
        assert_eq!(
            invocation["descriptor_ref"],
            "easynet:///r/example/ability/device.dev-a.identity.register_pubkey@1.0.0"
        );
    }

    #[test]
    fn build_list_signing_keys_invocation_targets_user_pubkey_read() {
        let request = base_request(json!({
            "owner_ura": "easynet:///r/example/user/alice",
            "limit": 25
        }));

        let invocation = build_list_signing_keys_invocation(&request).unwrap();

        assert_eq!(
            invocation["metadata"]["system_ability"],
            ABILITY_LIST_USER_PUBKEYS
        );
        assert_eq!(
            invocation["args"]["agent_ura"],
            "easynet:///r/example/user/alice"
        );
    }

    #[test]
    fn build_revoke_signing_key_invocation_requires_public_key_material() {
        let request = base_request(json!({
            "owner_ura": "easynet:///r/example/user/alice",
            "key_id": "alice-key-1",
            "public_key_base64": public_key(),
            "reason": "rotation"
        }));

        let invocation = build_revoke_signing_key_invocation(&request).unwrap();

        assert_eq!(
            invocation["metadata"]["system_ability"],
            ABILITY_REVOKE_USER_PUBKEY
        );
        assert_eq!(
            invocation["args"]["agent_ura"],
            "easynet:///r/example/user/alice"
        );
        assert_eq!(invocation["args"]["public_key_b64"], public_key());
    }

    #[test]
    fn build_register_signing_key_rejects_non_ed25519_algorithm() {
        let request = base_request(json!({
            "owner_ura": "easynet:///r/example/user/alice",
            "key_id": "alice-key-1",
            "algorithm": "rsa",
            "public_key_base64": public_key(),
            "usage": ["invocation"]
        }));

        let err = build_register_signing_key_invocation(&request).unwrap_err();

        assert!(err.to_string().contains("algorithm"));
    }

    #[test]
    fn project_signing_key_record_uses_request_and_daemon_ack() {
        let input = json!({
            "request": {
                "owner_ura": "easynet:///r/example/user/alice",
                "key_id": "alice-key-1",
                "algorithm": "ed25519",
                "public_key_base64": public_key(),
                "usage": ["invocation.sign"],
                "role": "user"
            },
            "result": {"ok": true}
        });

        let record = project_signing_key_record(&input).unwrap();

        assert_eq!(record["profile"], IDENTITY_PROFILE);
        assert_eq!(record["key_id"], "alice-key-1");
        assert_eq!(record["owner_ura"], "easynet:///r/example/user/alice");
        assert_eq!(record["state"], "active");
        assert_eq!(record["usage"], json!(["invocation.sign"]));
        assert_eq!(record["metadata"]["source"], "identity.register_pubkey");
        assert_eq!(record["metadata"]["daemon_ack"]["ok"], true);
    }

    #[test]
    fn project_signing_key_page_maps_daemon_key_inventory() {
        let input = json!({
            "request": {
                "owner_ura": "easynet:///r/example/user/alice",
                "limit": 1
            },
            "result": {
                "agent_ura": "easynet:///r/example/user/alice",
                "keys": [
                    {
                        "public_key_b64": public_key(),
                        "added_at_unix_ms": 1783100000123u64
                    },
                    {
                        "public_key_b64": public_key(),
                        "added_at_unix_ms": 1783100000456u64
                    }
                ],
                "rotation_epoch": 3,
                "revoked_key_count": 1
            }
        });

        let page = project_signing_key_page(&input).unwrap();

        assert_eq!(page["profile"], IDENTITY_PROFILE);
        assert_eq!(page["items"].as_array().unwrap().len(), 1);
        assert_eq!(
            page["items"][0]["key_id"],
            public_key_key_id(public_key()).unwrap()
        );
        assert_eq!(page["items"][0]["created_unix_ms"], 1783100000123u64);
        assert_eq!(page["next_cursor"], "1");
        assert_eq!(page["metadata"]["total_available"], 2);
        assert_eq!(page["metadata"]["rotation_epoch"], 3);
        assert_eq!(page["metadata"]["revoked_key_count"], 1);
    }

    #[test]
    fn project_signing_key_revoke_result_preserves_not_found_state() {
        let input = json!({
            "request": {
                "key_id": "alice-key-1",
                "public_key_base64": public_key(),
                "reason": "rotation"
            },
            "result": {"ok": true, "removed": false}
        });

        let result = project_signing_key_revoke_result(&input).unwrap();

        assert_eq!(result["profile"], IDENTITY_PROFILE);
        assert_eq!(result["key_id"], "alice-key-1");
        assert_eq!(result["revoked"], true);
        assert_eq!(result["state"], "not_found");
        assert_eq!(result["metadata"]["removed"], false);
        assert_eq!(result["metadata"]["reason"], "rotation");
    }

    #[test]
    fn project_signer_handle_uses_daemon_key_inventory() {
        let key_id = public_key_key_id(public_key()).unwrap();
        let input = json!({
            "request": {
                "owner_ura": "easynet:///r/example/user/alice",
                "key_id": key_id,
                "usage": "invocation.sign"
            },
            "result": {
                "agent_ura": "easynet:///r/example/user/alice",
                "keys": [
                    {
                        "public_key_b64": public_key(),
                        "added_at_unix_ms": 1783100000123u64
                    }
                ],
                "rotation_epoch": 3
            }
        });

        let handle = project_signer_handle(&input).unwrap();

        assert_eq!(handle["profile"], IDENTITY_PROFILE);
        assert_eq!(handle["key_id"], key_id);
        assert_eq!(handle["owner_ura"], "easynet:///r/example/user/alice");
        assert_eq!(handle["algorithm"], "ed25519");
        assert_eq!(handle["policy"]["mode"], "local_daemon_signing");
        assert_eq!(handle["policy"]["usage"], "invocation.sign");
        assert_eq!(
            handle["policy"]["inventory_owner_ura"],
            "easynet:///r/example/user/alice"
        );
        assert_eq!(handle["policy"]["key_state"], "active");
        assert!(handle["policy"]["policy_ref"]
            .as_str()
            .unwrap()
            .starts_with("daemon-key-inventory:sha256:"));
        assert_eq!(handle["metadata"]["source"], "identity.list_user_pubkeys");
        assert_eq!(
            handle["metadata"]["policy_ref"],
            handle["policy"]["policy_ref"]
        );
    }

    #[test]
    fn project_signer_handle_rejects_missing_daemon_key() {
        let input = json!({
            "request": {
                "owner_ura": "easynet:///r/example/user/alice",
                "key_id": "missing-key",
                "usage": "invocation.sign"
            },
            "result": {
                "agent_ura": "easynet:///r/example/user/alice",
                "keys": [
                    {
                        "public_key_b64": public_key()
                    }
                ]
            }
        });

        let err = project_signer_handle(&input).unwrap_err();

        assert!(err.to_string().contains("key_id"));
    }

    #[test]
    fn project_signer_handle_rejects_inactive_daemon_key() {
        let key_id = public_key_key_id(public_key()).unwrap();
        let input = json!({
            "request": {
                "owner_ura": "easynet:///r/example/user/alice",
                "key_id": key_id,
                "usage": "invocation.sign"
            },
            "result": {
                "agent_ura": "easynet:///r/example/user/alice",
                "keys": [
                    {
                        "public_key_b64": public_key(),
                        "state": "revoked"
                    }
                ]
            }
        });

        let err = project_signer_handle(&input).unwrap_err();

        assert!(err.to_string().contains("key_state"));
    }

    #[test]
    fn project_signing_key_record_rejects_unknown_role() {
        let input = json!({
            "request": {
                "owner_ura": "easynet:///r/example/user/alice",
                "key_id": "alice-key-1",
                "algorithm": "ed25519",
                "public_key_base64": public_key(),
                "usage": ["invocation.sign"],
                "role": "admin"
            },
            "result": {"ok": true}
        });

        let err = project_signing_key_record(&input).unwrap_err();

        assert!(err.to_string().contains("role"));
    }
}
