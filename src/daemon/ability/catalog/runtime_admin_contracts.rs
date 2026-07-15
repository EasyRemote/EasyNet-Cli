// EasyNet CLI — Axon runtime-admin descriptor contracts
// =====================================================
//
// File: src/daemon/ability/catalog/runtime_admin_contracts.rs
// Description: Control-plane descriptors for Axon SDK runtime-admin
//              handlers that execute through LocalRuntime but are not
//              daemon Invocation exact routes.

use crate::daemon::ability::authority::AuthorityScope;
use crate::daemon::ability::conformance::{
    BaselineSurface, HubBaseline, ABILITY_RUNTIME_BOOTSTRAP_SELF_IDENTITY, ABILITY_SESSION_OPEN,
};
use crate::daemon::ability::descriptors::ReceiptSemantics;
use crate::daemon::ability::dispatch::{AxonAbilityCatalog, ControlPlaneImplementation, OwnerKind};

pub(crate) const SESSION_OPEN_TEMPLATE_DEVICE_URA: &str =
    "easynet:///r/_system/device/session-open-template";

pub(crate) fn register(reg: &mut AxonAbilityCatalog) -> anyhow::Result<()> {
    let implementation = ControlPlaneImplementation::native_daemon();
    for ability in HubBaseline::required_abilities()
        .iter()
        .copied()
        .filter(|ability| ability.surface == BaselineSurface::AxonRuntimeAdmin)
        .filter(|ability| ability.name == ABILITY_RUNTIME_BOOTSTRAP_SELF_IDENTITY)
    {
        let manifest = super::system_manifest::registration_manifest(ability.name)?;
        reg.register_control_plane_descriptor_with_owner(
            ability.name,
            &OwnerKind::Hub,
            &manifest,
            ability.call_mode,
            ReceiptSemantics::Operational,
            &implementation,
        )?;
    }
    for ability in HubBaseline::required_abilities()
        .iter()
        .copied()
        .filter(|ability| ability.surface == BaselineSurface::AxonRuntimeAdmin)
        .filter(|ability| ability.name == ABILITY_SESSION_OPEN)
    {
        let manifest = super::system_manifest::registration_manifest(ability.name)?;
        let authority_scope = AuthorityScope::new("device", SESSION_OPEN_TEMPLATE_DEVICE_URA)?;
        reg.register_control_plane_descriptor_with_scope(
            ability.name,
            authority_scope,
            &manifest,
            ability.call_mode,
            ReceiptSemantics::Operational,
            &implementation,
        )?;
    }
    Ok(())
}
