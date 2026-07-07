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
    pub fn resolve(package_root: &Path, declared: &str) -> Self {
        Self::resolve_with_state_root(package_root, &local_state_root(), declared)
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

fn local_state_root() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".easynet")
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
}
