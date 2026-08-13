// EasyNet CLI — daemon-native ability ownership projection
// ==========================================================
//
// File: src/daemon/ability/catalog/ownership.rs
// Description: Projects deterministic system-registry ownership into the
//              device-sponsored SystemAgent identity used by routing.
//
// Protocol Responsibility
// -----------------------
// This module does not define URA syntax or routing policy. It exposes the
// daemon catalogue's one ownership fact for a public ability and verifies that
// a SystemAgent owner belongs to the declared daemon-native inventory.
//
// Implementation Approach
// -----------------------
// Ability ownership comes only from `system_ability_owner`, whose source is the
// deterministic registry/control-plane record. SystemAgent declaration comes
// only from the profile registry. No ability-name prefix or routing-local table
// participates in the decision.
//
// Usage Contract
// --------------
// Target routing may combine the returned SystemAgent id with a selected host
// Device URA. A `None` result means the ability is unknown, non-SystemAgent
// owned, or references an undeclared SystemAgent and must fail closed.
//
// Architectural Position
// ----------------------
// Daemon ability catalogue domain, below invocation routing and above the
// control-plane registry projection.

use crate::daemon::ability::dispatch::OwnerKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeviceSponsoredSystemAgentOwner {
    system_agent_id: String,
}

impl DeviceSponsoredSystemAgentOwner {
    pub(crate) fn system_agent_id(&self) -> &str {
        &self.system_agent_id
    }
}

pub(crate) fn device_sponsored_system_agent_owner_for_public_ability(
    public_ability: &str,
) -> Option<DeviceSponsoredSystemAgentOwner> {
    let public_ability = public_ability.trim();
    if public_ability.is_empty() {
        return None;
    }
    let OwnerKind::SystemAgent(system_agent_id) =
        super::catalog_metadata::unique_system_agent_owner_for_public_ability(public_ability)?
    else {
        return None;
    };
    if !super::profiles::is_declared_daemon_native_system_agent_id(&system_agent_id) {
        return None;
    }
    Some(DeviceSponsoredSystemAgentOwner { system_agent_id })
}

pub(crate) fn execution_target_owner_ura_for_public_ability(
    execution_target_ura: &str,
    public_ability: &str,
) -> anyhow::Result<String> {
    let target = crate::core::ura::parse_ura(execution_target_ura)
        .map_err(|error| anyhow::anyhow!("execution target URA is invalid: {error}"))?;
    match target.kind {
        crate::core::ura::URAKind::Authority => Ok(execution_target_ura.to_string()),
        crate::core::ura::URAKind::Device => {
            let device_id = target.device_id().ok_or_else(|| {
                anyhow::anyhow!(
                    "Device execution target is missing device id: {execution_target_ura}"
                )
            })?;
            let owner = device_sponsored_system_agent_owner_for_public_ability(public_ability)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Device execution target ability `{public_ability}` has no registry-owned device-sponsored SystemAgent"
                    )
                })?;
            Ok(crate::core::ura::device_agent_ura(
                &target.realm,
                device_id,
                owner.system_agent_id(),
            ))
        }
        other => anyhow::bail!(
            "execution target ability owner projection requires Device or Authority, got {other}"
        ),
    }
}

/// Recover the sponsoring Device execution host from a canonical declared
/// SystemAgent owner. This is the inverse placement projection used when a
/// descriptor query names a remote SystemAgent but catalogue access must be
/// sent to that SystemAgent's host runtime.
pub(crate) fn execution_host_ura_for_device_sponsored_system_agent(
    owner_ura: &str,
) -> anyhow::Result<Option<String>> {
    let owner = crate::core::ura::parse_ura(owner_ura)
        .map_err(|error| anyhow::anyhow!("ability owner URA is invalid: {error}"))?;
    let Some((device_id, system_agent_id)) = owner.device_agent_ids() else {
        return Ok(None);
    };
    if !super::profiles::is_declared_daemon_native_system_agent_id(system_agent_id) {
        anyhow::bail!(
            "device-sponsored Agent `{owner_ura}` is not a declared daemon-native SystemAgent"
        );
    }
    Ok(Some(crate::core::ura::device_ura(&owner.realm, device_id)))
}

/// Recover the execution host for any owner kind whose public descriptor is
/// sponsored by a Device runtime.
///
/// SystemAgent owners carry the sponsor Device id in their own URA. Service
/// owners are principal-scoped and deliberately do not contain placement, so an
/// attached descriptor read must combine the Service owner with the current
/// runtime owner. A Service owner in the same realm as an attached Device is
/// read from that Device's committed catalog.
pub(crate) fn execution_host_ura_for_device_sponsored_owner(
    owner_ura: &str,
    runtime_owner_ura: &str,
) -> anyhow::Result<Option<String>> {
    if let Some(host) = execution_host_ura_for_device_sponsored_system_agent(owner_ura)? {
        return Ok(Some(host));
    }
    let owner = crate::core::ura::parse_ura(owner_ura)
        .map_err(|error| anyhow::anyhow!("ability owner URA is invalid: {error}"))?;
    if owner.kind != crate::core::ura::URAKind::Service {
        return Ok(None);
    }
    let runtime_owner = crate::core::ura::parse_ura(runtime_owner_ura)
        .map_err(|error| anyhow::anyhow!("runtime owner URA is invalid: {error}"))?;
    if runtime_owner.kind != crate::core::ura::URAKind::Device {
        return Ok(None);
    }
    if owner.realm != runtime_owner.realm {
        return Ok(None);
    }
    Ok(Some(runtime_owner_ura.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_owner_projects_representative_system_agent_families() {
        for (ability, expected_system_agent) in [
            (
                crate::daemon::ability::names::device_control::TERMINAL_LIST,
                crate::daemon::ability::names::device_control::TERMINAL_SYSTEM_AGENT_ID,
            ),
            (
                crate::daemon::ability::names::agents::AGENT_LIST,
                crate::daemon::ability::names::agents::AGENT_MANAGEMENT_SYSTEM_AGENT_ID,
            ),
            (
                crate::daemon::ability::names::automation::MISSION_RUN,
                crate::daemon::ability::names::automation::AUTOMATION_SYSTEM_AGENT_ID,
            ),
            (
                crate::daemon::ability::names::governance::META_LIST_ABILITIES,
                crate::daemon::ability::names::governance::RUNTIME_INTROSPECTION_SYSTEM_AGENT_ID,
            ),
            (
                crate::daemon::ability::names::federation::ABILITY_DEPLOY,
                crate::daemon::ability::names::federation::ABILITY_MANAGEMENT_SYSTEM_AGENT_ID,
            ),
        ] {
            let owner = device_sponsored_system_agent_owner_for_public_ability(ability)
                .unwrap_or_else(|| panic!("{ability} must have a registry-owned SystemAgent"));
            assert_eq!(owner.system_agent_id(), expected_system_agent, "{ability}");
        }
    }

    #[test]
    fn projection_rejects_unknown_and_non_system_agent_abilities() {
        assert!(
            device_sponsored_system_agent_owner_for_public_ability("unknown.ability").is_none()
        );
        assert!(
            device_sponsored_system_agent_owner_for_public_ability(
                crate::daemon::ability::names::federation::JOIN,
            )
            .is_none(),
            "realm-Authority ability must not project to a device-sponsored SystemAgent"
        );
    }

    #[test]
    fn device_execution_target_projects_runtime_introspection_system_agent_owner() {
        assert_eq!(
            execution_target_owner_ura_for_public_ability(
                "easynet:///r/acme/device/dev-a",
                crate::daemon::ability::names::governance::META_LIST_ABILITIES,
            )
            .expect("runtime introspection owner"),
            "easynet:///r/acme/agent/device.dev-a.runtime-introspection"
        );
        assert_eq!(
            execution_target_owner_ura_for_public_ability(
                "easynet:///r/acme/authority",
                crate::daemon::ability::names::governance::META_LIST_ABILITIES,
            )
            .expect("Authority owner"),
            "easynet:///r/acme/authority"
        );
    }

    #[test]
    fn declared_system_agent_owner_projects_back_to_sponsor_execution_host() {
        assert_eq!(
            execution_host_ura_for_device_sponsored_system_agent(
                "easynet:///r/acme/agent/device.dev-a.runtime-introspection",
            )
            .expect("valid SystemAgent owner")
            .as_deref(),
            Some("easynet:///r/acme/device/dev-a")
        );
        assert_eq!(
            execution_host_ura_for_device_sponsored_system_agent(
                "easynet:///r/acme/agent/alice.assistant",
            )
            .expect("hosted Agent is canonical"),
            None
        );
    }

    #[test]
    fn service_owner_projects_to_attached_device_execution_host() {
        assert_eq!(
            execution_host_ura_for_device_sponsored_owner(
                "easynet:///r/acme/service/user-1.pages",
                "easynet:///r/acme/device/dev-a",
            )
            .expect("same-realm Service projects to attached Device")
            .as_deref(),
            Some("easynet:///r/acme/device/dev-a")
        );
        assert_eq!(
            execution_host_ura_for_device_sponsored_owner(
                "easynet:///r/other/service/user-1.pages",
                "easynet:///r/acme/device/dev-a",
            )
            .expect("foreign Service is not attached to local Device"),
            None
        );
        assert_eq!(
            execution_host_ura_for_device_sponsored_owner(
                "easynet:///r/acme/service/user-1.pages",
                "easynet:///r/acme/authority",
            )
            .expect("Authority runtime is not a Device sponsor"),
            None
        );
    }
}
