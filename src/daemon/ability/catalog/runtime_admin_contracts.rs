// EasyNet CLI — Axon runtime-admin descriptor contracts
// =====================================================
//
// File: src/daemon/ability/catalog/runtime_admin_contracts.rs
// Description: Authority-owned descriptor contracts for Axon runtime-admin
//              handlers and descriptor-bound daemon runtime providers.

use crate::daemon::ability::conformance::{
    BaselineSurface, HubBaseline, ABILITY_RUNTIME_BOOTSTRAP_SELF_IDENTITY, ABILITY_SESSION_OPEN,
};
use crate::daemon::ability::descriptors::ReceiptSemantics;
use crate::daemon::ability::dispatch::{AxonAbilityCatalog, ControlPlaneImplementation, OwnerKind};

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
            &OwnerKind::RealmAuthority,
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
        reg.register_control_plane_descriptor_with_owner(
            ability.name,
            &OwnerKind::RealmAuthority,
            &manifest,
            ability.call_mode,
            ReceiptSemantics::Operational,
            &implementation,
        )?;
    }
    Ok(())
}
