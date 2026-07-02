// EasyNet CLI — plugin host errors
// =================================
//
// File: src/daemon/plugins/errors.rs
// Description: Typed failure surface for package install, indexing, and load.

use std::path::PathBuf;

/// Errors emitted by the daemon plugin host.
///
/// What this is NOT: an ability invocation error surface. Invocation failures
/// remain owned by the registered ability handler; these errors describe host
/// package, descriptor, and loading failures before a call is dispatched.
#[derive(Debug, thiserror::Error)]
pub enum PluginHostError {
    #[error("unsupported plugin schema_version {0:?}")]
    UnsupportedSchema(String),
    #[error("plugin manifest is missing required field {0}")]
    MissingField(&'static str),
    #[error("plugin manifest must declare at least one ability glob")]
    MissingAbilityPattern,
    #[error(
        "plugin manifest declares multiple ability globs but host supports exactly one: {0:?}"
    )]
    MultipleAbilityPatterns(Vec<String>),
    #[error("unsupported plugin ability pattern {0:?}")]
    UnsupportedAbilityPattern(String),
    #[error("unsupported plugin manifest path {0:?}")]
    UnsupportedManifestPath(String),
    #[error("plugin manifest must declare [[ability_metadata]] entries")]
    MissingAbilityMetadata,
    #[error("plugin manifest declares invalid ability name {0:?}")]
    InvalidAbilityName(String),
    #[error("plugin manifest declares duplicate ability {0:?}")]
    DuplicateAbility(String),
    #[error(
        "plugin package index declares multiple active versions for {id}: {first_version} and {second_version}"
    )]
    DuplicatePackageId {
        id: String,
        first_version: String,
        second_version: String,
    },
    #[error("plugin package index declares duplicate package id/version {id}@{version}")]
    DuplicatePackageVersion { id: String, version: String },
    #[error("plugin ability {ability:?} is declared by multiple packages: {first} and {second}")]
    DuplicateAbilityOwner {
        ability: String,
        first: String,
        second: String,
    },
    #[error("installed plugin package {id}@{version} is not loadable in this release: {kind}")]
    InstallKindNotLoadableInThisRelease {
        id: String,
        version: String,
        kind: &'static str,
    },
    #[error("plugin manifest declares invalid runtime limit {0}; value must be greater than zero")]
    InvalidRuntimeLimit(&'static str),
    #[error("plugin manifest declares invalid declarative binding for {id}: {reason}")]
    InvalidDeclarativeBinding { id: String, reason: String },
    #[error("plugin manifest declares invalid realtime capability for {id}: {reason}")]
    InvalidRealtimeCapability { id: String, reason: String },
    #[error("plugin ability {ability:?} control-plane registration failed: {reason}")]
    ControlPlaneRegistrationFailed { ability: String, reason: String },
    #[error("plugin contribution for {package} ability {ability:?} is invalid: {reason}")]
    InvalidContribution {
        package: String,
        ability: String,
        reason: String,
    },
    #[error(
        "plugin manifest entrypoint {declared:?} does not match compiled binding {expected:?}"
    )]
    EntrypointMismatch {
        declared: String,
        expected: &'static str,
    },
    #[error("plugin package entrypoint is not executable: {path}")]
    EntrypointNotExecutable { path: PathBuf },
    #[error("plugin manifest does not match compiled builtin spec for {id}: {reason}")]
    BuiltinSpecMismatch { id: String, reason: String },
    #[error("default plugin package index is unavailable: {0}")]
    DefaultIndexUnavailable(String),
    #[error("parse plugin manifest {path}: {source}")]
    ManifestParseFailed {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("parse plugin ability descriptor {path}: {source}")]
    DescriptorParseFailed {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("plugin ability descriptor {path} is invalid: {reason}")]
    InvalidAbilityDescriptor { path: PathBuf, reason: String },
    #[error("plugin package path {path} escapes package root {root}")]
    PackagePathEscapesRoot { root: PathBuf, path: PathBuf },
    #[error(
        "plugin ability descriptor for {ability:?} cannot be projected into registry manifest: {reason}"
    )]
    DescriptorProjectionFailed { ability: String, reason: String },
    #[error("read plugin package path {path}: {source}")]
    ReadFailed {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("write plugin package path {path}: {source}")]
    WriteFailed {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("plugin package hash mismatch for {id}: expected {expected}, actual {actual}")]
    HashMismatch {
        id: String,
        expected: String,
        actual: String,
    },
    #[error("plugin package {0} has no compiled builtin binding")]
    MissingBuiltinBinding(String),
    #[error("plugin package {0} is already installed")]
    PackageAlreadyInstalled(String),
    #[error("plugin package {0} is not installed")]
    PackageNotInstalled(String),
    #[error("spawn sidecar plugin process {program}: {source}")]
    SidecarSpawnFailed {
        program: PathBuf,
        source: std::io::Error,
    },
    #[error("sidecar plugin process {program} did not expose stdin")]
    SidecarStdinUnavailable { program: PathBuf },
    #[error("sidecar plugin process {program} did not expose stdout")]
    SidecarStdoutUnavailable { program: PathBuf },
    #[error("encode sidecar JSON frame: {source}")]
    SidecarFrameEncodeFailed { source: serde_json::Error },
    #[error("decode sidecar JSON frame: {source}")]
    SidecarFrameDecodeFailed { source: serde_json::Error },
    #[error("sidecar protocol violation: {message}")]
    SidecarProtocolViolation { message: String },
    #[error("sidecar plugin process {program} exited with {status}; stderr={stderr}")]
    SidecarProcessFailed {
        program: PathBuf,
        status: std::process::ExitStatus,
        stderr: String,
    },
    #[error("sidecar plugin process {program} timed out after {timeout_ms} ms; stderr={stderr}")]
    SidecarProcessTimedOut {
        program: PathBuf,
        timeout_ms: u64,
        stderr: String,
    },
}

impl PartialEq for PluginHostError {
    fn eq(&self, other: &Self) -> bool {
        use PluginHostError::*;
        match (self, other) {
            (UnsupportedSchema(a), UnsupportedSchema(b)) => a == b,
            (MissingField(a), MissingField(b)) => a == b,
            (MissingAbilityPattern, MissingAbilityPattern) => true,
            (MultipleAbilityPatterns(a), MultipleAbilityPatterns(b)) => a == b,
            (UnsupportedAbilityPattern(a), UnsupportedAbilityPattern(b)) => a == b,
            (UnsupportedManifestPath(a), UnsupportedManifestPath(b)) => a == b,
            (MissingAbilityMetadata, MissingAbilityMetadata) => true,
            (InvalidAbilityName(a), InvalidAbilityName(b)) => a == b,
            (DuplicateAbility(a), DuplicateAbility(b)) => a == b,
            (
                DuplicatePackageId {
                    id: ai,
                    first_version: af,
                    second_version: as_,
                },
                DuplicatePackageId {
                    id: bi,
                    first_version: bf,
                    second_version: bs,
                },
            ) => ai == bi && af == bf && as_ == bs,
            (
                DuplicatePackageVersion {
                    id: ai,
                    version: av,
                },
                DuplicatePackageVersion {
                    id: bi,
                    version: bv,
                },
            ) => ai == bi && av == bv,
            (
                DuplicateAbilityOwner {
                    ability: aa,
                    first: af,
                    second: as_,
                },
                DuplicateAbilityOwner {
                    ability: ba,
                    first: bf,
                    second: bs,
                },
            ) => aa == ba && af == bf && as_ == bs,
            (
                InstallKindNotLoadableInThisRelease {
                    id: ai,
                    version: av,
                    kind: ak,
                },
                InstallKindNotLoadableInThisRelease {
                    id: bi,
                    version: bv,
                    kind: bk,
                },
            ) => ai == bi && av == bv && ak == bk,
            (InvalidRuntimeLimit(a), InvalidRuntimeLimit(b)) => a == b,
            (
                InvalidDeclarativeBinding { id: ai, reason: ar },
                InvalidDeclarativeBinding { id: bi, reason: br },
            ) => ai == bi && ar == br,
            (
                InvalidRealtimeCapability { id: ai, reason: ar },
                InvalidRealtimeCapability { id: bi, reason: br },
            ) => ai == bi && ar == br,
            (
                InvalidContribution {
                    package: ap,
                    ability: aa,
                    reason: ar,
                },
                InvalidContribution {
                    package: bp,
                    ability: ba,
                    reason: br,
                },
            ) => ap == bp && aa == ba && ar == br,
            (
                EntrypointMismatch {
                    declared: ad,
                    expected: ae,
                },
                EntrypointMismatch {
                    declared: bd,
                    expected: be,
                },
            ) => ad == bd && ae == be,
            (EntrypointNotExecutable { path: a }, EntrypointNotExecutable { path: b }) => a == b,
            (
                BuiltinSpecMismatch { id: ai, reason: ar },
                BuiltinSpecMismatch { id: bi, reason: br },
            ) => ai == bi && ar == br,
            (DefaultIndexUnavailable(a), DefaultIndexUnavailable(b)) => a == b,
            (
                InvalidAbilityDescriptor {
                    path: ap,
                    reason: ar,
                },
                InvalidAbilityDescriptor {
                    path: bp,
                    reason: br,
                },
            ) => ap == bp && ar == br,
            (
                PackagePathEscapesRoot { root: ar, path: ap },
                PackagePathEscapesRoot { root: br, path: bp },
            ) => ar == br && ap == bp,
            (
                DescriptorProjectionFailed {
                    ability: aa,
                    reason: ar,
                },
                DescriptorProjectionFailed {
                    ability: ba,
                    reason: br,
                },
            ) => aa == ba && ar == br,
            (
                HashMismatch {
                    id: ai,
                    expected: ae,
                    actual: aa,
                },
                HashMismatch {
                    id: bi,
                    expected: be,
                    actual: ba,
                },
            ) => ai == bi && ae == be && aa == ba,
            (MissingBuiltinBinding(a), MissingBuiltinBinding(b)) => a == b,
            (PackageAlreadyInstalled(a), PackageAlreadyInstalled(b)) => a == b,
            (PackageNotInstalled(a), PackageNotInstalled(b)) => a == b,
            (SidecarStdinUnavailable { program: a }, SidecarStdinUnavailable { program: b }) => {
                a == b
            }
            (SidecarStdoutUnavailable { program: a }, SidecarStdoutUnavailable { program: b }) => {
                a == b
            }
            (SidecarProtocolViolation { message: a }, SidecarProtocolViolation { message: b }) => {
                a == b
            }
            (
                SidecarProcessFailed {
                    program: ap,
                    status: as_,
                    stderr: ae,
                },
                SidecarProcessFailed {
                    program: bp,
                    status: bs,
                    stderr: be,
                },
            ) => ap == bp && as_ == bs && ae == be,
            (
                SidecarProcessTimedOut {
                    program: ap,
                    timeout_ms: at,
                    stderr: ae,
                },
                SidecarProcessTimedOut {
                    program: bp,
                    timeout_ms: bt,
                    stderr: be,
                },
            ) => ap == bp && at == bt && ae == be,
            _ => false,
        }
    }
}

impl Eq for PluginHostError {}

/// Plugin host result alias.
pub type Result<T> = std::result::Result<T, PluginHostError>;
