// EasyNet CLI — plugin install transactions
// =========================================
//
// File: src/runtime/plugin_host/install.rs
// Description: Transactional install/update/remove core for local plugins.

mod state;
mod transaction;
mod validation;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::runtime::plugin_host::errors::{PluginHostError, Result};
use crate::runtime::plugin_host::index::PluginPackageIndex;
use crate::runtime::plugin_host::install::state::PluginStateStore;
pub use crate::runtime::plugin_host::install::state::{InstalledPluginRecord, PluginStateToml};
use crate::runtime::plugin_host::install::transaction::{copy_tree, rollback_dir, txn_dir};
use crate::runtime::plugin_host::install::validation::validate_installable_in_this_release;
use crate::runtime::plugin_host::package::PluginPackage;

const INSTALLED_DIR: &str = "installed";

/// Transactional plugin installer rooted at `~/.easynet/plugins`.
pub struct PluginInstaller {
    root: PathBuf,
}

impl PluginInstaller {
    /// Construct an installer for a plugin root.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Install a package from an unpacked source directory.
    pub fn install(&self, source: &Path) -> Result<InstalledPluginRecord> {
        let txn = txn_dir(&self.root, "install")?;
        copy_tree(source, &txn)?;
        let package = PluginPackage::from_installed(&txn, None)?;
        if let Err(err) = validate_installable_in_this_release(&package) {
            let _ = fs::remove_dir_all(&txn);
            return Err(err);
        }
        if let Err(err) = self.ensure_active_index_accepts(&package) {
            let _ = fs::remove_dir_all(&txn);
            return Err(err);
        }
        let target = self.package_dir(package.manifest().id(), package.manifest().version());
        if target.exists() {
            let _ = fs::remove_dir_all(&txn);
            return Err(PluginHostError::PackageAlreadyInstalled(format!(
                "{}@{}",
                package.manifest().id(),
                package.manifest().version()
            )));
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|source| PluginHostError::WriteFailed {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        fs::rename(&txn, &target).map_err(|source| PluginHostError::WriteFailed {
            path: target.clone(),
            source,
        })?;
        let installed =
            InstalledPluginRecord::from_package(&PluginPackage::from_installed(&target, None)?);
        if let Err(err) = self.commit_record(installed.clone()) {
            let _ = fs::remove_dir_all(&target);
            return Err(err);
        }
        Ok(installed)
    }

    /// Update a package by installing a new unpacked version and preserving the
    /// previous package directory until state and lock files commit.
    pub fn update(&self, source: &Path) -> Result<InstalledPluginRecord> {
        let txn = txn_dir(&self.root, "update")?;
        copy_tree(source, &txn)?;
        let package = PluginPackage::from_installed(&txn, None)?;
        if let Err(err) = validate_installable_in_this_release(&package) {
            let _ = fs::remove_dir_all(&txn);
            return Err(err);
        }
        if let Err(err) = self.ensure_active_index_accepts(&package) {
            let _ = fs::remove_dir_all(&txn);
            return Err(err);
        }
        let id = package.manifest().id().to_string();
        let target = self.package_dir(&id, package.manifest().version());
        let previous_state = self.read_state()?;
        let previous_record = previous_state
            .plugins
            .iter()
            .find(|record| record.id == id)
            .cloned();
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|source| PluginHostError::WriteFailed {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let mut rollback_backup = None;
        if let Some(record) = &previous_record {
            let prior = self.package_dir(&id, &record.version);
            if prior.exists() {
                let backup = rollback_dir(&self.root, &id, &record.version)?;
                fs::rename(&prior, &backup).map_err(|source| PluginHostError::WriteFailed {
                    path: backup.clone(),
                    source,
                })?;
                rollback_backup = Some((prior, backup));
            }
        }
        if target.exists() {
            fs::remove_dir_all(&target).map_err(|source| PluginHostError::WriteFailed {
                path: target.clone(),
                source,
            })?;
        }
        fs::rename(&txn, &target).map_err(|source| PluginHostError::WriteFailed {
            path: target.clone(),
            source,
        })?;
        let installed =
            InstalledPluginRecord::from_package(&PluginPackage::from_installed(&target, None)?);
        if let Err(err) = self.commit_record(installed.clone()) {
            let _ = fs::remove_dir_all(&target);
            if let Some((prior, backup)) = rollback_backup {
                if backup.exists() {
                    let _ = fs::rename(&backup, &prior);
                }
            }
            return Err(err);
        }
        if let Some((_, backup)) = rollback_backup {
            let _ = fs::remove_dir_all(backup);
        }
        Ok(installed)
    }

    /// Remove an installed package version.
    pub fn remove(&self, id: &str, version: &str) -> Result<()> {
        let target = self.package_dir(id, version);
        if !target.exists() {
            return Err(PluginHostError::PackageNotInstalled(format!(
                "{id}@{version}"
            )));
        }
        let previous_state = self.read_state()?;
        self.remove_record(id, version)?;
        let backup = rollback_dir(&self.root, id, version)?;
        if let Err(err) = fs::rename(&target, &backup) {
            let _ = self.write_state(&previous_state);
            return Err(PluginHostError::WriteFailed {
                path: backup,
                source: err,
            });
        }
        if let Err(err) = fs::remove_dir_all(&backup) {
            let _ = fs::rename(&backup, &target);
            let _ = self.write_state(&previous_state);
            return Err(PluginHostError::WriteFailed {
                path: backup,
                source: err,
            });
        }
        Ok(())
    }

    fn commit_record(&self, record: InstalledPluginRecord) -> Result<()> {
        let mut state = self.read_state()?;
        state.plugins.retain(|existing| existing.id != record.id);
        state.plugins.push(record);
        state.plugins.sort_by(|a, b| a.id.cmp(&b.id));
        self.write_state(&state)
    }

    fn remove_record(&self, id: &str, version: &str) -> Result<()> {
        let mut state = self.read_state()?;
        state
            .plugins
            .retain(|existing| !(existing.id == id && existing.version == version));
        self.write_state(&state)
    }

    fn ensure_active_index_accepts(&self, package: &PluginPackage) -> Result<()> {
        let builtin = PluginPackageIndex::builtin()?;
        let active = PluginPackageIndex::installed(&self.root)?;
        let mut packages = builtin.packages().to_vec();
        packages.extend(
            active
                .packages()
                .iter()
                .filter(|existing| existing.id().as_str() != package.id().as_str())
                .cloned(),
        );
        packages.push(Arc::new(package.clone()));
        PluginPackageIndex::from_packages(packages).map(|_| ())
    }

    fn read_state(&self) -> Result<PluginStateToml> {
        PluginStateStore::new(&self.root).read()
    }

    fn write_state(&self, state: &PluginStateToml) -> Result<()> {
        PluginStateStore::new(&self.root).write(state)
    }

    fn package_dir(&self, id: &str, version: &str) -> PathBuf {
        self.root.join(INSTALLED_DIR).join(id).join(version)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::plugin_host::package::tests::write_test_package;

    #[test]
    fn plugin_host_install_rejects_not_loadable_declarative_packages() {
        let root = tempfile::tempdir().expect("root");
        let source_v1 = tempfile::tempdir().expect("source v1");
        write_test_package(source_v1.path(), "0.1.0");
        let installer = PluginInstaller::new(root.path());

        let err = installer
            .install(source_v1.path())
            .expect_err("unsupported package kind must not be installed");
        assert!(matches!(
            err,
            PluginHostError::InstallKindNotLoadableInThisRelease { .. }
        ));
        assert!(
            !root.path().join("installed/test.plugin/0.1.0").exists(),
            "rejected install must not create an active package directory"
        );
    }

    #[test]
    fn plugin_host_install_accepts_exec_declarative_packages() {
        let root = tempfile::tempdir().expect("root");
        let source_v1 = tempfile::tempdir().expect("source v1");
        write_exec_declarative_test_package(source_v1.path(), "0.1.0");
        let installer = PluginInstaller::new(root.path());

        let installed = installer
            .install(source_v1.path())
            .expect("exec declarative install");

        assert_eq!(installed.id, "test.declarative");
        assert_eq!(installed.version, "0.1.0");
        assert!(root
            .path()
            .join("installed/test.declarative/0.1.0/bin/exec-plugin")
            .exists());
    }

    #[test]
    fn plugin_host_install_accepts_eal_and_mcp_declarative_rpc_packages() {
        let root = tempfile::tempdir().expect("root");
        let installer = PluginInstaller::new(root.path());

        let eal_source = tempfile::tempdir().expect("eal source");
        write_eal_declarative_test_package(eal_source.path(), "0.1.0");
        let eal = installer
            .install(eal_source.path())
            .expect("eal declarative install");
        assert_eq!(eal.id, "test.declarative.eal");
        assert!(root
            .path()
            .join("installed/test.declarative.eal/0.1.0/plugin.toml")
            .exists());

        let mcp_source = tempfile::tempdir().expect("mcp source");
        write_mcp_declarative_test_package(mcp_source.path(), "0.1.0");
        let mcp = installer
            .install(mcp_source.path())
            .expect("mcp declarative install");
        assert_eq!(mcp.id, "test.declarative.mcp");
        assert!(root
            .path()
            .join("installed/test.declarative.mcp/0.1.0/plugin.toml")
            .exists());
    }

    #[test]
    fn plugin_host_install_update_remove_are_transactional_for_sidecar_packages() {
        let root = tempfile::tempdir().expect("root");
        let source_v1 = tempfile::tempdir().expect("source v1");
        write_sidecar_test_package(source_v1.path(), "0.1.0");
        let installer = PluginInstaller::new(root.path());

        let installed = installer.install(source_v1.path()).expect("install");
        assert_eq!(installed.id, "test.sidecar");
        assert_eq!(installed.version, "0.1.0");
        assert!(root
            .path()
            .join("installed/test.sidecar/0.1.0/plugin.toml")
            .exists());

        let source_v2 = tempfile::tempdir().expect("source v2");
        write_sidecar_test_package(source_v2.path(), "0.2.0");
        let updated = installer.update(source_v2.path()).expect("update");
        assert_eq!(updated.version, "0.2.0");
        assert!(root
            .path()
            .join("installed/test.sidecar/0.2.0/plugin.toml")
            .exists());
        assert!(
            !root.path().join("installed/test.sidecar/0.1.0").exists(),
            "successful update must retire the previous active version directory"
        );

        installer.remove("test.sidecar", "0.2.0").expect("remove");
        assert!(!root.path().join("installed/test.sidecar/0.2.0").exists());
        let state = std::fs::read_to_string(root.path().join("state/plugins.toml"))
            .expect("state file exists");
        assert!(
            !state.contains("0.2.0"),
            "removed package version must leave state"
        );
    }

    #[test]
    fn plugin_host_install_rolls_back_directory_when_state_commit_fails() {
        let root = tempfile::tempdir().expect("root");
        let source_v1 = tempfile::tempdir().expect("source v1");
        write_sidecar_test_package(source_v1.path(), "0.1.0");
        std::fs::create_dir_all(root.path().join("state/plugins.toml")).expect("poison state path");
        let installer = PluginInstaller::new(root.path());

        let err = installer
            .install(source_v1.path())
            .expect_err("state commit failure must abort install");

        assert!(matches!(err, PluginHostError::ReadFailed { .. }));
        assert!(
            !root.path().join("installed/test.sidecar/0.1.0").exists(),
            "failed state commit must remove the activated package directory"
        );
    }

    #[test]
    fn plugin_host_install_rejects_active_ability_collision() {
        let root = tempfile::tempdir().expect("root");
        let source_v1 = tempfile::tempdir().expect("source v1");
        write_sidecar_test_package_with_id(source_v1.path(), "test.sidecar", "0.1.0", "test.echo");
        let installer = PluginInstaller::new(root.path());
        installer.install(source_v1.path()).expect("first install");

        let source_v2 = tempfile::tempdir().expect("source v2");
        write_sidecar_test_package_with_id(source_v2.path(), "test.sidecar2", "0.1.0", "test.echo");
        let err = installer
            .install(source_v2.path())
            .expect_err("duplicate ability owner must be rejected before commit");
        assert!(matches!(err, PluginHostError::DuplicateAbilityOwner { .. }));
        assert!(
            !root.path().join("installed/test.sidecar2/0.1.0").exists(),
            "rejected package must not be activated"
        );
    }

    fn write_sidecar_test_package(root: &Path, version: &str) {
        write_sidecar_test_package_with_id(root, "test.sidecar", version, "test.echo");
    }

    fn write_sidecar_test_package_with_id(root: &Path, id: &str, version: &str, ability: &str) {
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
            test_descriptor(ability),
        )
        .expect("descriptor");
        write_executable(root.join("bin/echo-sidecar"), "#!/bin/sh\n");
    }

    fn write_exec_declarative_test_package(root: &Path, version: &str) {
        std::fs::create_dir_all(root.join("abilities")).expect("abilities dir");
        std::fs::create_dir_all(root.join("bin")).expect("bin dir");
        std::fs::write(
            root.join("plugin.toml"),
            format!(
                r#"
schema_version = "1"
id = "test.declarative"
version = "{version}"
kind = "declarative"
entrypoint = "declarative.exec"
abilities = ["abilities/*.ability.toml"]
permissions = []
resources = []
platforms = []

[limits]
max_sessions = 1
max_frame_queue = 1

[declarative]
kind = "exec"
argv = ["bin/exec-plugin"]

[[ability_metadata]]
name = "test.declarative_echo"
layer = "control"
"#
            ),
        )
        .expect("manifest");
        std::fs::write(
            root.join("abilities/test.declarative_echo.ability.toml"),
            test_descriptor("test.declarative_echo"),
        )
        .expect("descriptor");
        write_executable(root.join("bin/exec-plugin"), "#!/bin/sh\n");
    }

    fn write_eal_declarative_test_package(root: &Path, version: &str) {
        std::fs::create_dir_all(root.join("abilities")).expect("abilities dir");
        std::fs::write(
            root.join("plugin.toml"),
            format!(
                r#"
schema_version = "1"
id = "test.declarative.eal"
version = "{version}"
kind = "declarative"
entrypoint = "declarative.eal"
abilities = ["abilities/*.ability.toml"]
permissions = []
resources = []
platforms = []

[limits]
max_sessions = 1
max_frame_queue = 1

[declarative]
kind = "eal"
program = "mission \"noop\" {{}}"

[[ability_metadata]]
name = "test.declarative_eal"
layer = "control"
"#
            ),
        )
        .expect("manifest");
        std::fs::write(
            root.join("abilities/test.declarative_eal.ability.toml"),
            test_descriptor("test.declarative_eal"),
        )
        .expect("descriptor");
    }

    fn write_mcp_declarative_test_package(root: &Path, version: &str) {
        std::fs::create_dir_all(root.join("abilities")).expect("abilities dir");
        std::fs::write(
            root.join("plugin.toml"),
            format!(
                r#"
schema_version = "1"
id = "test.declarative.mcp"
version = "{version}"
kind = "declarative"
entrypoint = "declarative.mcp"
abilities = ["abilities/*.ability.toml"]
permissions = []
resources = []
platforms = []

[limits]
max_sessions = 1
max_frame_queue = 1

[declarative]
kind = "mcp"
server = "test-server"
tool = "test-tool"

[[ability_metadata]]
name = "test.declarative_mcp"
layer = "control"
"#
            ),
        )
        .expect("manifest");
        std::fs::write(
            root.join("abilities/test.declarative_mcp.ability.toml"),
            test_descriptor("test.declarative_mcp"),
        )
        .expect("descriptor");
    }

    fn write_executable(path: impl AsRef<Path>, body: &str) {
        let path = path.as_ref();
        std::fs::write(path, body).expect("executable");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let mut permissions = std::fs::metadata(path)
                .expect("executable metadata")
                .permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(path, permissions).expect("chmod executable");
        }
    }

    fn test_descriptor(ability: &str) -> String {
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
