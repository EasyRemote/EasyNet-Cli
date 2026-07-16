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

use std::collections::HashMap;

use easynet_axon::invocation::{AbilityOptions, CallMode, LocalRuntime};
use tonic::Status;

use crate::daemon::ability::dispatch::AxonAbilityCatalog;
use crate::daemon::invocation::dispatch::invocation_wire::{
    status_from_dispatch_key_mismatch, SIGNED_DESCRIPTOR_REF_METADATA_KEY,
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
        let proof_binding = self.options.proof_for_mode(mode);
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

    /// Normalize a request-supplied callable target and prove that it
    /// names the same governed ability selected by the daemon resolver.
    ///
    /// `wire_target` is intentionally accepted in both historic forms:
    /// an owner-local public ability name (`fs.read`, `chat`) or a
    /// descriptor-bound ability ref (`easynet:///.../ability/...@1.0.0`).
    /// The dispatcher may route on either form, but it must execute only
    /// the runtime ability selected by the daemon resolver.
    pub(crate) fn require_wire_target_matches(
        &self,
        surface: &'static str,
        callee_ura: &str,
        wire_target: &str,
        route_ura: &str,
    ) -> Result<String, Status> {
        let signed_ability_ura = crate::daemon::axon_bridge::descriptor_ref::ability_ura_for_wire(
            callee_ura,
            wire_target,
        )
        .map_err(|err| {
            Status::invalid_argument(format!(
                "{surface}: signed ability `{wire_target}` is not valid for callee \
                     `{callee_ura}`: {err}"
            ))
        })?;
        if signed_ability_ura != self.runtime_ability_ura {
            return Err(status_from_dispatch_key_mismatch(
                surface,
                wire_target,
                &self.runtime_ability_ura,
                route_ura,
            ));
        }
        Ok(signed_ability_ura)
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
            crate::daemon::axon_bridge::descriptor_ref::require_descriptor_ref_for_wire(
                callee_ura, raw,
            )
            .map_err(|err| {
                Status::invalid_argument(format!(
                    "{surface}: signed descriptor ref `{raw}` is invalid for callee \
                     `{callee_ura}`: {err}"
                ))
            })?;
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
        Ok(Some(DescriptorBoundAbilityRef { descriptor_ref }))
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
    let proof = options.proof_for_mode(mode);
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
) -> Result<String, easynet_axon::invocation::AxonError> {
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
    use easynet_axon::invocation::{
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

    fn local_callee() -> String {
        crate::daemon::identity::local_invocation::local_device_ura()
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
            let catalog = AxonAbilityCatalog::new();
            let runtime = LocalRuntime::new();
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
        let runtime = LocalRuntime::new();
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
            Some(&AxonAbilityCatalog::new()),
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
        let catalog = AxonAbilityCatalog::new();
        let runtime = LocalRuntime::new();
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
    fn signed_descriptor_ref_metadata_preserves_caller_signed_version() {
        let ref_ = format!(
            "{}@2.3.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!read",
            crate::core::ura::owner_ability_ura(CALLEE, "terminal.list").unwrap()
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
            "{}@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!read",
            crate::core::ura::owner_ability_ura(CALLEE, "skill.list").unwrap()
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
