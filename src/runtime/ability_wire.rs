// EasyNet CLI — ability wire profiles
// ====================================
//
// File: src/runtime/ability_wire.rs
// Description: Canonical runtime mapping from ability name to bidi wire codec.
//
// Protocol Responsibility:
// - Names the local bidi wire adapter used by daemon gRPC and `<self>.session`.
// - Keeps built-in ability adapters and plugin-declared adapters behind one
//   runtime query surface.
//
// Architectural Position:
// - Runtime metadata boundary. Services ask this module what wire profile an
//   ability uses; they do not inspect plugin packages directly.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use crate::runtime::plugin_host::{
    PluginBidiWireKind, PluginCallMode, PluginHostError, PluginRuntimeState,
};

/// Bidi wire codec used when an ability crosses the daemon/Axon session bridge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AbilityBidiWireKind {
    /// Terminal PTY stream. Binary chunks carry terminal bytes; supported
    /// control frames map to terminal control messages such as resize.
    Pty,
    /// File-transfer stream. Binary chunks and JSON control frames follow the
    /// daemon's file-transfer envelope contract.
    FileTransfer,
    /// JSON control-frame stream. Input and output payloads are structured JSON
    /// values owned by the ability implementation.
    JsonFrames,
}

/// Daemon-owned wire profile table for local bidi abilities.
///
/// What this is NOT: a package index or load planner. It is the service-facing
/// projection built after plugin load planning, so session dispatchers can
/// resolve wire codecs without reaching into process-global plugin helpers.
#[derive(Clone, Debug)]
pub struct AbilityWireRegistry {
    plugin_bidi: Arc<RwLock<BTreeMap<String, AbilityBidiWireKind>>>,
}

impl AbilityWireRegistry {
    /// Build the core non-plugin wire table. Tests and smoke boot paths use
    /// this when no plugin state has been wired yet.
    pub fn core() -> Self {
        Self::from_plugin_bidi(BTreeMap::new())
    }

    /// Build a wire table from one daemon plugin runtime snapshot.
    pub fn from_plugin_runtime_state(state: &PluginRuntimeState) -> Self {
        Self::from_plugin_bidi(plugin_bidi_from_state(state))
    }

    fn from_plugin_bidi(plugin_bidi: BTreeMap<String, AbilityBidiWireKind>) -> Self {
        Self {
            plugin_bidi: Arc::new(RwLock::new(plugin_bidi)),
        }
    }

    #[cfg(all(test, feature = "remote-desktop"))]
    pub(crate) fn for_test_plugin_bidi(
        entries: impl IntoIterator<Item = (String, AbilityBidiWireKind)>,
    ) -> Self {
        Self::from_plugin_bidi(entries.into_iter().collect())
    }

    /// Replace plugin-derived wire profiles in place while keeping the shared
    /// registry handle stable for already-booted services.
    pub fn replace_from_plugin_runtime_state(&self, state: &PluginRuntimeState) {
        *self
            .plugin_bidi
            .write()
            .expect("ability wire registry poisoned") = plugin_bidi_from_state(state);
    }

    /// Load the default daemon profile and project its wire table.
    ///
    /// Reads the shared default-state snapshot (F-050: directory reads =
    /// snapshot reads) — the write paths publish refreshed state, so this
    /// never re-indexes packages from disk on a warm process.
    pub fn load_default_profile() -> std::result::Result<Self, PluginHostError> {
        crate::runtime::plugin_host::default_state()
            .map(|state| Self::from_plugin_runtime_state(&state))
    }

    /// Return the declared bidi wire profile for a locally hosted ability.
    pub fn bidi_wire_kind_for(&self, ability: &str) -> Option<AbilityBidiWireKind> {
        core_bidi_wire_kind_for(ability).or_else(|| {
            self.plugin_bidi
                .read()
                .expect("ability wire registry poisoned")
                .get(ability)
                .copied()
        })
    }

    /// Return true when the runtime has a daemon/session wire adapter for
    /// `ability`.
    pub fn is_bidi_wire_ability(&self, ability: &str) -> bool {
        self.bidi_wire_kind_for(ability).is_some()
    }
}

fn plugin_bidi_from_state(state: &PluginRuntimeState) -> BTreeMap<String, AbilityBidiWireKind> {
    let mut plugin_bidi = BTreeMap::new();
    for entry in state.load_plan().entries() {
        if !entry.is_loaded() {
            continue;
        }
        for ability in entry.package().manifest().abilities() {
            if ability.call_mode() != PluginCallMode::Bidi {
                continue;
            }
            if let Some(kind) = ability.bidi_wire_kind() {
                plugin_bidi.insert(ability.name().to_string(), map_plugin_wire_kind(kind));
            }
        }
    }
    plugin_bidi
}

impl Default for AbilityWireRegistry {
    fn default() -> Self {
        Self::core()
    }
}

/// Return the declared bidi wire profile for a locally hosted ability.
pub fn bidi_wire_kind_for(ability: &str) -> Option<AbilityBidiWireKind> {
    core_bidi_wire_kind_for(ability).or_else(|| {
        AbilityWireRegistry::load_default_profile()
            .ok()
            .and_then(|registry| registry.bidi_wire_kind_for(ability))
    })
}

/// Return true when the runtime has a daemon/session wire adapter for `ability`.
pub fn is_bidi_wire_ability(ability: &str) -> bool {
    bidi_wire_kind_for(ability).is_some()
}

fn core_bidi_wire_kind_for(ability: &str) -> Option<AbilityBidiWireKind> {
    if ability == crate::runtime::agents::pty_attach_ability::ABILITY_PTY_SESSION_ATTACH {
        return Some(AbilityBidiWireKind::Pty);
    }
    if ability == crate::runtime::agents::file_transfer_ability::ABILITY_FILE_TRANSFER {
        return Some(AbilityBidiWireKind::FileTransfer);
    }
    None
}

fn map_plugin_wire_kind(kind: PluginBidiWireKind) -> AbilityBidiWireKind {
    match kind {
        PluginBidiWireKind::JsonFrames => AbilityBidiWireKind::JsonFrames,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn core_registry_contains_builtin_bidi_wires() {
        let registry = AbilityWireRegistry::core();
        assert_eq!(
            registry.bidi_wire_kind_for(
                crate::runtime::agents::pty_attach_ability::ABILITY_PTY_SESSION_ATTACH
            ),
            Some(AbilityBidiWireKind::Pty)
        );
        assert_eq!(
            registry.bidi_wire_kind_for(
                crate::runtime::agents::file_transfer_ability::ABILITY_FILE_TRANSFER
            ),
            Some(AbilityBidiWireKind::FileTransfer)
        );
    }

    #[test]
    fn shared_registry_handle_observes_replaced_plugin_bidi_snapshot() {
        let registry = AbilityWireRegistry::core();
        assert_eq!(
            registry.bidi_wire_kind_for("device.test.bidi"),
            None,
            "core registry starts without plugin wire profiles"
        );

        let root = tempfile::tempdir().expect("package root");
        write_bidi_sidecar_package(root.path(), "device.test.bidi");
        let package =
            crate::runtime::plugin_host::package::PluginPackage::from_installed(root.path(), None)
                .expect("package");
        let index =
            crate::runtime::plugin_host::PluginPackageIndex::from_packages(vec![Arc::new(package)])
                .expect("index");
        let state = crate::runtime::plugin_host::PluginRuntimeState::from_index(index);

        registry.replace_from_plugin_runtime_state(&state);

        assert_eq!(
            registry.bidi_wire_kind_for("device.test.bidi"),
            Some(AbilityBidiWireKind::JsonFrames),
            "existing registry handle must see hot-reloaded plugin bidi wire profiles"
        );
    }

    fn write_bidi_sidecar_package(root: &std::path::Path, ability: &str) {
        std::fs::create_dir_all(root.join("abilities")).expect("abilities dir");
        std::fs::create_dir_all(root.join("bin")).expect("bin dir");
        std::fs::write(
            root.join("plugin.toml"),
            format!(
                r#"
schema_version = "1"
id = "test.sidecar.bidi"
version = "0.1.0"
kind = "sidecar"
entrypoint = "bin/sidecar"
abilities = ["abilities/*.ability.toml"]
permissions = []
resources = []
platforms = []

[limits]
max_sessions = 1
max_frame_queue = 1

[[ability_metadata]]
name = "{ability}"
layer = "operational"
call_mode = "bidi"
bidi_wire_kind = "json_frames"
"#
            ),
        )
        .expect("manifest");
        std::fs::write(
            root.join(format!("abilities/{ability}.ability.toml")),
            format!(
                r#"schema_version = "1"
name = "{ability}"
description = "test descriptor for {ability}"

[input_schema]
type = "object"
additionalProperties = false
"#
            ),
        )
        .expect("descriptor");
        let sidecar_path = root.join("bin/sidecar");
        std::fs::write(&sidecar_path, "#!/bin/sh\n").expect("sidecar bin");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&sidecar_path)
                .expect("sidecar bin metadata")
                .permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&sidecar_path, perms).expect("sidecar bin executable");
        }
    }
}
