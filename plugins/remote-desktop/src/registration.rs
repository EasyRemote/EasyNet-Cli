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

use crate::daemon::ability::builtins::resources::media::screen_snapshot::ScreenSnapshotBackend;
#[cfg(all(not(feature = "native-media"), feature = "headless-media"))]
use crate::daemon::ability::builtins::resources::media::screen_snapshot::SyntheticScreenBackend;
#[cfg(feature = "native-media")]
use crate::daemon::ability::builtins::resources::media::screen_snapshot::XcapBackend;
use crate::daemon::ability::descriptors::AdmissionAction;
use crate::daemon::ability::dispatch::{
    AbilityHandlerFailure, BidiSource, EnvelopeContext, StreamSource,
};
use crate::daemon::ability::{AbilityImplSource, CallMode};
use crate::daemon::plugins::package::{BuiltinPluginAbilityHints, BuiltinPluginAbilitySpec};
use crate::daemon::plugins::remote_desktop::consent_registry::ConsentTicketError;
use crate::daemon::plugins::remote_desktop::constants::{
    ABILITY_ADD_ICE_CANDIDATE, ABILITY_ATTACH_SESSION, ABILITY_CREATE_SESSION, ABILITY_END_SESSION,
    ABILITY_GRANT_CONSENT, ABILITY_PERMISSION_STATUS, ABILITY_REFRESH_LEASE,
    ABILITY_REPORT_CLIENT_STATE, ABILITY_REQUEST_PERMISSION, ABILITY_SET_DESCRIPTION,
    ABILITY_SHOW_SESSION, ABILITY_WATCH_EVENTS,
};
use crate::daemon::plugins::remote_desktop::errors::RemoteDesktopError;
use crate::daemon::plugins::remote_desktop::handlers;
use crate::daemon::plugins::remote_desktop::runtime::RemoteDesktopPlugin;
use crate::daemon::plugins::remote_desktop::schema;
use crate::daemon::plugins::remote_desktop::target::RemoteAppTargetError;
use crate::daemon::plugins::{
    PluginAbilityLayer, PluginBidiWireKind, PluginContributionBuilder, PluginRuntimeLimits, Result,
};
use axon_sdk::invocation::{AxonError, AxonErrorKind, ErrorCode, ErrorStage, SecurityClass};

type PluginRpcHandler =
    fn(Arc<RemoteDesktopPlugin>, EnvelopeContext, Value) -> anyhow::Result<Value>;
type StatelessRpcHandler = fn(EnvelopeContext, Value) -> anyhow::Result<Value>;
type PluginStreamHandler =
    fn(Arc<RemoteDesktopPlugin>, EnvelopeContext, Value) -> anyhow::Result<StreamSource>;
type PluginBidiHandler =
    fn(Arc<RemoteDesktopPlugin>, EnvelopeContext, Value) -> anyhow::Result<BidiSource>;

const RESOURCE_SUBJECT_KINDS: &[&str] = &["resource"];
const PERMISSION_PROBE_SUBJECT_KINDS: &[&str] = &["agent", "resource"];

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
                Arc::new(move |env, args| {
                    classify_handler_result(handler(Arc::clone(&plugin), env, args))
                }),
            ),
            Self::StatelessRpc { handler } => builder.rpc(
                spec.name,
                manifest,
                AbilityImplSource::BuiltinPlugin,
                runtime_env,
                Arc::new(move |env, args| classify_handler_result(handler(env, args))),
            ),
            Self::Stream { handler } => builder.stream(
                spec.name,
                manifest,
                AbilityImplSource::BuiltinPlugin,
                runtime_env,
                Arc::new(move |env, args| {
                    classify_handler_result(handler(Arc::clone(&plugin), env, args))
                }),
            ),
            Self::Bidi { handler } => builder.bidi(
                spec.name,
                manifest,
                AbilityImplSource::BuiltinPlugin,
                runtime_env,
                Arc::new(move |env, args| {
                    classify_handler_result(handler(Arc::clone(&plugin), env, args))
                }),
            ),
        }
    }
}

fn classify_handler_result<T>(result: anyhow::Result<T>) -> anyhow::Result<T> {
    result.map_err(|error| {
        let axon_error = if let Some(error) = error.downcast_ref::<RemoteDesktopError>() {
            error.to_axon()
        } else if let Some(error) = error.downcast_ref::<ConsentTicketError>() {
            consent_ticket_error_to_axon(error)
        } else if let Some(error) = error.downcast_ref::<RemoteAppTargetError>() {
            error.to_axon()
        } else {
            AxonError::new(AxonErrorKind::Internal)
                .with_code(ErrorCode::ExecutionFailed)
                .with_stage(ErrorStage::Execution)
                .with_security_class(SecurityClass::Resource)
                .with_message(error.to_string())
        };
        anyhow::Error::new(AbilityHandlerFailure::new(axon_error))
    })
}

fn consent_ticket_error_to_axon(error: &ConsentTicketError) -> AxonError {
    let (kind, code, stage, security_class) = match error {
        ConsentTicketError::Full => (
            AxonErrorKind::ResourceExhausted,
            ErrorCode::ResourceExhausted,
            ErrorStage::Quota,
            SecurityClass::Resource,
        ),
        ConsentTicketError::Invalid => (
            AxonErrorKind::PermissionDenied,
            ErrorCode::AuthorityExpired,
            ErrorStage::AuthorityValidation,
            SecurityClass::Authority,
        ),
        ConsentTicketError::CallerMismatch => (
            AxonErrorKind::PermissionDenied,
            ErrorCode::AuthorityCallerMismatch,
            ErrorStage::AuthorityValidation,
            SecurityClass::Authority,
        ),
        ConsentTicketError::SubjectMismatch => (
            AxonErrorKind::PermissionDenied,
            ErrorCode::AuthoritySubjectMismatch,
            ErrorStage::AuthorityValidation,
            SecurityClass::Authority,
        ),
        ConsentTicketError::IntentMismatch => (
            AxonErrorKind::PermissionDenied,
            ErrorCode::AuthorityScopeViolation,
            ErrorStage::AuthorityValidation,
            SecurityClass::Authority,
        ),
    };
    AxonError::new(kind)
        .with_code(code)
        .with_stage(stage)
        .with_security_class(security_class)
        .with_message(error.to_string())
}

/// Single runtime-side source for every exported remote desktop ability.
pub(crate) fn compiled_ability_bindings() -> &'static [RemoteDesktopCompiledAbilityBinding] {
    &[
        RemoteDesktopCompiledAbilityBinding {
            spec: BuiltinPluginAbilitySpec {
                name: ABILITY_GRANT_CONSENT,
                layer: PluginAbilityLayer::Control,
                call_mode: CallMode::Rpc,
                admission_action: AdmissionAction::Manage,
                bidi_wire_kind: None,
                subject_ura_kinds: RESOURCE_SUBJECT_KINDS,
                hints: BuiltinPluginAbilityHints::NONE,
                description: schema::grant_consent_description,
                input_schema: schema::grant_consent_input_schema,
            },
            handler: RemoteDesktopAbilityBinding::Rpc {
                handler: handlers::grant_consent::handle,
            },
        },
        RemoteDesktopCompiledAbilityBinding {
            spec: BuiltinPluginAbilitySpec {
                name: ABILITY_CREATE_SESSION,
                layer: PluginAbilityLayer::Control,
                call_mode: CallMode::Rpc,
                admission_action: AdmissionAction::Manage,
                bidi_wire_kind: None,
                subject_ura_kinds: RESOURCE_SUBJECT_KINDS,
                hints: BuiltinPluginAbilityHints::NONE,
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
                admission_action: AdmissionAction::Read,
                bidi_wire_kind: None,
                subject_ura_kinds: RESOURCE_SUBJECT_KINDS,
                hints: BuiltinPluginAbilityHints::NONE,
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
                admission_action: AdmissionAction::Manage,
                bidi_wire_kind: None,
                subject_ura_kinds: RESOURCE_SUBJECT_KINDS,
                hints: BuiltinPluginAbilityHints::NONE,
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
                admission_action: AdmissionAction::Manage,
                bidi_wire_kind: None,
                subject_ura_kinds: RESOURCE_SUBJECT_KINDS,
                hints: BuiltinPluginAbilityHints::NONE,
                description: schema::add_ice_candidate_description,
                input_schema: schema::add_ice_candidate_input_schema,
            },
            handler: RemoteDesktopAbilityBinding::Rpc {
                handler: handlers::add_ice_candidate::handle,
            },
        },
        RemoteDesktopCompiledAbilityBinding {
            spec: BuiltinPluginAbilitySpec {
                name: ABILITY_REPORT_CLIENT_STATE,
                layer: PluginAbilityLayer::Control,
                call_mode: CallMode::Rpc,
                admission_action: AdmissionAction::Manage,
                bidi_wire_kind: None,
                subject_ura_kinds: RESOURCE_SUBJECT_KINDS,
                hints: BuiltinPluginAbilityHints::NONE,
                description: schema::report_client_state_description,
                input_schema: schema::report_client_state_input_schema,
            },
            handler: RemoteDesktopAbilityBinding::Rpc {
                handler: handlers::report_client_state::handle,
            },
        },
        RemoteDesktopCompiledAbilityBinding {
            spec: BuiltinPluginAbilitySpec {
                name: ABILITY_WATCH_EVENTS,
                layer: PluginAbilityLayer::Observation,
                call_mode: CallMode::Stream,
                admission_action: AdmissionAction::Stream,
                bidi_wire_kind: None,
                subject_ura_kinds: RESOURCE_SUBJECT_KINDS,
                hints: BuiltinPluginAbilityHints::NONE,
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
                admission_action: AdmissionAction::Manage,
                bidi_wire_kind: None,
                subject_ura_kinds: RESOURCE_SUBJECT_KINDS,
                hints: BuiltinPluginAbilityHints::NONE,
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
                admission_action: AdmissionAction::Manage,
                bidi_wire_kind: None,
                subject_ura_kinds: RESOURCE_SUBJECT_KINDS,
                hints: BuiltinPluginAbilityHints::NONE,
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
                admission_action: AdmissionAction::Stream,
                bidi_wire_kind: Some(PluginBidiWireKind::JsonFrames),
                subject_ura_kinds: RESOURCE_SUBJECT_KINDS,
                hints: BuiltinPluginAbilityHints::NONE,
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
                admission_action: AdmissionAction::Read,
                bidi_wire_kind: None,
                subject_ura_kinds: PERMISSION_PROBE_SUBJECT_KINDS,
                hints: BuiltinPluginAbilityHints::NONE,
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
                admission_action: AdmissionAction::Manage,
                bidi_wire_kind: None,
                subject_ura_kinds: PERMISSION_PROBE_SUBJECT_KINDS,
                hints: BuiltinPluginAbilityHints::NONE,
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
    #[cfg(feature = "native-media")]
    {
        return contribute_with_screen_backend(builder, Arc::new(XcapBackend), limits);
    }
    #[cfg(all(not(feature = "native-media"), feature = "headless-media"))]
    {
        return contribute_with_screen_backend(builder, Arc::new(SyntheticScreenBackend), limits);
    }
    #[cfg(not(any(feature = "native-media", feature = "headless-media")))]
    {
        let _ = builder;
        let _ = limits;
        return anyhow::bail!(
            "remote-desktop requires either native-media or headless-media provider support"
        );
    }
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
    use crate::daemon::ability::descriptors::ScopeRule;
    use crate::daemon::ability::dispatch::{AbilityAuthorityContext, AxonAbilityCatalog};
    use crate::daemon::ability::CallMode as DescriptorCallMode;
    use crate::daemon::plugins::remote_desktop::target::TargetResolutionError;
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
    fn consent_ticket_failures_are_machine_readable() {
        let err =
            classify_handler_result::<()>(Err(ConsentTicketError::Invalid.into())).unwrap_err();
        let failure = err
            .downcast_ref::<AbilityHandlerFailure>()
            .expect("registration must preserve structured handler failure");
        let axon_error = failure.axon_error();

        assert_eq!(axon_error.kind, AxonErrorKind::PermissionDenied);
        assert_eq!(axon_error.code, ErrorCode::AuthorityExpired);
        assert_eq!(axon_error.stage, Some(ErrorStage::AuthorityValidation));
        assert_eq!(axon_error.security_class, Some(SecurityClass::Authority));
        assert!(
            !axon_error.message.is_empty(),
            "public ability errors must not collapse to an empty body"
        );
    }

    #[test]
    fn target_resolution_failures_are_machine_readable() {
        let err = classify_handler_result::<()>(Err(RemoteAppTargetError::new(
            ABILITY_CREATE_SESSION,
            TargetResolutionError::TargetMetadataIncomplete,
            "window targets require window_id",
        )
        .into()))
        .unwrap_err();
        let failure = err
            .downcast_ref::<AbilityHandlerFailure>()
            .expect("registration must preserve structured target failure");
        let axon_error = failure.axon_error();

        assert_eq!(axon_error.kind, AxonErrorKind::InvalidArgument);
        assert_eq!(axon_error.code, ErrorCode::RequestMetadataInvalid);
        assert_eq!(axon_error.reason, "target_metadata_incomplete");
        assert_eq!(axon_error.stage, Some(ErrorStage::RequestValidation));
        assert_eq!(axon_error.security_class, Some(SecurityClass::Resource));
        assert_eq!(
            axon_error
                .context
                .get("frontend_action")
                .map(String::as_str),
            Some("show_unsupported")
        );
        assert!(
            axon_error
                .message
                .contains("reason=target_metadata_incomplete"),
            "target failure must keep canonical reason in message: {}",
            axon_error.message
        );
    }

    #[test]
    fn unknown_handler_failures_still_have_structured_payloads() {
        let err = classify_handler_result::<()>(Err(anyhow::anyhow!("boom"))).unwrap_err();
        let failure = err
            .downcast_ref::<AbilityHandlerFailure>()
            .expect("registration must wrap untyped errors for adapter projection");
        let axon_error = failure.axon_error();

        assert_eq!(axon_error.kind, AxonErrorKind::Internal);
        assert_eq!(axon_error.code, ErrorCode::ExecutionFailed);
        assert_eq!(axon_error.stage, Some(ErrorStage::Execution));
        assert_eq!(axon_error.security_class, Some(SecurityClass::Resource));
        assert_eq!(axon_error.message, "boom");
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
        let mut reg = AxonAbilityCatalog::new_metadata_only_with_authority_context(
            AbilityAuthorityContext::for_device_authority_root(
                "easynet:///r/acme/device/test-remote-desktop",
            )
            .expect("test Device authority root must be canonical"),
        );
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
            descriptor.scope_subjects,
            ScopeRule::OnlyUraKinds(vec!["resource".to_string()]),
            "remote_desktop.create_session descriptor must reject non-resource subjects before handler dispatch"
        );
        assert_eq!(
            descriptor.input_schema(),
            &schema::create_session_input_schema()
        );
        let permission_status = rows
            .iter()
            .find(|row| row.name == ABILITY_PERMISSION_STATUS)
            .expect("remote_desktop.permission_status must be catalogued");
        assert_eq!(
            permission_status.descriptor.scope_subjects,
            ScopeRule::OnlyUraKinds(vec!["agent".to_string(), "resource".to_string()]),
            "host-local permission probes must admit only descriptor-bound User invoke resources or local-system Agent subjects"
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
