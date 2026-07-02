// EasyNet CLI — plugin package index
// ==================================
//
// File: src/runtime/plugin_host/index.rs
// Description: Builtin + installed package discovery without load policy.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use crate::runtime::plugin_host::errors::{PluginHostError, Result};
use crate::runtime::plugin_host::install::PluginStateToml;
use crate::runtime::plugin_host::package::{PluginPackage, SharedPluginPackage};

const INSTALLED_DIR: &str = "installed";
const STATE_DIR: &str = "state";
const PLUGIN_LOCK_FILE: &str = "plugin-lock.toml";

/// Default local plugin root under `~/.easynet/plugins`.
pub fn default_plugin_root() -> PathBuf {
    crate::persistence::config::home_dir()
        .join(".easynet")
        .join("plugins")
}

/// Package index containing builtin and installed plugin packages.
///
/// What this is NOT: a load plan. Packages appear here even if the current
/// daemon boot will not load them because of env, platform, permissions, or
/// missing executable dependencies.
#[derive(Clone, Debug, Default)]
pub struct PluginPackageIndex {
    packages: Vec<SharedPluginPackage>,
}

/// Installed package row that could not be indexed for this daemon profile.
///
/// This is operator-facing load evidence, not an install transaction result.
/// A bad installed package must not prevent builtin packages from loading.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginPackageIndexError {
    pub id: String,
    pub version: String,
    pub package_dir: PathBuf,
    pub reason: String,
}

/// Resilient package-index load result.
#[derive(Clone, Debug, Default)]
pub struct PluginPackageIndexLoadReport {
    index: PluginPackageIndex,
    installed_errors: Vec<PluginPackageIndexError>,
}

impl PluginPackageIndexLoadReport {
    pub fn index(&self) -> &PluginPackageIndex {
        &self.index
    }

    pub fn installed_errors(&self) -> &[PluginPackageIndexError] {
        &self.installed_errors
    }

    pub fn into_parts(self) -> (PluginPackageIndex, Vec<PluginPackageIndexError>) {
        (self.index, self.installed_errors)
    }
}

impl PluginPackageIndex {
    /// Build an index from builtin packages compiled into this binary.
    ///
    /// The builtin binding set and its installable surface are fixed for the
    /// process lifetime, so the index is computed once and cloned per call.
    /// Construction hashes every file under each builtin package root
    /// (including `bin/` sidecars); without this cache each descriptor lookup
    /// re-reads all of it. Construction failures are not cached so error
    /// reporting stays live.
    pub fn builtin() -> Result<Self> {
        static BUILTIN: OnceLock<PluginPackageIndex> = OnceLock::new();
        if let Some(index) = BUILTIN.get() {
            return Ok(index.clone());
        }
        let mut packages = Vec::new();
        for binding in crate::plugins::builtin::builtin_bindings() {
            packages.push(Arc::new(PluginPackage::from_builtin(binding)?));
        }
        let index = Self::from_packages(packages)?;
        Ok(BUILTIN.get_or_init(|| index).clone())
    }

    /// Build an index from the active installed packages under a plugin root.
    ///
    /// The active set is the lock file written by [`PluginInstaller`], not a
    /// directory scan. Old package directories may remain on disk after update
    /// transactions, but they are not live unless the lock names them.
    pub fn installed(root: &Path) -> Result<Self> {
        let mut packages = Vec::new();
        let lock_path = root.join(STATE_DIR).join(PLUGIN_LOCK_FILE);
        if !lock_path.exists() {
            return Self::from_packages(packages);
        }
        let body =
            std::fs::read_to_string(&lock_path).map_err(|source| PluginHostError::ReadFailed {
                path: lock_path.clone(),
                source,
            })?;
        let state: PluginStateToml =
            toml::from_str(&body).map_err(|source| PluginHostError::ManifestParseFailed {
                path: lock_path,
                source,
            })?;
        for record in state.plugins {
            let package_dir = root
                .join(INSTALLED_DIR)
                .join(&record.id)
                .join(&record.version);
            packages.push(Arc::new(PluginPackage::from_installed(
                &package_dir,
                Some(&record.hash),
            )?));
        }
        Self::from_packages(packages)
    }

    /// Build an installed index without letting one bad package poison the
    /// entire daemon boot. Malformed lock files produce one operator-visible
    /// error row and no installed packages; malformed packages or package
    /// collisions drop only the offending row.
    pub fn installed_resilient(root: &Path) -> Result<PluginPackageIndexLoadReport> {
        let lock_path = root.join(STATE_DIR).join(PLUGIN_LOCK_FILE);
        if !lock_path.exists() {
            return Ok(PluginPackageIndexLoadReport::default());
        }
        let body = match std::fs::read_to_string(&lock_path) {
            Ok(body) => body,
            Err(source) => {
                return Ok(PluginPackageIndexLoadReport {
                    index: Self::default(),
                    installed_errors: vec![PluginPackageIndexError {
                        id: "plugin-lock".to_string(),
                        version: String::new(),
                        package_dir: lock_path,
                        reason: PluginHostError::ReadFailed {
                            path: root.join(STATE_DIR).join(PLUGIN_LOCK_FILE),
                            source,
                        }
                        .to_string(),
                    }],
                });
            }
        };
        let state: PluginStateToml = match toml::from_str(&body) {
            Ok(state) => state,
            Err(source) => {
                return Ok(PluginPackageIndexLoadReport {
                    index: Self::default(),
                    installed_errors: vec![PluginPackageIndexError {
                        id: "plugin-lock".to_string(),
                        version: String::new(),
                        package_dir: lock_path.clone(),
                        reason: PluginHostError::ManifestParseFailed {
                            path: lock_path,
                            source,
                        }
                        .to_string(),
                    }],
                });
            }
        };

        let mut uniqueness = PluginPackageUniqueness::default();
        let mut packages = Vec::<SharedPluginPackage>::new();
        let mut installed_errors = Vec::<PluginPackageIndexError>::new();
        for record in state.plugins {
            let package_dir = root
                .join(INSTALLED_DIR)
                .join(&record.id)
                .join(&record.version);
            let package = match PluginPackage::from_installed(&package_dir, Some(&record.hash)) {
                Ok(package) => Arc::new(package),
                Err(err) => {
                    installed_errors.push(PluginPackageIndexError {
                        id: record.id,
                        version: record.version,
                        package_dir,
                        reason: err.to_string(),
                    });
                    continue;
                }
            };

            if let Err(err) = uniqueness.insert(&package) {
                installed_errors.push(PluginPackageIndexError {
                    id: record.id,
                    version: record.version,
                    package_dir,
                    reason: err.to_string(),
                });
                continue;
            }
            packages.push(package);
        }

        Ok(PluginPackageIndexLoadReport {
            index: Self { packages },
            installed_errors,
        })
    }

    /// Load the default daemon package index: builtin plus installed packages.
    pub fn load_default() -> Result<Self> {
        let mut packages = Self::builtin()?.packages;
        packages.extend(Self::installed(&default_plugin_root())?.packages);
        Self::from_packages(packages)
    }

    /// Load the default daemon package index while preserving builtin packages
    /// if installed package rows are corrupt.
    pub fn load_default_resilient() -> Result<PluginPackageIndexLoadReport> {
        let mut packages = Self::builtin()?.packages;
        let mut uniqueness = PluginPackageUniqueness::from_packages(&packages)?;
        let installed = Self::installed_resilient(&default_plugin_root())?;
        let (installed_index, mut installed_errors) = installed.into_parts();
        for package in installed_index.packages {
            if let Err(err) = uniqueness.insert(&package) {
                installed_errors.push(PluginPackageIndexError {
                    id: package.id().as_str().to_string(),
                    version: package.version().as_str().to_string(),
                    package_dir: package.root().to_path_buf(),
                    reason: err.to_string(),
                });
                continue;
            }
            packages.push(package);
        }
        Ok(PluginPackageIndexLoadReport {
            index: Self { packages },
            installed_errors,
        })
    }

    /// Add every package from another index.
    pub fn extend(&mut self, other: Self) -> Result<()> {
        let mut next = self.packages.clone();
        next.extend(other.packages);
        validate_unique_packages(&next)?;
        self.packages = next;
        Ok(())
    }

    /// All indexed packages.
    pub fn packages(&self) -> &[SharedPluginPackage] {
        &self.packages
    }

    /// Find the package that declares an ability.
    pub fn package_for_ability(&self, name: &str) -> Option<SharedPluginPackage> {
        self.packages
            .iter()
            .find(|package| package.manifest().ability(name).is_some())
            .cloned()
    }

    /// Return a deterministic map of package id to its active indexed package.
    pub fn by_id(&self) -> BTreeMap<String, SharedPluginPackage> {
        let mut out = BTreeMap::new();
        for package in &self.packages {
            out.insert(package.id().as_str().to_string(), Arc::clone(package));
        }
        out
    }

    pub(crate) fn from_packages(packages: Vec<SharedPluginPackage>) -> Result<Self> {
        validate_unique_packages(&packages)?;
        Ok(Self { packages })
    }
}

fn validate_unique_packages(packages: &[SharedPluginPackage]) -> Result<()> {
    PluginPackageUniqueness::from_packages(packages).map(|_| ())
}

#[derive(Default)]
struct PluginPackageUniqueness {
    package_versions: BTreeMap<(String, String), ()>,
    package_ids: BTreeMap<String, String>,
    ability_owner: BTreeMap<String, String>,
}

impl PluginPackageUniqueness {
    fn from_packages(packages: &[SharedPluginPackage]) -> Result<Self> {
        let mut uniqueness = Self::default();
        for package in packages {
            uniqueness.insert(package)?;
        }
        Ok(uniqueness)
    }

    fn insert(&mut self, package: &SharedPluginPackage) -> Result<()> {
        let id = package.id().as_str().to_string();
        let version = package.version().as_str().to_string();
        if self
            .package_versions
            .contains_key(&(id.clone(), version.clone()))
        {
            return Err(PluginHostError::DuplicatePackageVersion { id, version });
        }
        if let Some(first_version) = self.package_ids.get(&id).cloned() {
            return Err(PluginHostError::DuplicatePackageId {
                id,
                first_version,
                second_version: version,
            });
        }
        let owner = format!("{id}@{version}");
        for ability in package.manifest().abilities() {
            if let Some(first) = self.ability_owner.get(ability.name()).cloned() {
                return Err(PluginHostError::DuplicateAbilityOwner {
                    ability: ability.name().to_string(),
                    first,
                    second: owner.clone(),
                });
            }
        }
        self.package_versions
            .insert((id.clone(), version.clone()), ());
        self.package_ids.insert(id, version);
        for ability in package.manifest().abilities() {
            self.ability_owner
                .insert(ability.name().to_string(), owner.clone());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::plugin_host::install::{InstalledPluginRecord, PluginStateToml};
    use crate::runtime::plugin_host::package::tests::write_test_package;

    #[test]
    fn plugin_index_reads_active_versions_from_lockfile_only() {
        let root = tempfile::tempdir().expect("root");
        let active = root.path().join("installed/test.plugin/0.2.0");
        let stale = root.path().join("installed/test.plugin/0.1.0");
        write_test_package(&active, "0.2.0");
        write_test_package(&stale, "0.1.0");
        let package = PluginPackage::from_installed(&active, None).expect("active package");
        write_lock(
            root.path(),
            PluginStateToml {
                plugins: vec![InstalledPluginRecord {
                    id: "test.plugin".to_string(),
                    version: "0.2.0".to_string(),
                    hash: package.hash().as_str().to_string(),
                }],
            },
        );

        let index = PluginPackageIndex::installed(root.path()).expect("installed index");
        assert_eq!(index.packages().len(), 1);
        assert_eq!(index.packages()[0].version().as_str(), "0.2.0");
    }

    #[test]
    fn plugin_index_rejects_duplicate_ability_across_packages() {
        let root = tempfile::tempdir().expect("root");
        let first = root.path().join("installed/test.plugin/0.1.0");
        let second = root.path().join("installed/test.plugin2/0.1.0");
        write_test_package_with_id(&first, "test.plugin", "0.1.0", "test.echo");
        write_test_package_with_id(&second, "test.plugin2", "0.1.0", "test.echo");
        let first_pkg = PluginPackage::from_installed(&first, None).expect("first package");
        let second_pkg = PluginPackage::from_installed(&second, None).expect("second package");
        write_lock(
            root.path(),
            PluginStateToml {
                plugins: vec![
                    InstalledPluginRecord {
                        id: "test.plugin".to_string(),
                        version: "0.1.0".to_string(),
                        hash: first_pkg.hash().as_str().to_string(),
                    },
                    InstalledPluginRecord {
                        id: "test.plugin2".to_string(),
                        version: "0.1.0".to_string(),
                        hash: second_pkg.hash().as_str().to_string(),
                    },
                ],
            },
        );

        let err = match PluginPackageIndex::installed(root.path()) {
            Ok(_) => panic!("duplicate ability must fail index construction"),
            Err(err) => err,
        };
        assert!(matches!(err, PluginHostError::DuplicateAbilityOwner { .. }));
    }

    #[test]
    fn plugin_index_rejects_multiple_active_versions_for_one_package() {
        let root = tempfile::tempdir().expect("root");
        let first = root.path().join("installed/test.plugin/0.1.0");
        let second = root.path().join("installed/test.plugin/0.2.0");
        write_test_package_with_id(&first, "test.plugin", "0.1.0", "test.echo");
        write_test_package_with_id(&second, "test.plugin", "0.2.0", "test.echo2");
        let first_pkg = PluginPackage::from_installed(&first, None).expect("first package");
        let second_pkg = PluginPackage::from_installed(&second, None).expect("second package");
        write_lock(
            root.path(),
            PluginStateToml {
                plugins: vec![
                    InstalledPluginRecord {
                        id: "test.plugin".to_string(),
                        version: "0.1.0".to_string(),
                        hash: first_pkg.hash().as_str().to_string(),
                    },
                    InstalledPluginRecord {
                        id: "test.plugin".to_string(),
                        version: "0.2.0".to_string(),
                        hash: second_pkg.hash().as_str().to_string(),
                    },
                ],
            },
        );

        let err = match PluginPackageIndex::installed(root.path()) {
            Ok(_) => panic!("one package id must not have multiple active versions"),
            Err(err) => err,
        };
        assert!(matches!(err, PluginHostError::DuplicatePackageId { .. }));
    }

    #[test]
    fn plugin_index_resilient_installed_skips_bad_package_and_keeps_good_package() {
        let root = tempfile::tempdir().expect("root");
        let good = root.path().join("installed/test.good/0.1.0");
        let bad = root.path().join("installed/test.bad/0.1.0");
        write_test_package_with_id(&good, "test.good", "0.1.0", "device.test.good");
        write_test_package_with_id(&bad, "test.bad", "0.1.0", "device.test.bad");
        let good_pkg = PluginPackage::from_installed(&good, None).expect("good package");
        let bad_pkg = PluginPackage::from_installed(&bad, None).expect("bad package before damage");
        std::fs::remove_file(bad.join("abilities/device.test.bad.ability.toml"))
            .expect("remove bad descriptor");
        write_lock(
            root.path(),
            PluginStateToml {
                plugins: vec![
                    InstalledPluginRecord {
                        id: "test.good".to_string(),
                        version: "0.1.0".to_string(),
                        hash: good_pkg.hash().as_str().to_string(),
                    },
                    InstalledPluginRecord {
                        id: "test.bad".to_string(),
                        version: "0.1.0".to_string(),
                        hash: bad_pkg.hash().as_str().to_string(),
                    },
                ],
            },
        );

        let report = PluginPackageIndex::installed_resilient(root.path()).expect("resilient index");
        assert_eq!(report.index().packages().len(), 1);
        assert_eq!(report.index().packages()[0].id().as_str(), "test.good");
        assert_eq!(report.installed_errors().len(), 1);
        assert_eq!(report.installed_errors()[0].id, "test.bad");
        assert!(report.installed_errors()[0]
            .reason
            .contains("read plugin package path"));
    }

    #[test]
    fn plugin_index_resilient_installed_skips_duplicate_ability_without_rebuilding_index() {
        let root = tempfile::tempdir().expect("root");
        let first = root.path().join("installed/test.first/0.1.0");
        let second = root.path().join("installed/test.second/0.1.0");
        write_test_package_with_id(&first, "test.first", "0.1.0", "test.echo");
        write_test_package_with_id(&second, "test.second", "0.1.0", "test.echo");
        let first_pkg = PluginPackage::from_installed(&first, None).expect("first package");
        let second_pkg = PluginPackage::from_installed(&second, None).expect("second package");
        write_lock(
            root.path(),
            PluginStateToml {
                plugins: vec![
                    InstalledPluginRecord {
                        id: "test.first".to_string(),
                        version: "0.1.0".to_string(),
                        hash: first_pkg.hash().as_str().to_string(),
                    },
                    InstalledPluginRecord {
                        id: "test.second".to_string(),
                        version: "0.1.0".to_string(),
                        hash: second_pkg.hash().as_str().to_string(),
                    },
                ],
            },
        );

        let report = PluginPackageIndex::installed_resilient(root.path()).expect("resilient index");

        assert_eq!(report.index().packages().len(), 1);
        assert_eq!(report.index().packages()[0].id().as_str(), "test.first");
        assert_eq!(report.installed_errors().len(), 1);
        assert_eq!(report.installed_errors()[0].id, "test.second");
        assert!(report.installed_errors()[0]
            .reason
            .contains("declared by multiple packages"));
    }

    #[test]
    fn plugin_index_default_resilient_skips_installed_collision_with_builtin() {
        let _home = crate::cli::test_support::HomeGuard::new();
        let root = default_plugin_root();
        let shadow = root.join("installed/easynet.remote_desktop/9.9.9");
        write_test_package_with_id(&shadow, "easynet.remote_desktop", "9.9.9", "test.shadow");
        let shadow_pkg = PluginPackage::from_installed(&shadow, None).expect("shadow package");
        write_lock(
            &root,
            PluginStateToml {
                plugins: vec![InstalledPluginRecord {
                    id: "easynet.remote_desktop".to_string(),
                    version: "9.9.9".to_string(),
                    hash: shadow_pkg.hash().as_str().to_string(),
                }],
            },
        );

        let report = PluginPackageIndex::load_default_resilient().expect("default resilient index");
        let remote_desktop_rows = report
            .index()
            .packages()
            .iter()
            .filter(|package| package.id().as_str() == "easynet.remote_desktop")
            .collect::<Vec<_>>();

        assert_eq!(remote_desktop_rows.len(), 1);
        assert_ne!(remote_desktop_rows[0].version().as_str(), "9.9.9");
        assert_eq!(report.installed_errors().len(), 1);
        assert_eq!(report.installed_errors()[0].id, "easynet.remote_desktop");
        assert!(report.installed_errors()[0]
            .reason
            .contains("multiple active versions"));
    }

    fn write_lock(root: &Path, state: PluginStateToml) {
        let state_dir = root.join(STATE_DIR);
        std::fs::create_dir_all(&state_dir).expect("state dir");
        std::fs::write(
            state_dir.join(PLUGIN_LOCK_FILE),
            toml::to_string_pretty(&state).expect("lock toml"),
        )
        .expect("write lock");
    }

    fn write_test_package_with_id(root: &Path, id: &str, version: &str, ability: &str) {
        std::fs::create_dir_all(root.join("abilities")).expect("abilities dir");
        std::fs::write(
            root.join("plugin.toml"),
            format!(
                r#"
schema_version = "1"
id = "{id}"
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
