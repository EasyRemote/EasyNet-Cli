// EasyNet CLI — remote desktop ability registration
// =================================================
//
// File: src/plugins/builtin/remote_desktop/registration.rs
// Description: Catalog mounting for the remote desktop plugin package.
//
// Protocol Responsibility:
// - Binds `remote_desktop.*` ability names to the package's handler
//   functions with the correct owner and call mode.
//
// Implementation Approach:
// - Iterate one compiled binding table. Each row owns both the public ability
//   spec and its handler kind.
// - Keep session, signaling, media, and permission behavior in `runtime.rs` and
//   sibling domain modules.
//
// Architectural Position:
// - Plugin package glue. This file is intentionally not a handler module and
//   not a manifest mirror; product metadata comes from the compiled ability spec
//   table, and `plugin.toml` is validated against that table at package-index
//   time.

use std::sync::Arc;

use serde_json::Value;

use crate::plugins::remote_desktop::constants::{
    ABILITY_ADD_ICE_CANDIDATE, ABILITY_ATTACH_SESSION, ABILITY_CREATE_SESSION, ABILITY_END_SESSION,
    ABILITY_PERMISSION_STATUS, ABILITY_REFRESH_LEASE, ABILITY_REQUEST_PERMISSION,
    ABILITY_SET_DESCRIPTION, ABILITY_SHOW_SESSION, ABILITY_WATCH_EVENTS,
};
use crate::plugins::remote_desktop::handlers;
use crate::plugins::remote_desktop::runtime::RemoteDesktopPlugin;
use crate::plugins::remote_desktop::schema;
use crate::runtime::ability_dispatch::{
    AxonAbilityCatalog, BidiSource, EnvelopeContext, OwnerKind, StreamSource,
};
use crate::runtime::agents::media::screen_snapshot::{ScreenSnapshotBackend, XcapBackend};
use crate::runtime::plugin_host::package::BuiltinPluginAbilitySpec;
use crate::runtime::plugin_host::{
    PluginAbilityLayer, PluginBidiWireKind, PluginCallMode, PluginRuntimeLimits,
};

type PluginRpcHandler =
    fn(Arc<RemoteDesktopPlugin>, EnvelopeContext, Value) -> anyhow::Result<Value>;
type StatelessRpcHandler = fn(EnvelopeContext, Value) -> anyhow::Result<Value>;
type PluginStreamHandler =
    fn(Arc<RemoteDesktopPlugin>, EnvelopeContext, Value) -> anyhow::Result<StreamSource>;
type PluginBidiHandler =
    fn(Arc<RemoteDesktopPlugin>, EnvelopeContext, Value) -> anyhow::Result<BidiSource>;

#[derive(Clone, Copy)]
enum RemoteDesktopAbilityBinding {
    Rpc { handler: PluginRpcHandler },
    StatelessRpc { handler: StatelessRpcHandler },
    Stream { handler: PluginStreamHandler },
    Bidi { handler: PluginBidiHandler },
}

/// One compiled remote-desktop ability row.
///
/// This is the Rust-side projection of `plugin.toml`: one public ability spec
/// plus exactly one executable handler binding. It prevents spec metadata and
/// handler dispatch from drifting into two independently-maintained tables.
#[derive(Clone, Copy)]
pub(crate) struct RemoteDesktopCompiledAbilityBinding {
    pub(crate) spec: BuiltinPluginAbilitySpec,
    handler: RemoteDesktopAbilityBinding,
}

impl RemoteDesktopAbilityBinding {
    #[cfg(test)]
    fn call_mode(self) -> PluginCallMode {
        match self {
            Self::Rpc { .. } | Self::StatelessRpc { .. } => PluginCallMode::Rpc,
            Self::Stream { .. } => PluginCallMode::Stream,
            Self::Bidi { .. } => PluginCallMode::Bidi,
        }
    }

    fn register(
        self,
        spec: &BuiltinPluginAbilitySpec,
        reg: &mut AxonAbilityCatalog,
        plugin: Arc<RemoteDesktopPlugin>,
    ) {
        match self {
            Self::Rpc { handler } => {
                reg.register_rpc_with_envelope_and_owner(
                    spec.name,
                    OwnerKind::Device,
                    Arc::new(move |env, args| handler(Arc::clone(&plugin), env, args)),
                );
            }
            Self::StatelessRpc { handler } => {
                reg.register_rpc_with_envelope_and_owner(
                    spec.name,
                    OwnerKind::Device,
                    Arc::new(handler),
                );
            }
            Self::Stream { handler } => {
                reg.register_stream_with_envelope_and_owner(
                    spec.name,
                    OwnerKind::Device,
                    Arc::new(move |env, args| handler(Arc::clone(&plugin), env, args)),
                );
            }
            Self::Bidi { handler } => {
                reg.register_bidi_with_envelope_and_owner(
                    spec.name,
                    OwnerKind::Device,
                    Arc::new(move |env, args| handler(Arc::clone(&plugin), env, args)),
                );
            }
        }
    }
}

/// Single runtime-side source for every exported remote desktop ability.
pub(crate) fn compiled_ability_bindings() -> &'static [RemoteDesktopCompiledAbilityBinding] {
    &[
        RemoteDesktopCompiledAbilityBinding {
            spec: BuiltinPluginAbilitySpec {
                name: ABILITY_CREATE_SESSION,
                layer: PluginAbilityLayer::Control,
                call_mode: PluginCallMode::Rpc,
                bidi_wire_kind: None,
                description: schema::create_session_description,
                input_schema: schema::create_session_input_schema,
            },
            handler: RemoteDesktopAbilityBinding::Rpc {
                handler: handlers::create_session::handle,
            },
        },
        RemoteDesktopCompiledAbilityBinding {
            spec: BuiltinPluginAbilitySpec {
                name: ABILITY_SHOW_SESSION,
                layer: PluginAbilityLayer::Observation,
                call_mode: PluginCallMode::Rpc,
                bidi_wire_kind: None,
                description: schema::show_session_description,
                input_schema: schema::show_session_input_schema,
            },
            handler: RemoteDesktopAbilityBinding::Rpc {
                handler: handlers::show_session::handle,
            },
        },
        RemoteDesktopCompiledAbilityBinding {
            spec: BuiltinPluginAbilitySpec {
                name: ABILITY_SET_DESCRIPTION,
                layer: PluginAbilityLayer::Control,
                call_mode: PluginCallMode::Rpc,
                bidi_wire_kind: None,
                description: schema::set_description_description,
                input_schema: schema::set_description_input_schema,
            },
            handler: RemoteDesktopAbilityBinding::Rpc {
                handler: handlers::set_description::handle,
            },
        },
        RemoteDesktopCompiledAbilityBinding {
            spec: BuiltinPluginAbilitySpec {
                name: ABILITY_ADD_ICE_CANDIDATE,
                layer: PluginAbilityLayer::Control,
                call_mode: PluginCallMode::Rpc,
                bidi_wire_kind: None,
                description: schema::add_ice_candidate_description,
                input_schema: schema::add_ice_candidate_input_schema,
            },
            handler: RemoteDesktopAbilityBinding::Rpc {
                handler: handlers::add_ice_candidate::handle,
            },
        },
        RemoteDesktopCompiledAbilityBinding {
            spec: BuiltinPluginAbilitySpec {
                name: ABILITY_WATCH_EVENTS,
                layer: PluginAbilityLayer::Observation,
                call_mode: PluginCallMode::Stream,
                bidi_wire_kind: None,
                description: schema::watch_events_description,
                input_schema: schema::watch_events_input_schema,
            },
            handler: RemoteDesktopAbilityBinding::Stream {
                handler: handlers::watch_events::handle,
            },
        },
        RemoteDesktopCompiledAbilityBinding {
            spec: BuiltinPluginAbilitySpec {
                name: ABILITY_REFRESH_LEASE,
                layer: PluginAbilityLayer::Control,
                call_mode: PluginCallMode::Rpc,
                bidi_wire_kind: None,
                description: schema::refresh_lease_description,
                input_schema: schema::refresh_lease_input_schema,
            },
            handler: RemoteDesktopAbilityBinding::Rpc {
                handler: handlers::refresh_lease::handle,
            },
        },
        RemoteDesktopCompiledAbilityBinding {
            spec: BuiltinPluginAbilitySpec {
                name: ABILITY_END_SESSION,
                layer: PluginAbilityLayer::Control,
                call_mode: PluginCallMode::Rpc,
                bidi_wire_kind: None,
                description: schema::end_session_description,
                input_schema: schema::end_session_input_schema,
            },
            handler: RemoteDesktopAbilityBinding::Rpc {
                handler: handlers::end_session::handle,
            },
        },
        RemoteDesktopCompiledAbilityBinding {
            spec: BuiltinPluginAbilitySpec {
                name: ABILITY_ATTACH_SESSION,
                layer: PluginAbilityLayer::Operational,
                call_mode: PluginCallMode::Bidi,
                bidi_wire_kind: Some(PluginBidiWireKind::JsonFrames),
                description: schema::attach_description,
                input_schema: schema::attach_input_schema,
            },
            handler: RemoteDesktopAbilityBinding::Bidi {
                handler: handlers::attach::handle,
            },
        },
        RemoteDesktopCompiledAbilityBinding {
            spec: BuiltinPluginAbilitySpec {
                name: ABILITY_PERMISSION_STATUS,
                layer: PluginAbilityLayer::Observation,
                call_mode: PluginCallMode::Rpc,
                bidi_wire_kind: None,
                description: schema::permission_status_description,
                input_schema: schema::permission_status_input_schema,
            },
            handler: RemoteDesktopAbilityBinding::StatelessRpc {
                handler: handlers::permission_status::handle,
            },
        },
        RemoteDesktopCompiledAbilityBinding {
            spec: BuiltinPluginAbilitySpec {
                name: ABILITY_REQUEST_PERMISSION,
                layer: PluginAbilityLayer::Control,
                call_mode: PluginCallMode::Rpc,
                bidi_wire_kind: None,
                description: schema::request_permission_description,
                input_schema: schema::request_permission_input_schema,
            },
            handler: RemoteDesktopAbilityBinding::StatelessRpc {
                handler: handlers::request_permission::handle,
            },
        },
    ]
}

/// Descriptor/package projection of the compiled binding table.
pub(crate) fn ability_specs() -> Vec<BuiltinPluginAbilitySpec> {
    compiled_ability_bindings()
        .iter()
        .map(|binding| binding.spec)
        .collect()
}

/// Register the remote desktop plugin with the production screen backend.
pub fn register(reg: &mut AxonAbilityCatalog, limits: PluginRuntimeLimits) {
    register_with_screen_backend(reg, Arc::new(XcapBackend), limits);
}

/// Register the remote desktop plugin with an injected screen backend.
///
/// This is the only non-production registration entry point. Unit tests inject
/// deterministic synthetic capture here while production uses [`register`].
pub(in crate::plugins::builtin::remote_desktop) fn register_with_screen_backend(
    reg: &mut AxonAbilityCatalog,
    backend: Arc<dyn ScreenSnapshotBackend>,
    limits: PluginRuntimeLimits,
) {
    let plugin = RemoteDesktopPlugin::new(backend, limits.into());
    for binding in compiled_ability_bindings() {
        binding
            .handler
            .register(&binding.spec, reg, Arc::clone(&plugin));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_remote_desktop_spec_is_projected_from_binding_table() {
        let specs = ability_specs();
        assert_eq!(specs.len(), compiled_ability_bindings().len());
        for (spec, binding) in specs.iter().zip(compiled_ability_bindings()) {
            assert_eq!(
                spec.name, binding.spec.name,
                "ability_specs must be a pure projection of the compiled binding table"
            );
        }
    }

    #[test]
    fn every_remote_desktop_handler_binding_matches_spec_call_mode() {
        for binding in compiled_ability_bindings() {
            assert_eq!(
                binding.handler.call_mode(),
                binding.spec.call_mode,
                "{} handler binding must match spec call_mode",
                binding.spec.name
            );
        }
    }
}
