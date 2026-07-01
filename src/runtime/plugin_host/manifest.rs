// EasyNet CLI — plugin manifest model
// ===================================
//
// File: src/runtime/plugin_host/manifest.rs
// Description: Typed `plugin.toml` package model and validation.

use serde::{Deserialize, Serialize};

use crate::runtime::plugin_host::errors::{PluginHostError, Result};

/// Axon invocation mode required by one plugin-owned ability.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PluginCallMode {
    /// Unary invoke: one JSON argument object, one JSON result object.
    Rpc,
    /// Server-stream invoke: one JSON argument object, many JSON result frames.
    ///
    /// Sidecar v1 implements this as a finite snapshot stream collected until a
    /// terminal frame. Long-running live sidecar transports must declare `bidi`
    /// so the daemon can own cancellation, backpressure, and a single terminal
    /// close path.
    Stream,
    /// Bidirectional invoke: both sides exchange frames until one terminal close.
    Bidi,
}

/// Wire adapter a bidi plugin ability expects when it crosses the
/// `session.open` bridge.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PluginBidiWireKind {
    /// Ability input/output frames are JSON control frames.
    JsonFrames,
}

/// Realtime device resource family declared by a plugin package.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PluginRealtimeKind {
    Camera,
    Mic,
    Screen,
    Speaker,
    Voice,
}

/// High-level realtime operation a plugin wants the daemon/UI to expose.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum PluginRealtimeMode {
    Snapshot,
    Subscribe,
    Record,
    Publish,
    Transcribe,
}

/// Preferred data-plane carrier for realtime plugin traffic.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PluginRealtimeTransport {
    InvokeStream,
    InvokeBidi,
    Webrtc,
}

/// Package-level realtime capability declaration.
///
/// This is activation metadata, not an AbilityDescriptor replacement. Concrete
/// callable names still live in `[[ability_metadata]]`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct PluginRealtimeCapability {
    kind: PluginRealtimeKind,
    modes: Vec<PluginRealtimeMode>,
    transport: PluginRealtimeTransport,
    #[serde(default)]
    fallback_transport: Option<PluginRealtimeTransport>,
    #[serde(default)]
    activation_abilities: Vec<String>,
    #[serde(default)]
    permissions: Vec<String>,
    #[serde(default)]
    resources: Vec<String>,
    #[serde(default)]
    quick_add: bool,
}

impl PluginRealtimeCapability {
    pub const fn kind(&self) -> PluginRealtimeKind {
        self.kind
    }

    pub fn modes(&self) -> &[PluginRealtimeMode] {
        &self.modes
    }

    pub const fn transport(&self) -> PluginRealtimeTransport {
        self.transport
    }

    pub const fn fallback_transport(&self) -> Option<PluginRealtimeTransport> {
        self.fallback_transport
    }

    pub fn activation_abilities(&self) -> &[String] {
        &self.activation_abilities
    }

    pub fn permissions(&self) -> &[String] {
        &self.permissions
    }

    pub fn resources(&self) -> &[String] {
        &self.resources
    }

    pub const fn quick_add(&self) -> bool {
        self.quick_add
    }
}

/// Product/runtime layer declared by a plugin-owned ability.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PluginAbilityLayer {
    Introspection,
    Control,
    Observation,
    Operational,
}

/// First-version plugin execution model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PluginKind {
    Declarative,
    Sidecar,
    Builtin,
}

impl<'de> Deserialize<'de> for PluginKind {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        match raw.as_str() {
            "declarative" => Ok(Self::Declarative),
            "sidecar" => Ok(Self::Sidecar),
            "builtin" | "stateful-device-plugin" => Ok(Self::Builtin),
            other => Err(serde::de::Error::custom(format!(
                "unsupported plugin kind {other:?}"
            ))),
        }
    }
}

/// Declarative plugin execution binding.
///
/// The package manifest is the source of truth. This enum deliberately models
/// every first-version declarative binding. `Eal` and `Mcp` intentionally
/// reuse the daemon's existing in-process executors instead of creating a
/// plugin-specific orchestration or MCP call path.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PluginDeclarativeBinding {
    /// Spawn an executable declared by argv[0]. The process speaks the same
    /// JSON frame envelope as sidecar packages.
    Exec { argv: Vec<String> },
    /// Run an embedded EAL program through the canonical EAL executor.
    Eal {
        program: String,
        #[serde(default)]
        result_binding: Option<String>,
    },
    /// Call one configured upstream MCP tool through the daemon MCP client.
    Mcp { server: String, tool: String },
}

impl PluginDeclarativeBinding {
    /// Return argv for an exec binding.
    pub fn exec_argv(&self) -> Option<&[String]> {
        match self {
            Self::Exec { argv } => Some(argv),
            Self::Eal { .. } | Self::Mcp { .. } => None,
        }
    }

    /// Whether this declarative binding is executable in this release.
    pub const fn loadable_in_this_release(&self) -> bool {
        matches!(
            self,
            Self::Exec { .. } | Self::Eal { .. } | Self::Mcp { .. }
        )
    }
}

/// Runtime limits declared by a plugin package manifest.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
pub struct PluginRuntimeLimits {
    max_sessions: usize,
    max_frame_queue: usize,
}

impl PluginRuntimeLimits {
    /// Construct host runtime limits for tests and builtin bindings.
    pub const fn new(max_sessions: usize, max_frame_queue: usize) -> Self {
        Self {
            max_sessions,
            max_frame_queue,
        }
    }

    /// Maximum concurrent sessions the package may keep in memory.
    pub const fn max_sessions(&self) -> usize {
        self.max_sessions
    }

    /// Maximum media frame queue depth declared by the package.
    pub const fn max_frame_queue(&self) -> usize {
        self.max_frame_queue
    }
}

/// One plugin-owned ability exported by a package.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginAbilityManifest {
    name: String,
    descriptor_path: String,
    layer: PluginAbilityLayer,
    call_mode: PluginCallMode,
    bidi_wire_kind: Option<PluginBidiWireKind>,
}

impl PluginAbilityManifest {
    /// Stable ability name exported through the daemon catalog.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Canonical descriptor TOML path for this ability.
    pub fn descriptor_path(&self) -> &str {
        &self.descriptor_path
    }

    /// Product/runtime layer used by ability completeness tests.
    pub const fn layer(&self) -> PluginAbilityLayer {
        self.layer
    }

    /// Invocation mode callers must use for this ability.
    pub const fn call_mode(&self) -> PluginCallMode {
        self.call_mode
    }

    /// Optional bidi wire profile declared by the plugin package.
    pub const fn bidi_wire_kind(&self) -> Option<PluginBidiWireKind> {
        self.bidi_wire_kind
    }
}

/// Parsed package manifest for one plugin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginPackageManifest {
    schema_version: String,
    id: String,
    version: String,
    kind: PluginKind,
    entrypoint: String,
    descriptor_dir: String,
    permissions: Vec<String>,
    resources: Vec<String>,
    platforms: Vec<String>,
    limits: PluginRuntimeLimits,
    declarative: Option<PluginDeclarativeBinding>,
    abilities: Vec<PluginAbilityManifest>,
    realtime_capabilities: Vec<PluginRealtimeCapability>,
}

impl PluginPackageManifest {
    /// Parse and validate one plugin manifest body.
    pub fn parse(manifest_path: &str, manifest_body: &str) -> Result<Self> {
        let raw: RawPluginToml = toml::from_str(manifest_body).map_err(|source| {
            PluginHostError::ManifestParseFailed {
                path: manifest_path.into(),
                source,
            }
        })?;
        parse_plugin_manifest(manifest_path, raw)
    }

    /// Stable plugin identifier declared by the plugin manifest.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Manifest schema version.
    pub fn schema_version(&self) -> &str {
        &self.schema_version
    }

    /// Plugin package version.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Plugin kind.
    pub const fn kind(&self) -> PluginKind {
        self.kind
    }

    /// Manifest-declared registration symbol.
    pub fn entrypoint(&self) -> &str {
        &self.entrypoint
    }

    /// Repository-relative directory that owns generated ability descriptors.
    pub fn descriptor_dir(&self) -> &str {
        &self.descriptor_dir
    }

    /// Platforms on which the host may load this plugin.
    pub fn platforms(&self) -> &[String] {
        &self.platforms
    }

    /// Host permissions declared by the plugin package.
    pub fn permissions(&self) -> &[String] {
        &self.permissions
    }

    /// Resource kinds declared by the plugin package.
    pub fn resources(&self) -> &[String] {
        &self.resources
    }

    /// Host-level runtime limits declared by the plugin package.
    pub const fn limits(&self) -> PluginRuntimeLimits {
        self.limits
    }

    /// Declarative execution binding, if this is a declarative package.
    pub fn declarative_binding(&self) -> Option<&PluginDeclarativeBinding> {
        self.declarative.as_ref()
    }

    /// Ability manifests exported by this package.
    pub fn abilities(&self) -> &[PluginAbilityManifest] {
        &self.abilities
    }

    /// Package-level realtime device capabilities.
    pub fn realtime_capabilities(&self) -> &[PluginRealtimeCapability] {
        &self.realtime_capabilities
    }

    /// Resolve one package-owned ability.
    pub fn ability(&self, name: &str) -> Option<&PluginAbilityManifest> {
        self.abilities.iter().find(|ability| ability.name() == name)
    }

    /// Whether this release can load every call mode declared by a sidecar package.
    ///
    /// Sidecar packages support rpc, finite stream snapshot, and live bidi
    /// through the daemon-owned JSON frame profile. Declarative plugins remain
    /// restricted by their declarative binding validator; sidecar mode itself
    /// is not the limiter.
    pub fn sidecar_call_modes_supported_in_this_release(&self) -> bool {
        true
    }
}

#[derive(Debug, Deserialize)]
struct RawPluginToml {
    schema_version: String,
    id: String,
    version: String,
    kind: PluginKind,
    entrypoint: String,
    abilities: Vec<String>,
    permissions: Vec<String>,
    resources: Vec<String>,
    platforms: Vec<String>,
    limits: PluginRuntimeLimits,
    #[serde(default)]
    declarative: Option<PluginDeclarativeBinding>,
    #[serde(default)]
    ability_metadata: Vec<RawPluginAbilityMetadata>,
    #[serde(default)]
    realtime_capability: Vec<PluginRealtimeCapability>,
}

#[derive(Debug, Deserialize)]
struct RawPluginAbilityMetadata {
    name: String,
    layer: PluginAbilityLayer,
    #[serde(default = "default_call_mode")]
    call_mode: PluginCallMode,
    #[serde(default)]
    bidi_wire_kind: Option<PluginBidiWireKind>,
}

fn parse_plugin_manifest(manifest_path: &str, raw: RawPluginToml) -> Result<PluginPackageManifest> {
    if raw.schema_version != "1" {
        return Err(PluginHostError::UnsupportedSchema(raw.schema_version));
    }
    if raw.id.trim().is_empty() {
        return Err(PluginHostError::MissingField("id"));
    }
    if raw.entrypoint.trim().is_empty() {
        return Err(PluginHostError::MissingField("entrypoint"));
    }
    let descriptor_dir = descriptor_dir_from_ability_patterns(manifest_path, &raw.abilities)?;
    if raw.ability_metadata.is_empty() {
        return Err(PluginHostError::MissingAbilityMetadata);
    }
    if raw.limits.max_sessions() == 0 {
        return Err(PluginHostError::InvalidRuntimeLimit("max_sessions"));
    }
    if raw.limits.max_frame_queue() == 0 {
        return Err(PluginHostError::InvalidRuntimeLimit("max_frame_queue"));
    }
    validate_declarative_binding(&raw)?;
    validate_realtime_capabilities(&raw.id, &raw.realtime_capability)?;

    let mut seen = std::collections::BTreeSet::new();
    let mut abilities = Vec::with_capacity(raw.ability_metadata.len());
    for ability in raw.ability_metadata {
        if ability.name.trim().is_empty() {
            return Err(PluginHostError::MissingField("ability_metadata.name"));
        }
        validate_ability_name(&ability.name)?;
        if !seen.insert(ability.name.clone()) {
            return Err(PluginHostError::DuplicateAbility(ability.name));
        }
        abilities.push(PluginAbilityManifest {
            descriptor_path: format!("{descriptor_dir}/{}.ability.toml", ability.name),
            name: ability.name,
            layer: ability.layer,
            call_mode: ability.call_mode,
            bidi_wire_kind: ability.bidi_wire_kind,
        });
    }
    validate_realtime_activation_abilities(&raw.id, &raw.realtime_capability, &seen)?;

    Ok(PluginPackageManifest {
        schema_version: raw.schema_version,
        id: raw.id,
        version: raw.version,
        kind: raw.kind,
        entrypoint: raw.entrypoint,
        descriptor_dir,
        permissions: raw.permissions,
        resources: raw.resources,
        platforms: raw.platforms,
        limits: raw.limits,
        declarative: raw.declarative,
        abilities,
        realtime_capabilities: raw.realtime_capability,
    })
}

fn validate_realtime_capabilities(
    id: &str,
    capabilities: &[PluginRealtimeCapability],
) -> Result<()> {
    for capability in capabilities {
        if capability.modes.is_empty() {
            return Err(PluginHostError::InvalidRealtimeCapability {
                id: id.to_string(),
                reason: "modes must not be empty".to_string(),
            });
        }
        let mut modes = std::collections::BTreeSet::new();
        for mode in &capability.modes {
            if !modes.insert(*mode) {
                return Err(PluginHostError::InvalidRealtimeCapability {
                    id: id.to_string(),
                    reason: format!("duplicate mode {:?}", mode),
                });
            }
            if !realtime_kind_allows_mode(capability.kind, *mode) {
                return Err(PluginHostError::InvalidRealtimeCapability {
                    id: id.to_string(),
                    reason: format!(
                        "kind {:?} does not support mode {:?}",
                        capability.kind, mode
                    ),
                });
            }
        }
        let mut activation_abilities = std::collections::BTreeSet::new();
        for ability in &capability.activation_abilities {
            if ability.trim().is_empty() {
                return Err(PluginHostError::InvalidRealtimeCapability {
                    id: id.to_string(),
                    reason: "activation ability must not be empty".to_string(),
                });
            }
            validate_ability_name(ability)?;
            if !activation_abilities.insert(ability) {
                return Err(PluginHostError::InvalidRealtimeCapability {
                    id: id.to_string(),
                    reason: format!("duplicate activation ability {ability:?}"),
                });
            }
        }
        if capability.fallback_transport == Some(capability.transport) {
            return Err(PluginHostError::InvalidRealtimeCapability {
                id: id.to_string(),
                reason: "fallback_transport must differ from transport".to_string(),
            });
        }
        if capability.quick_add && capability.resources.is_empty() {
            return Err(PluginHostError::InvalidRealtimeCapability {
                id: id.to_string(),
                reason: "quick_add capabilities must declare at least one resource kind"
                    .to_string(),
            });
        }
        for resource in &capability.resources {
            if resource.trim().is_empty() {
                return Err(PluginHostError::InvalidRealtimeCapability {
                    id: id.to_string(),
                    reason: "resource kind must not be empty".to_string(),
                });
            }
        }
        for permission in &capability.permissions {
            if permission.trim().is_empty() {
                return Err(PluginHostError::InvalidRealtimeCapability {
                    id: id.to_string(),
                    reason: "permission must not be empty".to_string(),
                });
            }
        }
    }
    Ok(())
}

fn validate_realtime_activation_abilities(
    id: &str,
    capabilities: &[PluginRealtimeCapability],
    package_abilities: &std::collections::BTreeSet<String>,
) -> Result<()> {
    for capability in capabilities {
        for ability in &capability.activation_abilities {
            if !package_abilities.contains(ability) {
                return Err(PluginHostError::InvalidRealtimeCapability {
                    id: id.to_string(),
                    reason: format!(
                        "activation ability {ability:?} must be declared in ability_metadata"
                    ),
                });
            }
        }
    }
    Ok(())
}

fn realtime_kind_allows_mode(kind: PluginRealtimeKind, mode: PluginRealtimeMode) -> bool {
    match kind {
        PluginRealtimeKind::Camera => matches!(
            mode,
            PluginRealtimeMode::Snapshot
                | PluginRealtimeMode::Subscribe
                | PluginRealtimeMode::Record
        ),
        PluginRealtimeKind::Mic => {
            matches!(
                mode,
                PluginRealtimeMode::Subscribe | PluginRealtimeMode::Record
            )
        }
        PluginRealtimeKind::Screen => matches!(
            mode,
            PluginRealtimeMode::Snapshot
                | PluginRealtimeMode::Subscribe
                | PluginRealtimeMode::Record
        ),
        PluginRealtimeKind::Speaker => matches!(mode, PluginRealtimeMode::Publish),
        PluginRealtimeKind::Voice => matches!(
            mode,
            PluginRealtimeMode::Subscribe | PluginRealtimeMode::Transcribe
        ),
    }
}

fn validate_declarative_binding(raw: &RawPluginToml) -> Result<()> {
    match (raw.kind, raw.declarative.as_ref()) {
        (PluginKind::Declarative, Some(PluginDeclarativeBinding::Exec { argv })) => {
            if argv.is_empty() || argv[0].trim().is_empty() {
                return Err(PluginHostError::InvalidDeclarativeBinding {
                    id: raw.id.clone(),
                    reason: "exec binding must declare non-empty argv".to_string(),
                });
            }
        }
        (
            PluginKind::Declarative,
            Some(PluginDeclarativeBinding::Eal {
                program,
                result_binding,
            }),
        ) => {
            if program.trim().is_empty() {
                return Err(PluginHostError::InvalidDeclarativeBinding {
                    id: raw.id.clone(),
                    reason: "eal binding must declare non-empty program".to_string(),
                });
            }
            if result_binding
                .as_deref()
                .map(str::trim)
                .is_some_and(str::is_empty)
            {
                return Err(PluginHostError::InvalidDeclarativeBinding {
                    id: raw.id.clone(),
                    reason: "eal binding result_binding, when set, must be non-empty".to_string(),
                });
            }
        }
        (PluginKind::Declarative, Some(PluginDeclarativeBinding::Mcp { server, tool })) => {
            if server.trim().is_empty() || tool.trim().is_empty() {
                return Err(PluginHostError::InvalidDeclarativeBinding {
                    id: raw.id.clone(),
                    reason: "mcp binding must declare non-empty server and tool".to_string(),
                });
            }
        }
        (PluginKind::Declarative, None) => {}
        (_, Some(_)) => {
            return Err(PluginHostError::InvalidDeclarativeBinding {
                id: raw.id.clone(),
                reason: "only declarative packages may declare [declarative]".to_string(),
            });
        }
        (_, None) => {}
    }
    Ok(())
}

fn descriptor_dir_from_ability_patterns(
    manifest_path: &str,
    patterns: &[String],
) -> Result<String> {
    let [pattern] = patterns else {
        if patterns.is_empty() {
            return Err(PluginHostError::MissingAbilityPattern);
        }
        return Err(PluginHostError::MultipleAbilityPatterns(patterns.to_vec()));
    };
    let Some(relative_dir) = pattern.strip_suffix("/*.ability.toml") else {
        return Err(PluginHostError::UnsupportedAbilityPattern(pattern.clone()));
    };
    let Some((package_dir, _)) = manifest_path.rsplit_once('/') else {
        return Err(PluginHostError::UnsupportedManifestPath(
            manifest_path.to_string(),
        ));
    };
    Ok(format!("{package_dir}/{relative_dir}"))
}

fn default_call_mode() -> PluginCallMode {
    PluginCallMode::Rpc
}

fn validate_ability_name(name: &str) -> Result<()> {
    let valid = name
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'));
    if !valid || name.contains("..") || name.starts_with('.') || name.ends_with('.') {
        return Err(PluginHostError::InvalidAbilityName(name.to_string()));
    }
    Ok(())
}

/// Validate a builtin manifest against its compiled binding symbol.
pub fn validate_builtin_entrypoint(
    manifest: &PluginPackageManifest,
    expected_entrypoint: &'static str,
) -> Result<()> {
    if manifest.entrypoint() != expected_entrypoint {
        return Err(PluginHostError::EntrypointMismatch {
            declared: manifest.entrypoint().to_string(),
            expected: expected_entrypoint,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_accepts_realtime_capability_contract() {
        let manifest = PluginPackageManifest::parse(
            "plugins/test/plugin.toml",
            &test_manifest(
                r#"
[[ability_metadata]]
name = "test.camera"
layer = "operational"
call_mode = "bidi"
bidi_wire_kind = "json_frames"

[[realtime_capability]]
kind = "camera"
modes = ["snapshot", "subscribe", "record"]
transport = "invoke_bidi"
fallback_transport = "invoke_stream"
activation_abilities = ["test.camera"]
permissions = ["camera"]
resources = ["camera"]
quick_add = true
"#,
            ),
        )
        .expect("valid realtime manifest");

        let capability = manifest
            .realtime_capabilities()
            .first()
            .expect("realtime capability parsed");
        assert_eq!(capability.kind(), PluginRealtimeKind::Camera);
        assert_eq!(
            capability.modes(),
            &[
                PluginRealtimeMode::Snapshot,
                PluginRealtimeMode::Subscribe,
                PluginRealtimeMode::Record,
            ]
        );
        assert!(capability.quick_add());
        assert_eq!(
            capability.activation_abilities(),
            &["test.camera".to_string()]
        );
    }

    #[test]
    fn manifest_rejects_invalid_realtime_mode_for_kind() {
        let err = PluginPackageManifest::parse(
            "plugins/test/plugin.toml",
            &test_manifest(
                r#"
[[ability_metadata]]
name = "test.speaker"
layer = "operational"
call_mode = "bidi"
bidi_wire_kind = "json_frames"

[[realtime_capability]]
kind = "speaker"
modes = ["subscribe"]
transport = "invoke_bidi"
resources = ["speaker"]
"#,
            ),
        )
        .expect_err("speaker subscribe must reject");

        assert!(
            matches!(err, PluginHostError::InvalidRealtimeCapability { .. }),
            "wrong error: {err}"
        );
    }

    #[test]
    fn manifest_rejects_quick_add_without_resource_kinds() {
        let err = PluginPackageManifest::parse(
            "plugins/test/plugin.toml",
            &test_manifest(
                r#"
[[ability_metadata]]
name = "test.mic"
layer = "operational"
call_mode = "stream"

[[realtime_capability]]
kind = "mic"
modes = ["subscribe"]
transport = "invoke_stream"
quick_add = true
"#,
            ),
        )
        .expect_err("quick add needs resources");

        assert!(
            matches!(err, PluginHostError::InvalidRealtimeCapability { .. }),
            "wrong error: {err}"
        );
    }

    #[test]
    fn manifest_rejects_realtime_activation_ability_outside_package() {
        let err = PluginPackageManifest::parse(
            "plugins/test/plugin.toml",
            &test_manifest(
                r#"
[[ability_metadata]]
name = "test.camera"
layer = "operational"
call_mode = "bidi"

[[realtime_capability]]
kind = "camera"
modes = ["snapshot"]
transport = "invoke_bidi"
activation_abilities = ["other.camera"]
resources = ["camera"]
"#,
            ),
        )
        .expect_err("activation ability must be package-owned");

        assert!(
            matches!(err, PluginHostError::InvalidRealtimeCapability { .. }),
            "wrong error: {err}"
        );
        assert!(
            format!("{err}").contains("ability_metadata"),
            "wrong error: {err}"
        );
    }

    fn test_manifest(extra: &str) -> String {
        format!(
            r#"
schema_version = "1"
id = "test.plugin"
version = "0.1.0"
kind = "sidecar"
entrypoint = "bin/plugin"
abilities = ["abilities/*.ability.toml"]
permissions = []
resources = []
platforms = []

[limits]
max_sessions = 1
max_frame_queue = 1

{extra}
"#
        )
    }
}
