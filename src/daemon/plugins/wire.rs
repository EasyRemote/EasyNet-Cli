// EasyNet CLI — plugin ability wire registry
// ==========================================
//
// File: src/daemon/plugins/wire.rs
// Description: Ability-layer and bidi wire profile lookup from package index.

use crate::daemon::plugins::index::PluginPackageIndex;
use crate::daemon::plugins::manifest::{PluginAbilityLayer, PluginBidiWireKind};

/// Registry over plugin wire metadata.
pub struct PluginWireRegistry<'a> {
    index: &'a PluginPackageIndex,
}

impl<'a> PluginWireRegistry<'a> {
    /// Build a registry from package index state.
    pub const fn new(index: &'a PluginPackageIndex) -> Self {
        Self { index }
    }

    /// Resolve product/runtime layer for a plugin ability.
    pub fn ability_layer_for(&self, name: &str) -> Option<PluginAbilityLayer> {
        self.index.package_for_ability(name).and_then(|package| {
            package
                .manifest()
                .ability(name)
                .map(|ability| ability.layer())
        })
    }

    /// Resolve plugin-declared bidi wire kind for a plugin ability.
    pub fn plugin_bidi_wire_kind(&self, name: &str) -> Option<PluginBidiWireKind> {
        self.index.package_for_ability(name).and_then(|package| {
            package
                .manifest()
                .ability(name)
                .and_then(|ability| ability.bidi_wire_kind())
        })
    }

    /// Resolve plugin-owned descriptor path.
    pub fn ability_descriptor_path(&self, name: &str) -> Option<String> {
        self.index.package_for_ability(name).and_then(|package| {
            package
                .manifest()
                .ability(name)
                .map(|ability| ability.descriptor_path().to_string())
        })
    }
}
