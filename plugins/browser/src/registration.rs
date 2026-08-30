//! Browser plugin compiled ability table and contribution binding.
//! ===============================================================
//!
//! File: plugins/browser/src/registration.rs
//! Description: Single source of descriptor metadata and executable handlers.
//!
//! Protocol Responsibility:
//! - Bind browser behavior to canonical Axon call modes and subject scopes.
//!
//! Implementation Approach:
//! - Compile seven descriptor/handler rows and project them through the generic
//!   daemon-owned plugin contribution builder.
//!
//! Usage Contract:
//! - The plugin host supplies runtime limits and owns authority assignment.
//!
//! Architectural Position:
//! - Browser package registration seam; no runtime or authority ownership.

use std::sync::Arc;

use serde_json::Value;

use crate::daemon::ability::descriptors::AdmissionAction;
use crate::daemon::ability::dispatch::{
    AbilityHandlerFailure, BidiSource, EnvelopeContext, StreamSource,
};
use crate::daemon::ability::{AbilityImplSource, CallMode};
use crate::daemon::plugins::package::{
    BuiltinPluginAbilityHints, BuiltinPluginAbilitySpec, BuiltinPluginFrontendContract,
};
use crate::daemon::plugins::{
    PluginAbilityLayer, PluginBidiWireKind, PluginContributionBuilder, PluginRuntimeLimits, Result,
};

use super::constants::*;
use super::errors::BrowserResult;
use super::handlers;
use super::runtime::BrowserRuntime;
use super::schema;

type RpcHandler = fn(Arc<BrowserRuntime>, EnvelopeContext, Value) -> BrowserResult<Value>;
type StreamHandler = fn(Arc<BrowserRuntime>, EnvelopeContext, Value) -> BrowserResult<StreamSource>;
type BidiHandler = fn(Arc<BrowserRuntime>, EnvelopeContext, Value) -> BrowserResult<BidiSource>;

#[derive(Clone, Copy)]
enum BrowserAbilityHandler {
    Rpc(RpcHandler),
    Stream(StreamHandler),
    Bidi(BidiHandler),
}

#[derive(Clone, Copy)]
struct BrowserCompiledAbility {
    spec: BuiltinPluginAbilitySpec,
    handler: BrowserAbilityHandler,
}

// Axon subjects are executable/operated entities. User URAs identify callers,
// not descriptor-bound subjects, so opening is scoped to the publishing Agent.
const OPEN_SUBJECT_KINDS: &[&str] = &["agent"];
const SESSION_SUBJECT_KINDS: &[&str] = &["resource"];

pub fn ability_specs() -> Vec<BuiltinPluginAbilitySpec> {
    compiled_abilities()
        .into_iter()
        .map(|row| row.spec)
        .collect()
}

pub fn contribute(
    builder: &mut PluginContributionBuilder,
    limits: PluginRuntimeLimits,
) -> Result<()> {
    let runtime = BrowserRuntime::new(limits.max_sessions(), limits.max_frame_queue());
    let runtime_env = builder.plugin_runtime_env();
    for row in compiled_abilities() {
        let manifest = row.spec.to_registry_manifest()?;
        match row.handler {
            BrowserAbilityHandler::Rpc(handler) => {
                let runtime = Arc::clone(&runtime);
                builder.rpc(
                    row.spec.name,
                    manifest,
                    AbilityImplSource::BuiltinPlugin,
                    runtime_env.clone(),
                    Arc::new(move |env, args| {
                        handler(Arc::clone(&runtime), env, args).map_err(classify_error)
                    }),
                )?;
            }
            BrowserAbilityHandler::Stream(handler) => {
                let runtime = Arc::clone(&runtime);
                builder.stream(
                    row.spec.name,
                    manifest,
                    AbilityImplSource::BuiltinPlugin,
                    runtime_env.clone(),
                    Arc::new(move |env, args| {
                        handler(Arc::clone(&runtime), env, args).map_err(classify_error)
                    }),
                )?;
            }
            BrowserAbilityHandler::Bidi(handler) => {
                let runtime = Arc::clone(&runtime);
                builder.bidi(
                    row.spec.name,
                    manifest,
                    AbilityImplSource::BuiltinPlugin,
                    runtime_env.clone(),
                    Arc::new(move |env, args| {
                        handler(Arc::clone(&runtime), env, args).map_err(classify_error)
                    }),
                )?;
            }
        }
    }
    Ok(())
}

fn classify_error(error: super::errors::BrowserError) -> anyhow::Error {
    anyhow::Error::new(AbilityHandlerFailure::new(error.to_axon()))
}

fn compiled_abilities() -> [BrowserCompiledAbility; 7] {
    [
        BrowserCompiledAbility {
            spec: BuiltinPluginAbilitySpec {
                name: ABILITY_OPEN_SESSION,
                layer: PluginAbilityLayer::Control,
                call_mode: CallMode::Rpc,
                admission_action: AdmissionAction::Manage,
                bidi_wire_kind: None,
                subject_ura_kinds: OPEN_SUBJECT_KINDS,
                hints: BuiltinPluginAbilityHints::NONE,
                frontend_contract: BuiltinPluginFrontendContract::OPERATOR_BROWSER,
                description: schema::open_session_description,
                input_schema: schema::open_session_input_schema,
            },
            handler: BrowserAbilityHandler::Rpc(handlers::open_session),
        },
        BrowserCompiledAbility {
            spec: BuiltinPluginAbilitySpec {
                name: ABILITY_SHOW_SESSION,
                layer: PluginAbilityLayer::Observation,
                call_mode: CallMode::Rpc,
                admission_action: AdmissionAction::Read,
                bidi_wire_kind: None,
                subject_ura_kinds: SESSION_SUBJECT_KINDS,
                hints: BuiltinPluginAbilityHints::READ_ONLY_IDEMPOTENT,
                frontend_contract: BuiltinPluginFrontendContract::OPERATOR_BROWSER,
                description: schema::show_session_description,
                input_schema: schema::show_session_input_schema,
            },
            handler: BrowserAbilityHandler::Rpc(handlers::show_session),
        },
        BrowserCompiledAbility {
            spec: BuiltinPluginAbilitySpec {
                name: ABILITY_SEND_INPUT,
                layer: PluginAbilityLayer::Control,
                call_mode: CallMode::Rpc,
                admission_action: AdmissionAction::Manage,
                bidi_wire_kind: None,
                subject_ura_kinds: SESSION_SUBJECT_KINDS,
                hints: BuiltinPluginAbilityHints::NONE,
                frontend_contract: BuiltinPluginFrontendContract::OPERATOR_BROWSER,
                description: schema::send_input_description,
                input_schema: schema::send_input_input_schema,
            },
            handler: BrowserAbilityHandler::Rpc(handlers::send_input),
        },
        BrowserCompiledAbility {
            spec: BuiltinPluginAbilitySpec {
                name: ABILITY_CAPTURE_VIEWPORT,
                layer: PluginAbilityLayer::Observation,
                call_mode: CallMode::Stream,
                admission_action: AdmissionAction::Stream,
                bidi_wire_kind: None,
                subject_ura_kinds: SESSION_SUBJECT_KINDS,
                hints: BuiltinPluginAbilityHints::READ_ONLY,
                frontend_contract: BuiltinPluginFrontendContract::OPERATOR_BROWSER,
                description: schema::capture_viewport_description,
                input_schema: schema::capture_viewport_input_schema,
            },
            handler: BrowserAbilityHandler::Stream(handlers::capture_viewport),
        },
        BrowserCompiledAbility {
            spec: BuiltinPluginAbilitySpec {
                name: ABILITY_CAPTURE_PAGE,
                layer: PluginAbilityLayer::Observation,
                call_mode: CallMode::Rpc,
                admission_action: AdmissionAction::Read,
                bidi_wire_kind: None,
                subject_ura_kinds: SESSION_SUBJECT_KINDS,
                hints: BuiltinPluginAbilityHints::READ_ONLY,
                frontend_contract: BuiltinPluginFrontendContract::OPERATOR_BROWSER,
                description: schema::capture_page_description,
                input_schema: schema::capture_page_input_schema,
            },
            handler: BrowserAbilityHandler::Rpc(handlers::capture_page),
        },
        BrowserCompiledAbility {
            spec: BuiltinPluginAbilitySpec {
                name: ABILITY_ATTACH_SESSION,
                layer: PluginAbilityLayer::Operational,
                call_mode: CallMode::Bidi,
                admission_action: AdmissionAction::Stream,
                bidi_wire_kind: Some(PluginBidiWireKind::JsonFrames),
                subject_ura_kinds: SESSION_SUBJECT_KINDS,
                hints: BuiltinPluginAbilityHints::NONE,
                frontend_contract: BuiltinPluginFrontendContract::OPERATOR_BROWSER,
                description: schema::attach_session_description,
                input_schema: schema::attach_session_input_schema,
            },
            handler: BrowserAbilityHandler::Bidi(handlers::attach_session),
        },
        BrowserCompiledAbility {
            spec: BuiltinPluginAbilitySpec {
                name: ABILITY_CLOSE_SESSION,
                layer: PluginAbilityLayer::Control,
                call_mode: CallMode::Rpc,
                admission_action: AdmissionAction::Manage,
                bidi_wire_kind: None,
                subject_ura_kinds: SESSION_SUBJECT_KINDS,
                hints: BuiltinPluginAbilityHints::DESTRUCTIVE_IDEMPOTENT,
                frontend_contract: BuiltinPluginFrontendContract::OPERATOR_BROWSER,
                description: schema::close_session_description,
                input_schema: schema::close_session_input_schema,
            },
            handler: BrowserAbilityHandler::Rpc(handlers::close_session),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiled_rows_cover_each_public_ability_once() {
        let rows = compiled_abilities();
        let names = rows
            .iter()
            .map(|row| row.spec.name)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(names.len(), rows.len());
        assert_eq!(names.len(), super::super::constants::PUBLIC_ABILITIES.len());
    }

    #[test]
    fn cdp_transport_is_axon_bidi_not_a_plugin_socket() {
        let attach = compiled_abilities()
            .into_iter()
            .find(|row| row.spec.name == ABILITY_ATTACH_SESSION)
            .expect("attach row");
        assert_eq!(attach.spec.call_mode, CallMode::Bidi);
        assert_eq!(
            attach.spec.bidi_wire_kind,
            Some(PluginBidiWireKind::JsonFrames)
        );
        let manifest = attach
            .spec
            .to_registry_manifest()
            .expect("browser attach registry manifest");
        assert_eq!(
            manifest.bidi_wire_kind(),
            Some(crate::daemon::ability::manifest::AbilityBidiWireKind::JsonFrames)
        );
    }

    #[test]
    fn session_operations_are_resource_subject_scoped() {
        for row in compiled_abilities() {
            if row.spec.name != ABILITY_OPEN_SESSION {
                assert_eq!(row.spec.subject_ura_kinds, SESSION_SUBJECT_KINDS);
            }
        }
    }

    #[test]
    fn open_subject_is_an_axon_entity_while_user_remains_the_caller() {
        let open = compiled_abilities()
            .into_iter()
            .find(|row| row.spec.name == ABILITY_OPEN_SESSION)
            .expect("open row");
        assert_eq!(open.spec.subject_ura_kinds, &["agent"]);
    }

    #[test]
    fn every_browser_binding_publishes_one_dedicated_product_surface() {
        for row in compiled_abilities() {
            assert_eq!(
                row.spec.frontend_contract,
                BuiltinPluginFrontendContract::OPERATOR_BROWSER
            );
            let manifest = row
                .spec
                .to_registry_manifest()
                .expect("browser registry manifest");
            assert_eq!(
                manifest.exposure(),
                Some(crate::daemon::ability::manifest::AbilityExposure::Operator)
            );
            assert_eq!(
                manifest.dedicated_surface(),
                Some(crate::daemon::ability::manifest::AbilityDedicatedSurface::Browser)
            );
            assert_eq!(
                manifest.subject_contract_kind(),
                Some(
                    crate::daemon::ability::manifest::AbilitySubjectContractKind::DedicatedSurface
                )
            );
        }
    }
}
