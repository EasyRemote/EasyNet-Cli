// EasyNet-Cli
// ===========
//
// File: src/ura.rs
// Description: CLI façade for Axon-owned URA builders and parser.
//
// URA is protocol state owned by Axon. This file deliberately contains
// no grammar implementation and no string construction logic; it only
// re-exports `easynet_axon::ura` so existing CLI modules can keep using
// `crate::ura::*` while the source of truth remains in Axon SDK.
//
// Canonical shapes, all built by Axon:
//
//   user      easynet:///r/<realm>/user/<user-id>
//   device    easynet:///r/<realm>/device/<device-id>
//   agent     easynet:///r/<realm>/agent/<user-id>.<agent-id>
//   ability   easynet:///r/<realm>/ability/<owner>.<namespace>.<ability-id>
//   hub       easynet:///r/<realm>/hub
//   resource  easynet:///r/<realm>/resource/<owner-id>/<path>
//
// Examples:
//
//   easynet:///r/localhost/device/8315ea5c-7cfd-473e-8fef-95340af6d971
//   easynet:///r/localhost/agent/u-9f4.frontend-engineer
//   easynet:///r/localhost/hub
//   easynet:///r/localhost/ability/u-9f4.frontend-engineer.chat
//   easynet:///r/localhost/ability/hub.federation.resolve
//   easynet:///r/localhost/resource/agent.u-9f4.frontend-engineer/skill/alive-video
//
// CLI-specific rule:
//
//   When a CLI feature needs a URA, call one of the re-exported Axon
//   builders below. Do not add `format!("easynet:///r/...")`,
//   `strip_prefix("easynet:///r/")`, or a parallel parser in CLI code.
//   The guard at `tests/scripts/test_no_raw_ura_construction.sh` exists
//   to keep that invariant enforceable.

pub use easynet_axon::ura::*;

/// Parsed canonical Ability URA selector.
///
/// What this is: a small boundary object that projects an Ability URA
/// into the three daemon-local facts every caller needs: owner URA,
/// public ability name, and local registry dispatch key.
///
/// What this is not: it is not a URA parser or grammar copy. Parsing
/// still belongs to Axon (`easynet_axon::ura::parse_ura`); this type
/// only packages the CLI daemon projection that otherwise tends to be
/// hand-written at each call site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbilitySelector {
    ability_ura: String,
    owner_ura: String,
    dispatch_target: String,
    public_name: String,
    local_registry_ability: String,
}

impl AbilitySelector {
    /// Parse a canonical Ability URA into a daemon-local selector.
    pub fn parse(ability_ura: &str) -> anyhow::Result<Self> {
        let ability_ura = ability_ura.trim();
        if ability_ura.is_empty() {
            anyhow::bail!("invalid Ability URA: value must not be empty");
        }

        let parsed = parse_ura(ability_ura)
            .map_err(|e| anyhow::anyhow!("invalid Ability URA {ability_ura:?}: {e}"))?;
        if parsed.kind != URAKind::Ability {
            anyhow::bail!("invalid Ability URA {ability_ura:?}: expected /ability/ role");
        }
        let Some(ability) = parsed.ability() else {
            anyhow::bail!("invalid Ability URA {ability_ura:?}: missing typed ability owner");
        };

        let (owner_ura, dispatch_target) = match ability.owner {
            AbilityOwner::Agent { user_id, agent_id } => (
                agent_ura(&parsed.realm, &user_id, &agent_id),
                agent_id.clone(),
            ),
            AbilityOwner::Device { device_id } => {
                let owner_ura = device_ura(&parsed.realm, &device_id);
                (owner_ura.clone(), owner_ura)
            }
            AbilityOwner::Hub => {
                let owner_ura = hub_ura(&parsed.realm);
                (owner_ura.clone(), owner_ura)
            }
        };
        let public_name = ability_name_from_parts(&parsed).ok_or_else(|| {
            anyhow::anyhow!("invalid Ability URA {ability_ura:?}: missing public ability name")
        })?;
        let local_registry_ability = local_dispatch_ability_key(&owner_ura, &public_name);

        Ok(Self {
            ability_ura: ability_ura.to_string(),
            owner_ura,
            dispatch_target,
            public_name,
            local_registry_ability,
        })
    }

    /// Original canonical Ability URA.
    pub fn ability_ura(&self) -> &str {
        &self.ability_ura
    }

    /// Canonical owner URA encoded by the Ability URA.
    pub fn owner_ura(&self) -> &str {
        &self.owner_ura
    }

    /// Dispatch target used by local/federation routing.
    pub fn dispatch_target(&self) -> &str {
        &self.dispatch_target
    }

    /// Owner-local public ability name.
    pub fn public_name(&self) -> &str {
        &self.public_name
    }

    /// Daemon `AxonAbilityCatalog` registry key.
    pub fn local_registry_ability(&self) -> &str {
        &self.local_registry_ability
    }
}

/// Project an internal registry ability name into the public name a
/// given owner publishes under RFC-005.
///
/// Agent, device, and hub owners publish owner-local public ability names.
/// The local daemon registry may store implementation-qualified keys such as
/// `claude.chat` or `fs.read`; those prefixes identify the local
/// dispatch table, not the public Ability URA tail.
pub fn owner_local_ability_name(owner_ura: &str, ability_name: &str) -> String {
    let name = ability_name.trim();
    if name.is_empty() {
        return String::new();
    }

    let Ok(owner) = parse_ura(owner_ura) else {
        return name.to_string();
    };

    match owner.kind {
        URAKind::Agent => {
            let Some((_, agent_id)) = owner.agent_ids() else {
                return name.to_string();
            };
            name.strip_prefix(&format!("{agent_id}."))
                .unwrap_or(name)
                .to_string()
        }
        URAKind::Device => name.strip_prefix("device.").unwrap_or(name).to_string(),
        URAKind::Hub => name.strip_prefix("hub.").unwrap_or(name).to_string(),
        _ => name.to_string(),
    }
}

/// Convert an owner-local ability name back to the daemon registry key
/// used for local dispatch.
///
/// This is deliberately not a URA builder. It is the inverse projection
/// for the local `AxonAbilityCatalog`, whose historical keys remain
/// owner-scoped so a generic `chat` route can dispatch to `<agent>.chat`
/// and a device-owned `fs.read` route can dispatch to `fs.read`.
pub fn local_dispatch_ability_key(target_ura: &str, ability: &str) -> String {
    let name = ability.trim();
    if name.is_empty() {
        return String::new();
    }

    let Ok(target) = parse_ura(target_ura) else {
        return name.to_string();
    };

    match target.kind {
        URAKind::Agent => {
            let Some((_, agent_id)) = target.agent_ids() else {
                return name.to_string();
            };
            let public_name = owner_local_ability_name(target_ura, name);
            let prefix = format!("{agent_id}.");
            if public_name.starts_with(&prefix) {
                public_name
            } else {
                format!("{agent_id}.{public_name}")
            }
        }
        URAKind::Device | URAKind::Hub => owner_local_ability_name(target_ura, name),
        _ => name.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_uses_axon_sdk_ura_builder() {
        assert_eq!(
            ability_ura("localhost", "hub", "federation", "resolve"),
            "easynet:///r/localhost/ability/hub.federation.resolve"
        );
        assert_eq!(
            resource_dot_ura(
                "localhost",
                "agent.dev.frontend-engineer",
                "skill/alive-video"
            ),
            "easynet:///r/localhost/resource/agent.dev.frontend-engineer/skill/alive-video"
        );
    }

    #[test]
    fn owner_local_ability_name_projects_registry_key_to_public_name() {
        assert_eq!(
            owner_local_ability_name("easynet:///r/localhost/device/dev-1", "fs.read"),
            "fs.read"
        );
        assert_eq!(
            owner_local_ability_name("easynet:///r/localhost/device/dev-1", "device.fs.read"),
            "fs.read"
        );
        assert_eq!(
            owner_local_ability_name("easynet:///r/localhost/hub", "hub.openai.chat"),
            "openai.chat"
        );
        assert_eq!(
            owner_local_ability_name("easynet:///r/localhost/agent/alice.claude", "claude.chat",),
            "chat"
        );
        assert_eq!(
            owner_local_ability_name("easynet:///r/localhost/agent/alice.claude", "chat",),
            "chat"
        );
    }

    #[test]
    fn local_dispatch_ability_key_rebuilds_local_dispatch_key() {
        assert_eq!(
            local_dispatch_ability_key("easynet:///r/localhost/device/dev-1", "fs.read"),
            "fs.read"
        );
        assert_eq!(
            local_dispatch_ability_key("easynet:///r/localhost/agent/alice.claude", "chat"),
            "claude.chat"
        );
    }

    #[test]
    fn ability_selector_projects_agent_owned_ability_ura() {
        let selector = AbilitySelector::parse("easynet:///r/acme/ability/user-1.claude.weather")
            .expect("agent ability selector");
        assert_eq!(
            selector.ability_ura(),
            "easynet:///r/acme/ability/user-1.claude.weather"
        );
        assert_eq!(
            selector.owner_ura(),
            "easynet:///r/acme/agent/user-1.claude"
        );
        assert_eq!(selector.dispatch_target(), "claude");
        assert_eq!(selector.public_name(), "weather");
        assert_eq!(selector.local_registry_ability(), "claude.weather");
    }

    #[test]
    fn ability_selector_projects_device_owned_ability_ura() {
        let selector = AbilitySelector::parse("easynet:///r/acme/ability/device.dev-1.fs.read")
            .expect("device ability selector");
        assert_eq!(selector.owner_ura(), "easynet:///r/acme/device/dev-1");
        assert_eq!(selector.dispatch_target(), "easynet:///r/acme/device/dev-1");
        assert_eq!(selector.public_name(), "fs.read");
        assert_eq!(selector.local_registry_ability(), "fs.read");
    }

    #[test]
    fn ability_selector_rejects_non_ability_ura() {
        let err = AbilitySelector::parse("easynet:///r/acme/device/dev-1")
            .expect_err("non-ability URA must fail");
        assert!(err.to_string().contains("/ability/"));
    }
}
