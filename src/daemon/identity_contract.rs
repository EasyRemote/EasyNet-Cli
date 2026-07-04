// EasyNet CLI — Identity shared contract
// ======================================
//
// File: src/daemon/identity_contract.rs
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
use serde_json::{json, Value};

use crate::daemon::sdk_contract::{
    build_system_invocation, object, optional_string_field, required_string, validate_ura,
    SdkContractError,
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
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(raw)
        .map_err(|err| IdentitySdkError::InvalidField("public_key_base64", err.to_string()))?;
    if decoded.len() != 32 {
        return Err(IdentitySdkError::InvalidField(
            "public_key_base64",
            format!("must decode to exactly 32 bytes, got {}", decoded.len()),
        ));
    }
    Ok(())
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
}
