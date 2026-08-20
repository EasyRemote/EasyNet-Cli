//! Transport-independent route-target semantics for ability invocation.

use anyhow::{anyhow, bail};

use crate::core::identity::RuntimeIdentityUra;
use crate::core::ura::{AbilitySelector, URAKind};

const URA_DISCOVERY_HINT: &str = "Where to find a canonical URA today (until PR-N3 cross-realm \
discovery lands): use `easynet node list` for Device URAs; `easynet ability list --json` for \
descriptor-owned Agent/SystemAgent/Service/Authority URAs; or inspect the target daemon's catalogue.";

/// Canonical Device placement locator used by APIs whose product contract is
/// explicitly device-hosted. It cannot name an Authority or callable actor.
pub(crate) fn parse_device_placement_ura(locator: &str) -> anyhow::Result<String> {
    let trimmed = locator.trim();
    let identity = RuntimeIdentityUra::parse(trimmed).map_err(|err| {
        anyhow!(
            "Device placement locator `{trimmed}` is not a canonical Axon Device URA: {err}. \
             A bare hostname or `https://...` URL is not accepted. \
             {URA_DISCOVERY_HINT}"
        )
    })?;

    match identity.kind() {
        URAKind::Device => Ok(identity.into_string()),
        other => bail!(
            "Device placement locator `{trimmed}` is a canonical Axon URA, but not a Device. \
             Got kind={other}. \
             {URA_DISCOVERY_HINT}"
        ),
    }
}

/// Typed routing target for public descriptor-bound ability invocation.
///
/// A Device is only a placement locator; the selected Ability keeps its
/// Agent/SystemAgent/Service owner. Agent (including a device-sponsored
/// SystemAgent), Service, and Authority inputs are exact callees and therefore
/// must equal the owner
/// encoded by the selected Ability URA. No Device-owned ability is inferred.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RemoteAbilityRouteTarget {
    DevicePlacement(String),
    ExactCallee(String),
}

impl RemoteAbilityRouteTarget {
    pub(crate) fn parse(raw: &str, selector: &AbilitySelector) -> anyhow::Result<Self> {
        let trimmed = raw.trim();
        let identity = RuntimeIdentityUra::parse(trimmed).map_err(|err| {
            anyhow!(
                 "--node `{trimmed}` is not a canonical Device placement or exact callable \
                 Agent/SystemAgent/Service/Authority URA: {err}. A bare hostname or URL is not accepted. \
                 {URA_DISCOVERY_HINT}"
            )
        })?;
        match identity.kind() {
            URAKind::Device => Ok(Self::DevicePlacement(identity.into_string())),
            URAKind::Agent | URAKind::Service | URAKind::Authority => {
                if identity.as_str() != selector.owner_ura() {
                    bail!(
                        "exact callable target `{}` does not own selected ability `{}`; \
                         expected exact callee `{}`",
                        identity.as_str(),
                        selector.ability_ura(),
                        selector.owner_ura()
                    );
                }
                Ok(Self::ExactCallee(identity.into_string()))
            }
            other => bail!(
                "--node `{trimmed}` is a canonical Axon URA, but cannot route an ability \
                 invocation; expected a Device placement or exact Agent/SystemAgent/Service/Authority \
                 callee, got kind={other}"
            ),
        }
    }

    pub(crate) fn as_str(&self) -> &str {
        match self {
            Self::DevicePlacement(value) | Self::ExactCallee(value) => value,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_placement_parser_accepts_only_canonical_device_identity() {
        assert!(parse_device_placement_ura("easynet:///r/realm/device/node-a").is_ok());
        assert!(parse_device_placement_ura("easynet:///r/realm/authority").is_err());
        assert!(parse_device_placement_ura("https://device.example").is_err());
    }

    #[test]
    fn device_placement_parser_rejects_authority_with_tail() {
        assert!(parse_device_placement_ura("easynet:///r/realm/authority/extra").is_err());
    }

    #[test]
    fn ability_route_target_accepts_device_placement_and_exact_actor_callees() {
        let system_agent_owner = crate::core::ura::device_agent_ura(
            "realm",
            "node-a",
            crate::daemon::ability::names::governance::RUNTIME_HEALTH_SYSTEM_AGENT_ID,
        );
        let system_ability = crate::core::ura::owner_ability_ura(
            &system_agent_owner,
            crate::daemon::ability::names::governance::OBSERVE_HEALTH,
        )
        .expect("SystemAgent ability URA");
        let system_selector =
            AbilitySelector::parse(&system_ability).expect("SystemAgent ability selector");

        assert_eq!(
            RemoteAbilityRouteTarget::parse("easynet:///r/realm/device/node-a", &system_selector,)
                .expect("sponsoring Device placement"),
            RemoteAbilityRouteTarget::DevicePlacement(
                "easynet:///r/realm/device/node-a".to_string()
            )
        );
        assert_eq!(
            RemoteAbilityRouteTarget::parse(&system_agent_owner, &system_selector)
                .expect("exact SystemAgent callee"),
            RemoteAbilityRouteTarget::ExactCallee(system_agent_owner.clone())
        );

        let agent_owner = crate::core::ura::agent_ura("realm", "user-a", "worker");
        let agent_ability =
            crate::core::ura::owner_ability_ura(&agent_owner, "chat").expect("Agent ability URA");
        let agent_selector =
            AbilitySelector::parse(&agent_ability).expect("Agent ability selector");
        assert_eq!(
            RemoteAbilityRouteTarget::parse(&agent_owner, &agent_selector)
                .expect("exact Agent callee"),
            RemoteAbilityRouteTarget::ExactCallee(agent_owner)
        );

        let service_owner = crate::core::ura::service_ura("realm", "user-a", "pages");
        let service_ability = crate::core::ura::owner_ability_ura(&service_owner, "project_list")
            .expect("Service ability URA");
        let service_selector =
            AbilitySelector::parse(&service_ability).expect("Service ability selector");
        assert_eq!(
            RemoteAbilityRouteTarget::parse(&service_owner, &service_selector)
                .expect("exact Service callee"),
            RemoteAbilityRouteTarget::ExactCallee(service_owner)
        );

        let authority_owner = crate::core::ura::authority_ura("realm");
        let authority_ability =
            crate::core::ura::owner_ability_ura(&authority_owner, "federation.status")
                .expect("Authority ability URA");
        let authority_selector =
            AbilitySelector::parse(&authority_ability).expect("Authority ability selector");
        assert_eq!(
            RemoteAbilityRouteTarget::parse(&authority_owner, &authority_selector)
                .expect("exact Authority callee"),
            RemoteAbilityRouteTarget::ExactCallee(authority_owner)
        );
    }

    #[test]
    fn ability_route_target_rejects_exact_callee_owner_mismatch() {
        let owner = crate::core::ura::agent_ura("realm", "user-a", "worker");
        let ability =
            crate::core::ura::owner_ability_ura(&owner, "chat").expect("Agent ability URA");
        let selector = AbilitySelector::parse(&ability).expect("Agent ability selector");
        let other_agent = crate::core::ura::agent_ura("realm", "user-a", "reviewer");

        let error = RemoteAbilityRouteTarget::parse(&other_agent, &selector)
            .expect_err("exact callee must own selected ability");
        assert!(error.to_string().contains("does not own selected ability"));
        assert!(
            RemoteAbilityRouteTarget::parse("easynet:///r/realm/authority", &selector).is_err()
        );
    }
}
