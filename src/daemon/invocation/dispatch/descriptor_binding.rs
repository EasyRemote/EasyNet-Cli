// EasyNet Daemon — invocation descriptor binding
// ==============================================
//
// File: src/daemon/invocation/descriptor_binding.rs
// Description: Resolves the daemon-selected runtime ability into the
//              descriptor-bound ability reference that Axon admission must
//              verify for a specific call mode.
//
// This module exists so unary, stream, bidi, and carrier-v1 session ingress
// cannot drift on descriptor-version binding. Product routing selects the
// owner/callee/ability. Axon runtime registration selects the descriptor proof
// version. This boundary joins the two and returns the only string that may be
// passed into `external_signed_from_wire_parts`.

use axon_sdk::invocation::{AbilityOptions, CallMode, LocalRuntime};
use axon_sdk::pb::axon::v1::InvocationTarget;
use tonic::Status;

use crate::daemon::ability::dispatch::AxonAbilityCatalog;
use crate::daemon::invocation::dispatch::invocation_wire::{
    descriptor_ref_from_invocation_target, status_from_dispatch_key_mismatch,
};
use crate::daemon::invocation::routing::route_resolver::SelectedInvokeRoute;

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
    selected_route_descriptor_ref: Option<SelectedRouteDescriptorRef>,
}

#[derive(Debug, Clone)]
struct SelectedRouteDescriptorRef {
    mode: CallMode,
    descriptor_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum WireAbilityTarget {
    DescriptorRef {
        wire_target: String,
        ability_ura: String,
    },
    OwnerLocal {
        wire_target: String,
        ability_ura: String,
    },
}

impl WireAbilityTarget {
    fn parse(surface: &'static str, callee_ura: &str, wire_target: &str) -> Result<Self, Status> {
        let wire_target = wire_target.trim();
        if wire_target.is_empty() {
            return Err(Status::invalid_argument(format!(
                "{surface}: signed ability target is empty"
            )));
        }
        if is_descriptor_bound_wire_target(wire_target) {
            let ability_ura =
                crate::daemon::axon_bridge::descriptor_ref::ability_ura_from_descriptor_ref(
                    wire_target,
                )
                .map_err(|err| {
                    Status::invalid_argument(format!(
                        "{surface}: descriptor-bound signed ability `{wire_target}` is invalid: {err}"
                    ))
                })?;
            return Ok(Self::DescriptorRef {
                wire_target: wire_target.to_string(),
                ability_ura,
            });
        }
        let ability_ura = crate::daemon::axon_bridge::descriptor_ref::ability_ura_for_wire(
            callee_ura,
            wire_target,
        )
        .map_err(|err| {
            Status::invalid_argument(format!(
                "{surface}: owner-local signed ability `{wire_target}` is not valid for callee \
                     `{callee_ura}`: {err}"
            ))
        })?;
        Ok(Self::OwnerLocal {
            wire_target: wire_target.to_string(),
            ability_ura,
        })
    }

    fn ability_ura(&self) -> &str {
        match self {
            Self::DescriptorRef { ability_ura, .. } | Self::OwnerLocal { ability_ura, .. } => {
                ability_ura
            }
        }
    }

    fn wire_target(&self) -> &str {
        match self {
            Self::DescriptorRef { wire_target, .. } | Self::OwnerLocal { wire_target, .. } => {
                wire_target
            }
        }
    }
}

fn is_descriptor_bound_wire_target(wire_target: &str) -> bool {
    wire_target.starts_with("easynet:///")
        && wire_target.contains('@')
        && wire_target.contains('#')
        && wire_target.contains('!')
}

impl RuntimeBoundAbility {
    pub(crate) async fn from_selected_route(
        surface: &'static str,
        runtime: &LocalRuntime,
        catalog: Option<&AxonAbilityCatalog>,
        route: &SelectedInvokeRoute,
        mode: CallMode,
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
        if !ability_options_supports_mode(&options, mode) {
            return Ok(Self {
                runtime_ability_ura,
                options,
                selected_route_descriptor_ref: None,
            });
        }
        let descriptor_ref = selected_route_descriptor_ref_from_catalog(
            surface,
            catalog,
            route,
            mode,
            &runtime_ability_ura,
            &options,
        )?;
        Ok(Self {
            runtime_ability_ura,
            options,
            selected_route_descriptor_ref: Some(SelectedRouteDescriptorRef {
                mode,
                descriptor_ref,
            }),
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
            selected_route_descriptor_ref: None,
        })
    }

    pub(crate) fn supports_mode(&self, mode: CallMode) -> bool {
        ability_options_supports_mode(&self.options, mode)
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
        if let Some(selected) = self.selected_route_descriptor_ref.as_ref() {
            if selected.mode != mode {
                return Err(Status::invalid_argument(format!(
                    "{surface}: selected route bound {} descriptor proof, but dispatch requested {}",
                    call_mode_label(selected.mode),
                    call_mode_label(mode)
                )));
            }
            return Ok(DescriptorBoundAbilityRef {
                descriptor_ref: selected.descriptor_ref.clone(),
            });
        }
        let proof_binding = self.options.proof_for_mode(mode).ok_or_else(|| {
            Status::failed_precondition(format!(
                "{surface}: {} has no descriptor proof for {}",
                route_context(route_ura, &self.runtime_ability_ura),
                call_mode_label(mode)
            ))
        })?;
        let descriptor_binding =
            crate::daemon::axon_bridge::descriptor_ref::descriptor_binding_for_wire(
                &proof_binding.descriptor_version,
                proof_binding.descriptor_hash,
                &proof_binding.admission_action,
            )
            .map_err(|err| {
                Status::failed_precondition(format!(
                    "{surface}: {} does not bind a complete descriptor proof for {}: {err}",
                    route_context(route_ura, &self.runtime_ability_ura),
                    call_mode_label(mode)
                ))
            })?;
        let descriptor_ref =
            crate::daemon::axon_bridge::descriptor_ref::ability_descriptor_ref_for_wire(
                callee_ura,
                &self.runtime_ability_ura,
                &descriptor_binding,
            )
            .map_err(|err| {
                Status::failed_precondition(format!(
                    "{surface}: {} cannot form a descriptor-bound ability ref: {err}",
                    route_context(route_ura, &self.runtime_ability_ura)
                ))
            })?;
        Ok(DescriptorBoundAbilityRef { descriptor_ref })
    }

    /// Normalize a request-supplied callable target into an explicit wire-target
    /// state and prove that it names the same governed ability selected by the
    /// daemon resolver.
    ///
    /// Descriptor-bound targets and owner-local selectors are distinct states
    /// inside this boundary. Descriptor-like malformed input fails closed rather
    /// than being reinterpreted as an owner-local selector.
    pub(crate) fn require_wire_target_matches(
        &self,
        surface: &'static str,
        callee_ura: &str,
        wire_target: &str,
        route_ura: &str,
    ) -> Result<String, Status> {
        let wire_target = WireAbilityTarget::parse(surface, callee_ura, wire_target)?;
        if wire_target.ability_ura() != self.runtime_ability_ura {
            return Err(status_from_dispatch_key_mismatch(
                surface,
                wire_target.wire_target(),
                &self.runtime_ability_ura,
                route_ura,
            ));
        }
        Ok(wire_target.ability_ura().to_string())
    }

    pub(crate) fn signed_descriptor_ref_from_target(
        &self,
        surface: &'static str,
        callee_ura: &str,
        mode: CallMode,
        target: Option<&InvocationTarget>,
    ) -> Result<DescriptorBoundAbilityRef, Status> {
        if !self.supports_mode(mode) {
            return Err(Status::invalid_argument(format!(
                "{surface}: ability `{}` is registered, but does not support {} Invoke",
                self.runtime_ability_ura,
                call_mode_label(mode)
            )));
        }
        let descriptor_ref = descriptor_ref_from_invocation_target(surface, callee_ura, target)?;
        let signed_ability_ura =
            crate::daemon::axon_bridge::descriptor_ref::ability_ura_from_descriptor_ref(
                &descriptor_ref,
            )
            .map_err(|err| {
                Status::invalid_argument(format!(
                    "{surface}: signed descriptor ref `{descriptor_ref}` is invalid: {err}"
                ))
            })?;
        if signed_ability_ura != self.runtime_ability_ura {
            return Err(Status::invalid_argument(format!(
                "{surface}: signed descriptor ref `{descriptor_ref}` targets \
                 `{signed_ability_ura}` but route selected `{}`",
                self.runtime_ability_ura
            )));
        }
        Ok(DescriptorBoundAbilityRef { descriptor_ref })
    }
}

pub(crate) fn signed_call_mode_from_target(
    surface: &'static str,
    callee_ura: &str,
    target: Option<&InvocationTarget>,
) -> Result<CallMode, Status> {
    let descriptor_ref = descriptor_ref_from_invocation_target(surface, callee_ura, target)?;
    let action = axon_sdk::invocation::admission_action_from_descriptor_ref(&descriptor_ref)
        .map_err(|err| {
            Status::invalid_argument(format!(
                "{surface}: signed descriptor ref `{descriptor_ref}` has invalid admission action: {err}"
            ))
        })?;
    if action == crate::daemon::ability::descriptors::AdmissionAction::Stream.as_str() {
        Ok(CallMode::Stream)
    } else {
        Ok(CallMode::Rpc)
    }
}

fn ability_options_supports_mode(options: &AbilityOptions, mode: CallMode) -> bool {
    match mode {
        CallMode::Rpc => options.modes.rpc,
        CallMode::Stream => options.modes.stream,
        CallMode::Bidi => options.modes.bidi,
    }
}

fn selected_route_descriptor_ref_from_catalog(
    surface: &'static str,
    catalog: Option<&AxonAbilityCatalog>,
    route: &SelectedInvokeRoute,
    mode: CallMode,
    runtime_ability_ura: &str,
    options: &AbilityOptions,
) -> Result<String, Status> {
    let catalog = catalog.ok_or_else(|| {
        Status::failed_precondition(format!(
            "{surface}: selected route `{}` cannot bind descriptor proof because the live ability control plane is not available",
            route.route_ura
        ))
    })?;
    let descriptor_mode = descriptor_call_mode(mode);
    let record = catalog
        .control_plane_record_for_authority_mode(
            &route.callee_ura,
            &route.dispatch_name,
            descriptor_mode,
        )
        .map_err(|error| {
            Status::failed_precondition(format!(
                "{surface}: selected route `{}` has ambiguous live control-plane descriptor proof for `{}` under `{}` in {}: {error}",
                route.route_ura,
                route.dispatch_name,
                route.callee_ura,
                descriptor_mode.as_str()
            ))
        })?
        .ok_or_else(|| {
            Status::failed_precondition(format!(
                "{surface}: selected route `{}` has no live control-plane descriptor proof for `{}` under `{}` in {}",
                route.route_ura,
                route.dispatch_name,
                route.callee_ura,
                descriptor_mode.as_str()
            ))
        })?;
    let proof = options.proof_for_mode(mode).ok_or_else(|| {
        Status::failed_precondition(format!(
            "{surface}: selected route `{}` runtime registration for `{runtime_ability_ura}` has no {} descriptor proof",
            route.route_ura,
            call_mode_label(mode)
        ))
    })?;
    let descriptor = record.descriptor();
    let implementation = record.implementation();
    let expected_descriptor_hash = descriptor.descriptor_hash_bytes();
    let expected_schema_hash = descriptor.schema_hash_bytes();
    let expected_impl_hash = implementation.impl_hash();
    if proof.descriptor_version != descriptor.version.as_str()
        || proof.descriptor_hash != expected_descriptor_hash
        || proof.schema_hash != expected_schema_hash
        || proof.impl_hash != expected_impl_hash
        || proof.admission_action != descriptor.admission_action().as_str()
    {
        return Err(Status::failed_precondition(format!(
            "{surface}: selected route `{}` runtime proof for `{runtime_ability_ura}` does not match live control-plane descriptor proof for `{}` under `{}` in {}",
            route.route_ura,
            route.dispatch_name,
            route.callee_ura,
            descriptor_mode.as_str()
        )));
    }
    let descriptor_binding =
        crate::daemon::axon_bridge::descriptor_ref::descriptor_binding_for_wire(
            &descriptor.version,
            expected_descriptor_hash,
            descriptor.admission_action().as_str(),
        )
        .map_err(|error| {
            Status::failed_precondition(format!(
                "{surface}: selected route `{}` cannot form live control-plane descriptor binding: {error}",
                route.route_ura
            ))
        })?;
    crate::daemon::axon_bridge::descriptor_ref::ability_descriptor_ref_for_wire(
        &route.callee_ura,
        runtime_ability_ura,
        &descriptor_binding,
    )
    .map_err(|error| {
        Status::failed_precondition(format!(
            "{surface}: selected route `{}` cannot form live control-plane descriptor ref: {error}",
            route.route_ura
        ))
    })
}

fn descriptor_call_mode(mode: CallMode) -> crate::daemon::ability::CallMode {
    match mode {
        CallMode::Rpc => crate::daemon::ability::CallMode::Rpc,
        CallMode::Stream => crate::daemon::ability::CallMode::Stream,
        CallMode::Bidi => crate::daemon::ability::CallMode::Bidi,
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
) -> Result<String, axon_sdk::invocation::AxonError> {
    crate::daemon::axon_bridge::descriptor_ref::ability_ura_for_wire(callee_ura, ability)
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
    use axon_sdk::invocation::{
        make_ability, AbilityCallModes, AbilityOptions, CallMode as AxonCallMode, LocalRuntime,
    };

    const CALLEE: &str = "easynet:///r/acme/device/dev-a";

    fn bound(ability: &str) -> RuntimeBoundAbility {
        RuntimeBoundAbility {
            runtime_ability_ura: crate::core::ura::owner_ability_ura(CALLEE, ability)
                .expect("test ability URA"),
            options: AbilityOptions::default(),
            selected_route_descriptor_ref: None,
        }
    }

    fn bound_all_modes(ability: &str) -> RuntimeBoundAbility {
        RuntimeBoundAbility {
            runtime_ability_ura: crate::core::ura::owner_ability_ura(CALLEE, ability)
                .expect("test ability URA"),
            options: AbilityOptions::default().with_modes(AbilityCallModes {
                rpc: true,
                stream: true,
                bidi: true,
            }),
            selected_route_descriptor_ref: None,
        }
    }

    fn local_callee() -> String {
        CALLEE.to_string()
    }

    fn route_manifest(ability: &str) -> crate::daemon::ability::manifest::AbilityManifest {
        crate::daemon::ability::manifest::AbilityManifest::new(
            ability.rsplit('.').next().unwrap_or(ability),
            "Selected route descriptor proof fixture",
            serde_json::json!({"type": "object"}),
        )
        .and_then(|manifest| manifest.with_admission_action("invoke"))
        .expect("test manifest is valid")
    }

    fn register_catalog_descriptor(
        catalog: &AxonAbilityCatalog,
        ability: &str,
        mode: crate::daemon::ability::CallMode,
    ) {
        catalog
            .register_control_plane_descriptor_with_owner(
                ability,
                &crate::daemon::ability::dispatch::OwnerKind::Device,
                &route_manifest(ability),
                mode,
                crate::daemon::ability::descriptors::ReceiptSemantics::Operational,
                &crate::daemon::ability::dispatch::ControlPlaneImplementation::native_daemon(),
            )
            .expect("catalog descriptor registers");
    }

    fn runtime_options_from_catalog(
        catalog: &AxonAbilityCatalog,
        callee_ura: &str,
        ability: &str,
        mode: crate::daemon::ability::CallMode,
    ) -> AbilityOptions {
        let record = catalog
            .control_plane_record_for_authority_mode(callee_ura, ability, mode)
            .expect("catalog proof lookup is unambiguous")
            .expect("catalog proof row exists");
        let descriptor = record.descriptor();
        let implementation = record.implementation();
        match mode {
            crate::daemon::ability::CallMode::Rpc => AbilityOptions::default()
                .with_modes(AbilityCallModes::RPC)
                .with_descriptor_proof(
                    descriptor.version.as_str(),
                    descriptor.admission_action().as_str(),
                    descriptor.descriptor_hash_bytes(),
                    descriptor.schema_hash_bytes(),
                    implementation.impl_hash(),
                ),
            crate::daemon::ability::CallMode::Stream => AbilityOptions::streaming()
                .with_mode_descriptor_proof(
                    AxonCallMode::Stream,
                    descriptor.version.as_str(),
                    descriptor.admission_action().as_str(),
                    descriptor.descriptor_hash_bytes(),
                    descriptor.schema_hash_bytes(),
                    implementation.impl_hash(),
                ),
            crate::daemon::ability::CallMode::Bidi => AbilityOptions::bidi()
                .with_mode_descriptor_proof(
                    AxonCallMode::Bidi,
                    descriptor.version.as_str(),
                    descriptor.admission_action().as_str(),
                    descriptor.descriptor_hash_bytes(),
                    descriptor.schema_hash_bytes(),
                    implementation.impl_hash(),
                ),
        }
    }

    fn mismatched_runtime_options(mode: crate::daemon::ability::CallMode) -> AbilityOptions {
        match mode {
            crate::daemon::ability::CallMode::Rpc => AbilityOptions::default()
                .with_modes(AbilityCallModes::RPC)
                .with_descriptor_proof("9.9.9", "invoke", [0x44; 32], [0x55; 32], [0x66; 32]),
            crate::daemon::ability::CallMode::Stream => AbilityOptions::streaming()
                .with_mode_descriptor_proof(
                    AxonCallMode::Stream,
                    "9.9.9",
                    "invoke",
                    [0x44; 32],
                    [0x55; 32],
                    [0x66; 32],
                ),
            crate::daemon::ability::CallMode::Bidi => AbilityOptions::bidi()
                .with_mode_descriptor_proof(
                    AxonCallMode::Bidi,
                    "9.9.9",
                    "invoke",
                    [0x44; 32],
                    [0x55; 32],
                    [0x66; 32],
                ),
        }
    }

    async fn register_runtime_ability(
        runtime: &LocalRuntime,
        callee_ura: &str,
        ability: &str,
        options: AbilityOptions,
    ) {
        runtime
            .register_ability_with_options(
                crate::core::ura::owner_ability_ura(callee_ura, ability)
                    .expect("runtime ability URA"),
                make_ability(|ctx| async move { Ok(ctx.payload.clone()) }),
                options,
            )
            .await
            .expect("runtime ability registers");
    }

    fn route_for(callee_ura: &str, ability: &str) -> SelectedInvokeRoute {
        SelectedInvokeRoute::test_local_runtime(callee_ura, ability, ability)
    }

    fn axon_mode(mode: crate::daemon::ability::CallMode) -> AxonCallMode {
        match mode {
            crate::daemon::ability::CallMode::Rpc => AxonCallMode::Rpc,
            crate::daemon::ability::CallMode::Stream => AxonCallMode::Stream,
            crate::daemon::ability::CallMode::Bidi => AxonCallMode::Bidi,
        }
    }

    #[tokio::test]
    async fn selected_route_descriptor_ref_comes_from_live_catalog_for_all_modes() {
        let callee_ura = local_callee();
        for (mode, ability) in [
            (crate::daemon::ability::CallMode::Rpc, "test.selected_rpc"),
            (
                crate::daemon::ability::CallMode::Stream,
                "test.selected_stream",
            ),
            (crate::daemon::ability::CallMode::Bidi, "test.selected_bidi"),
        ] {
            let catalog = AxonAbilityCatalog::new_test_metadata_for_device_authority(&callee_ura);
            let runtime = crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
                crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
                None,
            );
            register_catalog_descriptor(&catalog, ability, mode);
            register_runtime_ability(
                &runtime,
                &callee_ura,
                ability,
                runtime_options_from_catalog(&catalog, &callee_ura, ability, mode),
            )
            .await;

            let bound = RuntimeBoundAbility::from_selected_route(
                "test selected route",
                &runtime,
                Some(&catalog),
                &route_for(&callee_ura, ability),
                axon_mode(mode),
            )
            .await
            .expect("selected route binds catalog descriptor proof");

            let descriptor_ref = bound
                .descriptor_ref_for_mode(
                    "test selected route",
                    &callee_ura,
                    axon_mode(mode),
                    Some("route-ref"),
                )
                .expect("descriptor ref resolves")
                .into_descriptor_ref();
            let record = catalog
                .control_plane_record_for_authority_mode(&callee_ura, ability, mode)
                .expect("catalog lookup is unambiguous")
                .expect("catalog row exists");
            assert!(descriptor_ref.contains(record.descriptor().version.as_str()));
            assert!(
                descriptor_ref.contains(&hex::encode(record.descriptor().descriptor_hash_bytes()))
            );
        }
    }

    #[tokio::test]
    async fn selected_route_rejects_missing_catalog_descriptor_proof() {
        let callee_ura = local_callee();
        let ability = "test.missing_catalog_proof";
        let runtime = crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
            crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
            None,
        );
        register_runtime_ability(
            &runtime,
            &callee_ura,
            ability,
            AbilityOptions::default()
                .with_modes(AbilityCallModes::RPC)
                .with_descriptor_proof("1.0.0", "invoke", [0x44; 32], [0x55; 32], [0x66; 32]),
        )
        .await;

        let err = RuntimeBoundAbility::from_selected_route(
            "test selected route",
            &runtime,
            Some(&AxonAbilityCatalog::new_test_metadata_for_device_authority(
                &callee_ura,
            )),
            &route_for(&callee_ura, ability),
            AxonCallMode::Rpc,
        )
        .await
        .expect_err("selected route without catalog proof must fail closed");

        assert!(
            err.message()
                .contains("no live control-plane descriptor proof"),
            "unexpected error: {}",
            err.message()
        );
    }

    #[tokio::test]
    async fn selected_route_rejects_runtime_proof_that_drifted_from_catalog() {
        let callee_ura = local_callee();
        let ability = "test.drifted_runtime_proof";
        let catalog = AxonAbilityCatalog::new_test_metadata_for_device_authority(&callee_ura);
        let runtime = crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
            crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
            None,
        );
        register_catalog_descriptor(&catalog, ability, crate::daemon::ability::CallMode::Rpc);
        register_runtime_ability(
            &runtime,
            &callee_ura,
            ability,
            mismatched_runtime_options(crate::daemon::ability::CallMode::Rpc),
        )
        .await;

        let err = RuntimeBoundAbility::from_selected_route(
            "test selected route",
            &runtime,
            Some(&catalog),
            &route_for(&callee_ura, ability),
            AxonCallMode::Rpc,
        )
        .await
        .expect_err("drifted runtime proof must be rejected");

        assert!(
            err.message()
                .contains("does not match live control-plane descriptor proof"),
            "unexpected error: {}",
            err.message()
        );
    }

    #[test]
    fn wire_target_match_accepts_owner_local_selector_explicitly() {
        let got = bound("terminal.list")
            .require_wire_target_matches("test route match", CALLEE, "terminal.list", "route-ref")
            .expect("owner-local target matches selected route");

        assert_eq!(
            got,
            crate::core::ura::owner_ability_ura(CALLEE, "terminal.list").unwrap()
        );
    }

    #[test]
    fn wire_target_match_accepts_descriptor_bound_selector_explicitly() {
        let descriptor_ref = format!(
            "{}@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
            crate::core::ura::owner_ability_ura(CALLEE, "terminal.list").unwrap()
        );

        let got = bound("terminal.list")
            .require_wire_target_matches("test route match", CALLEE, &descriptor_ref, "route-ref")
            .expect("descriptor-bound target matches selected route");

        assert_eq!(
            got,
            crate::core::ura::owner_ability_ura(CALLEE, "terminal.list").unwrap()
        );
    }

    #[test]
    fn wire_target_match_rejects_malformed_descriptor_like_target_without_owner_local_reinterpretation(
    ) {
        let malformed = format!(
            "{}@1.0.0#not-a-hash!invoke",
            crate::core::ura::owner_ability_ura(CALLEE, "terminal.list").unwrap()
        );

        let err = bound("terminal.list")
            .require_wire_target_matches("test route match", CALLEE, &malformed, "route-ref")
            .expect_err("malformed descriptor-like target must not fall back to owner-local");

        assert!(
            err.message().contains("descriptor-bound signed ability"),
            "unexpected error: {}",
            err.message()
        );
        assert!(
            !err.message().contains("owner-local signed ability"),
            "malformed descriptor-like target must not be reparsed as owner-local: {}",
            err.message()
        );
    }

    #[test]
    fn signed_descriptor_ref_target_preserves_caller_signed_version() {
        let ref_ = format!(
            "{}@2.3.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!read",
            crate::core::ura::owner_ability_ura(CALLEE, "terminal.list").unwrap()
        );
        let target = crate::daemon::invocation::dispatch::invocation_wire::wire_invocation_target(
            &ref_,
            "terminal.list",
        )
        .unwrap();

        let got = bound("terminal.list")
            .signed_descriptor_ref_from_target(
                "test carrier-v1",
                CALLEE,
                CallMode::Rpc,
                Some(&target),
            )
            .expect("typed target descriptor ref is valid");

        assert_eq!(got.into_descriptor_ref(), ref_);
    }

    #[test]
    fn signed_descriptor_ref_target_must_match_selected_route() {
        let ref_ = format!(
            "{}@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!read",
            crate::core::ura::owner_ability_ura(CALLEE, "skill.list").unwrap()
        );
        let target = crate::daemon::invocation::dispatch::invocation_wire::wire_invocation_target(
            ref_,
            "terminal.list",
        )
        .unwrap();

        let err = bound("terminal.list")
            .signed_descriptor_ref_from_target(
                "test carrier-v1",
                CALLEE,
                CallMode::Rpc,
                Some(&target),
            )
            .expect_err("target descriptor ref for another ability must reject");

        assert!(
            err.message().contains("route selected"),
            "unexpected error: {}",
            err.message()
        );
    }

    #[test]
    fn signed_descriptor_ref_rejects_route_only_typed_target() {
        let target = crate::daemon::invocation::dispatch::invocation_wire::wire_invocation_target(
            "terminal.list",
            "terminal.list",
        )
        .unwrap();

        let err = bound("terminal.list")
            .signed_descriptor_ref_from_target(
                "test signed public Invoke",
                CALLEE,
                CallMode::Rpc,
                Some(&target),
            )
            .expect_err("route-only target must not authorize signed public admission");

        assert!(
            err.message().contains("complete descriptor ref"),
            "unexpected error: {}",
            err.message()
        );
    }

    #[test]
    fn signed_descriptor_ref_ignores_legacy_target_fields() {
        let target = InvocationTarget::default();

        let err = bound("terminal.list")
            .signed_descriptor_ref_from_target(
                "test signed public Invoke",
                CALLEE,
                CallMode::Rpc,
                Some(&target),
            )
            .expect_err("legacy target fields must not carry descriptor proof");

        assert!(
            err.message().contains("typed Ability target"),
            "unexpected error: {}",
            err.message()
        );
    }

    #[test]
    fn unary_stream_and_bidi_share_the_same_typed_descriptor_rule() {
        let descriptor_ref = format!(
            "{}@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
            crate::core::ura::owner_ability_ura(CALLEE, "terminal.list").unwrap()
        );
        let descriptor_target =
            crate::daemon::invocation::dispatch::invocation_wire::wire_invocation_target(
                &descriptor_ref,
                "terminal.list",
            )
            .unwrap();
        let route_only_target =
            crate::daemon::invocation::dispatch::invocation_wire::wire_invocation_target(
                "terminal.list",
                "terminal.list",
            )
            .unwrap();
        let bound = bound_all_modes("terminal.list");

        for mode in [CallMode::Rpc, CallMode::Stream, CallMode::Bidi] {
            assert_eq!(
                bound
                    .signed_descriptor_ref_from_target(
                        "test signed public geometry",
                        CALLEE,
                        mode,
                        Some(&descriptor_target),
                    )
                    .unwrap()
                    .into_descriptor_ref(),
                descriptor_ref
            );
            let error = bound
                .signed_descriptor_ref_from_target(
                    "test signed public geometry",
                    CALLEE,
                    mode,
                    Some(&route_only_target),
                )
                .expect_err("route-only target must fail for every signed geometry");
            assert!(
                error.message().contains("complete descriptor ref"),
                "{mode:?}: unexpected error: {}",
                error.message()
            );
        }
    }
}
