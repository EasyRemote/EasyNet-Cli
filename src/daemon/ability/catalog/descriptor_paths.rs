use std::path::{Component, Path, PathBuf};

/// Canonical root for daemon-owned AbilityDescriptor TOMLs.
///
/// The helper owns the descriptor root so production code does not concatenate
/// the path directly. Future grouping under this root remains transparent to
/// callers that use this module.
pub const SYSTEM_ABILITY_DESCRIPTOR_ROOT: &str = "ability-descriptors/system";

#[derive(Debug, thiserror::Error)]
pub enum DescriptorPathError {
    #[error("system ability descriptor name must not be empty")]
    EmptyName,
    #[error("system ability descriptor name must be relative and contain no path separators: {0}")]
    UnsafeName(String),
}

#[must_use]
pub fn system_ability_descriptor_root() -> PathBuf {
    PathBuf::from(SYSTEM_ABILITY_DESCRIPTOR_ROOT)
}

pub fn try_system_ability_descriptor_path(
    ability_name: &str,
) -> Result<PathBuf, DescriptorPathError> {
    validate_ability_descriptor_name(ability_name)?;
    Ok(system_ability_descriptor_root().join(format!("{ability_name}.ability.toml")))
}

#[must_use]
pub fn system_ability_descriptor_path(ability_name: &str) -> PathBuf {
    try_system_ability_descriptor_path(ability_name)
        .unwrap_or_else(|error| panic!("invalid system ability descriptor path: {error}"))
}

/// Iterate current system descriptor files without assuming future grouping.
///
/// Missing roots produce an empty iterator; callers that require the directory
/// to exist should validate `system_ability_descriptor_root()` first.
pub fn iter_system_ability_descriptor_paths() -> impl Iterator<Item = PathBuf> {
    let mut paths = Vec::new();
    collect_descriptor_paths(&system_ability_descriptor_root(), &mut paths);
    paths.sort();
    paths.into_iter()
}

fn collect_descriptor_paths(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_descriptor_paths(&path, out);
        } else if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(".ability.toml"))
        {
            out.push(path);
        }
    }
}

fn validate_ability_descriptor_name(name: &str) -> Result<(), DescriptorPathError> {
    if name.trim().is_empty() {
        return Err(DescriptorPathError::EmptyName);
    }
    let path = Path::new(name);
    let safe = path.components().all(|component| {
        matches!(
            component,
            Component::Normal(_) if !name.contains(std::path::MAIN_SEPARATOR)
        )
    });
    if safe && !name.contains('/') && !name.contains('\\') {
        Ok(())
    } else {
        Err(DescriptorPathError::UnsafeName(name.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_path_uses_migration_root() {
        assert_eq!(
            system_ability_descriptor_path("observe.health"),
            PathBuf::from("ability-descriptors/system/observe.health.ability.toml")
        );
    }

    #[test]
    fn descriptor_path_rejects_path_escape() {
        assert!(try_system_ability_descriptor_path("../x").is_err());
        assert!(try_system_ability_descriptor_path("group/x").is_err());
    }
}
