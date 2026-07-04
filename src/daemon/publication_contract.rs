// EasyNet CLI — Publication shared contract
// ==========================================
//
// File: src/daemon/publication_contract.rs
// Description: Shared daemon SDK contract for Publication ResourceRef,
//              package validation, and system-ability Invocation carriers.
//
// Protocol Responsibility
// -----------------------
// Own the EasyNet-Cli SDK Publication DTO projection that turns local
// implementation resources into daemon-submittable carrier objects and
// delegates plugin package lifecycle mutations to the daemon plugin installer.
// This module does not sign Invocations, verify receipts, or introspect
// product host languages.
//
// Implementation Approach
// -----------------------
// Reuse daemon filesystem ResourceRef construction, AbilityManifest parsing,
// and Axon descriptor-ref canonicalization. Build complete Invocation JSON
// carriers for existing daemon system abilities instead of inventing
// profile-local transports.
//
// Usage Contract
// --------------
// Callers supply explicit Invocation tuple fields. Local paths must be absolute
// and under an existing daemon virtual root. Package validation reads only the
// package manifest (`ability.json`) and reports deterministic errors.
//
// Architectural Position
// ----------------------
// EasyNet-Cli daemon SDK Publication profile. Product facades may generate
// package contents, decorators, and host process code, but not ResourceRef,
// system-ability carrier, or descriptor-ref semantics.

use std::path::{Component, Path};

use easynet_axon::invocation::{ability_ura_from_descriptor_ref, canonical_ability_descriptor_ref};
use serde_json::{json, Map, Value};

use crate::core::ability::spec::{AbilityExec, AbilityManifest};
use crate::core::ura;
use crate::daemon::ability::builtins::device_control::ability_management::store::manifest_digest;
use crate::daemon::resources::files::{self as filesystem, FilesystemResourceCapability};
use crate::daemon::sdk_contract::{
    build_system_invocation, object, optional_string_field, required_string, validate_ura,
    SdkContractError,
};

const SYSTEM_ABILITY_DEPLOY: &str = crate::daemon::ability::names::federation::ABILITY_DEPLOY;
const SYSTEM_ABILITY_UNPUBLISH: &str = crate::daemon::ability::names::federation::ABILITY_UNPUBLISH;
const SYSTEM_ABILITY_LIST: &str = crate::daemon::ability::names::governance::META_LIST_ABILITIES;
const PUBLICATION_PROFILE: &str = "publication";
const PUBLICATION_SOURCE: &str = "read_model";
const DEFAULT_PUBLISHED_ABILITY_PAGE_SIZE: usize = 50;
const MAX_PUBLISHED_ABILITY_PAGE_SIZE: usize = 500;
const RESERVED_DEVICE_ABILITY_NAMESPACES: &[&str] = &[
    "ability", "device", "hub", "meta", "node", "remote", "system",
];

pub(crate) fn build_local_resource_ref(request: &Value) -> Result<Value, PublicationError> {
    let obj = object(request, "LocalResourceRefRequest")?;
    let path = required_string(obj, "path")?;
    let capability = parse_capability(required_string(obj, "capability")?)?;
    let path = validate_absolute_path(path, "path")?;
    if capability != FilesystemResourceCapability::Write && !path.exists() {
        return Err(PublicationError::InvalidField(
            "path",
            format!("must exist for {} resource refs", capability.as_str()),
        ));
    }
    let resource_ref = filesystem::resource_ref_for_local_path(path, capability)
        .map_err(|err| PublicationError::Contract(err.to_string()))?;
    Ok(resource_ref)
}

pub(crate) fn validate_package(request: &Value) -> Result<Value, PublicationError> {
    let obj = object(request, "ValidatePackageRequest")?;
    let path = validate_absolute_path(required_string(obj, "path")?, "path")?;
    let package = load_package(path)?;
    let valid = package.validation_json(path);
    Ok(valid)
}

pub(crate) fn install_plugin(request: &Value) -> Result<Value, PublicationError> {
    let obj = object(request, "PluginInstallRequest")?;
    let source = required_string(obj, "source")?;
    let source_path = validate_local_source_path(source, "source")?;
    if !source_path.is_dir() {
        return Err(PublicationError::InvalidField(
            "source",
            "must be an unpacked plugin package directory".to_string(),
        ));
    }
    let installed = crate::daemon::plugins::PluginInstaller::new(
        crate::daemon::plugins::index::default_plugin_root(),
    )
    .install(source_path)
    .map_err(|err| PublicationError::Contract(err.to_string()))?;
    Ok(json!({
        "profile": PUBLICATION_PROFILE,
        "kind": "plugin_install",
        "source": source,
        "install_id": format!("{}@{}", installed.id, installed.version),
        "status": "installed",
        "metadata": {
            "profile": PUBLICATION_PROFILE,
            "package_id": installed.id,
            "version": installed.version,
            "hash": sdk_hash(&installed.hash),
            "request_metadata": typed_metadata(obj)?,
        },
    }))
}

pub(crate) fn build_deploy_invocation(request: &Value) -> Result<Value, PublicationError> {
    let obj = object(request, "AbilityDeployRequest")?;
    let resource_ref = obj
        .get("resource_ref")
        .ok_or(PublicationError::MissingField("resource_ref"))?
        .clone();
    object(&resource_ref, "resource_ref")?;
    let node_id = required_string(obj, "node_id")?;
    let args = json!({
        "resource_ref": resource_ref,
        "node_id": node_id,
    });
    build_system_invocation(obj, PUBLICATION_PROFILE, SYSTEM_ABILITY_DEPLOY, args)
}

pub(crate) fn project_ability_deploy_result(input: &Value) -> Result<Value, PublicationError> {
    let obj = object(projection_payload(input), "AbilityDeployResult")?;
    let public_name = required_string(obj, "public_name")?;
    let namespace = required_string(obj, "namespace")?;
    parse_namespace(Some(namespace))?;
    let ability_ura = required_string(obj, "ability_ura")?;
    let parsed = ura::parse_ura(ability_ura)
        .map_err(|err| PublicationError::InvalidField("ability_ura", err.to_string()))?;
    if parsed.kind != ura::URAKind::Ability {
        return Err(PublicationError::InvalidField(
            "ability_ura",
            format!("must be an Ability URA, got {}", parsed.kind),
        ));
    }
    let node_id = required_string(obj, "node_id")?;
    let install_id = required_string(obj, "install_id")?;
    let state = normalized_deploy_state(required_string(obj, "state")?)?;
    let mutated_by = optional_string_field(obj, "mutated_by")?;
    if let Some(mutated_by) = mutated_by.as_deref() {
        validate_ura(mutated_by, "mutated_by")?;
    }
    let bundle = optional_string_field(obj, "bundle")?;
    Ok(json!({
        "profile": PUBLICATION_PROFILE,
        "kind": "ability_deploy_result",
        "public_name": public_name,
        "namespace": namespace,
        "ability_ura": ability_ura,
        "node_id": node_id,
        "install_id": install_id,
        "state": state,
        "mutated_by": mutated_by,
        "bundle": bundle,
        "metadata": {
            "profile": PUBLICATION_PROFILE,
            "source_ability": SYSTEM_ABILITY_DEPLOY,
            "raw_result": projection_payload(input),
        },
    }))
}

pub(crate) fn build_list_abilities_invocation(request: &Value) -> Result<Value, PublicationError> {
    let obj = object(request, "PublishedAbilityQuery")?;
    let _ = PageControls::from_request(obj)?;
    let args = list_abilities_args(obj)?;
    build_system_invocation(obj, PUBLICATION_PROFILE, SYSTEM_ABILITY_LIST, args)
}

pub(crate) fn build_show_ability_invocation(request: &Value) -> Result<Value, PublicationError> {
    let obj = object(request, "PublishedAbilityShowRequest")?;
    let descriptor_ref = canonical_descriptor_ref(required_string(obj, "descriptor_ref")?)?;
    let ability_ura = ability_ura_from_descriptor_ref(&descriptor_ref)
        .map_err(|err| PublicationError::InvalidField("descriptor_ref", err.to_string()))?;
    let mut args = Map::new();
    if let Some(owner_ura) = optional_string_field(obj, "owner_ura")? {
        validate_ura(&owner_ura, "owner_ura")?;
        args.insert("agent_ura".to_string(), Value::String(owner_ura));
    }
    args.insert(
        "subject_ura".to_string(),
        Value::String(ability_ura.to_string()),
    );
    build_system_invocation(
        obj,
        PUBLICATION_PROFILE,
        SYSTEM_ABILITY_LIST,
        Value::Object(args),
    )
}

pub(crate) fn project_published_ability_page(input: &Value) -> Result<Value, PublicationError> {
    let page_input = PageInput::parse(input)?;
    let rows = rows_from_value(
        page_input.result,
        "abilities",
        "items",
        "PublishedAbilityRows",
    )?;
    let page = page_input.controls.slice(rows)?;
    let mut items = Vec::with_capacity(page.rows.len());
    for row in page.rows {
        items.push(project_published_ability(row)?);
    }
    Ok(json!({
        "profile": PUBLICATION_PROFILE,
        "kind": "published_ability_page",
        "item_kind": "published_ability",
        "items": items,
        "next_cursor": page.next_cursor,
        "limit": page_input.controls.limit,
        "source": PUBLICATION_SOURCE,
        "metadata": {
            "profile": PUBLICATION_PROFILE,
            "source_ability": SYSTEM_ABILITY_LIST,
            "total_items": rows.len(),
        },
    }))
}

pub(crate) fn project_published_ability_record(input: &Value) -> Result<Value, PublicationError> {
    let obj = object(input, "PublishedAbilityProjection")?;
    let descriptor_ref = canonical_descriptor_ref(required_string(obj, "descriptor_ref")?)?;
    let payload = mutation_projection_payload(input);
    let rows = rows_from_value(payload, "abilities", "items", "PublishedAbilityRows")?;
    let mut matched = Vec::new();
    for row in rows {
        let projected = project_published_ability(row)?;
        if projected
            .get("descriptor")
            .and_then(Value::as_object)
            .and_then(|descriptor| descriptor.get("descriptor_ref"))
            .and_then(Value::as_str)
            == Some(descriptor_ref.as_str())
        {
            matched.push(projected);
        }
    }
    match matched.len() {
        1 => Ok(matched.remove(0)),
        0 => Err(PublicationError::InvalidField(
            "descriptor_ref",
            format!("published ability {descriptor_ref:?} was not found"),
        )),
        _ => Err(PublicationError::Contract(format!(
            "published ability {descriptor_ref:?} is ambiguous"
        ))),
    }
}

pub(crate) fn build_unpublish_invocation(request: &Value) -> Result<Value, PublicationError> {
    let obj = object(request, "UnpublishAbilityRequest")?;
    let ability_ura = required_string(obj, "ability_ura")?;
    let parsed = ura::parse_ura(ability_ura)
        .map_err(|err| PublicationError::InvalidField("ability_ura", err.to_string()))?;
    if parsed.kind != ura::URAKind::Ability {
        return Err(PublicationError::InvalidField(
            "ability_ura",
            format!("must be an Ability URA, got {}", parsed.kind),
        ));
    }
    build_system_invocation(
        obj,
        PUBLICATION_PROFILE,
        SYSTEM_ABILITY_UNPUBLISH,
        json!({
            "ability_ura": ability_ura,
        }),
    )
}

pub(crate) fn project_ability_unpublish_result(input: &Value) -> Result<Value, PublicationError> {
    let payload = mutation_projection_payload(input);
    let obj = object(payload, "AbilityUnpublishResult")?;
    if matches!(obj.get("ok").and_then(Value::as_bool), Some(false)) {
        return Err(PublicationError::InvalidField(
            "ok",
            "must be true for ability_unpublished projection".to_string(),
        ));
    }
    let ability_ura = required_string(obj, "ability_ura")?;
    let parsed = ura::parse_ura(ability_ura)
        .map_err(|err| PublicationError::InvalidField("ability_ura", err.to_string()))?;
    if parsed.kind != ura::URAKind::Ability {
        return Err(PublicationError::InvalidField(
            "ability_ura",
            format!("must be an Ability URA, got {}", parsed.kind),
        ));
    }
    let owner_ura = optional_string_field(obj, "owner_ura")?;
    if let Some(owner_ura) = owner_ura.as_deref() {
        validate_ura(owner_ura, "owner_ura")?;
    }
    let descriptor_version = projection_descriptor_version(input, obj)?;
    let descriptor_ref =
        canonical_ability_descriptor_ref(&format!("{ability_ura}@{descriptor_version}"))
            .map_err(|err| PublicationError::InvalidField("descriptor_ref", err.to_string()))?;
    Ok(json!({
        "profile": PUBLICATION_PROFILE,
        "kind": "ability_unpublished",
        "descriptor_ref": descriptor_ref,
        "owner_ura": owner_ura,
        "status": "unpublished",
        "metadata": {
            "profile": PUBLICATION_PROFILE,
            "source_ability": SYSTEM_ABILITY_UNPUBLISH,
            "ability_ura": ability_ura,
            "descriptor_version": descriptor_version,
            "public_name": optional_string_field(obj, "public_name")?,
            "removed_path": optional_string_field(obj, "removed_path")?,
            "content_hash": optional_string_field(obj, "content_hash")?,
            "raw_result": payload,
        },
    }))
}

fn list_abilities_args(obj: &Map<String, Value>) -> Result<Value, PublicationError> {
    let mut args = Map::new();
    if let Some(owner_ura) = optional_string_field(obj, "owner_ura")? {
        validate_ura(&owner_ura, "owner_ura")?;
        args.insert("agent_ura".to_string(), Value::String(owner_ura));
    }
    if let Some(ability_ura) = optional_string_field(obj, "ability_ura")? {
        let parsed = ura::parse_ura(&ability_ura)
            .map_err(|err| PublicationError::InvalidField("ability_ura", err.to_string()))?;
        if parsed.kind != ura::URAKind::Ability {
            return Err(PublicationError::InvalidField(
                "ability_ura",
                format!("must be an Ability URA, got {}", parsed.kind),
            ));
        }
        args.insert("subject_ura".to_string(), Value::String(ability_ura));
    }
    Ok(Value::Object(args))
}

#[derive(Debug, Clone, Copy)]
struct PageControls {
    limit: usize,
    offset: usize,
}

impl PageControls {
    fn from_request(obj: &Map<String, Value>) -> Result<Self, PublicationError> {
        let limit = optional_usize(obj, "limit")?.unwrap_or(DEFAULT_PUBLISHED_ABILITY_PAGE_SIZE);
        validate_limit(limit)?;
        let offset = optional_cursor_offset(obj, "cursor")?.unwrap_or(0);
        Ok(Self { limit, offset })
    }

    fn slice<'a, T>(&self, rows: &'a [T]) -> Result<PageSlice<'a, T>, PublicationError> {
        if self.offset > rows.len() {
            return Err(PublicationError::InvalidField(
                "cursor",
                "must not point past the current read-model snapshot".to_string(),
            ));
        }
        let end = self.offset.saturating_add(self.limit).min(rows.len());
        let next_cursor = if end < rows.len() {
            Some(end.to_string())
        } else {
            None
        };
        Ok(PageSlice {
            rows: &rows[self.offset..end],
            next_cursor,
        })
    }
}

struct PageSlice<'a, T> {
    rows: &'a [T],
    next_cursor: Option<String>,
}

struct PageInput<'a> {
    result: &'a Value,
    controls: PageControls,
}

impl<'a> PageInput<'a> {
    fn parse(input: &'a Value) -> Result<Self, PublicationError> {
        let input = projection_payload(input);
        let Some(obj) = input.as_object() else {
            return Ok(Self {
                result: input,
                controls: PageControls {
                    limit: DEFAULT_PUBLISHED_ABILITY_PAGE_SIZE,
                    offset: 0,
                },
            });
        };
        if let Some(result) = obj.get("result").filter(|value| !value.is_null()) {
            return Ok(Self {
                result,
                controls: PageControls::from_request(obj)?,
            });
        }
        Ok(Self {
            result: input,
            controls: PageControls::from_request(obj)?,
        })
    }
}

fn canonical_descriptor_ref(raw: &str) -> Result<String, PublicationError> {
    canonical_ability_descriptor_ref(raw)
        .map_err(|err| PublicationError::InvalidField("descriptor_ref", err.to_string()))
}

fn projection_payload(input: &Value) -> &Value {
    input
        .as_object()
        .and_then(|obj| obj.get("output_json").filter(|value| !value.is_null()))
        .unwrap_or(input)
}

fn mutation_projection_payload(input: &Value) -> &Value {
    input
        .as_object()
        .and_then(|obj| obj.get("result").filter(|value| !value.is_null()))
        .unwrap_or_else(|| projection_payload(input))
}

fn rows_from_value<'a>(
    input: &'a Value,
    primary: &'static str,
    fallback: &'static str,
    label: &'static str,
) -> Result<&'a Vec<Value>, PublicationError> {
    let obj = object(input, label)?;
    obj.get(primary)
        .or_else(|| obj.get(fallback))
        .and_then(Value::as_array)
        .ok_or_else(|| PublicationError::InvalidField(primary, "must be an array".to_string()))
}

fn project_published_ability(row: &Value) -> Result<Value, PublicationError> {
    let obj = object(row, "PublishedAbilityRow")?;
    let descriptor = project_published_descriptor(obj)?;
    let implementation = obj
        .get("implementation")
        .filter(|value| value.is_object())
        .cloned()
        .unwrap_or_else(|| json!({}));
    let mut metadata = obj
        .get("metadata")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    metadata.insert(
        "profile".to_string(),
        Value::String(PUBLICATION_PROFILE.to_string()),
    );
    metadata.insert(
        "source_ability".to_string(),
        Value::String(SYSTEM_ABILITY_LIST.to_string()),
    );
    Ok(json!({
        "descriptor": descriptor,
        "implementation": implementation,
        "metadata": Value::Object(metadata),
    }))
}

fn project_published_descriptor(obj: &Map<String, Value>) -> Result<Value, PublicationError> {
    let mut descriptor = obj
        .get("descriptor")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_else(|| obj.clone());
    let ability_ura = optional_string_field(&descriptor, "ability_ura")?
        .or_else(|| optional_string_field(obj, "ability_ura").ok().flatten())
        .ok_or(PublicationError::MissingField("ability_ura"))?;
    let parsed = ura::parse_ura(&ability_ura)
        .map_err(|err| PublicationError::InvalidField("ability_ura", err.to_string()))?;
    if parsed.kind != ura::URAKind::Ability {
        return Err(PublicationError::InvalidField(
            "ability_ura",
            format!("must be an Ability URA, got {}", parsed.kind),
        ));
    }
    let descriptor_version = optional_string_field(&descriptor, "descriptor_version")?
        .or_else(|| optional_string_field(&descriptor, "version").ok().flatten())
        .or_else(|| {
            optional_string_field(obj, "descriptor_version")
                .ok()
                .flatten()
        })
        .or_else(|| optional_string_field(obj, "version").ok().flatten())
        .unwrap_or_else(|| crate::daemon::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION.to_string());
    let descriptor_ref = optional_string_field(&descriptor, "descriptor_ref")?
        .or_else(|| optional_string_field(obj, "descriptor_ref").ok().flatten())
        .map(Ok)
        .unwrap_or_else(|| {
            canonical_ability_descriptor_ref(&format!("{ability_ura}@{descriptor_version}"))
                .map_err(|err| PublicationError::InvalidField("descriptor_ref", err.to_string()))
        })?;
    if let Some(owner_ura) = optional_string_field(&descriptor, "owner_ura")?
        .or_else(|| optional_string_field(obj, "owner_ura").ok().flatten())
    {
        validate_ura(&owner_ura, "owner_ura")?;
        descriptor.insert("owner_ura".to_string(), Value::String(owner_ura));
    }
    descriptor.insert("ability_ura".to_string(), Value::String(ability_ura));
    descriptor.insert(
        "descriptor_version".to_string(),
        Value::String(descriptor_version),
    );
    descriptor.insert("descriptor_ref".to_string(), Value::String(descriptor_ref));
    if !descriptor.contains_key("schema_hash") {
        if let Some(schema_hash) = optional_string_field(obj, "schema_hash")? {
            descriptor.insert("schema_hash".to_string(), Value::String(schema_hash));
        }
    }
    Ok(Value::Object(descriptor))
}

fn projection_descriptor_version(
    input: &Value,
    payload: &Map<String, Value>,
) -> Result<String, PublicationError> {
    let request_version = input.as_object().and_then(|obj| {
        optional_string_field(obj, "descriptor_version")
            .ok()
            .flatten()
    });
    let payload_version = optional_string_field(payload, "descriptor_version")?;
    Ok(request_version
        .or(payload_version)
        .unwrap_or_else(|| crate::daemon::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION.to_string()))
}

fn validate_limit(limit: usize) -> Result<(), PublicationError> {
    if limit == 0 || limit > MAX_PUBLISHED_ABILITY_PAGE_SIZE {
        return Err(PublicationError::InvalidField(
            "limit",
            format!("must be between 1 and {MAX_PUBLISHED_ABILITY_PAGE_SIZE}"),
        ));
    }
    Ok(())
}

fn normalized_deploy_state(raw: &str) -> Result<&'static str, PublicationError> {
    match raw {
        "ACTIVE" | "active" | "enabled" => Ok("enabled"),
        "INSTALLED" | "installed" => Ok("installed"),
        other => Err(PublicationError::InvalidField(
            "state",
            format!("unsupported deploy state {other:?}"),
        )),
    }
}

fn optional_usize(
    obj: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<usize>, PublicationError> {
    match obj.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(number)) => number
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
            .map(Some)
            .ok_or_else(|| PublicationError::InvalidField(field, "must be unsigned".to_string())),
        Some(Value::String(raw)) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Ok(None);
            }
            trimmed
                .parse::<usize>()
                .map(Some)
                .map_err(|err| PublicationError::InvalidField(field, err.to_string()))
        }
        Some(_) => Err(PublicationError::InvalidField(
            field,
            "must be an integer or decimal string".to_string(),
        )),
    }
}

fn optional_cursor_offset(
    obj: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<usize>, PublicationError> {
    match obj.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(raw)) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Ok(None);
            }
            if trimmed.starts_with('-') || !trimmed.chars().all(|ch| ch.is_ascii_digit()) {
                return Err(PublicationError::InvalidField(
                    field,
                    "must be a non-negative decimal offset cursor".to_string(),
                ));
            }
            trimmed
                .parse::<usize>()
                .map(Some)
                .map_err(|err| PublicationError::InvalidField(field, err.to_string()))
        }
        Some(_) => Err(PublicationError::InvalidField(
            field,
            "must be a cursor string".to_string(),
        )),
    }
}

struct AbilityPackage {
    manifest_bytes: Vec<u8>,
    manifest: AbilityManifest,
    namespace: String,
    wire_key: String,
}

impl AbilityPackage {
    fn validation_json(&self, package_path: &Path) -> Value {
        json!({
            "profile": PUBLICATION_PROFILE,
            "kind": "package_validation",
            "valid": true,
            "package_path": package_path.display().to_string(),
            "manifest_path": package_path.join("ability.json").display().to_string(),
            "manifest_hash": manifest_digest(&self.manifest_bytes),
            "manifest": {
                "name": self.manifest.name(),
                "namespace": self.namespace,
                "wire_key": self.wire_key,
                "descriptor_version": self.manifest.descriptor_version(),
                "description": self.manifest.description(),
                "exec_kind": exec_kind(self.manifest.exec()),
                "timeout_seconds": self.manifest.timeout_seconds(),
                "input_schema": self.manifest.input_schema(),
                "output_schema": self.manifest.output_schema(),
            },
            "errors": [],
            "metadata": {
                "profile": PUBLICATION_PROFILE,
                "frame_contract_owner": "daemon_sdk",
            },
        })
    }
}

fn load_package(path: &Path) -> Result<AbilityPackage, PublicationError> {
    if !path.is_dir() {
        return Err(PublicationError::InvalidField(
            "path",
            "must be an ability package directory".to_string(),
        ));
    }
    let manifest_path = path.join("ability.json");
    if !manifest_path.is_file() {
        return Err(PublicationError::InvalidField(
            "path",
            "package directory must contain ability.json".to_string(),
        ));
    }
    let manifest_bytes = std::fs::read(&manifest_path)
        .map_err(|err| PublicationError::Contract(format!("read ability.json: {err}")))?;
    let raw_manifest: Value = serde_json::from_slice(&manifest_bytes)
        .map_err(|err| PublicationError::InvalidField("ability.json", err.to_string()))?;
    let namespace = parse_namespace(raw_manifest.get("namespace").and_then(Value::as_str))?;
    let manifest = AbilityManifest::from_json_slice(&manifest_bytes)
        .map_err(|err| PublicationError::InvalidField("ability.json", err.to_string()))?;
    let wire_key = format!("{}.{}", namespace, manifest.name());
    Ok(AbilityPackage {
        manifest_bytes,
        manifest,
        namespace,
        wire_key,
    })
}

fn parse_namespace(raw: Option<&str>) -> Result<String, PublicationError> {
    let raw = raw
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or(PublicationError::MissingField("namespace"))?;
    if RESERVED_DEVICE_ABILITY_NAMESPACES.contains(&raw) {
        return Err(PublicationError::InvalidField(
            "namespace",
            format!("{raw:?} is reserved for daemon-owned ability surfaces"),
        ));
    }
    let mut chars = raw.chars();
    let first = chars
        .next()
        .expect("namespace was checked non-empty before validation");
    if !first.is_ascii_alphabetic() {
        return Err(PublicationError::InvalidField(
            "namespace",
            "must start with an ASCII letter".to_string(),
        ));
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err(PublicationError::InvalidField(
            "namespace",
            "may contain only ASCII letters, digits, `_`, or `-`".to_string(),
        ));
    }
    Ok(raw.to_string())
}

fn exec_kind(exec: Option<&AbilityExec>) -> &'static str {
    match exec {
        None => "agent_chat",
        Some(AbilityExec::Shell(_)) => "shell",
        Some(AbilityExec::Http(_)) => "http",
        Some(AbilityExec::HostStream(_)) => "host_stream",
        Some(AbilityExec::Eal(_)) => "eal",
        Some(AbilityExec::Mcp(_)) => "mcp",
    }
}

fn parse_capability(raw: &str) -> Result<FilesystemResourceCapability, PublicationError> {
    match raw.trim() {
        "list" => Ok(FilesystemResourceCapability::List),
        "stat" => Ok(FilesystemResourceCapability::Stat),
        "read" => Ok(FilesystemResourceCapability::Read),
        "write" => Ok(FilesystemResourceCapability::Write),
        other => Err(PublicationError::InvalidField(
            "capability",
            format!("unsupported filesystem capability {other:?}"),
        )),
    }
}

fn validate_absolute_path<'a>(
    raw: &'a str,
    field: &'static str,
) -> Result<&'a Path, PublicationError> {
    let path = Path::new(raw);
    if !path.is_absolute() {
        return Err(PublicationError::InvalidField(
            field,
            "must be an absolute path".to_string(),
        ));
    }
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(PublicationError::InvalidField(
            field,
            "must not contain `..` components".to_string(),
        ));
    }
    Ok(path)
}

fn validate_local_source_path<'a>(
    raw: &'a str,
    field: &'static str,
) -> Result<&'a Path, PublicationError> {
    let path = raw.strip_prefix("file://").unwrap_or(raw);
    validate_absolute_path(path, field)
}

fn typed_metadata(obj: &Map<String, Value>) -> Result<Value, PublicationError> {
    match obj.get("metadata") {
        None | Some(Value::Null) => Ok(json!({})),
        Some(value @ Value::Object(_)) => Ok(value.clone()),
        Some(_) => Err(PublicationError::InvalidField(
            "metadata",
            "must be an object or null".to_string(),
        )),
    }
}

fn sdk_hash(raw: &str) -> String {
    if raw.starts_with("sha256:") {
        raw.to_string()
    } else {
        format!("sha256:{raw}")
    }
}

pub(crate) type PublicationError = SdkContractError;

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn nonce() -> &'static str {
        "AQIDBAUGBwgJCgsMDQ4PEA=="
    }

    fn write_package(dir: &Path, namespace: &str) {
        let body = format!(
            r#"{{
                "name": "weather",
                "namespace": "{namespace}",
                "description": "Weather stream",
                "input_schema": {{"type": "object", "properties": {{}}}},
                "exec": {{
                    "kind": "host_stream",
                    "host_socket": "/tmp/easynet-weather.sock",
                    "function": "weather.stream"
                }}
            }}"#
        );
        let mut file = std::fs::File::create(dir.join("ability.json")).unwrap();
        file.write_all(body.as_bytes()).unwrap();
    }

    fn write_plugin_package(root: &Path, id: &str, version: &str, ability: &str) {
        std::fs::create_dir_all(root.join("abilities")).expect("abilities dir");
        std::fs::create_dir_all(root.join("bin")).expect("bin dir");
        std::fs::write(
            root.join("plugin.toml"),
            format!(
                r#"
schema_version = "1"
id = "{id}"
version = "{version}"
kind = "sidecar"
entrypoint = "bin/echo-sidecar"
abilities = ["abilities/*.ability.toml"]
permissions = []
resources = []
platforms = []

[limits]
max_sessions = 1
max_frame_queue = 1

[[ability_metadata]]
name = "{ability}"
layer = "control"
"#
            ),
        )
        .expect("manifest");
        std::fs::write(
            root.join(format!("abilities/{ability}.ability.toml")),
            format!(
                r#"schema_version = "1"
name = "{ability}"
description = "test descriptor for {ability}"

[input_schema]
type = "object"
additionalProperties = false
"#
            ),
        )
        .expect("descriptor");
        let executable = root.join("bin/echo-sidecar");
        std::fs::write(&executable, "#!/bin/sh\n").expect("executable");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let mut permissions = std::fs::metadata(&executable)
                .expect("executable metadata")
                .permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&executable, permissions).expect("chmod executable");
        }
    }

    fn carrier_request(resource_ref: Value) -> Value {
        json!({
            "caller_ura": "easynet:///r/example/agent/alice.sdk",
            "callee_ura": "easynet:///r/example/device/dev-a",
            "subject_ura": "easynet:///r/example/device/dev-a",
            "descriptor_version": "1.0.0",
            "nonce_base64": nonce(),
            "causal_context": {"form": "none"},
            "resource_ref": resource_ref,
            "node_id": "local"
        })
    }

    #[test]
    fn validate_package_projects_manifest_facts() {
        let dir = tempfile::tempdir().unwrap();
        write_package(dir.path(), "er");

        let validation =
            validate_package(&json!({"path": dir.path().display().to_string()})).unwrap();

        assert_eq!(validation["profile"], PUBLICATION_PROFILE);
        assert_eq!(validation["valid"], true);
        assert_eq!(validation["manifest"]["name"], "weather");
        assert_eq!(validation["manifest"]["namespace"], "er");
        assert_eq!(validation["manifest"]["wire_key"], "er.weather");
        assert_eq!(validation["manifest"]["exec_kind"], "host_stream");
        assert!(validation["manifest_hash"]
            .as_str()
            .unwrap()
            .starts_with("sha256:"));
    }

    #[test]
    fn validate_package_rejects_reserved_namespace() {
        let dir = tempfile::tempdir().unwrap();
        write_package(dir.path(), "device");

        let err = validate_package(&json!({"path": dir.path().display().to_string()})).unwrap_err();

        assert!(format!("{err}").contains("reserved"));
    }

    #[test]
    fn install_plugin_delegates_to_daemon_plugin_installer() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let home = std::path::PathBuf::from(std::env::var("HOME").unwrap());
        let source = tempfile::tempdir().unwrap();
        write_plugin_package(
            source.path(),
            "sdk.publication.test",
            "0.1.0",
            "sdk.publication_echo",
        );

        let result = install_plugin(&json!({
            "source": source.path().display().to_string(),
            "metadata": {"request_id": "plugin-install-1"}
        }))
        .unwrap();

        assert_eq!(result["profile"], PUBLICATION_PROFILE);
        assert_eq!(result["kind"], "plugin_install");
        assert_eq!(result["install_id"], "sdk.publication.test@0.1.0");
        assert_eq!(result["status"], "installed");
        assert_eq!(result["metadata"]["package_id"], "sdk.publication.test");
        assert!(result["metadata"]["hash"]
            .as_str()
            .unwrap()
            .starts_with("sha256:"));
        assert_eq!(
            result["metadata"]["request_metadata"]["request_id"],
            "plugin-install-1"
        );
        assert!(home
            .join(".easynet/plugins/installed/sdk.publication.test/0.1.0/plugin.toml")
            .exists());
    }

    #[test]
    fn build_local_resource_ref_uses_daemon_filesystem_ref_shape() {
        let dir = tempfile::tempdir().unwrap();
        let package_dir = dir.path().join("pkg");
        std::fs::create_dir(&package_dir).unwrap();

        let resource_ref = build_local_resource_ref(&json!({
            "path": package_dir.display().to_string(),
            "capability": "read"
        }))
        .unwrap();

        assert_eq!(resource_ref["namespace"], "fs");
        assert_eq!(resource_ref["capability"], "read");
        assert!(resource_ref["resource_ura"]
            .as_str()
            .unwrap()
            .contains("/resource/"));
        assert_eq!(resource_ref["revision"], "fs-local-mapping-v1");
    }

    #[test]
    fn build_deploy_invocation_returns_complete_tuple_for_system_ability() {
        let resource_ref = json!({
            "resource_ura": "easynet:///r/example/resource/device.dev-a/fs/tmp/pkg",
            "owner_ura": "easynet:///r/example/device/dev-a",
            "namespace": "fs",
            "display_path": "tmp/pkg",
            "capability": "read",
            "expires_unix_ms": 4102444800000i64,
            "revision": "fs-local-mapping-v1"
        });

        let invocation = build_deploy_invocation(&carrier_request(resource_ref)).unwrap();

        assert_eq!(
            invocation["caller_ura"],
            "easynet:///r/example/agent/alice.sdk"
        );
        assert_eq!(invocation["args"]["node_id"], "local");
        assert_eq!(
            invocation["metadata"]["system_ability"],
            SYSTEM_ABILITY_DEPLOY
        );
        assert_eq!(invocation["content_type"], "application/json");
        assert!(invocation["descriptor_ref"]
            .as_str()
            .unwrap()
            .contains("ability.deploy@1.0.0"));
    }

    #[test]
    fn project_ability_deploy_result_normalizes_daemon_state() {
        let result = project_ability_deploy_result(&json!({
            "public_name": "weather",
            "namespace": "er",
            "ability_ura": "easynet:///r/example/ability/device.dev-a.er.weather",
            "node_id": "dev-a",
            "mutated_by": "easynet:///r/example/device/dev-a",
            "install_id": "install-1",
            "bundle": "tmp/pkg",
            "state": "ACTIVE"
        }))
        .unwrap();

        assert_eq!(result["profile"], PUBLICATION_PROFILE);
        assert_eq!(result["kind"], "ability_deploy_result");
        assert_eq!(result["state"], "enabled");
        assert_eq!(result["metadata"]["source_ability"], SYSTEM_ABILITY_DEPLOY);
        assert_eq!(result["metadata"]["raw_result"]["state"], "ACTIVE");
    }

    #[test]
    fn build_list_abilities_invocation_returns_complete_tuple_for_catalog_read() {
        let request = json!({
            "caller_ura": "easynet:///r/example/agent/alice.sdk",
            "callee_ura": "easynet:///r/example/device/dev-a",
            "subject_ura": "easynet:///r/example/device/dev-a",
            "descriptor_version": "1.0.0",
            "nonce_base64": nonce(),
            "causal_context": {"form": "none"},
            "limit": 25,
            "ability_ura": "easynet:///r/example/ability/device.dev-a.er.weather"
        });

        let invocation = build_list_abilities_invocation(&request).unwrap();

        assert_eq!(
            invocation["metadata"]["system_ability"],
            SYSTEM_ABILITY_LIST
        );
        assert_eq!(
            invocation["args"]["subject_ura"],
            "easynet:///r/example/ability/device.dev-a.er.weather"
        );
        assert!(invocation["descriptor_ref"]
            .as_str()
            .unwrap()
            .contains("meta.list_abilities@1.0.0"));
    }

    #[test]
    fn build_show_ability_invocation_uses_descriptor_ref_projection() {
        let request = json!({
            "caller_ura": "easynet:///r/example/agent/alice.sdk",
            "callee_ura": "easynet:///r/example/device/dev-a",
            "subject_ura": "easynet:///r/example/device/dev-a",
            "descriptor_version": "1.0.0",
            "nonce_base64": nonce(),
            "causal_context": {"form": "none"},
            "descriptor_ref": "easynet:///r/example/ability/device.dev-a.er.weather@2.0.0",
            "owner_ura": "easynet:///r/example/device/dev-a"
        });

        let invocation = build_show_ability_invocation(&request).unwrap();

        assert_eq!(
            invocation["metadata"]["system_ability"],
            SYSTEM_ABILITY_LIST
        );
        assert_eq!(
            invocation["args"]["subject_ura"],
            "easynet:///r/example/ability/device.dev-a.er.weather"
        );
        assert_eq!(
            invocation["args"]["agent_ura"],
            "easynet:///r/example/device/dev-a"
        );
    }

    #[test]
    fn project_published_ability_page_bounds_and_stamps_descriptor_ref() {
        let page = project_published_ability_page(&json!({
            "result": {
                "abilities": [
                    {
                        "name": "weather",
                        "ability_ura": "easynet:///r/example/ability/device.dev-a.er.weather",
                        "owner_ura": "easynet:///r/example/device/dev-a",
                        "version": "1.0.0",
                        "schema_hash": "sha256:abc"
                    },
                    {
                        "name": "camera",
                        "ability_ura": "easynet:///r/example/ability/device.dev-a.er.camera",
                        "owner_ura": "easynet:///r/example/device/dev-a",
                        "version": "1.0.0",
                        "schema_hash": "sha256:def"
                    }
                ]
            },
            "limit": 1
        }))
        .unwrap();

        assert_eq!(page["profile"], PUBLICATION_PROFILE);
        assert_eq!(page["limit"], 1);
        assert_eq!(page["next_cursor"], "1");
        assert_eq!(
            page["items"][0]["descriptor"]["descriptor_ref"],
            "easynet:///r/example/ability/device.dev-a.er.weather@1.0.0"
        );
        assert_eq!(page["items"][0]["implementation"], json!({}));
    }

    #[test]
    fn project_published_ability_record_selects_exact_descriptor_ref() {
        let ability = project_published_ability_record(&json!({
            "descriptor_ref": "easynet:///r/example/ability/device.dev-a.er.weather@2.0.0",
            "result": {
                "abilities": [
                    {
                        "name": "weather",
                        "ability_ura": "easynet:///r/example/ability/device.dev-a.er.weather",
                        "owner_ura": "easynet:///r/example/device/dev-a",
                        "version": "1.0.0",
                        "schema_hash": "sha256:old"
                    },
                    {
                        "name": "weather",
                        "ability_ura": "easynet:///r/example/ability/device.dev-a.er.weather",
                        "owner_ura": "easynet:///r/example/device/dev-a",
                        "version": "2.0.0",
                        "schema_hash": "sha256:new",
                        "implementation": {"impl_id": "impl-2"}
                    }
                ]
            }
        }))
        .unwrap();

        assert_eq!(
            ability["descriptor"]["descriptor_ref"],
            "easynet:///r/example/ability/device.dev-a.er.weather@2.0.0"
        );
        assert_eq!(ability["implementation"]["impl_id"], "impl-2");
        assert_eq!(ability["metadata"]["source_ability"], SYSTEM_ABILITY_LIST);
    }

    #[test]
    fn project_published_ability_record_rejects_missing_match() {
        let err = project_published_ability_record(&json!({
            "descriptor_ref": "easynet:///r/example/ability/device.dev-a.er.weather@2.0.0",
            "result": {
                "abilities": [{
                    "name": "weather",
                    "ability_ura": "easynet:///r/example/ability/device.dev-a.er.weather",
                    "owner_ura": "easynet:///r/example/device/dev-a",
                    "version": "1.0.0"
                }]
            }
        }))
        .unwrap_err();

        assert!(format!("{err}").contains("not found"));
    }

    #[test]
    fn build_unpublish_invocation_rejects_non_ability_ura() {
        let request = json!({
            "caller_ura": "easynet:///r/example/agent/alice.sdk",
            "callee_ura": "easynet:///r/example/device/dev-a",
            "subject_ura": "easynet:///r/example/device/dev-a",
            "descriptor_version": "1.0.0",
            "nonce_base64": nonce(),
            "causal_context": {"form": "none"},
            "ability_ura": "easynet:///r/example/device/dev-a"
        });

        let err = build_unpublish_invocation(&request).unwrap_err();

        assert!(format!("{err}").contains("Ability URA"));
    }

    #[test]
    fn project_ability_unpublish_result_uses_request_descriptor_version() {
        let result = project_ability_unpublish_result(&json!({
            "descriptor_version": "1.2.3",
            "result": {
                "ok": true,
                "owner_ura": "easynet:///r/example/device/dev-a",
                "public_name": "weather",
                "ability_ura": "easynet:///r/example/ability/device.dev-a.er.weather",
                "removed_path": "/tmp/easynet/abilities/weather.ability.json",
                "content_hash": "sha256:abc"
            }
        }))
        .unwrap();

        assert_eq!(result["profile"], PUBLICATION_PROFILE);
        assert_eq!(result["kind"], "ability_unpublished");
        assert_eq!(result["status"], "unpublished");
        assert_eq!(
            result["descriptor_ref"],
            "easynet:///r/example/ability/device.dev-a.er.weather@1.2.3"
        );
        assert_eq!(
            result["metadata"]["source_ability"],
            SYSTEM_ABILITY_UNPUBLISH
        );
        assert_eq!(
            result["metadata"]["raw_result"]["content_hash"],
            "sha256:abc"
        );
    }

    #[test]
    fn project_ability_unpublish_result_rejects_failed_daemon_ack() {
        let err = project_ability_unpublish_result(&json!({
            "descriptor_version": "1.0.0",
            "result": {
                "ok": false,
                "ability_ura": "easynet:///r/example/ability/device.dev-a.er.weather"
            }
        }))
        .unwrap_err();

        assert!(format!("{err}").contains("ok"));
    }
}
