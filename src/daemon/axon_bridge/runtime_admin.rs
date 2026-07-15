//! Descriptor-bound installer for Axon SDK runtime-admin handlers.
//!
//! Axon supplies the handler implementation for
//! `runtime.bootstrap_self_identity`, but its convenience installer registers
//! the handler under the bare ability name with an unbound proof. EasyNet
//! daemon dispatch is descriptor-bound, so boot must re-key that handler under
//! the canonical Hub-owned Ability URA and attach the system descriptor proof.

use std::sync::Arc;

use easynet_axon::invocation::{AbilityCallModes, AbilityOptions, LocalRuntime};

use crate::daemon::ability::conformance::ABILITY_RUNTIME_BOOTSTRAP_SELF_IDENTITY;
use crate::daemon::ability::dispatch::AxonAbilityCatalog;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuntimeAdminInstall {
    pub runtime_key: String,
}

pub(crate) async fn install_bootstrap_self_identity_admin(
    runtime: &Arc<LocalRuntime>,
    catalog: &AxonAbilityCatalog,
    hub_ura: &str,
) -> anyhow::Result<RuntimeAdminInstall> {
    runtime
        .install_bootstrap_self_identity_admin()
        .await
        .map_err(|error| anyhow::anyhow!("install Axon bootstrap admin handler: {error}"))?;

    let runtime_key = crate::daemon::axon_bridge::descriptor_ref::ability_ura_for_wire(
        hub_ura,
        ABILITY_RUNTIME_BOOTSTRAP_SELF_IDENTITY,
    )
    .map_err(|error| {
        anyhow::anyhow!(
            "derive runtime-admin Ability URA for `{hub_ura}` \
             `{ABILITY_RUNTIME_BOOTSTRAP_SELF_IDENTITY}`: {error}"
        )
    })?;
    let options = runtime_admin_options(catalog, hub_ura)?;

    let _ = runtime.unregister_ability(&runtime_key).await;
    runtime
        .rename_ability(ABILITY_RUNTIME_BOOTSTRAP_SELF_IDENTITY, &runtime_key)
        .await
        .map_err(|error| {
            anyhow::anyhow!(
                "re-key runtime admin `{ABILITY_RUNTIME_BOOTSTRAP_SELF_IDENTITY}` \
                 to descriptor-bound `{runtime_key}`: {error}"
            )
        })?;
    runtime
        .update_ability_options(&runtime_key, options)
        .await
        .map_err(|error| {
            anyhow::anyhow!("bind runtime admin descriptor proof for `{runtime_key}`: {error}")
        })?
        .ok_or_else(|| {
            anyhow::anyhow!(
                "runtime admin `{runtime_key}` disappeared before descriptor proof binding"
            )
        })?;

    Ok(RuntimeAdminInstall { runtime_key })
}

fn runtime_admin_options(
    catalog: &AxonAbilityCatalog,
    hub_ura: &str,
) -> anyhow::Result<AbilityOptions> {
    let record = catalog
        .control_plane_record_for_mode(
            ABILITY_RUNTIME_BOOTSTRAP_SELF_IDENTITY,
            crate::daemon::ability::CallMode::Rpc,
        )
        .map_err(|error| {
            anyhow::anyhow!(
                "runtime-admin descriptor lookup for \
                 `{ABILITY_RUNTIME_BOOTSTRAP_SELF_IDENTITY}` is ambiguous: {error}"
            )
        })?
        .ok_or_else(|| {
            anyhow::anyhow!(
                "runtime-admin descriptor missing for \
                 `{ABILITY_RUNTIME_BOOTSTRAP_SELF_IDENTITY}`"
            )
        })?;
    let descriptor = record
        .descriptor()
        .clone()
        .rebind_owner_ura(hub_ura)
        .map_err(|error| {
            anyhow::anyhow!(
                "runtime-admin descriptor cannot bind to Hub owner `{hub_ura}`: {error}"
            )
        })?;
    Ok(AbilityOptions::default()
        .with_modes(AbilityCallModes::RPC)
        .with_descriptor_proof(
            descriptor.version.as_str(),
            descriptor.admission_action().as_str(),
            descriptor.descriptor_hash_bytes(),
            descriptor.schema_hash_bytes(),
            record.implementation().impl_hash(),
        ))
}
