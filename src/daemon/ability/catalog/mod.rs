//! Built-in daemon ability catalog projection.
//!
//! This module owns descriptor rendering and descriptor-path resolution for
//! daemon-owned system abilities. Handler bodies live under
//! `daemon::ability::builtins`; control-plane registration stays under
//! `daemon::ability`.

pub mod ability_toml;
#[cfg(test)]
mod assembly_tests;
pub mod build;
pub mod catalog_metadata;
pub(crate) mod daemon_invocation_contracts;
mod descriptor_paths;
pub(crate) mod ownership;
pub mod profiles;
pub(crate) mod publication;
pub(crate) mod runtime_admin_contracts;
pub(crate) mod system_manifest;

pub use build::*;
pub use catalog_metadata::*;
pub use descriptor_paths::{
    iter_system_ability_descriptor_paths, system_ability_descriptor_path,
    system_ability_descriptor_root, try_system_ability_descriptor_path, DescriptorPathError,
    SystemAbilityDescriptorGroup, SYSTEM_ABILITY_DESCRIPTOR_ROOT,
};
pub(crate) use publication::LocalAbilityPublicationSnapshot;
