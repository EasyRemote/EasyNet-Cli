//! Built-in daemon ability catalog projection.
//!
//! This module owns descriptor rendering and descriptor-path resolution for
//! daemon-owned system abilities. Handler bodies live under
//! `daemon::ability::builtins`; control-plane registration stays under
//! `runtime::ability`.

pub mod ability_toml;
#[cfg(test)]
mod assembly_tests;
pub mod build;
pub mod catalog_metadata;
mod descriptor_paths;
pub mod profiles;
pub(crate) mod system_manifest;

pub use build::*;
pub use catalog_metadata::*;
pub use descriptor_paths::{
    iter_system_ability_descriptor_paths, system_ability_descriptor_path,
    system_ability_descriptor_root, try_system_ability_descriptor_path, DescriptorPathError,
    SYSTEM_ABILITY_DESCRIPTOR_ROOT,
};
