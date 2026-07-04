// EasyNet CLI — Identity C ABI projection
// ========================================
//
// File: src/ffi/identity/mod.rs
// Description: C ABI Directory + Identity profile projection helpers.
//
// Protocol Responsibility
// -----------------------
// Expose URA and AbilityDescriptorRef validation/building through the daemon
// SDK without creating a second grammar in EasyNet-Cli.
//
// Implementation Approach
// -----------------------
// Keep C pointer validation, handle validation, and JSON allocation at this
// boundary. Delegate all URA parsing/building to `crate::core::ura`, which is
// already the CLI facade over Axon-owned URA helpers. Delegate descriptor-ref
// canonicalization to `easynet_axon::invocation`.
//
// Usage Contract
// --------------
// Callers must pass a live `EasynetHandle`, valid UTF-8 input, and a non-null
// output pointer. Returned strings are caller-owned and freed through
// `easynet_string_free`.
//
// Architectural Position
// ----------------------
// This is the Directory + Identity projection boundary. URA/descriptor helpers
// delegate protocol truth to Axon-facing core helpers, while signing-key
// lifecycle functions build daemon identity ability carriers and project daemon
// outputs into SDK DTOs.

use std::os::raw::c_char;

use easynet_axon::invocation::{
    ability_ura_from_descriptor_ref, canonical_ability_descriptor_ref,
    descriptor_version_from_descriptor_ref,
};

use crate::core::ura::{
    self, AbilityOwner, AbilitySelector, ParsedURA, URAKind, PROFILE_STRICT_V2,
};
use crate::daemon::identity_contract::{
    build_list_signing_keys_invocation, build_register_signing_key_invocation,
    build_revoke_signing_key_invocation, project_signing_key_page, project_signing_key_record,
    project_signing_key_revoke_result, IdentitySdkError,
};
use crate::ffi::client::handle::{get, EasynetHandle};
use crate::ffi::errors::{
    clear_last_error, set_last_error_code, EASYNET_OK, ERR_GENERIC, ERR_INVALID_ARG,
    ERR_INVALID_HANDLE, ERR_INVALID_UTF8, ERR_NULL_POINTER,
};
use crate::ffi::profile_json::{project_profile_json, ProfileJsonSpec};
use crate::ffi::strings::{alloc_output_cstring, read_cstr, StringError};

/// Project a canonical EasyNet URA into a typed identity DTO.
///
/// # Safety
/// `ura` must be a valid UTF-8 C string and `out_identity_json` must be a
/// non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_identity_project_ura(
    handle: EasynetHandle,
    ura: *const c_char,
    out_identity_json: *mut *mut c_char,
) -> i32 {
    let raw = match read_identity_args(
        handle,
        ura,
        out_identity_json,
        "easynet_identity_project_ura",
        "out_identity_json",
        "ura",
    ) {
        Ok(raw) => raw,
        Err(code) => return code,
    };
    let projection = match project_ura_json(raw, "easynet_identity_project_ura", None) {
        Ok(value) => value,
        Err(err) => {
            set_last_error_code(
                ERR_INVALID_ARG,
                format!("easynet_identity_project_ura: {err}"),
            );
            return ERR_INVALID_ARG;
        }
    };
    write_json_output(
        "easynet_identity_project_ura",
        out_identity_json,
        projection,
    )
}

/// Build a canonical EasyNet URA from a typed JSON request.
///
/// # Safety
/// `request_json` must be a valid UTF-8 C string and `out_identity_json` must
/// be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_identity_build_ura(
    handle: EasynetHandle,
    request_json: *const c_char,
    out_identity_json: *mut *mut c_char,
) -> i32 {
    let raw = match read_identity_args(
        handle,
        request_json,
        out_identity_json,
        "easynet_identity_build_ura",
        "out_identity_json",
        "request_json",
    ) {
        Ok(raw) => raw,
        Err(code) => return code,
    };
    let obj = match parse_json_object(raw, "easynet_identity_build_ura", "request_json") {
        Ok(obj) => obj,
        Err(code) => return code,
    };
    let projection = match build_ura_json(&obj) {
        Ok(value) => value,
        Err(err) => {
            set_last_error_code(
                ERR_INVALID_ARG,
                format!("easynet_identity_build_ura: {err}"),
            );
            return ERR_INVALID_ARG;
        }
    };
    write_json_output("easynet_identity_build_ura", out_identity_json, projection)
}

/// Project an AbilityDescriptorRef into its ability URA, version, and owner
/// facts.
///
/// # Safety
/// `descriptor_ref` must be a valid UTF-8 C string and
/// `out_descriptor_json` must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_identity_project_descriptor_ref(
    handle: EasynetHandle,
    descriptor_ref: *const c_char,
    out_descriptor_json: *mut *mut c_char,
) -> i32 {
    let raw = match read_identity_args(
        handle,
        descriptor_ref,
        out_descriptor_json,
        "easynet_identity_project_descriptor_ref",
        "out_descriptor_json",
        "descriptor_ref",
    ) {
        Ok(raw) => raw,
        Err(code) => return code,
    };
    let projection =
        match project_descriptor_ref_json(raw, "easynet_identity_project_descriptor_ref") {
            Ok(value) => value,
            Err(err) => {
                set_last_error_code(
                    ERR_INVALID_ARG,
                    format!("easynet_identity_project_descriptor_ref: {err}"),
                );
                return ERR_INVALID_ARG;
            }
        };
    write_json_output(
        "easynet_identity_project_descriptor_ref",
        out_descriptor_json,
        projection,
    )
}

/// Build an AbilityDescriptorRef from typed ability facts.
///
/// # Safety
/// `request_json` must be a valid UTF-8 C string and `out_descriptor_json`
/// must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_identity_build_descriptor_ref(
    handle: EasynetHandle,
    request_json: *const c_char,
    out_descriptor_json: *mut *mut c_char,
) -> i32 {
    let raw = match read_identity_args(
        handle,
        request_json,
        out_descriptor_json,
        "easynet_identity_build_descriptor_ref",
        "out_descriptor_json",
        "request_json",
    ) {
        Ok(raw) => raw,
        Err(code) => return code,
    };
    let obj = match parse_json_object(raw, "easynet_identity_build_descriptor_ref", "request_json")
    {
        Ok(obj) => obj,
        Err(code) => return code,
    };
    let projection = match build_descriptor_ref_json(&obj) {
        Ok(value) => value,
        Err(err) => {
            set_last_error_code(
                ERR_INVALID_ARG,
                format!("easynet_identity_build_descriptor_ref: {err}"),
            );
            return ERR_INVALID_ARG;
        }
    };
    write_json_output(
        "easynet_identity_build_descriptor_ref",
        out_descriptor_json,
        projection,
    )
}

/// Build a complete Invocation JSON carrier for `identity.register_pubkey`.
///
/// # Safety
/// `request_json` must be a valid UTF-8 C string and `out_invocation_json`
/// must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_identity_build_register_signing_key_invocation(
    handle: EasynetHandle,
    request_json: *const c_char,
    out_invocation_json: *mut *mut c_char,
) -> i32 {
    project_identity_profile_json(
        handle,
        request_json,
        out_invocation_json,
        "easynet_identity_build_register_signing_key_invocation",
        "out_invocation_json",
        "request_json",
        build_register_signing_key_invocation,
    )
}

/// Build a complete Invocation JSON carrier for `identity.list_user_pubkeys`.
///
/// # Safety
/// `request_json` must be a valid UTF-8 C string and `out_invocation_json`
/// must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_identity_build_list_signing_keys_invocation(
    handle: EasynetHandle,
    request_json: *const c_char,
    out_invocation_json: *mut *mut c_char,
) -> i32 {
    project_identity_profile_json(
        handle,
        request_json,
        out_invocation_json,
        "easynet_identity_build_list_signing_keys_invocation",
        "out_invocation_json",
        "request_json",
        build_list_signing_keys_invocation,
    )
}

/// Build a complete Invocation JSON carrier for `identity.revoke_user_pubkey`.
///
/// # Safety
/// `request_json` must be a valid UTF-8 C string and `out_invocation_json`
/// must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_identity_build_revoke_signing_key_invocation(
    handle: EasynetHandle,
    request_json: *const c_char,
    out_invocation_json: *mut *mut c_char,
) -> i32 {
    project_identity_profile_json(
        handle,
        request_json,
        out_invocation_json,
        "easynet_identity_build_revoke_signing_key_invocation",
        "out_invocation_json",
        "request_json",
        build_revoke_signing_key_invocation,
    )
}

/// Project daemon `identity.register_pubkey` output into a SigningKeyRecord DTO.
///
/// # Safety
/// `result_json` must be a valid UTF-8 C string and `out_record_json` must be
/// a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_identity_project_signing_key_record(
    handle: EasynetHandle,
    result_json: *const c_char,
    out_record_json: *mut *mut c_char,
) -> i32 {
    project_identity_profile_json(
        handle,
        result_json,
        out_record_json,
        "easynet_identity_project_signing_key_record",
        "out_record_json",
        "result_json",
        project_signing_key_record,
    )
}

/// Project daemon `identity.list_user_pubkeys` output into a SigningKeyPage DTO.
///
/// # Safety
/// `result_json` must be a valid UTF-8 C string and `out_page_json` must be
/// a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_identity_project_signing_key_page(
    handle: EasynetHandle,
    result_json: *const c_char,
    out_page_json: *mut *mut c_char,
) -> i32 {
    project_identity_profile_json(
        handle,
        result_json,
        out_page_json,
        "easynet_identity_project_signing_key_page",
        "out_page_json",
        "result_json",
        project_signing_key_page,
    )
}

/// Project daemon `identity.revoke_user_pubkey` output into a revoke DTO.
///
/// # Safety
/// `result_json` must be a valid UTF-8 C string and `out_result_json` must be
/// a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_identity_project_signing_key_revoke_result(
    handle: EasynetHandle,
    result_json: *const c_char,
    out_result_json: *mut *mut c_char,
) -> i32 {
    project_identity_profile_json(
        handle,
        result_json,
        out_result_json,
        "easynet_identity_project_signing_key_revoke_result",
        "out_result_json",
        "result_json",
        project_signing_key_revoke_result,
    )
}

fn project_identity_profile_json(
    handle: EasynetHandle,
    input: *const c_char,
    output: *mut *mut c_char,
    function: &'static str,
    output_name: &'static str,
    input_name: &'static str,
    project: fn(&serde_json::Value) -> Result<serde_json::Value, IdentitySdkError>,
) -> i32 {
    project_profile_json(
        handle,
        input,
        output,
        ProfileJsonSpec {
            function,
            output_name,
            input_name,
            profile: "directory_identity",
        },
        project,
    )
}

fn read_identity_args<'a>(
    handle: EasynetHandle,
    input: *const c_char,
    output: *mut *mut c_char,
    function: &'static str,
    output_name: &'static str,
    input_name: &'static str,
) -> Result<&'a str, i32> {
    if output.is_null() {
        set_last_error_code(
            ERR_NULL_POINTER,
            format!("{function}: {output_name} pointer is null"),
        );
        return Err(ERR_NULL_POINTER);
    }
    unsafe { *output = std::ptr::null_mut() };

    if get(handle).is_none() {
        set_last_error_code(
            ERR_INVALID_HANDLE,
            format!("{function}: handle {handle} is not registered"),
        );
        return Err(ERR_INVALID_HANDLE);
    }

    match read_cstr(input) {
        Ok(raw) => Ok(raw),
        Err(StringError::Null) => {
            set_last_error_code(
                ERR_NULL_POINTER,
                format!("{function}: {input_name} pointer is null"),
            );
            Err(ERR_NULL_POINTER)
        }
        Err(StringError::NotUtf8) => {
            set_last_error_code(
                ERR_INVALID_UTF8,
                format!("{function}: {input_name} is not valid UTF-8"),
            );
            Err(ERR_INVALID_UTF8)
        }
    }
}

fn parse_json_object(
    raw: &str,
    function: &'static str,
    input_name: &'static str,
) -> Result<serde_json::Map<String, serde_json::Value>, i32> {
    let value: serde_json::Value = match serde_json::from_str(raw) {
        Ok(value) => value,
        Err(err) => {
            set_last_error_code(
                ERR_INVALID_ARG,
                format!("{function}: decode {input_name} failed: {err}"),
            );
            return Err(ERR_INVALID_ARG);
        }
    };
    match value {
        serde_json::Value::Object(obj) => Ok(obj),
        _ => {
            set_last_error_code(
                ERR_INVALID_ARG,
                format!("{function}: {input_name} must be an object"),
            );
            Err(ERR_INVALID_ARG)
        }
    }
}

fn write_json_output(
    function: &'static str,
    output: *mut *mut c_char,
    value: serde_json::Value,
) -> i32 {
    let ptr = alloc_output_cstring(value.to_string());
    if ptr.is_null() {
        set_last_error_code(
            ERR_GENERIC,
            format!("{function}: out-of-memory allocating identity JSON"),
        );
        return ERR_GENERIC;
    }
    unsafe { *output = ptr };
    clear_last_error();
    EASYNET_OK
}

fn build_ura_json(
    obj: &serde_json::Map<String, serde_json::Value>,
) -> Result<serde_json::Value, IdentityError> {
    let kind = required_string(obj, "kind")?.to_ascii_lowercase();
    let ura = match kind.as_str() {
        "user" => {
            let realm = required_string(obj, "realm")?;
            let user_id = required_string(obj, "user_id")?;
            ura::user_ura(realm, user_id)
        }
        "device" => {
            let realm = required_string(obj, "realm")?;
            let device_id = required_string(obj, "device_id")?;
            ura::device_ura(realm, device_id)
        }
        "agent" => build_agent_ura(obj)?,
        "hub" => {
            let realm = required_string(obj, "realm")?;
            ura::hub_ura(realm)
        }
        "ability" => {
            let owner_ura = required_string(obj, "owner_ura")?;
            let ability_name = required_string(obj, "ability_name")?;
            ura::owner_ability_ura(owner_ura, ability_name).ok_or_else(|| {
                IdentityError::InvalidBuilder(format!(
                    "cannot build ability URA for owner_ura {owner_ura:?} and ability_name {ability_name:?}"
                ))
            })?
        }
        "resource" => build_resource_ura(obj)?,
        other => return Err(IdentityError::UnsupportedKind(other.to_string())),
    };

    project_ura_json(&ura, "easynet_identity_build_ura", Some(kind.as_str()))
}

fn build_agent_ura(
    obj: &serde_json::Map<String, serde_json::Value>,
) -> Result<String, IdentityError> {
    let realm = required_string(obj, "realm")?;
    let agent_id = required_string(obj, "agent_id")?;
    let owner_kind = optional_string(obj, "owner_kind").unwrap_or("user");
    match owner_kind {
        "device" => {
            let device_id = required_string(obj, "device_id")?;
            Ok(ura::device_agent_ura(realm, device_id, agent_id))
        }
        "user" => {
            let user_id = required_string(obj, "user_id")?;
            Ok(ura::agent_ura(realm, user_id, agent_id))
        }
        other => Err(IdentityError::UnsupportedOwnerKind(other.to_string())),
    }
}

fn build_resource_ura(
    obj: &serde_json::Map<String, serde_json::Value>,
) -> Result<String, IdentityError> {
    let owner_ura = required_string(obj, "owner_ura")?;
    let path = required_string(obj, "path")?;
    let owner = ura::parse_ura(owner_ura)
        .map_err(|err| IdentityError::InvalidUra(format!("{owner_ura:?}: {err}")))?;
    let owner_id = ura::protocol_resource_owner_id_from_ura(owner_ura).ok_or_else(|| {
        IdentityError::InvalidBuilder(format!(
            "owner_ura {owner_ura:?} cannot own protocol resource refs"
        ))
    })?;
    Ok(ura::resource_dot_ura(&owner.realm, &owner_id, path))
}

fn build_descriptor_ref_json(
    obj: &serde_json::Map<String, serde_json::Value>,
) -> Result<serde_json::Value, IdentityError> {
    let descriptor_version = required_string(obj, "descriptor_version")?;
    let ability_ura = match optional_string(obj, "ability_ura") {
        Some(ability_ura) => {
            let parsed = ura::parse_ura(ability_ura)
                .map_err(|err| IdentityError::InvalidUra(format!("{ability_ura:?}: {err}")))?;
            if parsed.kind != URAKind::Ability {
                return Err(IdentityError::InvalidBuilder(format!(
                    "ability_ura must be an Ability URA, got {}",
                    parsed.kind
                )));
            }
            ability_ura.to_string()
        }
        None => {
            let owner_ura = required_string(obj, "owner_ura")?;
            let ability_name = required_string(obj, "ability_name")?;
            ura::owner_ability_ura(owner_ura, ability_name).ok_or_else(|| {
                IdentityError::InvalidBuilder(format!(
                    "cannot build ability descriptor ref for owner_ura {owner_ura:?} and ability_name {ability_name:?}"
                ))
            })?
        }
    };
    let raw = format!("{ability_ura}@{descriptor_version}");
    project_descriptor_ref_json(&raw, "easynet_identity_build_descriptor_ref")
}

fn project_ura_json(
    raw: &str,
    source: &'static str,
    builder_kind: Option<&str>,
) -> Result<serde_json::Value, IdentityError> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(IdentityError::InvalidUra("URA is empty".to_string()));
    }
    let parsed =
        ura::parse_ura(raw).map_err(|err| IdentityError::InvalidUra(format!("{raw:?}: {err}")))?;
    let components = components_for_ura(&parsed)?;
    let mut metadata = serde_json::Map::new();
    metadata.insert(
        "grammar_owner".to_string(),
        serde_json::Value::String("axon".to_string()),
    );
    metadata.insert(
        "source".to_string(),
        serde_json::Value::String(source.to_string()),
    );
    if let Some(kind) = builder_kind {
        metadata.insert(
            "builder_kind".to_string(),
            serde_json::Value::String(kind.to_string()),
        );
    }
    Ok(serde_json::json!({
        "kind": parsed.kind.to_string(),
        "valid": true,
        "ura": parsed.raw,
        "realm": parsed.realm,
        "profile": PROFILE_STRICT_V2,
        "display_id": ura::display_id(raw),
        "components": components,
        "metadata": metadata,
    }))
}

fn project_descriptor_ref_json(
    raw: &str,
    source: &'static str,
) -> Result<serde_json::Value, IdentityError> {
    let canonical = canonical_ability_descriptor_ref(raw)
        .map_err(|err| IdentityError::InvalidDescriptorRef(err.to_string()))?;
    let ability_ura = ability_ura_from_descriptor_ref(&canonical)
        .map_err(|err| IdentityError::InvalidDescriptorRef(err.to_string()))?;
    let descriptor_version = descriptor_version_from_descriptor_ref(&canonical)
        .map_err(|err| IdentityError::InvalidDescriptorRef(err.to_string()))?;
    let selector = AbilitySelector::parse(ability_ura)
        .map_err(|err| IdentityError::InvalidUra(format!("{ability_ura:?}: {err}")))?;
    let mut components = serde_json::Map::new();
    components.insert(
        "owner_ura".to_string(),
        serde_json::Value::String(selector.owner_ura().to_string()),
    );
    components.insert(
        "owner_kind".to_string(),
        serde_json::Value::String(selector.owner_kind().to_string()),
    );
    components.insert(
        "public_name".to_string(),
        serde_json::Value::String(selector.public_name().to_string()),
    );
    components.insert(
        "local_registry_ability".to_string(),
        serde_json::Value::String(selector.local_registry_ability().to_string()),
    );
    Ok(serde_json::json!({
        "kind": "descriptor_ref",
        "valid": true,
        "descriptor_ref": canonical,
        "ability_ura": ability_ura,
        "descriptor_version": descriptor_version,
        "profile": PROFILE_STRICT_V2,
        "components": components,
        "metadata": {
            "grammar_owner": "axon",
            "source": source,
        },
    }))
}

fn components_for_ura(parsed: &ParsedURA) -> Result<serde_json::Value, IdentityError> {
    let mut components = serde_json::Map::new();
    match parsed.kind {
        URAKind::User => {
            insert_string(&mut components, "user_id", parsed.user_id());
        }
        URAKind::Device => {
            insert_string(&mut components, "device_id", parsed.device_id());
        }
        URAKind::Agent => {
            if let Some((device_id, agent_id)) = parsed.device_agent_ids() {
                components.insert(
                    "owner_kind".to_string(),
                    serde_json::Value::String("device".to_string()),
                );
                components.insert(
                    "device_id".to_string(),
                    serde_json::Value::String(device_id.to_string()),
                );
                components.insert(
                    "agent_id".to_string(),
                    serde_json::Value::String(agent_id.to_string()),
                );
            } else if let Some((user_id, agent_id)) = parsed.agent_ids() {
                components.insert(
                    "owner_kind".to_string(),
                    serde_json::Value::String("user".to_string()),
                );
                components.insert(
                    "user_id".to_string(),
                    serde_json::Value::String(user_id.to_string()),
                );
                components.insert(
                    "agent_id".to_string(),
                    serde_json::Value::String(agent_id.to_string()),
                );
            }
        }
        URAKind::Hub => {}
        URAKind::Resource => {
            insert_string(&mut components, "owner_id", parsed.resource_owner_id());
            insert_string(&mut components, "path", parsed.resource_path());
        }
        URAKind::Ability => {
            let selector = AbilitySelector::parse(&parsed.raw)
                .map_err(|err| IdentityError::InvalidUra(err.to_string()))?;
            components.insert(
                "owner_ura".to_string(),
                serde_json::Value::String(selector.owner_ura().to_string()),
            );
            components.insert(
                "owner_kind".to_string(),
                serde_json::Value::String(selector.owner_kind().to_string()),
            );
            components.insert(
                "public_name".to_string(),
                serde_json::Value::String(selector.public_name().to_string()),
            );
            components.insert(
                "local_registry_ability".to_string(),
                serde_json::Value::String(selector.local_registry_ability().to_string()),
            );
            if let Some(ability) = parsed.ability() {
                insert_ability_components(&mut components, ability);
            }
        }
        URAKind::Unknown => {}
    }
    Ok(serde_json::Value::Object(components))
}

fn insert_ability_components(
    components: &mut serde_json::Map<String, serde_json::Value>,
    ability: ura::ParsedAbility,
) {
    components.insert(
        "namespace".to_string(),
        serde_json::Value::String(ability.namespace),
    );
    components.insert(
        "local_name".to_string(),
        serde_json::Value::String(ability.local_name),
    );
    match ability.owner {
        AbilityOwner::Hub => {
            components.insert(
                "ability_owner_token".to_string(),
                serde_json::Value::String("hub".to_string()),
            );
        }
        AbilityOwner::Device { device_id } => {
            components.insert(
                "ability_owner_token".to_string(),
                serde_json::Value::String("device".to_string()),
            );
            components.insert(
                "device_id".to_string(),
                serde_json::Value::String(device_id),
            );
        }
        AbilityOwner::Agent { user_id, agent_id } => {
            components.insert(
                "ability_owner_token".to_string(),
                serde_json::Value::String("agent".to_string()),
            );
            components.insert("user_id".to_string(), serde_json::Value::String(user_id));
            components.insert("agent_id".to_string(), serde_json::Value::String(agent_id));
        }
    }
}

fn insert_string(
    obj: &mut serde_json::Map<String, serde_json::Value>,
    key: &'static str,
    value: Option<&str>,
) {
    if let Some(value) = value {
        obj.insert(
            key.to_string(),
            serde_json::Value::String(value.to_string()),
        );
    }
}

fn required_string<'a>(
    obj: &'a serde_json::Map<String, serde_json::Value>,
    key: &'static str,
) -> Result<&'a str, IdentityError> {
    optional_string(obj, key).ok_or(IdentityError::MissingField(key))
}

fn optional_string<'a>(
    obj: &'a serde_json::Map<String, serde_json::Value>,
    key: &'static str,
) -> Option<&'a str> {
    obj.get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

#[derive(Debug, thiserror::Error)]
enum IdentityError {
    #[error("missing or empty field `{0}`")]
    MissingField(&'static str),
    #[error("unsupported URA kind `{0}`")]
    UnsupportedKind(String),
    #[error("unsupported owner_kind `{0}`")]
    UnsupportedOwnerKind(String),
    #[error("invalid URA: {0}")]
    InvalidUra(String),
    #[error("invalid AbilityDescriptorRef: {0}")]
    InvalidDescriptorRef(String),
    #[error("{0}")]
    InvalidBuilder(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffi::client::handle::{alloc, release, test_session};
    use std::ffi::{CStr, CString};

    fn handle() -> EasynetHandle {
        let (handle, _) = alloc(test_session());
        handle
    }

    fn read_json(ptr: *mut c_char) -> serde_json::Value {
        let value = unsafe { serde_json::from_str(CStr::from_ptr(ptr).to_str().unwrap()).unwrap() };
        unsafe { crate::ffi::strings::easynet_string_free(ptr) };
        value
    }

    #[test]
    fn identity_project_ura_delegates_to_axon_parser() {
        let handle = handle();
        let raw = CString::new("easynet:///r/acme/ability/device.dev-1.fs.read").unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();

        let code = unsafe { easynet_identity_project_ura(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(value["kind"], "ability");
        assert_eq!(value["realm"], "acme");
        assert_eq!(
            value["components"]["owner_ura"],
            "easynet:///r/acme/device/dev-1"
        );
        assert_eq!(value["components"]["public_name"], "fs.read");
        release(handle);
    }

    #[test]
    fn identity_project_ura_rejects_invalid_ura() {
        let handle = handle();
        let raw = CString::new("not-a-ura").unwrap();
        let mut out: *mut c_char = std::ptr::dangling_mut();

        let code = unsafe { easynet_identity_project_ura(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, ERR_INVALID_ARG);
        assert!(out.is_null());
        release(handle);
    }

    #[test]
    fn identity_build_ura_builds_device_agent_with_owner_kind() {
        let handle = handle();
        let raw = CString::new(
            serde_json::json!({
                "kind": "agent",
                "owner_kind": "device",
                "realm": "acme",
                "device_id": "dev-1",
                "agent_id": "terminal"
            })
            .to_string(),
        )
        .unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();

        let code = unsafe { easynet_identity_build_ura(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(
            value["ura"],
            "easynet:///r/acme/agent/device.dev-1.terminal"
        );
        assert_eq!(value["components"]["owner_kind"], "device");
        release(handle);
    }

    #[test]
    fn identity_build_ura_rejects_missing_builder_field() {
        let handle = handle();
        let raw = CString::new(serde_json::json!({"kind": "device", "realm": "acme"}).to_string())
            .unwrap();
        let mut out: *mut c_char = std::ptr::dangling_mut();

        let code = unsafe { easynet_identity_build_ura(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, ERR_INVALID_ARG);
        assert!(out.is_null());
        release(handle);
    }

    #[test]
    fn identity_project_descriptor_ref_rejects_malformed_ref() {
        let handle = handle();
        let raw = CString::new("easynet:///r/acme/ability/device.dev-1.fs.read@v1@v2").unwrap();
        let mut out: *mut c_char = std::ptr::dangling_mut();

        let code =
            unsafe { easynet_identity_project_descriptor_ref(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, ERR_INVALID_ARG);
        assert!(out.is_null());
        release(handle);
    }

    #[test]
    fn identity_build_descriptor_ref_uses_owner_builder_and_version() {
        let handle = handle();
        let raw = CString::new(
            serde_json::json!({
                "owner_ura": "easynet:///r/acme/device/dev-1",
                "ability_name": "fs.read",
                "descriptor_version": "1.0.0"
            })
            .to_string(),
        )
        .unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();

        let code = unsafe { easynet_identity_build_descriptor_ref(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(
            value["descriptor_ref"],
            "easynet:///r/acme/ability/device.dev-1.fs.read@1.0.0"
        );
        assert_eq!(
            value["ability_ura"],
            "easynet:///r/acme/ability/device.dev-1.fs.read"
        );
        assert_eq!(value["components"]["owner_kind"], "device");
        release(handle);
    }

    #[test]
    fn identity_build_descriptor_ref_requires_namespaced_hub_ability() {
        let handle = handle();
        let raw = CString::new(
            serde_json::json!({
                "owner_ura": "easynet:///r/acme/hub",
                "ability_name": "chat",
                "descriptor_version": "1.0.0"
            })
            .to_string(),
        )
        .unwrap();
        let mut out: *mut c_char = std::ptr::dangling_mut();

        let code = unsafe { easynet_identity_build_descriptor_ref(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, ERR_INVALID_ARG);
        assert!(out.is_null());
        release(handle);
    }

    fn base_request(extra: serde_json::Value) -> CString {
        let mut request = serde_json::json!({
            "caller_ura": "easynet:///r/example/agent/alice.sdk",
            "callee_ura": "easynet:///r/example/device/dev-a",
            "subject_ura": "easynet:///r/example/user/alice",
            "descriptor_version": "1.0.0",
            "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
            "causal_context": {"form": "none"},
            "metadata": {"request_id": "identity-1"}
        });
        let serde_json::Value::Object(extra) = extra else {
            return CString::new(request.to_string()).unwrap();
        };
        let obj = request.as_object_mut().unwrap();
        for (key, value) in extra {
            obj.insert(key, value);
        }
        CString::new(request.to_string()).unwrap()
    }

    #[test]
    fn identity_build_signing_key_invocations_project_complete_carriers() {
        let handle = handle();
        let public_key = "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=";
        let register = base_request(serde_json::json!({
            "owner_ura": "easynet:///r/example/user/alice",
            "key_id": "alice-key-1",
            "algorithm": "ed25519",
            "public_key_base64": public_key,
            "usage": ["invocation.sign"],
            "role": "user"
        }));
        let list = base_request(serde_json::json!({
            "owner_ura": "easynet:///r/example/user/alice",
            "limit": 25
        }));
        let revoke = base_request(serde_json::json!({
            "owner_ura": "easynet:///r/example/user/alice",
            "key_id": "alice-key-1",
            "public_key_base64": public_key,
            "reason": "rotation"
        }));
        let mut out: *mut c_char = std::ptr::null_mut();

        let code = unsafe {
            easynet_identity_build_register_signing_key_invocation(
                handle,
                register.as_ptr(),
                &mut out,
            )
        };
        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(
            value["metadata"]["system_ability"],
            "identity.register_pubkey"
        );
        assert_eq!(
            value["args"]["agent_ura"],
            "easynet:///r/example/user/alice"
        );
        assert_eq!(value["args"]["public_key_b64"], public_key);

        let code = unsafe {
            easynet_identity_build_list_signing_keys_invocation(handle, list.as_ptr(), &mut out)
        };
        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(
            value["descriptor_ref"],
            "easynet:///r/example/ability/device.dev-a.identity.list_user_pubkeys@1.0.0"
        );
        assert_eq!(
            value["args"]["agent_ura"],
            "easynet:///r/example/user/alice"
        );

        let code = unsafe {
            easynet_identity_build_revoke_signing_key_invocation(handle, revoke.as_ptr(), &mut out)
        };
        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(
            value["metadata"]["system_ability"],
            "identity.revoke_user_pubkey"
        );
        assert_eq!(value["args"]["public_key_b64"], public_key);
        release(handle);
    }

    #[test]
    fn identity_project_signing_key_lifecycle_outputs_sdk_dtos() {
        let handle = handle();
        let public_key = "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=";
        let register = CString::new(
            serde_json::json!({
                "request": {
                    "owner_ura": "easynet:///r/example/user/alice",
                    "key_id": "alice-key-1",
                    "algorithm": "ed25519",
                    "public_key_base64": public_key,
                    "usage": ["invocation.sign"],
                    "role": "user"
                },
                "result": {"ok": true}
            })
            .to_string(),
        )
        .unwrap();
        let list = CString::new(
            serde_json::json!({
                "request": {
                    "owner_ura": "easynet:///r/example/user/alice",
                    "limit": 50
                },
                "result": {
                    "agent_ura": "easynet:///r/example/user/alice",
                    "keys": [{
                        "public_key_b64": public_key,
                        "added_at_unix_ms": 1783100000123u64
                    }],
                    "rotation_epoch": 3,
                    "revoked_key_count": 1
                }
            })
            .to_string(),
        )
        .unwrap();
        let revoke = CString::new(
            serde_json::json!({
                "request": {
                    "key_id": "alice-key-1",
                    "public_key_base64": public_key,
                    "reason": "rotation"
                },
                "result": {"ok": true, "removed": true}
            })
            .to_string(),
        )
        .unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();

        let code = unsafe {
            easynet_identity_project_signing_key_record(handle, register.as_ptr(), &mut out)
        };
        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(value["profile"], "directory_identity");
        assert_eq!(value["key_id"], "alice-key-1");
        assert_eq!(value["metadata"]["source"], "identity.register_pubkey");

        let code =
            unsafe { easynet_identity_project_signing_key_page(handle, list.as_ptr(), &mut out) };
        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(value["profile"], "directory_identity");
        assert_eq!(value["items"].as_array().unwrap().len(), 1);
        assert_eq!(value["metadata"]["rotation_epoch"], 3);

        let code = unsafe {
            easynet_identity_project_signing_key_revoke_result(handle, revoke.as_ptr(), &mut out)
        };
        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(value["profile"], "directory_identity");
        assert_eq!(value["key_id"], "alice-key-1");
        assert_eq!(value["state"], "revoked");
        release(handle);
    }

    #[test]
    fn identity_project_ura_rejects_invalid_handle_after_zeroing_output() {
        let raw = CString::new("easynet:///r/acme/device/dev-1").unwrap();
        let mut out: *mut c_char = std::ptr::dangling_mut();

        let code = unsafe { easynet_identity_project_ura(9_999_999, raw.as_ptr(), &mut out) };

        assert_eq!(code, ERR_INVALID_HANDLE);
        assert!(out.is_null());
    }
}
