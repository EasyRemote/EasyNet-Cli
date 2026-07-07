// EasyNet CLI — plugin install validation
// =======================================
//
// File: src/daemon/plugins/install/validation.rs
// Description: Release-supported package validation before activation.

use std::path::Path;

use crate::daemon::plugins::errors::{PluginHostError, Result};
use crate::daemon::plugins::manifest::{PluginCallMode, PluginDeclarativeBinding, PluginKind};
use crate::daemon::plugins::package::PluginPackage;

/// Validate that a package kind can be installed by this release.
///
/// What this is NOT: load planning. Platform mismatch and disabled env state
/// are boot-time load-plan concerns; installation validates package shape and
/// entrypoint consistency only.
pub(super) fn validate_installable_in_this_release(package: &PluginPackage) -> Result<()> {
    let manifest = package.manifest();
    match manifest.kind() {
        PluginKind::Sidecar => {
            validate_sidecar_entrypoint(package)?;
            Ok(())
        }
        PluginKind::Declarative => validate_declarative_installable(package),
        PluginKind::DesktopCompanion => validate_companion_installable(package),
        other => Err(PluginHostError::InstallKindNotLoadableInThisRelease {
            id: manifest.id().to_string(),
            version: manifest.version().to_string(),
            kind: plugin_kind_label(other),
        }),
    }
}

fn validate_companion_installable(package: &PluginPackage) -> Result<()> {
    let manifest = package.manifest();
    let companion =
        manifest
            .companion()
            .ok_or_else(|| PluginHostError::InvalidCompanionManifest {
                id: manifest.id().to_string(),
                reason: "desktop_companion packages must declare [companion]".to_string(),
            })?;
    if let Some(macos) = companion.macos() {
        validate_package_artifact(package, macos.app_bundle())?;
    }
    if let Some(windows) = companion.windows() {
        validate_package_artifact(package, windows.exe())?;
    }
    if let Some(linux) = companion.linux() {
        validate_package_artifact(package, linux.exe())?;
    }
    Ok(())
}

fn validate_package_artifact(package: &PluginPackage, artifact: &str) -> Result<()> {
    let path = package.root().join(artifact);
    if !path.exists() {
        return Err(PluginHostError::ReadFailed {
            path,
            source: std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "declared companion artifact does not exist",
            ),
        });
    }
    Ok(())
}

fn validate_declarative_installable(package: &PluginPackage) -> Result<()> {
    let manifest = package.manifest();
    match manifest.declarative_binding() {
        Some(PluginDeclarativeBinding::Exec { argv }) => {
            validate_package_executable(package, argv.first().map(String::as_str))?;
            Ok(())
        }
        Some(PluginDeclarativeBinding::Eal { .. }) => validate_declarative_rpc_only(package, "eal"),
        Some(PluginDeclarativeBinding::Mcp { .. }) => validate_declarative_rpc_only(package, "mcp"),
        None => Err(PluginHostError::InstallKindNotLoadableInThisRelease {
            id: manifest.id().to_string(),
            version: manifest.version().to_string(),
            kind: "declarative",
        }),
    }
}

fn validate_declarative_rpc_only(package: &PluginPackage, kind: &'static str) -> Result<()> {
    let manifest = package.manifest();
    if manifest
        .abilities()
        .iter()
        .all(|ability| ability.call_mode() == PluginCallMode::Rpc)
    {
        Ok(())
    } else {
        Err(PluginHostError::InstallKindNotLoadableInThisRelease {
            id: manifest.id().to_string(),
            version: manifest.version().to_string(),
            kind,
        })
    }
}

fn validate_sidecar_entrypoint(package: &PluginPackage) -> Result<()> {
    validate_resolved_executable(package.entrypoint_path())
}

fn validate_package_executable(package: &PluginPackage, executable: Option<&str>) -> Result<()> {
    let executable = executable.ok_or_else(|| PluginHostError::InvalidDeclarativeBinding {
        id: package.manifest().id().to_string(),
        reason: "exec binding must declare argv[0]".to_string(),
    })?;
    let entrypoint = Path::new(executable);
    let path = if entrypoint.is_absolute() {
        entrypoint.to_path_buf()
    } else {
        package.root().join(entrypoint)
    };
    validate_resolved_executable(path)
}

fn validate_resolved_executable(path: std::path::PathBuf) -> Result<()> {
    let meta = std::fs::metadata(&path).map_err(|source| PluginHostError::ReadFailed {
        path: path.clone(),
        source,
    })?;
    if !meta.is_file() {
        return Err(PluginHostError::EntrypointNotExecutable { path });
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        if meta.permissions().mode() & 0o111 == 0 {
            return Err(PluginHostError::EntrypointNotExecutable { path });
        }
    }
    Ok(())
}

fn plugin_kind_label(kind: PluginKind) -> &'static str {
    match kind {
        PluginKind::Declarative => "declarative",
        PluginKind::Sidecar => "sidecar",
        PluginKind::Builtin => "builtin",
        PluginKind::DesktopCompanion => "desktop_companion",
    }
}
