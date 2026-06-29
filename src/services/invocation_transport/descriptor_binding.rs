// EasyNet Daemon — invocation descriptor binding
// ==============================================
//
// File: src/services/invocation_transport/descriptor_binding.rs
// Description: Resolves the daemon-selected runtime ability into the
//              descriptor-bound ability reference that Axon admission must
//              verify for a specific call mode.
//
// This module exists so unary, stream, bidi, and carrier-v1 session ingress
// cannot drift on descriptor-version binding. Product routing selects the
// owner/callee/ability. Axon runtime registration selects the descriptor proof
// version. This boundary joins the two and returns the only string that may be
// passed into `external_signed_from_wire_parts`.

use std::collections::HashMap;

use easynet_axon::invocation::{AbilityOptions, CallMode, LocalRuntime};
use tonic::Status;

use crate::services::invocation_transport::invocation_wire::SIGNED_DESCRIPTOR_REF_METADATA_KEY;
use crate::services::invocation_transport::route_resolver::SelectedInvokeRoute;

/// Runtime registration plus descriptor proof context for one ability.
///
/// Invariants:
/// 1. `runtime_ability_ura` is a canonical Ability URA owned by `callee_ura`.
/// 2. `options` came from the live Axon `LocalRuntime` row for
///    `runtime_ability_ura`.
/// 3. Callers must derive wire refs through [`Self::descriptor_ref_for_mode`]
///    so the call-mode-specific descriptor proof version is preserved.
#[derive(Debug, Clone)]
pub(crate) struct RuntimeBoundAbility {
    runtime_ability_ura: String,
    options: AbilityOptions,
}

impl RuntimeBoundAbility {
    pub(crate) async fn from_selected_route(
        surface: &'static str,
        runtime: &LocalRuntime,
        route: &SelectedInvokeRoute,
    ) -> Result<Self, Status> {
        let runtime_ability_ura =
            runtime_ability_ura(surface, &route.callee_ura, &route.ability_ura).map_err(|err| {
                Status::failed_precondition(format!(
                    "{surface}: selected route `{}` cannot derive a runtime ability from `{}` \
                     for callee `{}`: {err}",
                    route.route_ura, route.ability_ura, route.callee_ura
                ))
            })?;
        let options = runtime
            .ability_options(&runtime_ability_ura)
            .await
            .ok_or_else(|| {
                Status::not_found(format!(
                    "{surface}: selected route `{}` dispatches `{}` but that ability is not \
                     registered in Axon LocalRuntime as `{}`",
                    route.route_ura, route.ability_ura, runtime_ability_ura
                ))
            })?;
        Ok(Self {
            runtime_ability_ura,
            options,
        })
    }

    pub(crate) async fn from_wire_target(
        surface: &'static str,
        runtime: &LocalRuntime,
        callee_ura: &str,
        ability: &str,
    ) -> Result<Self, Status> {
        let runtime_ability_ura =
            runtime_ability_ura(surface, callee_ura, ability).map_err(|err| {
                Status::invalid_argument(format!(
                    "{surface}: ability `{ability}` is not valid for callee `{callee_ura}`: {err}"
                ))
            })?;
        let options = runtime
            .ability_options(&runtime_ability_ura)
            .await
            .ok_or_else(|| {
                Status::not_found(format!(
                    "{surface}: ability `{ability}` is not registered in Axon LocalRuntime as \
                     `{runtime_ability_ura}`"
                ))
            })?;
        Ok(Self {
            runtime_ability_ura,
            options,
        })
    }

    pub(crate) fn supports_mode(&self, mode: CallMode) -> bool {
        match mode {
            CallMode::Rpc => self.options.modes.rpc,
            CallMode::Stream => self.options.modes.stream,
            CallMode::Bidi => self.options.modes.bidi,
        }
    }

    pub(crate) fn descriptor_ref_for_mode(
        &self,
        surface: &'static str,
        callee_ura: &str,
        mode: CallMode,
        route_ura: Option<&str>,
    ) -> Result<DescriptorBoundAbilityRef, Status> {
        if !self.supports_mode(mode) {
            return Err(Status::invalid_argument(format!(
                "{surface}: ability `{}` is registered, but does not support {} Invoke",
                self.runtime_ability_ura,
                call_mode_label(mode)
            )));
        }
        let proof_binding = self.options.proof_for_mode(mode);
        let descriptor_version = proof_binding.descriptor_version.trim();
        if descriptor_version.is_empty() {
            return Err(Status::failed_precondition(format!(
                "{surface}: {} does not bind a descriptor version for {}",
                route_context(route_ura, &self.runtime_ability_ura),
                call_mode_label(mode)
            )));
        }
        let descriptor_ref =
            crate::runtime::axon_bridge::descriptor_ref::ability_descriptor_ref_for_wire(
                callee_ura,
                &self.runtime_ability_ura,
                descriptor_version,
            )
            .map_err(|err| {
                Status::failed_precondition(format!(
                    "{surface}: {} cannot form a descriptor-bound ability ref: {err}",
                    route_context(route_ura, &self.runtime_ability_ura)
                ))
            })?;
        Ok(DescriptorBoundAbilityRef { descriptor_ref })
    }

    pub(crate) fn signed_descriptor_ref_from_metadata(
        &self,
        surface: &'static str,
        callee_ura: &str,
        mode: CallMode,
        metadata: &HashMap<String, String>,
    ) -> Result<Option<DescriptorBoundAbilityRef>, Status> {
        if !self.supports_mode(mode) {
            return Err(Status::invalid_argument(format!(
                "{surface}: ability `{}` is registered, but does not support {} Invoke",
                self.runtime_ability_ura,
                call_mode_label(mode)
            )));
        }
        let Some(raw) = metadata
            .get(SIGNED_DESCRIPTOR_REF_METADATA_KEY)
            .map(String::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return Ok(None);
        };
        let descriptor_ref =
            crate::runtime::axon_bridge::descriptor_ref::require_descriptor_ref_for_wire(
                callee_ura, raw,
            )
            .map_err(|err| {
                Status::invalid_argument(format!(
                    "{surface}: signed descriptor ref `{raw}` is invalid for callee \
                     `{callee_ura}`: {err}"
                ))
            })?;
        let signed_ability_ura = descriptor_ref
            .rsplit_once('@')
            .map(|(ability_ura, _)| ability_ura)
            .ok_or_else(|| {
                Status::invalid_argument(format!(
                    "{surface}: signed descriptor ref `{descriptor_ref}` is missing a \
                     descriptor version"
                ))
            })?;
        if signed_ability_ura != self.runtime_ability_ura {
            return Err(Status::invalid_argument(format!(
                "{surface}: signed descriptor ref `{descriptor_ref}` targets \
                 `{signed_ability_ura}` but route selected `{}`",
                self.runtime_ability_ura
            )));
        }
        Ok(Some(DescriptorBoundAbilityRef { descriptor_ref }))
    }
}

/// Descriptor-bound wire target for a runtime ability.
///
/// `descriptor_ref` is the only value that may enter Axon's signed wire
/// reassembly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DescriptorBoundAbilityRef {
    descriptor_ref: String,
}

impl DescriptorBoundAbilityRef {
    pub(crate) fn into_descriptor_ref(self) -> String {
        self.descriptor_ref
    }
}

fn runtime_ability_ura(
    _surface: &'static str,
    callee_ura: &str,
    ability: &str,
) -> Result<String, easynet_axon::invocation::AxonError> {
    crate::runtime::axon_bridge::descriptor_ref::ability_ura_for_wire(callee_ura, ability)
}

fn route_context(route_ura: Option<&str>, runtime_ability_ura: &str) -> String {
    match route_ura {
        Some(route_ura) => {
            format!("selected route `{route_ura}` dispatching `{runtime_ability_ura}`")
        }
        None => format!("runtime ability `{runtime_ability_ura}`"),
    }
}

fn call_mode_label(mode: CallMode) -> &'static str {
    match mode {
        CallMode::Rpc => "RPC",
        CallMode::Stream => "Stream",
        CallMode::Bidi => "Bidi",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CALLEE: &str = "easynet:///r/acme/device/dev-a";

    fn bound(ability: &str) -> RuntimeBoundAbility {
        RuntimeBoundAbility {
            runtime_ability_ura: crate::ura::owner_ability_ura(CALLEE, ability)
                .expect("test ability URA"),
            options: AbilityOptions::default(),
        }
    }

    #[test]
    fn signed_descriptor_ref_metadata_preserves_caller_signed_version() {
        let ref_ = format!(
            "{}@2.3.0",
            crate::ura::owner_ability_ura(CALLEE, "terminal.list").unwrap()
        );
        let mut metadata = HashMap::new();
        metadata.insert(SIGNED_DESCRIPTOR_REF_METADATA_KEY.to_string(), ref_.clone());

        let got = bound("terminal.list")
            .signed_descriptor_ref_from_metadata(
                "test carrier-v1",
                CALLEE,
                CallMode::Rpc,
                &metadata,
            )
            .expect("metadata descriptor ref is valid")
            .expect("metadata descriptor ref is present");

        assert_eq!(got.into_descriptor_ref(), ref_);
    }

    #[test]
    fn signed_descriptor_ref_metadata_must_match_selected_route() {
        let ref_ = format!(
            "{}@1.0.0",
            crate::ura::owner_ability_ura(CALLEE, "skill.list").unwrap()
        );
        let mut metadata = HashMap::new();
        metadata.insert(SIGNED_DESCRIPTOR_REF_METADATA_KEY.to_string(), ref_);

        let err = bound("terminal.list")
            .signed_descriptor_ref_from_metadata(
                "test carrier-v1",
                CALLEE,
                CallMode::Rpc,
                &metadata,
            )
            .expect_err("metadata descriptor ref for another ability must reject");

        assert!(
            err.message().contains("route selected"),
            "unexpected error: {}",
            err.message()
        );
    }
}
