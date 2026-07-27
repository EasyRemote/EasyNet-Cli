// EasyNet CLI — plugin manifest model
// ===================================
//
// File: src/daemon/plugins/manifest.rs
// Description: Typed `plugin.toml` package model and validation.

use std::path::{Component, Path};

use serde::{Deserialize, Serialize};

use crate::daemon::ability::CallMode;
use crate::daemon::plugins::errors::{PluginHostError, Result};

/// Plugin package metadata declares the same governed invocation mode used by
/// descriptors and routing. A plugin never owns a parallel transport taxonomy.

/// Wire adapter a bidi plugin ability expects when it crosses the
/// `session.open` bridge.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
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
#[serde(deny_unknown_fields)]
pub struct PluginRealtimeCapability {
    kind: PluginRealtimeKind,
    modes: Vec<PluginRealtimeMode>,
    transport: PluginRealtimeTransport,
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
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
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
    DesktopCompanion,
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
            "builtin" => Ok(Self::Builtin),
            "desktop_companion" => Ok(Self::DesktopCompanion),
            other => Err(serde::de::Error::custom(format!(
                "unsupported plugin kind {other:?}"
            ))),
        }
    }
}

/// Desktop companion lifecycle declared by a plugin package.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PluginCompanionLifecycle {
    UserSession,
}

/// Startup policy for a desktop companion process.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PluginCompanionBootPolicy {
    Manual,
    EnsureRunningAfterDaemonReady,
}

/// Stop policy for a desktop companion process.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PluginCompanionStopPolicy {
    KeepRunning,
    StopOnRuntimeStop,
    StopOnPluginDisable,
}

/// Health observation mode for a desktop companion process.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PluginCompanionHealthMode {
    ProcessName,
    StatusFile,
    LocalIpc,
}

/// Platform-specific desktop companion supervisor declaration.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PluginCompanionMacos {
    bundle_id: String,
    app_bundle: String,
    supervisor: String,
    launch_agent_label: String,
    session: String,
}

impl PluginCompanionMacos {
    pub fn bundle_id(&self) -> &str {
        &self.bundle_id
    }

    pub fn app_bundle(&self) -> &str {
        &self.app_bundle
    }

    pub fn supervisor(&self) -> &str {
        &self.supervisor
    }

    pub fn launch_agent_label(&self) -> &str {
        &self.launch_agent_label
    }

    pub fn session(&self) -> &str {
        &self.session
    }
}

/// Platform-specific Windows companion declaration.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PluginCompanionWindows {
    exe: String,
    supervisor: String,
    task_name: String,
    session: String,
}

impl PluginCompanionWindows {
    pub fn exe(&self) -> &str {
        &self.exe
    }

    pub fn supervisor(&self) -> &str {
        &self.supervisor
    }

    pub fn task_name(&self) -> &str {
        &self.task_name
    }

    pub fn session(&self) -> &str {
        &self.session
    }
}

/// Platform-specific Linux companion declaration.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PluginCompanionLinux {
    exe: String,
    supervisor: String,
    unit_name: String,
    session: String,
}

impl PluginCompanionLinux {
    pub fn exe(&self) -> &str {
        &self.exe
    }

    pub fn supervisor(&self) -> &str {
        &self.supervisor
    }

    pub fn unit_name(&self) -> &str {
        &self.unit_name
    }

    pub fn session(&self) -> &str {
        &self.session
    }
}

/// Desktop companion metadata declared by a package manifest.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PluginCompanionManifest {
    display_name: String,
    lifecycle: PluginCompanionLifecycle,
    boot_policy: PluginCompanionBootPolicy,
    stop_policy: PluginCompanionStopPolicy,
    health: PluginCompanionHealthMode,
    #[serde(default)]
    status_file: Option<String>,
    #[serde(default)]
    macos: Option<PluginCompanionMacos>,
    #[serde(default)]
    windows: Option<PluginCompanionWindows>,
    #[serde(default)]
    linux: Option<PluginCompanionLinux>,
}

impl PluginCompanionManifest {
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    pub const fn lifecycle(&self) -> PluginCompanionLifecycle {
        self.lifecycle
    }

    pub const fn boot_policy(&self) -> PluginCompanionBootPolicy {
        self.boot_policy
    }

    pub const fn stop_policy(&self) -> PluginCompanionStopPolicy {
        self.stop_policy
    }

    pub const fn health(&self) -> PluginCompanionHealthMode {
        self.health
    }

    pub fn status_file(&self) -> Option<&str> {
        self.status_file.as_deref()
    }

    pub fn macos(&self) -> Option<&PluginCompanionMacos> {
        self.macos.as_ref()
    }

    pub fn windows(&self) -> Option<&PluginCompanionWindows> {
        self.windows.as_ref()
    }

    pub fn linux(&self) -> Option<&PluginCompanionLinux> {
        self.linux.as_ref()
    }
}

/// Declarative plugin execution binding.
///
/// The package manifest is the source of truth. This enum deliberately models
/// every first-version declarative binding. `Eal` and `Mcp` intentionally
/// reuse the daemon's existing in-process executors instead of creating a
/// plugin-specific orchestration or MCP call path.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
    call_mode: CallMode,
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
    pub const fn call_mode(&self) -> CallMode {
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
    companion: Option<PluginCompanionManifest>,
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

    /// Desktop companion metadata, if this package declares a companion process.
    pub fn companion(&self) -> Option<&PluginCompanionManifest> {
        self.companion.as_ref()
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
#[serde(deny_unknown_fields)]
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
    companion: Option<PluginCompanionManifest>,
    #[serde(default)]
    ability_metadata: Vec<RawPluginAbilityMetadata>,
    #[serde(default)]
    realtime_capability: Vec<PluginRealtimeCapability>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPluginAbilityMetadata {
    name: String,
    layer: PluginAbilityLayer,
    call_mode: CallMode,
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
    let descriptor_dir = if raw.abilities.is_empty() {
        String::new()
    } else {
        descriptor_dir_from_ability_patterns(manifest_path, &raw.abilities)?
    };
    if raw.kind != PluginKind::DesktopCompanion && raw.abilities.is_empty() {
        return Err(PluginHostError::MissingAbilityPattern);
    }
    if raw.kind != PluginKind::DesktopCompanion && raw.ability_metadata.is_empty() {
        return Err(PluginHostError::MissingAbilityMetadata);
    }
    if raw.kind == PluginKind::DesktopCompanion
        && raw.abilities.is_empty()
        && !raw.ability_metadata.is_empty()
    {
        return Err(PluginHostError::MissingAbilityPattern);
    }
    if raw.kind == PluginKind::DesktopCompanion
        && !raw.abilities.is_empty()
        && raw.ability_metadata.is_empty()
    {
        return Err(PluginHostError::MissingAbilityMetadata);
    }
    if raw.limits.max_sessions() == 0 {
        return Err(PluginHostError::InvalidRuntimeLimit("max_sessions"));
    }
    if raw.limits.max_frame_queue() == 0 {
        return Err(PluginHostError::InvalidRuntimeLimit("max_frame_queue"));
    }
    validate_declarative_binding(&raw)?;
    validate_companion_manifest(&raw)?;
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
        companion: raw.companion,
        abilities,
        realtime_capabilities: raw.realtime_capability,
    })
}

fn validate_companion_manifest(raw: &RawPluginToml) -> Result<()> {
    match (raw.kind, raw.companion.as_ref()) {
        (PluginKind::DesktopCompanion, Some(companion)) => {
            validate_companion_fields(&raw.id, companion)
        }
        (PluginKind::DesktopCompanion, None) => Err(PluginHostError::InvalidCompanionManifest {
            id: raw.id.clone(),
            reason: "desktop_companion packages must declare [companion]".to_string(),
        }),
        (_, Some(_)) => Err(PluginHostError::InvalidCompanionManifest {
            id: raw.id.clone(),
            reason: "only desktop_companion packages may declare [companion]".to_string(),
        }),
        (_, None) => Ok(()),
    }
}

fn validate_companion_fields(id: &str, companion: &PluginCompanionManifest) -> Result<()> {
    if companion.display_name.trim().is_empty() {
        return Err(invalid_companion(id, "display_name must not be empty"));
    }
    if companion.health == PluginCompanionHealthMode::LocalIpc {
        return Err(invalid_companion(
            id,
            "health = \"local_ipc\" is reserved for a later release",
        ));
    }
    if companion.health == PluginCompanionHealthMode::StatusFile {
        let status_file = companion.status_file.as_deref().ok_or_else(|| {
            invalid_companion(id, "status_file is required for status_file health")
        })?;
        validate_relative_manifest_path(id, "status_file", status_file)?;
    }
    if companion.macos.is_none() && companion.windows.is_none() && companion.linux.is_none() {
        return Err(invalid_companion(
            id,
            "at least one companion platform section is required",
        ));
    }
    if let Some(macos) = &companion.macos {
        validate_non_empty(id, "companion.macos.bundle_id", &macos.bundle_id)?;
        validate_non_empty(
            id,
            "companion.macos.launch_agent_label",
            &macos.launch_agent_label,
        )?;
        validate_exact(
            id,
            "companion.macos.supervisor",
            &macos.supervisor,
            "launch_agent",
        )?;
        validate_exact(id, "companion.macos.session", &macos.session, "aqua")?;
        validate_relative_artifact_path(id, "companion.macos.app_bundle", &macos.app_bundle)?;
    }
    if let Some(windows) = &companion.windows {
        validate_non_empty(id, "companion.windows.task_name", &windows.task_name)?;
        validate_exact(
            id,
            "companion.windows.supervisor",
            &windows.supervisor,
            "startup_task",
        )?;
        validate_exact(
            id,
            "companion.windows.session",
            &windows.session,
            "interactive_desktop",
        )?;
        validate_relative_artifact_path(id, "companion.windows.exe", &windows.exe)?;
    }
    if let Some(linux) = &companion.linux {
        validate_non_empty(id, "companion.linux.unit_name", &linux.unit_name)?;
        validate_exact(
            id,
            "companion.linux.supervisor",
            &linux.supervisor,
            "systemd_user",
        )?;
        validate_exact(id, "companion.linux.session", &linux.session, "graphical")?;
        validate_relative_artifact_path(id, "companion.linux.exe", &linux.exe)?;
    }
    Ok(())
}

fn validate_non_empty(id: &str, field: &'static str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        return Err(invalid_companion(id, &format!("{field} must not be empty")));
    }
    Ok(())
}

fn validate_exact(
    id: &str,
    field: &'static str,
    actual: &str,
    expected: &'static str,
) -> Result<()> {
    if actual != expected {
        return Err(invalid_companion(
            id,
            &format!("{field} must be {expected:?}"),
        ));
    }
    Ok(())
}

fn validate_relative_manifest_path(id: &str, field: &'static str, raw: &str) -> Result<()> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(invalid_companion(id, &format!("{field} must not be empty")));
    }
    let path = Path::new(trimmed);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(invalid_companion(
            id,
            &format!("{field} must be a relative path"),
        ));
    }
    Ok(())
}

fn validate_relative_artifact_path(id: &str, field: &'static str, raw: &str) -> Result<()> {
    validate_relative_manifest_path(id, field, raw)?;
    let mut components = Path::new(raw.trim()).components();
    match components.next() {
        Some(Component::Normal(root)) if root == "bin" || root == "dist" => Ok(()),
        _ => Err(invalid_companion(
            id,
            &format!("{field} must be under bin/ or dist/ so package hashing covers it"),
        )),
    }
}

fn invalid_companion(id: &str, reason: &str) -> PluginHostError {
    PluginHostError::InvalidCompanionManifest {
        id: id.to_string(),
        reason: reason.to_string(),
    }
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

    #[test]
    fn manifest_rejects_unknown_top_level_fields() {
        assert_manifest_parse_unknown_field(
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
retired_kind_alias = "stateful-device-plugin"

[limits]
max_sessions = 1
max_frame_queue = 1

[[ability_metadata]]
name = "test.echo"
layer = "operational"
call_mode = "rpc"
"#,
            "retired_kind_alias",
        );
    }

    #[test]
    fn manifest_rejects_retired_plugin_kind_alias() {
        let body = test_manifest(
            r#"
[[ability_metadata]]
name = "test.echo"
layer = "operational"
call_mode = "rpc"
"#,
        )
        .replace("kind = \"sidecar\"", "kind = \"stateful-device-plugin\"");
        let err = PluginPackageManifest::parse("plugins/test/plugin.toml", &body)
            .expect_err("retired plugin kind aliases must stay removed");
        assert!(
            matches!(err, PluginHostError::ManifestParseFailed { .. }),
            "kind alias must fail during typed parse, got: {err}"
        );
        assert!(
            format!("{err}").contains("unsupported plugin kind"),
            "kind alias rejection should name unsupported kind: {err}"
        );
    }

    #[test]
    fn manifest_rejects_unknown_ability_metadata_fields() {
        assert_manifest_parse_unknown_field(
            &test_manifest(
                r#"
[[ability_metadata]]
name = "test.echo"
layer = "operational"
call_mode = "rpc"
retired_call_mode = "rpc"
"#,
            ),
            "retired_call_mode",
        );
    }

    #[test]
    fn manifest_rejects_missing_ability_call_mode() {
        let err = PluginPackageManifest::parse(
            "plugins/test/plugin.toml",
            &test_manifest(
                r#"
[[ability_metadata]]
name = "test.echo"
layer = "operational"
"#,
            ),
        )
        .expect_err("ability call_mode must be explicit");

        assert!(
            matches!(err, PluginHostError::ManifestParseFailed { .. }),
            "missing call_mode must fail at typed parse, got: {err}"
        );
        assert!(
            format!("{err}").contains("missing field `call_mode`"),
            "missing call_mode rejection should name field: {err}"
        );
    }

    #[test]
    fn manifest_rejects_unknown_realtime_capability_fields() {
        assert_manifest_parse_unknown_field(
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
resources = ["camera"]
retired_media_bus = "webrtc-v0"
"#,
            ),
            "retired_media_bus",
        );
    }

    #[test]
    fn manifest_rejects_unknown_runtime_limit_fields() {
        assert_manifest_parse_unknown_field(
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
retired_queue = 1024

[[ability_metadata]]
name = "test.echo"
layer = "operational"
call_mode = "rpc"
"#,
            "retired_queue",
        );
    }

    #[test]
    fn manifest_rejects_unknown_declarative_binding_fields() {
        assert_manifest_parse_unknown_field(
            r#"
schema_version = "1"
id = "test.plugin"
version = "0.1.0"
kind = "declarative"
entrypoint = "bin/plugin"
abilities = ["abilities/*.ability.toml"]
permissions = []
resources = []
platforms = []

[limits]
max_sessions = 1
max_frame_queue = 1

[declarative]
kind = "exec"
argv = ["bin/plugin"]
retired_shell = true

[[ability_metadata]]
name = "test.echo"
layer = "operational"
call_mode = "rpc"
"#,
            "retired_shell",
        );
    }

    #[test]
    fn manifest_rejects_unknown_companion_platform_fields() {
        assert_manifest_parse_unknown_field(
            r#"
schema_version = "1"
id = "easynet.desktop.menubar"
version = "0.1.0"
kind = "desktop_companion"
entrypoint = "dist/macos/EasyNetMenuBar.app"
abilities = []
permissions = ["clipboard_read"]
resources = ["desktop_session"]
platforms = ["macos"]

[limits]
max_sessions = 1
max_frame_queue = 1

[companion]
display_name = "EasyNet Menu Bar"
lifecycle = "user_session"
boot_policy = "ensure_running_after_daemon_ready"
stop_policy = "keep_running"
health = "status_file"
status_file = "companions/easynet.desktop.menubar/status.json"

[companion.macos]
bundle_id = "tech.silan.easynet.menubar"
app_bundle = "dist/macos/EasyNetMenuBar.app"
supervisor = "launch_agent"
launch_agent_label = "tech.silan.easynet.menubar"
session = "aqua"
retired_plist_label = "tech.silan.old"
"#,
            "retired_plist_label",
        );
    }

    #[test]
    fn manifest_accepts_desktop_companion_without_abilities() {
        let manifest = PluginPackageManifest::parse(
            "plugins/easynet.desktop.menubar/plugin.toml",
            r#"
schema_version = "1"
id = "easynet.desktop.menubar"
version = "0.1.0"
kind = "desktop_companion"
entrypoint = "dist/macos/EasyNetMenuBar.app"
abilities = []
permissions = ["clipboard_read"]
resources = ["desktop_session"]
platforms = ["macos"]

[limits]
max_sessions = 1
max_frame_queue = 1

[companion]
display_name = "EasyNet Menu Bar"
lifecycle = "user_session"
boot_policy = "ensure_running_after_daemon_ready"
stop_policy = "keep_running"
health = "status_file"
status_file = "companions/easynet.desktop.menubar/status.json"

[companion.macos]
bundle_id = "tech.silan.easynet.menubar"
app_bundle = "dist/macos/EasyNetMenuBar.app"
supervisor = "launch_agent"
launch_agent_label = "tech.silan.easynet.menubar"
session = "aqua"
"#,
        )
        .expect("desktop companion manifest");

        assert_eq!(manifest.kind(), PluginKind::DesktopCompanion);
        assert!(manifest.abilities().is_empty());
        let companion = manifest.companion().expect("companion metadata");
        assert_eq!(companion.display_name(), "EasyNet Menu Bar");
        assert_eq!(
            companion.macos().unwrap().app_bundle(),
            "dist/macos/EasyNetMenuBar.app"
        );
    }

    #[test]
    fn manifest_rejects_desktop_companion_without_companion_section() {
        let err = PluginPackageManifest::parse(
            "plugins/easynet.desktop.menubar/plugin.toml",
            r#"
schema_version = "1"
id = "easynet.desktop.menubar"
version = "0.1.0"
kind = "desktop_companion"
entrypoint = "dist/macos/EasyNetMenuBar.app"
abilities = []
permissions = []
resources = []
platforms = ["macos"]

[limits]
max_sessions = 1
max_frame_queue = 1
"#,
        )
        .expect_err("companion section is required");

        assert!(matches!(
            err,
            PluginHostError::InvalidCompanionManifest { .. }
        ));
    }

    #[test]
    fn manifest_rejects_desktop_companion_artifact_outside_hashed_roots() {
        let err = PluginPackageManifest::parse(
            "plugins/easynet.desktop.menubar/plugin.toml",
            r#"
schema_version = "1"
id = "easynet.desktop.menubar"
version = "0.1.0"
kind = "desktop_companion"
entrypoint = "dist/macos/EasyNetMenuBar.app"
abilities = []
permissions = ["clipboard_read"]
resources = ["desktop_session"]
platforms = ["macos"]

[limits]
max_sessions = 1
max_frame_queue = 1

[companion]
display_name = "EasyNet Menu Bar"
lifecycle = "user_session"
boot_policy = "ensure_running_after_daemon_ready"
stop_policy = "keep_running"
health = "status_file"
status_file = "companions/easynet.desktop.menubar/status.json"

[companion.macos]
bundle_id = "tech.silan.easynet.menubar"
app_bundle = "release/macos/EasyNetMenuBar.app"
supervisor = "launch_agent"
launch_agent_label = "tech.silan.easynet.menubar"
session = "aqua"
"#,
        )
        .expect_err("companion artifacts outside bin/ or dist/ must be rejected");

        assert!(
            matches!(err, PluginHostError::InvalidCompanionManifest { .. }),
            "wrong error: {err}"
        );
        assert!(
            format!("{err}").contains("package hashing covers it"),
            "wrong error detail: {err}"
        );
    }

    #[test]
    fn real_plugin_manifests_parse_under_strict_schema() {
        let remote_desktop = PluginPackageManifest::parse(
            "plugins/remote-desktop/plugin.toml",
            include_str!("../../../plugins/remote-desktop/plugin.toml"),
        )
        .expect("remote desktop plugin manifest");
        assert_eq!(remote_desktop.kind(), PluginKind::Builtin);

        let desktop_menubar = PluginPackageManifest::parse(
            "plugins/desktop-menubar/plugin.toml",
            include_str!("../../../plugins/desktop-menubar/plugin.toml"),
        )
        .expect("desktop menubar plugin manifest");
        assert_eq!(desktop_menubar.kind(), PluginKind::DesktopCompanion);
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

    fn assert_manifest_parse_unknown_field(body: &str, field: &str) {
        let err = PluginPackageManifest::parse("plugins/test/plugin.toml", body)
            .expect_err("unknown plugin manifest field must fail at typed parse");
        assert!(
            matches!(err, PluginHostError::ManifestParseFailed { .. }),
            "unknown field should fail before semantic validation, got: {err}"
        );
        assert!(
            format!("{err}").contains(&format!("unknown field `{field}`")),
            "parse error should name unknown field {field:?}, got: {err}"
        );
    }
}
