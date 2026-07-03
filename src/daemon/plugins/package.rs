// EasyNet CLI — plugin package identity
// =====================================
//
// File: src/daemon/plugins/package.rs
// Description: Package ids, versions, hashes, and compiled builtin bindings.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::daemon::plugins::errors::{PluginHostError, Result};
use crate::daemon::plugins::manifest::{
    validate_builtin_entrypoint, PluginAbilityLayer, PluginBidiWireKind, PluginCallMode,
    PluginPackageManifest, PluginRuntimeLimits,
};

/// Stable plugin package identifier.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PackageId(String);

impl PackageId {
    /// Construct a package id from a validated manifest id.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Borrow the id string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Plugin package version string.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PackageVersion(String);

impl PackageVersion {
    /// Construct a package version from a validated manifest version.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Borrow the version string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// SHA-256 hash over the installable package surface.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageHash(String);

impl PackageHash {
    /// Borrow the lowercase hex digest.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Compile-time metadata for one builtin plugin ability.
///
/// This is the single runtime-side source for a builtin plugin ability's public
/// name, product layer, bidi wire profile, description, and input schema. The
/// package manifest is validated against this table at index time; generated
/// descriptor TOMLs are projections of this table, not independent facts.
#[derive(Clone, Copy, Debug)]
pub struct BuiltinPluginAbilitySpec {
    pub name: &'static str,
    pub layer: PluginAbilityLayer,
    pub call_mode: PluginCallMode,
    pub bidi_wire_kind: Option<PluginBidiWireKind>,
    pub description: fn() -> &'static str,
    pub input_schema: fn() -> Value,
}

impl BuiltinPluginAbilitySpec {
    /// Project this compiled builtin plugin spec into the daemon registry
    /// manifest shape used by `meta.list_abilities`.
    ///
    /// Builtin plugin ability names are full daemon names
    /// (`remote_desktop.create_session`), while `AbilityManifest` names are
    /// verb-local. The catalog key remains the full ability name at
    /// registration; only the manifest body stores the local verb.
    pub fn to_registry_manifest(&self) -> Result<crate::core::ability::spec::AbilityManifest> {
        let verb = self.name.rsplit('.').next().ok_or_else(|| {
            PluginHostError::DescriptorProjectionFailed {
                ability: self.name.to_string(),
                reason: "ability name has no verb segment".to_string(),
            }
        })?;
        crate::core::ability::spec::AbilityManifest::new(
            verb,
            (self.description)(),
            (self.input_schema)(),
        )
        .map_err(|source| PluginHostError::DescriptorProjectionFailed {
            ability: self.name.to_string(),
            reason: source.to_string(),
        })
    }
}

/// Descriptor metadata loaded from the package ability descriptor surface.
///
/// What this is NOT: a handler binding. It is the discovery/schema projection
/// for one ability; registration still comes from builtin Rust, declarative
/// bindings, or sidecar process commands.
#[derive(Clone, Debug, PartialEq)]
pub struct PluginAbilityDescriptor {
    name: String,
    description: String,
    input_schema: Value,
    output_schema: Option<Value>,
}

impl PluginAbilityDescriptor {
    /// Full daemon ability name declared by the descriptor.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Human-readable ability description.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// JSON schema for invocation arguments.
    pub fn input_schema(&self) -> &Value {
        &self.input_schema
    }

    /// Optional JSON schema for the result body.
    pub fn output_schema(&self) -> Option<&Value> {
        self.output_schema.as_ref()
    }

    /// Project this plugin descriptor into the daemon registry manifest shape.
    ///
    /// What this is NOT: the package truth. The descriptor remains the package
    /// discovery source; `AbilityManifest` is only the registry projection used
    /// by `meta.list_abilities` and UI schema rendering. `AbilityManifest`
    /// names are verb-local, so plugin ability names such as
    /// `plugin.echo` project to `echo` while the catalog key remains the
    /// full ability name.
    pub fn to_registry_manifest(&self) -> Result<crate::core::ability::spec::AbilityManifest> {
        let verb = self.name.rsplit('.').next().ok_or_else(|| {
            PluginHostError::DescriptorProjectionFailed {
                ability: self.name.clone(),
                reason: "ability name has no verb segment".to_string(),
            }
        })?;
        let mut manifest = crate::core::ability::spec::AbilityManifest::new(
            verb,
            self.description.clone(),
            self.input_schema.clone(),
        )
        .map_err(|source| PluginHostError::DescriptorProjectionFailed {
            ability: self.name.clone(),
            reason: source.to_string(),
        })?;
        if let Some(output_schema) = &self.output_schema {
            manifest = manifest
                .with_output_schema(output_schema.clone())
                .map_err(|source| PluginHostError::DescriptorProjectionFailed {
                    ability: self.name.clone(),
                    reason: source.to_string(),
                })?;
        }
        Ok(manifest)
    }
}

/// Compile-time binding between a parsed builtin manifest and executable Rust.
///
/// What this is NOT: package metadata. If a field can live in `plugin.toml`, it
/// belongs in [`PluginPackageManifest`] and is reached through the package.
#[derive(Clone, Copy, Debug)]
pub struct BuiltinPluginBinding {
    pub manifest_path: &'static str,
    pub manifest_body: &'static str,
    pub expected_entrypoint: &'static str,
    pub enabled_env_var: Option<&'static str>,
    pub ability_specs: fn() -> Vec<BuiltinPluginAbilitySpec>,
    pub contribute: fn(
        &mut crate::daemon::plugins::contribution::PluginContributionBuilder,
        PluginRuntimeLimits,
    ) -> Result<()>,
}

/// Source class for a package in the package index.
#[derive(Clone, Debug)]
pub enum PluginPackageSource {
    Builtin(BuiltinPluginBinding),
    Installed,
}

/// Indexed plugin package.
#[derive(Clone, Debug)]
pub struct PluginPackage {
    root: PathBuf,
    manifest_path: PathBuf,
    manifest: PluginPackageManifest,
    descriptors: BTreeMap<String, Arc<PluginAbilityDescriptor>>,
    hash: PackageHash,
    source: PluginPackageSource,
}

impl PluginPackage {
    /// Build one builtin package from a compiled binding.
    pub fn from_builtin(binding: BuiltinPluginBinding) -> Result<Self> {
        let manifest = PluginPackageManifest::parse(binding.manifest_path, binding.manifest_body)?;
        validate_builtin_entrypoint(&manifest, binding.expected_entrypoint)?;
        let specs = (binding.ability_specs)();
        validate_builtin_specs(&manifest, &specs)?;
        let descriptors = builtin_descriptors(&manifest, &specs)?;
        let root = Path::new(binding.manifest_path)
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let hash = hash_installable_surface(&root)?;
        Ok(Self {
            root,
            manifest_path: PathBuf::from(binding.manifest_path),
            manifest,
            descriptors,
            hash,
            source: PluginPackageSource::Builtin(binding),
        })
    }

    /// Build one installed package from an on-disk package root.
    pub fn from_installed(root: &Path, expected_hash: Option<&str>) -> Result<Self> {
        let manifest_path = root.join("plugin.toml");
        let body = std::fs::read_to_string(&manifest_path).map_err(|source| {
            PluginHostError::ReadFailed {
                path: manifest_path.clone(),
                source,
            }
        })?;
        let manifest = PluginPackageManifest::parse(&manifest_path.display().to_string(), &body)?;
        let descriptors = installed_descriptors(root, &manifest)?;
        let hash = hash_installable_surface(root)?;
        if let Some(expected) = expected_hash {
            if expected != hash.as_str() {
                return Err(PluginHostError::HashMismatch {
                    id: manifest.id().to_string(),
                    expected: expected.to_string(),
                    actual: hash.as_str().to_string(),
                });
            }
        }
        Ok(Self {
            root: root.to_path_buf(),
            manifest_path,
            manifest,
            descriptors,
            hash,
            source: PluginPackageSource::Installed,
        })
    }

    /// Package id.
    pub fn id(&self) -> PackageId {
        PackageId::new(self.manifest.id())
    }

    /// Package version.
    pub fn version(&self) -> PackageVersion {
        PackageVersion::new(self.manifest.version())
    }

    /// Package manifest.
    pub fn manifest(&self) -> &PluginPackageManifest {
        &self.manifest
    }

    /// Descriptor metadata for one package-owned ability.
    pub fn ability_descriptor(&self, ability: &str) -> Option<Arc<PluginAbilityDescriptor>> {
        self.descriptors.get(ability).map(Arc::clone)
    }

    /// Project one package-owned ability descriptor into the registry manifest
    /// cache shape.
    pub fn ability_registry_manifest(
        &self,
        ability: &str,
    ) -> Result<crate::core::ability::spec::AbilityManifest> {
        let descriptor = self.ability_descriptor(ability).ok_or_else(|| {
            PluginHostError::InvalidAbilityDescriptor {
                path: self.manifest_path.clone(),
                reason: format!("ability descriptor {ability:?} is not indexed"),
            }
        })?;
        descriptor.to_registry_manifest()
    }

    /// Descriptor metadata for every package-owned ability.
    pub fn ability_descriptors(&self) -> impl Iterator<Item = Arc<PluginAbilityDescriptor>> + '_ {
        self.descriptors.values().map(Arc::clone)
    }

    /// Package root directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Sidecar entrypoint resolved against this package root.
    pub fn entrypoint_path(&self) -> PathBuf {
        let entrypoint = Path::new(self.manifest.entrypoint());
        if entrypoint.is_absolute() {
            entrypoint.to_path_buf()
        } else {
            self.root.join(entrypoint)
        }
    }

    /// Manifest path.
    pub fn manifest_path(&self) -> &Path {
        &self.manifest_path
    }

    /// Installable-surface hash.
    pub fn hash(&self) -> &PackageHash {
        &self.hash
    }

    /// Compiled builtin binding, when this package is builtin.
    pub fn builtin_binding(&self) -> Option<BuiltinPluginBinding> {
        match self.source {
            PluginPackageSource::Builtin(binding) => Some(binding),
            PluginPackageSource::Installed => None,
        }
    }
}

/// Shared package handle.
pub type SharedPluginPackage = Arc<PluginPackage>;

#[derive(Debug, Deserialize)]
struct RawPluginAbilityDescriptor {
    #[serde(default)]
    schema_version: Option<String>,
    name: String,
    description: String,
    input_schema: Value,
    #[serde(default)]
    output_schema: Option<Value>,
}

fn builtin_descriptors(
    manifest: &PluginPackageManifest,
    specs: &[BuiltinPluginAbilitySpec],
) -> Result<BTreeMap<String, Arc<PluginAbilityDescriptor>>> {
    let mut out = BTreeMap::new();
    for spec in specs {
        if manifest.ability(spec.name).is_none() {
            return Err(PluginHostError::BuiltinSpecMismatch {
                id: manifest.id().to_string(),
                reason: format!("compiled descriptor {:?} has no manifest row", spec.name),
            });
        }
        out.insert(
            spec.name.to_string(),
            Arc::new(PluginAbilityDescriptor {
                name: spec.name.to_string(),
                description: (spec.description)().to_string(),
                input_schema: (spec.input_schema)(),
                output_schema: None,
            }),
        );
    }
    Ok(out)
}

fn installed_descriptors(
    root: &Path,
    manifest: &PluginPackageManifest,
) -> Result<BTreeMap<String, Arc<PluginAbilityDescriptor>>> {
    let mut out = BTreeMap::new();
    let ability_root = root.join("abilities");
    for ability in manifest.abilities() {
        let path = PathBuf::from(ability.descriptor_path());
        validate_package_child_path(root, &ability_root, &path)?;
        let body =
            std::fs::read_to_string(&path).map_err(|source| PluginHostError::ReadFailed {
                path: path.clone(),
                source,
            })?;
        let raw: RawPluginAbilityDescriptor =
            toml::from_str(&body).map_err(|source| PluginHostError::DescriptorParseFailed {
                path: path.clone(),
                source,
            })?;
        let descriptor = validate_descriptor(&path, ability.name(), raw)?;
        if out
            .insert(ability.name().to_string(), Arc::new(descriptor))
            .is_some()
        {
            return Err(PluginHostError::DuplicateAbility(
                ability.name().to_string(),
            ));
        }
    }
    Ok(out)
}

fn validate_descriptor(
    path: &Path,
    expected_name: &str,
    raw: RawPluginAbilityDescriptor,
) -> Result<PluginAbilityDescriptor> {
    if raw
        .schema_version
        .as_deref()
        .is_some_and(|version| version != "1")
    {
        return Err(PluginHostError::InvalidAbilityDescriptor {
            path: path.to_path_buf(),
            reason: format!(
                "unsupported schema_version {:?}",
                raw.schema_version.unwrap_or_default()
            ),
        });
    }
    if raw.name != expected_name {
        return Err(PluginHostError::InvalidAbilityDescriptor {
            path: path.to_path_buf(),
            reason: format!(
                "descriptor name {:?} does not match plugin.toml ability {:?}",
                raw.name, expected_name
            ),
        });
    }
    if raw.description.trim().is_empty() {
        return Err(PluginHostError::InvalidAbilityDescriptor {
            path: path.to_path_buf(),
            reason: "description must be non-empty".to_string(),
        });
    }
    if !raw.input_schema.is_object() {
        return Err(PluginHostError::InvalidAbilityDescriptor {
            path: path.to_path_buf(),
            reason: "input_schema must be a JSON object".to_string(),
        });
    }
    if raw
        .output_schema
        .as_ref()
        .is_some_and(|schema| !schema.is_object())
    {
        return Err(PluginHostError::InvalidAbilityDescriptor {
            path: path.to_path_buf(),
            reason: "output_schema, when present, must be a JSON object".to_string(),
        });
    }
    Ok(PluginAbilityDescriptor {
        name: raw.name,
        description: raw.description,
        input_schema: raw.input_schema,
        output_schema: raw.output_schema,
    })
}

fn validate_builtin_specs(
    manifest: &PluginPackageManifest,
    specs: &[BuiltinPluginAbilitySpec],
) -> Result<()> {
    if manifest.abilities().len() != specs.len() {
        return Err(PluginHostError::BuiltinSpecMismatch {
            id: manifest.id().to_string(),
            reason: format!(
                "manifest declares {} abilities but compiled binding declares {}",
                manifest.abilities().len(),
                specs.len()
            ),
        });
    }

    let mut spec_by_name = std::collections::BTreeMap::new();
    for spec in specs {
        if spec_by_name.insert(spec.name, *spec).is_some() {
            return Err(PluginHostError::BuiltinSpecMismatch {
                id: manifest.id().to_string(),
                reason: format!(
                    "compiled binding declares duplicate ability {:?}",
                    spec.name
                ),
            });
        }
    }

    for ability in manifest.abilities() {
        let Some(spec) = spec_by_name.get(ability.name()) else {
            return Err(PluginHostError::BuiltinSpecMismatch {
                id: manifest.id().to_string(),
                reason: format!("manifest ability {:?} has no compiled spec", ability.name()),
            });
        };
        if ability.layer() != spec.layer {
            return Err(PluginHostError::BuiltinSpecMismatch {
                id: manifest.id().to_string(),
                reason: format!(
                    "ability {:?} layer mismatch: manifest={:?}, compiled={:?}",
                    ability.name(),
                    ability.layer(),
                    spec.layer
                ),
            });
        }
        if ability.call_mode() != spec.call_mode {
            return Err(PluginHostError::BuiltinSpecMismatch {
                id: manifest.id().to_string(),
                reason: format!(
                    "ability {:?} call mode mismatch: manifest={:?}, compiled={:?}",
                    ability.name(),
                    ability.call_mode(),
                    spec.call_mode
                ),
            });
        }
        if ability.bidi_wire_kind() != spec.bidi_wire_kind {
            return Err(PluginHostError::BuiltinSpecMismatch {
                id: manifest.id().to_string(),
                reason: format!(
                    "ability {:?} bidi wire mismatch: manifest={:?}, compiled={:?}",
                    ability.name(),
                    ability.bidi_wire_kind(),
                    spec.bidi_wire_kind
                ),
            });
        }
    }
    Ok(())
}

/// Compute SHA-256 over `plugin.toml`, `abilities/`, and `bin/`.
pub fn hash_installable_surface(root: &Path) -> Result<PackageHash> {
    let mut files = Vec::new();
    collect_existing_files(root, Path::new("plugin.toml"), &mut files)?;
    collect_existing_files(root, Path::new("abilities"), &mut files)?;
    collect_existing_files(root, Path::new("bin"), &mut files)?;
    files.sort();

    let mut hasher = Sha256::new();
    for rel in files {
        let path = root.join(&rel);
        let meta = std::fs::metadata(&path).map_err(|source| PluginHostError::ReadFailed {
            path: path.clone(),
            source,
        })?;
        let body = std::fs::read(&path).map_err(|source| PluginHostError::ReadFailed {
            path: path.clone(),
            source,
        })?;
        hasher.update(rel.to_string_lossy().as_bytes());
        hasher.update([0]);
        hasher.update(file_hash_metadata(&meta).as_bytes());
        hasher.update([0]);
        hasher.update(body);
        hasher.update([0]);
    }
    Ok(PackageHash(format!("{:x}", hasher.finalize())))
}

fn file_hash_metadata(meta: &std::fs::Metadata) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        format!("file:{:o}", meta.permissions().mode() & 0o777)
    }
    #[cfg(not(unix))]
    {
        let readonly = meta.permissions().readonly();
        format!("file:readonly={readonly}")
    }
}

fn collect_existing_files(root: &Path, rel: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let path = root.join(rel);
    if !path.exists() {
        return Ok(());
    }
    validate_package_child_path(root, root, &path)?;
    let meta = std::fs::metadata(&path).map_err(|source| PluginHostError::ReadFailed {
        path: path.clone(),
        source,
    })?;
    if meta.is_file() {
        out.push(rel.to_path_buf());
        return Ok(());
    }
    for entry in std::fs::read_dir(&path).map_err(|source| PluginHostError::ReadFailed {
        path: path.clone(),
        source,
    })? {
        let entry = entry.map_err(|source| PluginHostError::ReadFailed {
            path: path.clone(),
            source,
        })?;
        let name = entry.file_name();
        collect_existing_files(root, &rel.join(name), out)?;
    }
    Ok(())
}

fn validate_package_child_path(root: &Path, allowed_root: &Path, path: &Path) -> Result<()> {
    let canonical_root =
        std::fs::canonicalize(root).map_err(|source| PluginHostError::ReadFailed {
            path: root.to_path_buf(),
            source,
        })?;
    let canonical_allowed =
        std::fs::canonicalize(allowed_root).map_err(|source| PluginHostError::ReadFailed {
            path: allowed_root.to_path_buf(),
            source,
        })?;
    if !canonical_allowed.starts_with(&canonical_root) {
        return Err(PluginHostError::PackagePathEscapesRoot {
            root: canonical_root,
            path: canonical_allowed,
        });
    }
    let canonical_path =
        std::fs::canonicalize(path).map_err(|source| PluginHostError::ReadFailed {
            path: path.to_path_buf(),
            source,
        })?;
    if canonical_path.starts_with(&canonical_allowed) {
        Ok(())
    } else {
        Err(PluginHostError::PackagePathEscapesRoot {
            root: canonical_root,
            path: canonical_path,
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::daemon::plugins::manifest::{PluginAbilityLayer, PluginCallMode};

    #[test]
    fn plugin_host_package_rejects_hash_mismatch() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_test_package(dir.path(), "0.1.0");
        let err = match PluginPackage::from_installed(dir.path(), Some("bad-hash")) {
            Ok(_) => panic!("hash mismatch must fail"),
            Err(err) => err,
        };
        assert!(matches!(err, PluginHostError::HashMismatch { .. }));
    }

    #[test]
    fn plugin_host_builtin_package_rejects_manifest_spec_drift() {
        fn description() -> &'static str {
            "test plugin ability"
        }
        fn input_schema() -> Value {
            serde_json::json!({"type": "object", "additionalProperties": false})
        }
        fn contribute(
            _: &mut crate::daemon::plugins::PluginContributionBuilder,
            _: PluginRuntimeLimits,
        ) -> Result<()> {
            Ok(())
        }
        fn ability_specs() -> Vec<BuiltinPluginAbilitySpec> {
            vec![BuiltinPluginAbilitySpec {
                name: "test.echo",
                layer: PluginAbilityLayer::Observation,
                call_mode: PluginCallMode::Rpc,
                bidi_wire_kind: None,
                description,
                input_schema,
            }]
        }

        let manifest = r#"
schema_version = "1"
id = "test.plugin"
version = "0.1.0"
kind = "builtin"
entrypoint = "test::register"
abilities = ["abilities/*.ability.toml"]
permissions = []
resources = []
platforms = []

[limits]
max_sessions = 1
max_frame_queue = 1

[[ability_metadata]]
name = "test.echo"
layer = "control"
"#;

        let err = match PluginPackage::from_builtin(BuiltinPluginBinding {
            manifest_path: "plugins/test/plugin.toml",
            manifest_body: manifest,
            expected_entrypoint: "test::register",
            enabled_env_var: None,
            ability_specs,
            contribute,
        }) {
            Ok(_) => panic!("manifest layer must match compiled spec"),
            Err(err) => err,
        };

        assert!(matches!(err, PluginHostError::BuiltinSpecMismatch { .. }));
    }

    #[test]
    fn plugin_host_installed_package_rejects_descriptor_name_drift() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_test_package(dir.path(), "0.1.0");
        std::fs::write(
            dir.path().join("abilities/test.echo.ability.toml"),
            test_descriptor("device.test.other"),
        )
        .expect("drifted descriptor");

        let err = PluginPackage::from_installed(dir.path(), None)
            .expect_err("descriptor name drift must fail package indexing");
        assert!(matches!(
            err,
            PluginHostError::InvalidAbilityDescriptor { .. }
        ));
    }

    #[test]
    fn plugin_host_installed_package_rejects_descriptor_escape_from_package_root() {
        let parent = tempfile::tempdir().expect("tempdir");
        let package_root = parent.path().join("pkg");
        let outside = parent.path().join("outside");
        std::fs::create_dir_all(package_root.join("abilities")).expect("package abilities dir");
        std::fs::create_dir_all(&outside).expect("outside dir");
        std::fs::write(
            package_root.join("plugin.toml"),
            r#"
schema_version = "1"
id = "test.plugin"
version = "0.1.0"
kind = "declarative"
entrypoint = "sidecar"
abilities = ["../outside/*.ability.toml"]
permissions = []
resources = []
platforms = []

[limits]
max_sessions = 1
max_frame_queue = 1

[[ability_metadata]]
name = "test.echo"
layer = "control"
"#,
        )
        .expect("manifest");
        std::fs::write(
            outside.join("test.echo.ability.toml"),
            test_descriptor("test.echo"),
        )
        .expect("escaped descriptor");

        let err = PluginPackage::from_installed(&package_root, None)
            .expect_err("descriptor escape must fail package indexing");
        assert!(matches!(
            err,
            PluginHostError::PackagePathEscapesRoot { .. }
        ));
    }

    pub(crate) fn write_test_package(root: &Path, version: &str) {
        std::fs::create_dir_all(root.join("abilities")).expect("abilities dir");
        std::fs::write(
            root.join("plugin.toml"),
            format!(
                r#"
schema_version = "1"
id = "test.plugin"
version = "{version}"
kind = "declarative"
entrypoint = "sidecar"
abilities = ["abilities/*.ability.toml"]
permissions = []
resources = []
platforms = []

[limits]
max_sessions = 1
max_frame_queue = 1

[[ability_metadata]]
name = "test.echo"
layer = "control"
"#
            ),
        )
        .expect("manifest");
        std::fs::write(
            root.join("abilities/test.echo.ability.toml"),
            test_descriptor("test.echo"),
        )
        .expect("descriptor");
    }

    pub(crate) fn test_descriptor(ability: &str) -> String {
        format!(
            r#"schema_version = "1"
name = "{ability}"
description = "test descriptor for {ability}"

[input_schema]
type = "object"
additionalProperties = false
"#
        )
    }
}
