// EasyNet CLI — desktop companion status-file paths
// =================================================
//
// File: src/daemon/plugins/companion/status_file.rs
// Description: Resolves manifest-declared companion heartbeat paths.

use std::path::{Component, Path, PathBuf};

/// Runtime-owned status-file path resolved from a companion manifest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompanionStatusFilePath {
    absolute: PathBuf,
}

impl CompanionStatusFilePath {
    pub fn resolve(package_root: &Path, declared: &str) -> Result<Self, String> {
        Ok(Self::resolve_with_state_root(
            package_root,
            &local_state_root()?,
            declared,
        ))
    }

    pub fn into_path_buf(self) -> PathBuf {
        self.absolute
    }

    fn resolve_with_state_root(package_root: &Path, state_root: &Path, declared: &str) -> Self {
        let relative = Path::new(declared.trim());
        let root = if is_state_relative(relative) {
            state_root
        } else {
            package_root
        };
        Self {
            absolute: root.join(relative),
        }
    }
}

fn local_state_root() -> Result<PathBuf, String> {
    local_state_root_for_home(dirs::home_dir())
}

fn local_state_root_for_home(home: Option<PathBuf>) -> Result<PathBuf, String> {
    let home = home.ok_or_else(|| {
        "desktop companion status-file state root is unavailable: home directory is unavailable"
            .to_string()
    })?;
    if home.as_os_str().is_empty() {
        return Err(
            "desktop companion status-file state root is unavailable: home directory is empty"
                .to_string(),
        );
    }
    Ok(home.join(".easynet"))
}

fn is_state_relative(path: &Path) -> bool {
    matches!(
        path.components().next(),
        Some(Component::Normal(root)) if root == "state" || root == "companions"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn companions_status_file_is_state_relative() {
        let path = CompanionStatusFilePath::resolve_with_state_root(
            Path::new("/pkg"),
            Path::new("/state"),
            "companions/easynet.desktop.menubar/status.json",
        );

        assert_eq!(
            path.into_path_buf(),
            PathBuf::from("/state/companions/easynet.desktop.menubar/status.json")
        );
    }

    #[test]
    fn state_prefix_is_state_relative() {
        let path = CompanionStatusFilePath::resolve_with_state_root(
            Path::new("/pkg"),
            Path::new("/state"),
            "state/easynet-menubar.status.json",
        );

        assert_eq!(
            path.into_path_buf(),
            PathBuf::from("/state/state/easynet-menubar.status.json")
        );
    }

    #[test]
    fn other_status_paths_are_package_relative() {
        let path = CompanionStatusFilePath::resolve_with_state_root(
            Path::new("/pkg"),
            Path::new("/state"),
            "runtime/status.json",
        );

        assert_eq!(
            path.into_path_buf(),
            PathBuf::from("/pkg/runtime/status.json")
        );
    }

    #[test]
    fn local_state_root_rejects_missing_home_before_cwd_fallback() {
        let error = local_state_root_for_home(None)
            .expect_err("missing home must fail before cwd fallback");

        assert!(
            error.contains("home directory is unavailable"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn local_state_root_rejects_empty_home_before_cwd_fallback() {
        let error = local_state_root_for_home(Some(PathBuf::new()))
            .expect_err("empty home must fail before cwd fallback");

        assert!(
            error.contains("home directory is empty"),
            "unexpected error: {error}"
        );
    }
}
