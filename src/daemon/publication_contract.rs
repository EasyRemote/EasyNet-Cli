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
// implementation resources into daemon-submittable carrier objects. This
// module does not execute publication, sign Invocations, verify receipts, or
// introspect product host languages.
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

use serde_json::{json, Value};

use crate::core::ability::spec::{AbilityExec, AbilityManifest};
use crate::core::ura;
use crate::daemon::ability::builtins::device_control::ability_management::store::manifest_digest;
use crate::daemon::resources::files::{self as filesystem, FilesystemResourceCapability};
use crate::daemon::sdk_contract::{
    build_system_invocation, object, required_string, SdkContractError,
};

const SYSTEM_ABILITY_DEPLOY: &str = crate::daemon::ability::names::federation::ABILITY_DEPLOY;
const SYSTEM_ABILITY_UNPUBLISH: &str = crate::daemon::ability::names::federation::ABILITY_UNPUBLISH;
const PUBLICATION_PROFILE: &str = "publication";
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
}
