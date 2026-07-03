// EasyNet CLI — Filesystem ResourceRef Domain
// ===========================================
//
// File: src/daemon/resources/filesystem.rs
// Description: ResourceRef generation and revalidation for daemon-local
//              filesystem abilities.
//
// Protocol Responsibility
// -----------------------
// Owns the EasyNet-Cli daemon policy that maps an RFC-005 filesystem
// ResourceRef to one local host path. It does not define Axon Invocation
// canonicalization, admission, signatures, receipts, or cross-realm routing.
//
// Implementation Approach
// -----------------------
// Callers pass `resource_ref` objects. This module validates namespace,
// revision, expiry, capability, Resource URA owner, virtual root label, and
// relative path syntax before returning a bounded local path. Raw host paths
// are intentionally not accepted by public filesystem abilities.
//
// Usage Contract
// --------------
// Ability handlers consume `ResolvedFilesystemPath` and perform exactly one
// filesystem verb. They do not parse ResourceRef JSON, derive local roots, or
// decide capability implication rules.
//
// Architectural Position
// ----------------------
// EasyNet-Cli daemon resource plane. Shared by `fs.*`,
// `fs.edit`, and `fs.transfer`; not exported as an Axon SDK API.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Result};
use serde::Deserialize;
use serde_json::{json, Value};

const RESOURCE_NAMESPACE_FS: &str = "fs";
const VIRTUAL_ROOT_WORKSPACE: &str = "workspace";
const VIRTUAL_ROOT_TMP: &str = "tmp";
const VIRTUAL_ROOT_HOME: &str = "home";
const LOCAL_RESOURCE_REF_REVISION: &str = "fs-local-mapping-v1";
const DEFAULT_LOCAL_RESOURCE_REF_TTL_MS: i64 = 5 * 60 * 1000;

/// Filesystem operation class carried by a ResourceRef.
///
/// This enum is intentionally ordered by domain semantics, not by wire
/// privilege level. `permits` is the single source for capability implication:
/// write can read/stat, read can stat, list can stat a directory entry.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FilesystemResourceCapability {
    List,
    Stat,
    Read,
    Write,
}

impl FilesystemResourceCapability {
    /// Lowercase wire form used inside the `resource_ref.capability` field.
    pub fn as_str(self) -> &'static str {
        match self {
            FilesystemResourceCapability::List => "list",
            FilesystemResourceCapability::Stat => "stat",
            FilesystemResourceCapability::Read => "read",
            FilesystemResourceCapability::Write => "write",
        }
    }

    fn permits(self, requested: Self) -> bool {
        self == requested
            || matches!(
                (self, requested),
                (
                    FilesystemResourceCapability::Write,
                    FilesystemResourceCapability::Read
                ) | (
                    FilesystemResourceCapability::Write,
                    FilesystemResourceCapability::Stat
                ) | (
                    FilesystemResourceCapability::Read,
                    FilesystemResourceCapability::Stat
                ) | (
                    FilesystemResourceCapability::List,
                    FilesystemResourceCapability::Stat
                )
            )
    }
}

/// Local filesystem path produced after ResourceRef revalidation.
///
/// Invariants:
/// 1. `local_path` is rooted under `virtual_root_path` when that field is set.
/// 2. `display_path` is presentation-only and never used for syscalls.
/// 3. `virtual_root_path` exists for filesystem ResourceRefs so write paths can
///    re-check symlink and parent escapes after resolving the final target.
#[derive(Debug, Clone)]
pub struct ResolvedFilesystemPath {
    pub local_path: PathBuf,
    pub display_path: String,
    pub virtual_root_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize)]
struct FilesystemResourceRef {
    resource_ura: String,
    owner_ura: String,
    namespace: String,
    #[serde(default)]
    display_path: String,
    capability: FilesystemResourceCapability,
    expires_unix_ms: i64,
    revision: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LocalFilesystemResourcePath {
    virtual_root: String,
    relative_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FilesystemResourceParts {
    owner_ura: String,
    virtual_root: String,
    relative_path: String,
}

/// Create a short-lived local filesystem ResourceRef for a host path.
///
/// The returned object is suitable for tests, local session dispatch, and
/// daemon-internal request construction. It refuses paths outside the built-in
/// virtual roots; callers cannot mint arbitrary raw host path references.
pub fn resource_ref_for_local_path(
    path: &Path,
    capability: FilesystemResourceCapability,
) -> Result<Value> {
    let mapped = map_local_path_to_virtual_resource(path)?;
    let expires_unix_ms = now_unix_ms().saturating_add(DEFAULT_LOCAL_RESOURCE_REF_TTL_MS);
    Ok(resource_ref_value(
        &mapped.virtual_root,
        &mapped.relative_path,
        capability,
        expires_unix_ms,
    ))
}

/// Resolve and revalidate the `resource_ref` field from ability args.
pub(crate) fn resolve_filesystem_path(
    args: &Value,
    requested_capability: FilesystemResourceCapability,
) -> Result<ResolvedFilesystemPath> {
    resolve_filesystem_path_with_mode(args, requested_capability, true)
}

/// Resolve a `resource_ref` while allowing a missing final target.
///
/// Used by upload-style abilities that need to create the target after
/// validating that its nearest existing ancestor remains inside the root.
pub(crate) fn resolve_filesystem_path_without_existing_target(
    args: &Value,
    requested_capability: FilesystemResourceCapability,
) -> Result<ResolvedFilesystemPath> {
    resolve_filesystem_path_with_mode(args, requested_capability, false)
}

fn resolve_filesystem_path_with_mode(
    args: &Value,
    requested_capability: FilesystemResourceCapability,
    canonicalize_existing_target: bool,
) -> Result<ResolvedFilesystemPath> {
    let resource_ref = args
        .get("resource_ref")
        .ok_or_else(|| anyhow!("resource_ref: missing required object"))?;
    let reference: FilesystemResourceRef = serde_json::from_value(resource_ref.clone())
        .map_err(|e| anyhow!("resource_ref: invalid shape: {e}"))?;
    resolve_resource_ref(
        reference,
        requested_capability,
        canonicalize_existing_target && requested_capability != FilesystemResourceCapability::Write,
    )
}

fn resolve_resource_ref(
    reference: FilesystemResourceRef,
    requested_capability: FilesystemResourceCapability,
    canonicalize_existing_target: bool,
) -> Result<ResolvedFilesystemPath> {
    if reference.namespace != RESOURCE_NAMESPACE_FS {
        return Err(anyhow!(
            "resource_ref: namespace {:?} is not fs",
            reference.namespace
        ));
    }
    if reference.revision != LOCAL_RESOURCE_REF_REVISION {
        return Err(anyhow!(
            "resource_ref: revision mismatch, expected {} got {}",
            LOCAL_RESOURCE_REF_REVISION,
            reference.revision
        ));
    }
    if !reference.capability.permits(requested_capability) {
        return Err(anyhow!(
            "resource_ref: capability {} does not permit {}",
            reference.capability.as_str(),
            requested_capability.as_str()
        ));
    }
    let now = now_unix_ms();
    if reference.expires_unix_ms <= now {
        return Err(anyhow!(
            "resource_ref: expired at {}, now {}",
            reference.expires_unix_ms,
            now
        ));
    }

    let parts = parse_filesystem_resource_ura(&reference.resource_ura)?;
    if parts.owner_ura != reference.owner_ura {
        return Err(anyhow!(
            "resource_ref: owner mismatch, resource_ura owner {} but ref owner {}",
            parts.owner_ura,
            reference.owner_ura
        ));
    }
    let root = virtual_root_path(&parts.virtual_root).ok_or_else(|| {
        anyhow!(
            "resource_ref: virtual root {} has no local mapping",
            parts.virtual_root
        )
    })?;
    let mut local_path = root.join(&parts.relative_path);
    if canonicalize_existing_target {
        ensure_path_under_root(&local_path, &root)?;
        local_path = std::fs::canonicalize(&local_path).map_err(|e| {
            anyhow!("resource_ref: path {local_path:?} cannot be canonicalized: {e}")
        })?;
    }
    let display_path = if reference.display_path.trim().is_empty() {
        format!("{}/{}", parts.virtual_root, parts.relative_path)
    } else {
        reference.display_path
    };

    Ok(ResolvedFilesystemPath {
        local_path,
        display_path,
        virtual_root_path: Some(root),
    })
}

fn resource_ref_value(
    virtual_root: &str,
    relative_path: &str,
    capability: FilesystemResourceCapability,
    expires_unix_ms: i64,
) -> Value {
    // Bind the local-mapping ResourceRef to this device's REAL paired identity
    // (realm + node_id), not a fixed `localhost`/`local-device` placeholder. The
    // realm is hub-issued at join and every other URA mint honours it; this was
    // the one production site that advertised `easynet:///r/localhost/...` while
    // the device's real URA lives under its paired realm. Fall back to the
    // canonical unpaired sentinel (default/local) when no credentials exist,
    // mirroring `local_invocation_identity::local_device_ura`.
    let (realm, device_id) = crate::daemon::persistence::config::load_credentials()
        .ok()
        .map(|c| (c.realm, c.node_id))
        .unwrap_or_else(|| {
            (
                crate::daemon::identity::local_invocation::UNPAIRED_LOCAL_REALM.to_string(),
                crate::daemon::identity::local_invocation::UNPAIRED_LOCAL_DEVICE_ID.to_string(),
            )
        });
    let owner_token = format!("device.{device_id}");
    let owner_ura = crate::core::ura::device_ura(&realm, &device_id);
    json!({
        "resource_ura": crate::core::ura::resource_dot_ura(
            &realm,
            &owner_token,
            &format!("fs/{virtual_root}/{relative_path}")
        ),
        "owner_ura": owner_ura,
        "namespace": RESOURCE_NAMESPACE_FS,
        "display_path": format!("{virtual_root}/{relative_path}"),
        "capability": capability.as_str(),
        "expires_unix_ms": expires_unix_ms,
        "revision": LOCAL_RESOURCE_REF_REVISION
    })
}

fn map_local_path_to_virtual_resource(path: &Path) -> Result<LocalFilesystemResourcePath> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| anyhow!("resource_ref: current directory unavailable: {e}"))?
            .join(path)
    };
    let candidates = [
        (
            VIRTUAL_ROOT_WORKSPACE,
            virtual_root_path(VIRTUAL_ROOT_WORKSPACE),
        ),
        (VIRTUAL_ROOT_TMP, virtual_root_path(VIRTUAL_ROOT_TMP)),
        (VIRTUAL_ROOT_HOME, virtual_root_path(VIRTUAL_ROOT_HOME)),
    ];

    for (label, root) in candidates {
        let Some(root) = root else {
            continue;
        };
        if let Some(relative_path) = relative_path_under_root(&absolute, &root)? {
            validate_relative_path(&relative_path)?;
            return Ok(LocalFilesystemResourcePath {
                virtual_root: label.to_string(),
                relative_path,
            });
        }
    }

    Err(anyhow!(
        "resource_ref: local path {:?} is outside built-in filesystem virtual roots",
        path
    ))
}

fn relative_path_under_root(path: &Path, root: &Path) -> Result<Option<String>> {
    let canonical_root = std::fs::canonicalize(root)
        .map_err(|e| anyhow!("resource_ref: virtual root mapping {root:?} unavailable: {e}"))?;
    let root_spelled_path = canonical_parent_spelled_path(path)?;
    if let Ok(relative) = root_spelled_path.strip_prefix(&canonical_root) {
        return relative_path_to_wire(relative).map(Some);
    }

    let existing = nearest_existing_ancestor(&root_spelled_path);
    let canonical_existing = std::fs::canonicalize(&existing)
        .map_err(|e| anyhow!("resource_ref: path ancestor {existing:?} unavailable: {e}"))?;
    if !canonical_existing.starts_with(&canonical_root) {
        return Ok(None);
    }

    let existing_rel = canonical_existing
        .strip_prefix(&canonical_root)
        .map_err(|e| anyhow!("resource_ref: strip virtual root prefix: {e}"))?;
    let suffix = root_spelled_path
        .strip_prefix(&existing)
        .map_err(|e| anyhow!("resource_ref: strip existing path prefix: {e}"))?;
    let relative = if suffix.as_os_str().is_empty() {
        existing_rel.to_path_buf()
    } else {
        existing_rel.join(suffix)
    };
    relative_path_to_wire(&relative).map(Some)
}

fn canonical_parent_spelled_path(path: &Path) -> Result<PathBuf> {
    if let (Some(parent), Some(file_name)) = (path.parent(), path.file_name()) {
        if parent.exists() {
            return Ok(std::fs::canonicalize(parent)
                .map_err(|e| anyhow!("resource_ref: path parent {parent:?} unavailable: {e}"))?
                .join(file_name));
        }
    }
    Ok(path.to_path_buf())
}

fn relative_path_to_wire(relative: &Path) -> Result<String> {
    let relative = relative
        .to_str()
        .ok_or_else(|| anyhow!("resource_ref: relative path must be UTF-8"))?
        .trim_start_matches('/')
        .to_string();
    if relative.is_empty() {
        return Err(anyhow!("resource_ref: relative path must not be empty"));
    }
    Ok(relative)
}

fn parse_filesystem_resource_ura(resource_ura: &str) -> Result<FilesystemResourceParts> {
    let parsed = crate::core::ura::parse_ura(resource_ura)
        .map_err(|e| anyhow!("resource_ref: invalid resource_ura {resource_ura:?}: {e}"))?;
    if parsed.kind != crate::core::ura::URAKind::Resource {
        return Err(anyhow!(
            "resource_ref: resource_ura must be a Resource URA, got {}",
            parsed.kind
        ));
    }
    let owner_id = parsed
        .resource_owner_id()
        .ok_or_else(|| anyhow!("resource_ref: resource owner missing"))?;
    let device_id = owner_id
        .strip_prefix("device.")
        .ok_or_else(|| anyhow!("resource_ref: resource owner must be device.<device-id>"))?;
    let mut segments = parsed
        .resource_path()
        .unwrap_or_default()
        .split('/')
        .filter(|segment| !segment.is_empty());
    let namespace = segments
        .next()
        .ok_or_else(|| anyhow!("resource_ref: resource path missing namespace"))?;
    if namespace != RESOURCE_NAMESPACE_FS {
        return Err(anyhow!(
            "resource_ref: resource namespace {namespace:?} is not fs"
        ));
    }
    let virtual_root = segments
        .next()
        .ok_or_else(|| anyhow!("resource_ref: fs resource path missing virtual root"))?;
    validate_virtual_root_label(virtual_root)?;
    let relative_path = segments.collect::<Vec<_>>().join("/");
    validate_relative_path(&relative_path)?;
    Ok(FilesystemResourceParts {
        owner_ura: crate::core::ura::device_ura(&parsed.realm, device_id),
        virtual_root: virtual_root.to_string(),
        relative_path,
    })
}

fn validate_virtual_root_label(virtual_root: &str) -> Result<()> {
    if virtual_root.trim().is_empty() {
        return Err(anyhow!("resource_ref: virtual root must not be empty"));
    }
    if virtual_root
        .bytes()
        .any(|b| !(b.is_ascii_alphanumeric() || b == b'-' || b == b'_'))
    {
        return Err(anyhow!(
            "resource_ref: virtual root {virtual_root:?} is outside allowed label syntax"
        ));
    }
    Ok(())
}

fn validate_relative_path(relative_path: &str) -> Result<()> {
    if relative_path.trim().is_empty() {
        return Err(anyhow!("resource_ref: relative path must not be empty"));
    }
    if relative_path.starts_with('/')
        || relative_path.contains('\\')
        || looks_like_windows_drive(relative_path)
    {
        return Err(anyhow!(
            "resource_ref: relative path {relative_path:?} must not be a raw host path"
        ));
    }
    if relative_path
        .split('/')
        .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return Err(anyhow!(
            "resource_ref: relative path {relative_path:?} contains traversal"
        ));
    }
    Ok(())
}

fn looks_like_windows_drive(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn virtual_root_path(virtual_root: &str) -> Option<PathBuf> {
    match virtual_root {
        VIRTUAL_ROOT_WORKSPACE => std::env::current_dir().ok(),
        VIRTUAL_ROOT_TMP => Some(std::env::temp_dir()),
        VIRTUAL_ROOT_HOME => Some(crate::daemon::persistence::config::home_dir()),
        _ => None,
    }
}

/// Ensure the nearest existing parent for a future write stays inside `root`.
pub(crate) fn ensure_write_parent_under_root(path: &Path, root: &Path) -> Result<()> {
    let parent = path.parent().unwrap_or(root);
    let existing = nearest_existing_ancestor(parent);
    ensure_path_under_root(&existing, root)
}

fn nearest_existing_ancestor(path: &Path) -> PathBuf {
    let mut current = path.to_path_buf();
    loop {
        if current.exists() {
            return current;
        }
        if !current.pop() {
            return PathBuf::from(".");
        }
    }
}

/// Ensure an existing path canonicalizes under the expected virtual root.
pub(crate) fn ensure_path_under_root(path: &Path, root: &Path) -> Result<()> {
    let canonical_root = std::fs::canonicalize(root)
        .map_err(|e| anyhow!("resource_ref: virtual root mapping {root:?} unavailable: {e}"))?;
    let canonical_path = std::fs::canonicalize(path)
        .map_err(|e| anyhow!("resource_ref: path {path:?} cannot be canonicalized: {e}"))?;
    if !canonical_path.starts_with(&canonical_root) {
        return Err(anyhow!(
            "resource_ref: path {canonical_path:?} escapes virtual root {canonical_root:?}"
        ));
    }
    Ok(())
}

/// JSON schema fragment shared by filesystem ability input schemas.
pub(crate) fn resource_ref_schema() -> Value {
    json!({
        "type": "object",
        "required": [
            "resource_ura",
            "owner_ura",
            "namespace",
            "capability",
            "expires_unix_ms",
            "revision"
        ],
        "additionalProperties": false,
        "properties": {
            "resource_ura": { "type": "string", "minLength": 1 },
            "owner_ura": { "type": "string", "minLength": 1 },
            "namespace": { "type": "string", "enum": ["fs"] },
            "display_path": { "type": "string" },
            "capability": {
                "type": "string",
                "enum": ["list", "stat", "read", "write"]
            },
            "expires_unix_ms": { "type": "integer" },
            "revision": { "type": "string", "minLength": 1 }
        }
    })
}

fn now_unix_ms() -> i64 {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}

#[cfg(test)]
pub(crate) fn resource_ref_for_tmp_relative_path(
    relative_path: &str,
    capability: FilesystemResourceCapability,
    expires_unix_ms: i64,
) -> Value {
    resource_ref_value(VIRTUAL_ROOT_TMP, relative_path, capability, expires_unix_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uuid_suffix() -> String {
        uuid::Uuid::new_v4().simple().to_string()
    }

    fn unique_resource_rel(file_name: &str) -> String {
        format!("easynet-resource-ref-test-{}/{}", uuid_suffix(), file_name)
    }

    fn expires_in_ms(delta_ms: i64) -> i64 {
        now_unix_ms().saturating_add(delta_ms)
    }

    fn resource_ref(
        relative_path: &str,
        capability: FilesystemResourceCapability,
        expires_unix_ms: i64,
    ) -> Value {
        resource_ref_for_tmp_relative_path(relative_path, capability, expires_unix_ms)
    }

    #[test]
    fn resource_ref_for_local_path_rejects_device_paths_outside_virtual_roots() {
        if !Path::new("/dev/zero").exists() {
            return;
        }
        let err =
            resource_ref_for_local_path(Path::new("/dev/zero"), FilesystemResourceCapability::Read)
                .unwrap_err();
        assert!(err
            .to_string()
            .contains("outside built-in filesystem virtual roots"));
    }

    #[test]
    fn read_capability_permits_stat() {
        let rel = unique_resource_rel("stat.txt");
        let local = std::env::temp_dir().join(&rel);
        std::fs::create_dir_all(local.parent().unwrap()).unwrap();
        std::fs::write(&local, "metadata").unwrap();

        let resolved = resolve_filesystem_path(
            &json!({
                "resource_ref": resource_ref(
                    &rel,
                    FilesystemResourceCapability::Read,
                    expires_in_ms(60_000)
                ),
            }),
            FilesystemResourceCapability::Stat,
        )
        .unwrap();

        assert_eq!(resolved.local_path, std::fs::canonicalize(&local).unwrap());
        std::fs::remove_file(&local).ok();
        std::fs::remove_dir_all(local.parent().unwrap()).ok();
    }

    #[test]
    fn write_capability_resolves_missing_target_under_virtual_root() {
        let rel = unique_resource_rel("write.txt");
        let local = std::env::temp_dir().join(&rel);

        let resolved = resolve_filesystem_path(
            &json!({
                "resource_ref": resource_ref(
                    &rel,
                    FilesystemResourceCapability::Write,
                    expires_in_ms(60_000)
                ),
            }),
            FilesystemResourceCapability::Write,
        )
        .unwrap();

        assert_eq!(resolved.local_path, local);
        assert_eq!(resolved.display_path, format!("tmp/{rel}"));
        std::fs::remove_dir_all(local.parent().unwrap()).ok();
    }

    #[test]
    fn expired_reference_rejects_before_path_access() {
        let rel = unique_resource_rel("expired.txt");
        let err = resolve_filesystem_path(
            &json!({
                "resource_ref": resource_ref(&rel, FilesystemResourceCapability::Read, 1),
            }),
            FilesystemResourceCapability::Read,
        )
        .unwrap_err();

        assert!(err.to_string().contains("expired"));
    }

    #[test]
    fn missing_resource_ref_is_rejected() {
        let err =
            resolve_filesystem_path(&json!({}), FilesystemResourceCapability::Read).unwrap_err();
        assert!(err
            .to_string()
            .contains("resource_ref: missing required object"));
    }

    #[test]
    fn revision_mismatch_rejects() {
        let rel = unique_resource_rel("stale-revision.txt");
        let mut reference = resource_ref(
            &rel,
            FilesystemResourceCapability::Read,
            expires_in_ms(60_000),
        );
        reference["revision"] = json!("stale");

        let err = resolve_filesystem_path(
            &json!({ "resource_ref": reference }),
            FilesystemResourceCapability::Read,
        )
        .unwrap_err();

        assert!(err.to_string().contains("revision mismatch"));
    }

    #[test]
    fn insufficient_capability_rejects() {
        let rel = unique_resource_rel("capability.txt");
        let err = resolve_filesystem_path(
            &json!({
                "resource_ref": resource_ref(
                    &rel,
                    FilesystemResourceCapability::Read,
                    expires_in_ms(60_000)
                ),
            }),
            FilesystemResourceCapability::Write,
        )
        .unwrap_err();

        assert!(err.to_string().contains("does not permit write"));
    }

    #[test]
    fn traversal_relative_path_rejects() {
        let err = resolve_filesystem_path(
            &json!({
                "resource_ref": resource_ref(
                    "../escape.txt",
                    FilesystemResourceCapability::Read,
                    expires_in_ms(60_000)
                ),
            }),
            FilesystemResourceCapability::Read,
        )
        .unwrap_err();

        assert!(err.to_string().contains("traversal"));
    }

    #[test]
    fn unmapped_virtual_root_rejects() {
        // Mint a ResourceRef the normal way, then re-point its resource_ura at an
        // UNMAPPED virtual root (`vault`) keyed on the SAME minted identity so
        // the owner-consistency check passes and we exercise the virtual-root
        // rejection, not an owner mismatch. We parse the Resource URA instead
        // of splitting by `/`, because URA grammar belongs to Axon and tests
        // must not duplicate segment math that can drift under new subjects.
        let mut reference = resource_ref(
            "file.txt",
            FilesystemResourceCapability::Read,
            expires_in_ms(60_000),
        );
        let minted = reference["resource_ura"]
            .as_str()
            .expect("minted resource_ura is a string")
            .to_string();
        let parsed = crate::core::ura::parse_ura(&minted).expect("minted resource_ura parses");
        let owner_token = parsed
            .resource_owner_id()
            .expect("resource_ura carries an owner token");
        reference["resource_ura"] = json!(crate::core::ura::resource_dot_ura(
            &parsed.realm,
            owner_token,
            "fs/vault/file.txt"
        ));

        let err = resolve_filesystem_path(
            &json!({ "resource_ref": reference }),
            FilesystemResourceCapability::Read,
        )
        .unwrap_err();

        assert!(
            err.to_string().contains("no local mapping"),
            "expected unmapped virtual-root rejection, got: {err}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlink_escape_rejects_for_read_like_resolution() {
        let outside = std::env::current_dir()
            .unwrap()
            .join(format!(".easynet-fs-outside-{}", uuid_suffix()));
        std::fs::create_dir_all(&outside).unwrap();
        let outside_file = outside.join("outside.txt");
        std::fs::write(&outside_file, "outside").unwrap();

        let rel_dir = unique_resource_rel("links");
        let local_dir = std::env::temp_dir().join(&rel_dir);
        std::fs::create_dir_all(&local_dir).unwrap();
        let link = local_dir.join("escape.txt");
        std::os::unix::fs::symlink(&outside_file, &link).unwrap();
        let rel = format!("{rel_dir}/escape.txt");

        let err = resolve_filesystem_path(
            &json!({
                "resource_ref": resource_ref(
                    &rel,
                    FilesystemResourceCapability::Read,
                    expires_in_ms(60_000)
                ),
            }),
            FilesystemResourceCapability::Read,
        )
        .unwrap_err();

        assert!(err.to_string().contains("escapes virtual root"));
        std::fs::remove_dir_all(&local_dir).ok();
        std::fs::remove_dir_all(&outside).ok();
    }

    #[test]
    fn schema_is_resource_ref_only() {
        let schema = resource_ref_schema();
        assert_eq!(schema["type"], json!("object"));
        assert!(schema["properties"].get("path").is_none());
        assert!(schema["required"]
            .as_array()
            .unwrap()
            .contains(&json!("resource_ura")));
    }
}
