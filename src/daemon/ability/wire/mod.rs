// EasyNet CLI — ability wire profiles
// ====================================
//
// File: src/daemon/ability/wire/mod.rs
// Description: Canonical daemon mapping from ability name to bidi wire codec.
//
// Protocol Responsibility:
// - Names the local bidi wire adapter used by daemon gRPC and `session.open`.
// - Keeps built-in ability adapters and plugin-declared adapters behind one
//   daemon query surface.
//
// Architectural Position:
// - Daemon ability metadata boundary. Invocation dispatch asks this module what
//   wire profile an ability uses; it does not inspect plugin packages directly.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use crate::daemon::ability::CallMode;
use crate::daemon::plugins::{PluginBidiWireKind, PluginHostError, PluginRuntimeState};

/// Reserved binary stream carrying structured PTY lifecycle controls that do
/// not have a first-class Axon `BidiControl` variant.
pub(crate) const PTY_CONTROL_STREAM_ID: u32 = u32::MAX;

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
        crate::daemon::plugins::default_state().map(|state| Self::from_plugin_runtime_state(&state))
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
            if ability.call_mode() != CallMode::Bidi {
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

pub(crate) fn core_bidi_wire_kind_for(ability: &str) -> Option<AbilityBidiWireKind> {
    if ability == crate::daemon::ability::builtins::device_control::terminal::attach::ABILITY_TERMINAL_ATTACH {
        return Some(AbilityBidiWireKind::Pty);
    }
    if ability
        == crate::daemon::ability::builtins::device_control::file_transfer::ABILITY_FILE_TRANSFER
    {
        return Some(AbilityBidiWireKind::FileTransfer);
    }
    if ability == crate::daemon::ability::builtins::device_control::net_tunnel::ABILITY_NET_TUNNEL {
        return Some(AbilityBidiWireKind::JsonFrames);
    }
    None
}

fn map_plugin_wire_kind(kind: PluginBidiWireKind) -> AbilityBidiWireKind {
    match kind {
        PluginBidiWireKind::JsonFrames => AbilityBidiWireKind::JsonFrames,
        PluginBidiWireKind::MetadataJsonPlusBinary => AbilityBidiWireKind::JsonFrames,
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
                crate::daemon::ability::builtins::device_control::terminal::attach::ABILITY_TERMINAL_ATTACH
            ),
            Some(AbilityBidiWireKind::Pty)
        );
        assert_eq!(
            registry.bidi_wire_kind_for(
                crate::daemon::ability::builtins::device_control::file_transfer::ABILITY_FILE_TRANSFER
            ),
            Some(AbilityBidiWireKind::FileTransfer)
        );
        assert_eq!(
            registry.bidi_wire_kind_for(
                crate::daemon::ability::builtins::device_control::net_tunnel::ABILITY_NET_TUNNEL
            ),
            Some(AbilityBidiWireKind::JsonFrames)
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
            crate::daemon::plugins::package::PluginPackage::from_installed(root.path(), None)
                .expect("package");
        let index =
            crate::daemon::plugins::PluginPackageIndex::from_packages(vec![Arc::new(package)])
                .expect("index");
        let state = crate::daemon::plugins::PluginRuntimeState::from_index(index);

        registry.replace_from_plugin_runtime_state(&state);

        assert_eq!(
            registry.bidi_wire_kind_for("device.test.bidi"),
            Some(AbilityBidiWireKind::JsonFrames),
            "existing registry handle must see hot-reloaded plugin bidi wire profiles"
        );
    }

    #[test]
    fn plugin_metadata_json_plus_binary_maps_to_binary_capable_json_adapter() {
        let root = tempfile::tempdir().expect("package root");
        write_bidi_sidecar_package_with_wire_kind(
            root.path(),
            "remote_desktop.attach",
            "metadata_json_plus_binary",
        );
        let package =
            crate::daemon::plugins::package::PluginPackage::from_installed(root.path(), None)
                .expect("package");
        let index =
            crate::daemon::plugins::PluginPackageIndex::from_packages(vec![Arc::new(package)])
                .expect("index");
        let state = crate::daemon::plugins::PluginRuntimeState::from_index(index);
        let registry = AbilityWireRegistry::from_plugin_runtime_state(&state);

        assert_eq!(
            registry.bidi_wire_kind_for("remote_desktop.attach"),
            Some(AbilityBidiWireKind::JsonFrames),
            "metadata/binary product declaration must keep using the existing binary-capable local adapter"
        );
    }

    fn write_bidi_sidecar_package(root: &std::path::Path, ability: &str) {
        write_bidi_sidecar_package_with_wire_kind(root, ability, "json_frames")
    }

    fn write_bidi_sidecar_package_with_wire_kind(
        root: &std::path::Path,
        ability: &str,
        wire_kind: &str,
    ) {
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
bidi_wire_kind = "{wire_kind}"
"#
            ),
        )
        .expect("manifest");
        std::fs::write(
            root.join(format!("abilities/{ability}.ability.toml")),
            format!(
                r#"schema_version = "3"
name = "{ability}"
descriptor_version = "1.2.3"
description = "test descriptor for {ability}"
exposure = "task"
dedicated_surface = "none"
subject_contract_kind = "authenticated-user"
admission_action = "stream"

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
