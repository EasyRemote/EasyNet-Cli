// EasyNet CLI — plugin install transactions
// =========================================
//
// File: src/daemon/plugins/install.rs
// Description: Transactional install/update/remove core for local plugins.

mod state;
mod transaction;
mod validation;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::daemon::plugins::companion::DesktopCompanionManager;
use crate::daemon::plugins::errors::{PluginHostError, Result};
use crate::daemon::plugins::index::PluginPackageIndex;
use crate::daemon::plugins::install::state::PluginStateStore;
pub use crate::daemon::plugins::install::state::{InstalledPluginRecord, PluginStateToml};
use crate::daemon::plugins::install::transaction::{copy_tree, rollback_dir, txn_dir};
use crate::daemon::plugins::install::validation::validate_installable_in_this_release;
use crate::daemon::plugins::manifest::PluginKind;
use crate::daemon::plugins::package::PluginPackage;

const INSTALLED_DIR: &str = "installed";

/// Transactional plugin installer rooted at `~/.easynet/plugins`.
pub struct PluginInstaller {
    root: PathBuf,
}

struct CompanionUpdateRollback<'a> {
    target: &'a Path,
    previous_state: &'a PluginStateToml,
    rollback_backup: Option<&'a (PathBuf, PathBuf)>,
    installed_package: &'a Arc<PluginPackage>,
    previous_package: Option<&'a Arc<PluginPackage>>,
    previous_companion_status:
        Option<&'a crate::daemon::plugins::companion::DesktopCompanionStatus>,
    companion_manager: &'a DesktopCompanionManager,
}

impl PluginInstaller {
    /// Construct an installer for a plugin root.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Install a package from an unpacked source directory.
    pub fn install(&self, source: &Path) -> Result<InstalledPluginRecord> {
        self.install_with_companion_commit(source, None)
    }

    /// Install a package and commit companion supervisor artifacts when the
    /// package kind participates in the desktop companion lifecycle.
    pub fn install_with_companion_manager(
        &self,
        source: &Path,
        companion_manager: &DesktopCompanionManager,
    ) -> Result<InstalledPluginRecord> {
        self.install_with_companion_commit(source, Some(companion_manager))
    }

    fn install_with_companion_commit(
        &self,
        source: &Path,
        companion_manager: Option<&DesktopCompanionManager>,
    ) -> Result<InstalledPluginRecord> {
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
        let installed_package = PluginPackage::from_installed(&target, None)?;
        let installed = InstalledPluginRecord::from_package(&installed_package);
        if let Err(err) = self.commit_record(installed.clone()) {
            let _ = fs::remove_dir_all(&target);
            return Err(err);
        }
        if let Some(companion_manager) = companion_manager {
            if installed_package.manifest().kind() == PluginKind::DesktopCompanion {
                let shared_package = Arc::new(installed_package);
                if let Err(err) = companion_manager.commit_package_install(&shared_package) {
                    let install_error = err.to_string();
                    if let Err(rollback_error) = self.rollback_failed_companion_install(
                        &target,
                        &installed,
                        &shared_package,
                        companion_manager,
                    ) {
                        return Err(PluginHostError::CompanionInstallRollbackFailed {
                            id: installed.id,
                            version: installed.version,
                            install_error,
                            rollback_error,
                            stale_path: target,
                        });
                    }
                    return Err(err);
                }
            }
        }
        Ok(installed)
    }

    /// Update a package by installing a new unpacked version and preserving the
    /// previous package directory until state and lock files commit.
    pub fn update(&self, source: &Path) -> Result<InstalledPluginRecord> {
        self.update_with_companion_commit(source, None)
    }

    /// Update a package and commit companion supervisor artifacts when the
    /// package kind participates in the desktop companion lifecycle.
    pub fn update_with_companion_manager(
        &self,
        source: &Path,
        companion_manager: &DesktopCompanionManager,
    ) -> Result<InstalledPluginRecord> {
        self.update_with_companion_commit(source, Some(companion_manager))
    }

    fn update_with_companion_commit(
        &self,
        source: &Path,
        companion_manager: Option<&DesktopCompanionManager>,
    ) -> Result<InstalledPluginRecord> {
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
        let previous_package = previous_record.as_ref().and_then(|record| {
            PluginPackage::from_installed(
                &self.package_dir(&id, &record.version),
                Some(&record.hash),
            )
            .ok()
            .map(Arc::new)
        });
        let previous_companion_status = match (companion_manager, previous_package.as_ref()) {
            (Some(manager), Some(previous))
                if previous.manifest().kind() == PluginKind::DesktopCompanion =>
            {
                manager.status_for_package(previous).ok()
            }
            _ => None,
        };
        let executable_artifact_changed = match (companion_manager, previous_package.as_ref()) {
            (Some(manager), Some(previous))
                if previous.manifest().kind() == PluginKind::DesktopCompanion
                    && package.manifest().kind() == PluginKind::DesktopCompanion =>
            {
                manager.executable_artifact_changed(previous, &Arc::new(package.clone()))?
            }
            (Some(_), None) if package.manifest().kind() == PluginKind::DesktopCompanion => true,
            _ => false,
        };
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
        let installed_package = Arc::new(PluginPackage::from_installed(&target, None)?);
        let installed = InstalledPluginRecord::from_package(&installed_package);
        if let Err(err) = self.commit_record(installed.clone()) {
            let _ = fs::remove_dir_all(&target);
            if let Some((prior, backup)) = rollback_backup {
                if backup.exists() {
                    let _ = fs::rename(&backup, &prior);
                }
            }
            return Err(err);
        }
        if let Some(companion_manager) = companion_manager {
            if installed_package.manifest().kind() == PluginKind::DesktopCompanion {
                if let Err(err) = companion_manager.commit_package_update(
                    &installed_package,
                    previous_companion_status.as_ref(),
                    executable_artifact_changed,
                ) {
                    let update_error = err.to_string();
                    let rollback_error =
                        self.rollback_failed_companion_update(CompanionUpdateRollback {
                            target: &target,
                            previous_state: &previous_state,
                            rollback_backup: rollback_backup.as_ref(),
                            installed_package: &installed_package,
                            previous_package: previous_package.as_ref(),
                            previous_companion_status: previous_companion_status.as_ref(),
                            companion_manager,
                        });
                    if let Err(rollback_error) = rollback_error {
                        return Err(PluginHostError::CompanionUpdateRollbackFailed {
                            id: installed.id,
                            version: installed.version,
                            update_error,
                            rollback_error,
                            stale_path: target,
                        });
                    }
                    return Err(err);
                }
            }
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
        let backup = rollback_dir(&self.root, id, version)?;
        self.remove_record(id, version)?;
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

    /// Remove an installed package version and clean desktop companion
    /// supervisor state under the same transaction owner.
    pub fn remove_with_companion_manager(
        &self,
        id: &str,
        version: &str,
        companion_manager: &DesktopCompanionManager,
    ) -> Result<()> {
        let target = self.package_dir(id, version);
        if !target.exists() {
            return Err(PluginHostError::PackageNotInstalled(format!(
                "{id}@{version}"
            )));
        }
        let package = Arc::new(PluginPackage::from_installed(&target, None)?);
        if package.manifest().kind() != PluginKind::DesktopCompanion {
            return self.remove(id, version);
        }
        let previous_status = companion_manager.status_for_package(&package).ok();
        companion_manager.remove(&package)?;
        if let Err(err) = self.remove(id, version) {
            let remove_error = err.to_string();
            if let Some(previous_status) = previous_status.as_ref() {
                if let Err(rollback_err) =
                    companion_manager.restore_package_after_failed_update(&package, previous_status)
                {
                    return Err(PluginHostError::CompanionRemoveRollbackFailed {
                        id: id.to_string(),
                        version: version.to_string(),
                        remove_error,
                        rollback_error: rollback_err.to_string(),
                        stale_path: target,
                    });
                }
            }
            return Err(err);
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

    fn rollback_failed_companion_install(
        &self,
        target: &Path,
        installed: &InstalledPluginRecord,
        package: &Arc<PluginPackage>,
        companion_manager: &DesktopCompanionManager,
    ) -> std::result::Result<(), String> {
        let mut failures = Vec::new();
        if let Err(err) = companion_manager.remove(package) {
            failures.push(format!("companion_remove={err}"));
        }
        if let Err(err) = self.remove_record(&installed.id, &installed.version) {
            failures.push(format!("state_remove={err}"));
        }
        if target.exists() {
            if let Err(err) = fs::remove_dir_all(target) {
                failures.push(format!("package_remove={err}"));
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(failures.join("; "))
        }
    }

    fn rollback_failed_companion_update(
        &self,
        rollback: CompanionUpdateRollback<'_>,
    ) -> std::result::Result<(), String> {
        let CompanionUpdateRollback {
            target,
            previous_state,
            rollback_backup,
            installed_package,
            previous_package,
            previous_companion_status,
            companion_manager,
        } = rollback;
        let mut failures = Vec::new();
        if let Err(err) = companion_manager.remove(installed_package) {
            failures.push(format!("companion_remove_new={err}"));
        }
        if target.exists() {
            if let Err(err) = fs::remove_dir_all(target) {
                failures.push(format!("package_remove_new={err}"));
            }
        }
        if let Some((prior, backup)) = rollback_backup {
            if backup.exists() {
                if let Err(err) = fs::rename(backup, prior) {
                    failures.push(format!("package_restore_previous={err}"));
                }
            }
        }
        if let Err(err) = self.write_state(previous_state) {
            failures.push(format!("state_restore={err}"));
        }
        if let (Some(previous_package), Some(previous_status)) =
            (previous_package, previous_companion_status)
        {
            if let Err(err) = companion_manager
                .restore_package_after_failed_update(previous_package, previous_status)
            {
                failures.push(format!("companion_restore_previous={err}"));
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(failures.join("; "))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use crate::daemon::plugins::companion::{
        CompanionActionReport, CompanionObservation, CompanionObservedState,
        CompanionSessionStatus, CompanionSupervisorState, DesktopCompanionPlan,
        DesktopCompanionPlanner, DesktopCompanionStateStore, DesktopCompanionSupervisor,
    };
    use crate::daemon::plugins::package::tests::write_test_package;

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

    #[test]
    fn plugin_host_install_accepts_desktop_companion_package() {
        let root = tempfile::tempdir().expect("root");
        let source = tempfile::tempdir().expect("source");
        write_companion_test_package(source.path(), true);
        let installer = PluginInstaller::new(root.path());

        let record = installer.install(source.path()).expect("install companion");

        assert_eq!(record.id, "test.desktop.menubar");
        assert_eq!(record.version, "0.1.0");
        assert!(
            root.path()
                .join("installed/test.desktop.menubar/0.1.0/dist/macos/EasyNetMenuBar.app")
                .exists(),
            "declared app bundle must be installed"
        );
    }

    #[test]
    fn plugin_host_install_commits_desktop_companion_supervisor_artifacts() {
        let root = tempfile::tempdir().expect("root");
        let source = tempfile::tempdir().expect("source");
        write_companion_test_package(source.path(), true);
        let installer = PluginInstaller::new(root.path());
        let (manager, calls) =
            recording_companion_manager(root.path().join("companions/state.toml"), false);

        let record = installer
            .install_with_companion_manager(source.path(), &manager)
            .expect("install companion with supervisor commit");

        assert_eq!(record.id, "test.desktop.menubar");
        assert_eq!(record.version, "0.1.0");
        assert_eq!(
            *calls.lock().expect("calls"),
            vec!["install", "enable"],
            "package install must commit supervisor install and enablement"
        );
        let companion_state = std::fs::read_to_string(root.path().join("companions/state.toml"))
            .expect("companion desired state");
        assert!(companion_state.contains("desired_state = \"enabled\""));
        assert!(root
            .path()
            .join("installed/test.desktop.menubar/0.1.0/plugin.toml")
            .exists());
    }

    #[test]
    fn plugin_host_install_rolls_back_desktop_companion_package_when_supervisor_commit_fails() {
        let root = tempfile::tempdir().expect("root");
        let source = tempfile::tempdir().expect("source");
        write_companion_test_package(source.path(), true);
        let installer = PluginInstaller::new(root.path());
        let (manager, calls) =
            recording_companion_manager(root.path().join("companions/state.toml"), true);

        let err = installer
            .install_with_companion_manager(source.path(), &manager)
            .expect_err("supervisor failure must abort install");

        assert!(matches!(
            err,
            PluginHostError::InvalidCompanionManifest { .. }
        ));
        assert_eq!(
            *calls.lock().expect("calls"),
            vec!["install", "enable", "stop", "remove"],
            "failed supervisor commit must be followed by companion cleanup"
        );
        assert!(
            !root
                .path()
                .join("installed/test.desktop.menubar/0.1.0")
                .exists(),
            "failed companion commit must remove the activated package directory"
        );
        let plugin_state =
            std::fs::read_to_string(root.path().join("state/plugins.toml")).unwrap_or_default();
        assert!(
            !plugin_state.contains("test.desktop.menubar"),
            "failed companion commit must remove the package lock row"
        );
        let companion_state =
            std::fs::read_to_string(root.path().join("companions/state.toml")).unwrap_or_default();
        assert!(
            !companion_state.contains("test.desktop.menubar"),
            "failed companion commit must remove desired companion state"
        );
    }

    #[test]
    fn plugin_host_update_commits_desktop_companion_supervisor_artifacts() {
        let root = tempfile::tempdir().expect("root");
        let source_v1 = tempfile::tempdir().expect("source v1");
        write_companion_test_package_version(source_v1.path(), "0.1.0", true);
        let source_v2 = tempfile::tempdir().expect("source v2");
        write_companion_test_package_version(source_v2.path(), "0.2.0", true);
        let installer = PluginInstaller::new(root.path());
        let (manager, calls) =
            recording_companion_manager(root.path().join("companions/state.toml"), false);
        installer
            .install_with_companion_manager(source_v1.path(), &manager)
            .expect("install v1");
        calls.lock().expect("calls").clear();

        let record = installer
            .update_with_companion_manager(source_v2.path(), &manager)
            .expect("update companion");

        assert_eq!(record.version, "0.2.0");
        assert_eq!(
            *calls.lock().expect("calls"),
            vec!["install", "enable"],
            "companion update must install and preserve enabled supervisor state"
        );
        assert!(root
            .path()
            .join("installed/test.desktop.menubar/0.2.0/plugin.toml")
            .exists());
        assert!(
            !root
                .path()
                .join("installed/test.desktop.menubar/0.1.0")
                .exists(),
            "successful update must retire previous companion package directory"
        );
        let plugin_state =
            std::fs::read_to_string(root.path().join("state/plugins.toml")).expect("plugin state");
        assert!(plugin_state.contains("version = \"0.2.0\""));
        assert!(!plugin_state.contains("version = \"0.1.0\""));
        let companion_state = std::fs::read_to_string(root.path().join("companions/state.toml"))
            .expect("companion desired state");
        assert!(companion_state.contains("version = \"0.2.0\""));
        assert!(!companion_state.contains("version = \"0.1.0\""));
    }

    #[test]
    fn plugin_host_update_rolls_back_desktop_companion_when_supervisor_commit_fails() {
        let root = tempfile::tempdir().expect("root");
        let source_v1 = tempfile::tempdir().expect("source v1");
        write_companion_test_package_version(source_v1.path(), "0.1.0", true);
        let source_v2 = tempfile::tempdir().expect("source v2");
        write_companion_test_package_version(source_v2.path(), "0.2.0", true);
        let installer = PluginInstaller::new(root.path());
        let (ok_manager, _) =
            recording_companion_manager(root.path().join("companions/state.toml"), false);
        installer
            .install_with_companion_manager(source_v1.path(), &ok_manager)
            .expect("install v1");
        let (failing_manager, calls) =
            recording_companion_manager(root.path().join("companions/state.toml"), true);

        let err = installer
            .update_with_companion_manager(source_v2.path(), &failing_manager)
            .expect_err("supervisor failure must abort update");

        assert!(matches!(
            err,
            PluginHostError::InvalidCompanionManifest { .. }
        ));
        assert_eq!(
            *calls.lock().expect("calls"),
            vec!["install", "enable", "stop", "remove", "install", "enable"],
            "failed companion update must clean new artifacts and restore previous supervisor state"
        );
        assert!(root
            .path()
            .join("installed/test.desktop.menubar/0.1.0/plugin.toml")
            .exists());
        assert!(
            !root
                .path()
                .join("installed/test.desktop.menubar/0.2.0")
                .exists(),
            "failed update must remove the new package directory"
        );
        let plugin_state =
            std::fs::read_to_string(root.path().join("state/plugins.toml")).expect("plugin state");
        assert!(plugin_state.contains("version = \"0.1.0\""));
        assert!(!plugin_state.contains("version = \"0.2.0\""));
        let companion_state = std::fs::read_to_string(root.path().join("companions/state.toml"))
            .expect("companion desired state");
        assert!(companion_state.contains("version = \"0.1.0\""));
        assert!(!companion_state.contains("version = \"0.2.0\""));
    }

    #[test]
    fn plugin_host_remove_allocates_rollback_before_removing_state_record() {
        let root = tempfile::tempdir().expect("root");
        let source_v1 = tempfile::tempdir().expect("source v1");
        write_sidecar_test_package(source_v1.path(), "0.1.0");
        let installer = PluginInstaller::new(root.path());
        installer.install(source_v1.path()).expect("install");
        std::fs::write(root.path().join(".rollback"), "not a directory")
            .expect("poison rollback root");

        let err = installer
            .remove("test.sidecar", "0.1.0")
            .expect_err("rollback allocation failure must abort remove");

        assert!(matches!(err, PluginHostError::WriteFailed { .. }));
        assert!(root
            .path()
            .join("installed/test.sidecar/0.1.0/plugin.toml")
            .exists());
        let plugin_state =
            std::fs::read_to_string(root.path().join("state/plugins.toml")).expect("plugin state");
        assert!(plugin_state.contains("test.sidecar"));
        assert!(plugin_state.contains("version = \"0.1.0\""));
    }

    #[test]
    fn plugin_host_remove_commits_desktop_companion_supervisor_cleanup() {
        let root = tempfile::tempdir().expect("root");
        let source = tempfile::tempdir().expect("source");
        write_companion_test_package(source.path(), true);
        let installer = PluginInstaller::new(root.path());
        let (manager, calls) =
            recording_companion_manager(root.path().join("companions/state.toml"), false);
        installer
            .install_with_companion_manager(source.path(), &manager)
            .expect("install companion");
        calls.lock().expect("calls").clear();

        installer
            .remove_with_companion_manager("test.desktop.menubar", "0.1.0", &manager)
            .expect("remove companion");

        assert_eq!(
            *calls.lock().expect("calls"),
            vec!["stop", "remove"],
            "companion remove must stop and remove supervisor artifacts"
        );
        assert!(
            !root
                .path()
                .join("installed/test.desktop.menubar/0.1.0")
                .exists(),
            "successful companion remove must delete package directory"
        );
        let plugin_state =
            std::fs::read_to_string(root.path().join("state/plugins.toml")).unwrap_or_default();
        assert!(!plugin_state.contains("test.desktop.menubar"));
        let companion_state =
            std::fs::read_to_string(root.path().join("companions/state.toml")).unwrap_or_default();
        assert!(!companion_state.contains("test.desktop.menubar"));
    }

    #[test]
    fn plugin_host_remove_keeps_desktop_companion_package_when_supervisor_cleanup_fails() {
        let root = tempfile::tempdir().expect("root");
        let source = tempfile::tempdir().expect("source");
        write_companion_test_package(source.path(), true);
        let installer = PluginInstaller::new(root.path());
        let (ok_manager, _) =
            recording_companion_manager(root.path().join("companions/state.toml"), false);
        installer
            .install_with_companion_manager(source.path(), &ok_manager)
            .expect("install companion");
        let (failing_manager, calls) = recording_companion_manager_with_failures(
            root.path().join("companions/state.toml"),
            false,
            true,
        );

        let err = installer
            .remove_with_companion_manager("test.desktop.menubar", "0.1.0", &failing_manager)
            .expect_err("supervisor cleanup failure must abort package remove");

        assert!(matches!(
            err,
            PluginHostError::InvalidCompanionManifest { .. }
        ));
        assert_eq!(
            *calls.lock().expect("calls"),
            vec!["stop", "remove"],
            "failed supervisor cleanup must happen before package removal"
        );
        assert!(root
            .path()
            .join("installed/test.desktop.menubar/0.1.0/plugin.toml")
            .exists());
        let plugin_state =
            std::fs::read_to_string(root.path().join("state/plugins.toml")).expect("plugin state");
        assert!(plugin_state.contains("test.desktop.menubar"));
        assert!(plugin_state.contains("version = \"0.1.0\""));
        let companion_state = std::fs::read_to_string(root.path().join("companions/state.toml"))
            .expect("companion state");
        assert!(companion_state.contains("test.desktop.menubar"));
        assert!(companion_state.contains("version = \"0.1.0\""));
    }

    #[test]
    fn plugin_host_remove_restores_desktop_companion_when_package_remove_fails() {
        let root = tempfile::tempdir().expect("root");
        let source = tempfile::tempdir().expect("source");
        write_companion_test_package(source.path(), true);
        let installer = PluginInstaller::new(root.path());
        let (manager, calls) =
            recording_companion_manager(root.path().join("companions/state.toml"), false);
        installer
            .install_with_companion_manager(source.path(), &manager)
            .expect("install companion");
        calls.lock().expect("calls").clear();
        std::fs::write(root.path().join(".rollback"), "not a directory")
            .expect("poison rollback root");

        let err = installer
            .remove_with_companion_manager("test.desktop.menubar", "0.1.0", &manager)
            .expect_err("package remove failure must restore companion state");

        assert!(matches!(err, PluginHostError::WriteFailed { .. }));
        assert_eq!(
            *calls.lock().expect("calls"),
            vec!["stop", "remove", "install", "enable"],
            "package remove failure after companion cleanup must restore supervisor state"
        );
        assert!(root
            .path()
            .join("installed/test.desktop.menubar/0.1.0/plugin.toml")
            .exists());
        let plugin_state =
            std::fs::read_to_string(root.path().join("state/plugins.toml")).expect("plugin state");
        assert!(plugin_state.contains("test.desktop.menubar"));
        assert!(plugin_state.contains("version = \"0.1.0\""));
        let companion_state = std::fs::read_to_string(root.path().join("companions/state.toml"))
            .expect("companion state");
        assert!(companion_state.contains("test.desktop.menubar"));
        assert!(companion_state.contains("version = \"0.1.0\""));
    }

    #[test]
    fn plugin_host_install_rejects_desktop_companion_with_missing_artifact() {
        let root = tempfile::tempdir().expect("root");
        let source = tempfile::tempdir().expect("source");
        write_companion_test_package(source.path(), false);
        let installer = PluginInstaller::new(root.path());

        let err = installer
            .install(source.path())
            .expect_err("missing companion artifact must reject");

        assert!(matches!(err, PluginHostError::ReadFailed { .. }));
        assert!(
            !root
                .path()
                .join("installed/test.desktop.menubar/0.1.0")
                .exists(),
            "rejected companion must not be activated"
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

    fn write_companion_test_package(root: &Path, include_artifact: bool) {
        write_companion_test_package_version(root, "0.1.0", include_artifact)
    }

    fn write_companion_test_package_version(root: &Path, version: &str, include_artifact: bool) {
        if include_artifact {
            std::fs::create_dir_all(root.join("dist/macos/EasyNetMenuBar.app/Contents/MacOS"))
                .expect("app bundle dir");
            std::fs::write(
                root.join("dist/macos/EasyNetMenuBar.app/Contents/MacOS/EasyNetMenuBar"),
                "",
            )
            .expect("app executable");
        }
        std::fs::write(
            root.join("plugin.toml"),
            format!(
                r#"
schema_version = "1"
id = "test.desktop.menubar"
version = "{version}"
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
status_file = "companions/test.desktop.menubar/status.json"

[companion.macos]
bundle_id = "tech.silan.easynet.menubar"
app_bundle = "dist/macos/EasyNetMenuBar.app"
supervisor = "launch_agent"
launch_agent_label = "tech.silan.easynet.menubar"
session = "aqua"
"#
            ),
        )
        .expect("manifest");
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

    fn recording_companion_manager(
        state_path: PathBuf,
        fail_enable: bool,
    ) -> (DesktopCompanionManager, Arc<Mutex<Vec<&'static str>>>) {
        recording_companion_manager_with_failures(state_path, fail_enable, false)
    }

    fn recording_companion_manager_with_failures(
        state_path: PathBuf,
        fail_enable: bool,
        fail_remove: bool,
    ) -> (DesktopCompanionManager, Arc<Mutex<Vec<&'static str>>>) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let supervisor = RecordingCompanionSupervisor {
            fail_next_enable: Arc::new(Mutex::new(fail_enable)),
            fail_next_remove: Arc::new(Mutex::new(fail_remove)),
            calls: calls.clone(),
        };
        (
            DesktopCompanionManager::new(
                DesktopCompanionPlanner::new("macos"),
                Box::new(supervisor),
                DesktopCompanionStateStore::new(state_path),
            ),
            calls,
        )
    }

    struct RecordingCompanionSupervisor {
        fail_next_enable: Arc<Mutex<bool>>,
        fail_next_remove: Arc<Mutex<bool>>,
        calls: Arc<Mutex<Vec<&'static str>>>,
    }

    impl RecordingCompanionSupervisor {
        fn record(&self, action: &'static str) {
            self.calls.lock().expect("calls").push(action);
        }
    }

    impl DesktopCompanionSupervisor for RecordingCompanionSupervisor {
        fn platform(&self) -> &'static str {
            "macos"
        }

        fn probe_session(&self) -> CompanionSessionStatus {
            CompanionSessionStatus::Available
        }

        fn install(&self, _plan: &DesktopCompanionPlan) -> Result<CompanionActionReport> {
            self.record("install");
            Ok(CompanionActionReport::changed("installed"))
        }

        fn enable(&self, plan: &DesktopCompanionPlan) -> Result<CompanionActionReport> {
            self.record("enable");
            let mut fail_next = self.fail_next_enable.lock().expect("fail flag");
            if *fail_next {
                *fail_next = false;
                return Err(PluginHostError::InvalidCompanionManifest {
                    id: plan.package_id.clone(),
                    reason: "injected supervisor failure".to_string(),
                });
            }
            Ok(CompanionActionReport::changed("enabled"))
        }

        fn disable(&self, _plan: &DesktopCompanionPlan) -> Result<CompanionActionReport> {
            self.record("disable");
            Ok(CompanionActionReport::changed("disabled"))
        }

        fn remove(&self, _plan: &DesktopCompanionPlan) -> Result<CompanionActionReport> {
            self.record("remove");
            let mut fail_next = self.fail_next_remove.lock().expect("fail flag");
            if *fail_next {
                *fail_next = false;
                return Err(PluginHostError::InvalidCompanionManifest {
                    id: "test.desktop.menubar".to_string(),
                    reason: "injected supervisor remove failure".to_string(),
                });
            }
            Ok(CompanionActionReport::changed("removed"))
        }

        fn start(&self, _plan: &DesktopCompanionPlan) -> Result<CompanionActionReport> {
            self.record("start");
            Ok(CompanionActionReport::changed("started"))
        }

        fn stop(&self, _plan: &DesktopCompanionPlan) -> Result<CompanionActionReport> {
            self.record("stop");
            Ok(CompanionActionReport::changed("stopped"))
        }

        fn supervisor_state(&self, _plan: &DesktopCompanionPlan) -> CompanionSupervisorState {
            CompanionSupervisorState::InstalledEnabled
        }

        fn observe(&self, _plan: &DesktopCompanionPlan) -> CompanionObservation {
            CompanionObservation {
                observed_state: CompanionObservedState::NotRunning,
                ..CompanionObservation::default()
            }
        }
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
            r#"schema_version = "2"
name = "{ability}"
descriptor_version = "1.2.3"
description = "test descriptor for {ability}"
admission_action = "invoke"

[input_schema]
type = "object"
additionalProperties = false
"#
        )
    }
}
