// EasyNet CLI — remote desktop ability registration
// =================================================
//
// File: plugins/remote-desktop/src/registration.rs
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

use crate::daemon::ability::builtins::resources::media::screen_snapshot::{
    ScreenSnapshotBackend, XcapBackend,
};
use crate::daemon::ability::dispatch::{BidiSource, EnvelopeContext, StreamSource};
use crate::daemon::ability::AbilityImplSource;
use crate::daemon::plugins::package::BuiltinPluginAbilitySpec;
use crate::daemon::plugins::remote_desktop::constants::{
    ABILITY_ADD_ICE_CANDIDATE, ABILITY_ATTACH_SESSION, ABILITY_CREATE_SESSION, ABILITY_END_SESSION,
    ABILITY_PERMISSION_STATUS, ABILITY_REFRESH_LEASE, ABILITY_REQUEST_PERMISSION,
    ABILITY_SET_DESCRIPTION, ABILITY_SHOW_SESSION, ABILITY_WATCH_EVENTS,
};
use crate::daemon::plugins::remote_desktop::handlers;
use crate::daemon::plugins::remote_desktop::runtime::RemoteDesktopPlugin;
use crate::daemon::plugins::remote_desktop::schema;
use crate::daemon::plugins::{
    CallMode, PluginAbilityLayer, PluginBidiWireKind, PluginContributionBuilder,
    PluginRuntimeLimits, Result,
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
    fn call_mode(self) -> CallMode {
        match self {
            Self::Rpc { .. } | Self::StatelessRpc { .. } => CallMode::Rpc,
            Self::Stream { .. } => CallMode::Stream,
            Self::Bidi { .. } => CallMode::Bidi,
        }
    }

    fn contribute(
        self,
        spec: &BuiltinPluginAbilitySpec,
        builder: &mut PluginContributionBuilder,
        plugin: Arc<RemoteDesktopPlugin>,
    ) -> Result<()> {
        let manifest = spec.to_registry_manifest()?;
        let runtime_env = builder.plugin_runtime_env();
        match self {
            Self::Rpc { handler } => builder.rpc(
                spec.name,
                manifest,
                AbilityImplSource::BuiltinPlugin,
                runtime_env,
                Arc::new(move |env, args| handler(Arc::clone(&plugin), env, args)),
            ),
            Self::StatelessRpc { handler } => builder.rpc(
                spec.name,
                manifest,
                AbilityImplSource::BuiltinPlugin,
                runtime_env,
                Arc::new(handler),
            ),
            Self::Stream { handler } => builder.stream(
                spec.name,
                manifest,
                AbilityImplSource::BuiltinPlugin,
                runtime_env,
                Arc::new(move |env, args| handler(Arc::clone(&plugin), env, args)),
            ),
            Self::Bidi { handler } => builder.bidi(
                spec.name,
                manifest,
                AbilityImplSource::BuiltinPlugin,
                runtime_env,
                Arc::new(move |env, args| handler(Arc::clone(&plugin), env, args)),
            ),
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
                call_mode: CallMode::Rpc,
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
                call_mode: CallMode::Rpc,
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
                call_mode: CallMode::Rpc,
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
                call_mode: CallMode::Rpc,
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
                call_mode: CallMode::Stream,
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
                call_mode: CallMode::Rpc,
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
                call_mode: CallMode::Rpc,
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
                call_mode: CallMode::Bidi,
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
                call_mode: CallMode::Rpc,
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
                call_mode: CallMode::Rpc,
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

/// Contribute the remote desktop plugin with the production screen backend.
pub fn contribute(
    builder: &mut PluginContributionBuilder,
    limits: PluginRuntimeLimits,
) -> Result<()> {
    contribute_with_screen_backend(builder, Arc::new(XcapBackend), limits)
}

/// Contribute the remote desktop plugin with an injected screen backend.
///
/// This is the only non-production registration entry point. Unit tests inject
/// deterministic synthetic capture here while production uses [`contribute`].
pub(in crate::daemon::plugins::remote_desktop) fn contribute_with_screen_backend(
    builder: &mut PluginContributionBuilder,
    backend: Arc<dyn ScreenSnapshotBackend>,
    limits: PluginRuntimeLimits,
) -> Result<()> {
    let plugin = RemoteDesktopPlugin::new(backend, limits.into());
    for binding in compiled_ability_bindings() {
        binding
            .handler
            .contribute(&binding.spec, builder, Arc::clone(&plugin))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::ability::builtins::resources::media::screen_snapshot::SyntheticScreenBackend;
    use crate::daemon::ability::dispatch::AxonAbilityCatalog;
    use crate::daemon::ability::CallMode as DescriptorCallMode;
    use crate::daemon::plugins::{
        DaemonPluginBinder, PluginContributionSet, PluginKind, PluginRequirementSet,
    };

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

    #[test]
    fn registration_publishes_remote_desktop_descriptor_to_catalog_snapshot() {
        let limits = crate::daemon::plugins::remote_desktop::test_support::test_runtime_limits();
        let mut builder = PluginContributionBuilder::new(
            "easynet.remote_desktop",
            "0.1.0",
            PluginKind::Builtin,
            limits,
            PluginRequirementSet::default(),
            Vec::new(),
        );
        contribute_with_screen_backend(&mut builder, Arc::new(SyntheticScreenBackend), limits)
            .expect("remote desktop contribution");
        let contribution = builder
            .finish()
            .expect("remote desktop package contribution");
        let contributions = PluginContributionSet::new(vec![contribution]);
        let mut reg = AxonAbilityCatalog::new();
        DaemonPluginBinder::static_catalog(&mut reg)
            .bind_set(&contributions)
            .expect("bind remote desktop contribution");

        let rows = reg.authority_ability_catalog_snapshot();
        let create_session = rows
            .iter()
            .find(|row| row.name == ABILITY_CREATE_SESSION)
            .expect("remote_desktop.create_session must be catalogued");
        let descriptor = &create_session.descriptor;

        assert_eq!(descriptor.description, schema::create_session_description());
        assert_eq!(
            descriptor.input_schema(),
            &schema::create_session_input_schema()
        );
        let record = reg
            .control_plane_record_for_mode(ABILITY_CREATE_SESSION, DescriptorCallMode::Rpc)
            .expect("remote desktop control-plane lookup is unambiguous")
            .expect("remote desktop control-plane record");
        assert_eq!(
            *record.implementation().source(),
            AbilityImplSource::BuiltinPlugin
        );
    }
}
