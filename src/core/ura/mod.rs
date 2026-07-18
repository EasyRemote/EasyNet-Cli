// EasyNet-Cli
// ===========
//
// File: src/core/ura/mod.rs
// Description: CLI façade for Axon-owned URA builders and parser.
//
// URA is protocol state owned by Axon. This file deliberately contains
// no grammar implementation; it re-exports `axon_sdk::ura` and
// centralizes the few CLI-local projections that sit immediately on top
// of Axon's canonical builders. Existing CLI modules can keep using
// `crate::core::ura::*` while the source of truth remains in Axon SDK.
//
// Canonical shapes, all built by Axon:
//
//   user      easynet:///r/<realm>/user/<user-id>
//   device    easynet:///r/<realm>/device/<device-id>
//   agent     easynet:///r/<realm>/agent/<user-id>.<agent-id>
//   ability   easynet:///r/<realm>/ability/<owner>.<namespace>.<ability-id>
//   hub       easynet:///r/<realm>/authority
//   resource  easynet:///r/<realm>/resource/<owner-id>/<path>
//
// Examples:
//
//   easynet:///r/localhost/device/8315ea5c-7cfd-473e-8fef-95340af6d971
//   easynet:///r/localhost/agent/u-9f4.frontend-engineer
//   easynet:///r/localhost/authority
//   easynet:///r/localhost/ability/u-9f4.frontend-engineer.chat
//   easynet:///r/localhost/ability/authority.federation.resolve
//   easynet:///r/localhost/resource/agent.u-9f4.frontend-engineer/skill/alive-video
//
// CLI-specific rule:
//
//   When a CLI feature needs a URA, call one of the re-exported Axon
//   builders below. Do not add `format!("easynet:///r/...")`,
//   `strip_prefix("easynet:///r/")`, or a parallel parser in CLI code.
//   The guard at `tests/scripts/test_no_raw_ura_construction.sh` exists
//   to keep that invariant enforceable.

pub use axon_sdk::ura::*;

pub mod provisional;

/// EasyNet product default realm.
///
/// This policy default is intentionally owned by the CLI facade rather than
/// Axon's product-neutral URA grammar.
pub const REALM_EASYNET: &str = "easynet.run";

/// Product-facing Hub identity projected onto Axon's generic authority URA.
///
/// Hub policy and lifecycle remain CLI-owned; Axon sees only the canonical
/// system-authority owner kind.
pub fn hub_ura(realm: &str) -> String {
    authority_ura(realm)
}

/// Product-facing Hub ability projected onto Axon's generic authority owner.
pub fn hub_ability_ura(realm: &str, ability_name: &str) -> String {
    authority_ability_ura(realm, ability_name)
}

/// Synthetic system Agent URA for daemon-internal LocalRuntime calls.
///
/// This is a CLI-owned identity layered on top of Axon's URA grammar: it is
/// not a user, device, or hub identity, but it still uses the canonical Agent
/// URA shape so admission snapshots and persisted grants can compare it
/// without depending on runtime internals.
pub(crate) const LOCAL_SYSTEM_AGENT_URA: &str = "easynet:///r/_system/agent/_system.local";

/// Canonical whole-realm prefix used by directory/federation filters.
///
/// Directory queries use prefix matching instead of a concrete role URA.
/// Axon exposes canonical role builders, so the CLI derives the prefix
/// from the product Hub facade here instead of letting callers assemble
/// scheme fragments.
pub fn realm_prefix_ura(realm: &str) -> anyhow::Result<String> {
    let hub = hub_ura(realm);
    let prefix = hub.strip_suffix("/authority").ok_or_else(|| {
        anyhow::anyhow!("Axon authority_ura returned unexpected identity shape: {hub:?}")
    })?;
    Ok(format!("{prefix}/"))
}

/// Parsed canonical Ability URA selector.
///
/// What this is: a small boundary object that projects an Ability URA
/// into the three daemon-local facts every caller needs: owner URA,
/// public ability name, and local registry dispatch key.
///
/// What this is not: it is not a URA parser or grammar copy. Parsing
/// still belongs to Axon (`axon_sdk::ura::parse_ura`); this type
/// only packages the CLI daemon projection that otherwise tends to be
/// hand-written at each call site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbilitySelector {
    ability_ura: String,
    owner_ura: String,
    owner_kind: &'static str,
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

        let (owner_ura, owner_kind, dispatch_target) = match ability.owner {
            AbilityOwner::Agent { user_id, agent_id } => (
                agent_ura(&parsed.realm, &user_id, &agent_id),
                "agent",
                agent_id.clone(),
            ),
            AbilityOwner::Device { device_id } => {
                let owner_ura = device_ura(&parsed.realm, &device_id);
                (owner_ura.clone(), "device", owner_ura)
            }
            AbilityOwner::Authority => {
                let owner_ura = hub_ura(&parsed.realm);
                (owner_ura.clone(), "hub", owner_ura)
            }
        };
        let public_name = ability_name_from_parts(&parsed).ok_or_else(|| {
            anyhow::anyhow!("invalid Ability URA {ability_ura:?}: missing public ability name")
        })?;
        let local_registry_ability = local_dispatch_ability_key(&owner_ura, &public_name);

        Ok(Self {
            ability_ura: ability_ura.to_string(),
            owner_ura,
            owner_kind,
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

    /// Owner kind encoded by the Ability URA: `"agent"`, `"device"`,
    /// or `"hub"`. Derived from the typed `AbilityOwner` arm at parse
    /// time — consumers never re-sniff URA strings (F-047).
    pub fn owner_kind(&self) -> &'static str {
        self.owner_kind
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

/// Parsed owner-local registry key in the CLI's `<agent>.<ability>` shape.
///
/// What this is: a daemon-local value object for registry keys that combine a
/// hosted agent short name with the public ability name it advertises.
///
/// What this is not: it is not a URA parser and it is not a network identity.
/// The owner segment is the local short name from `local-agents.json`; callers
/// must still resolve it to an Agent URA before minting protocol identities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerLocalAbilityName {
    registry_name: String,
    owner: String,
    public_name: String,
}

impl OwnerLocalAbilityName {
    /// Parse `<agent>.<ability>` exactly once for local registry surfaces.
    ///
    /// Invariant 1: agent short names are dot-free (`AgentSpec` validates this),
    /// so the first dot is the only owner/ability boundary.
    ///
    /// Invariant 2: public ability names may contain dots (`fs.read`,
    /// `meta.acquire`), so callers must not use `rsplit_once('.')`.
    pub fn parse(raw: &str) -> anyhow::Result<Self> {
        let raw = raw.trim();
        let Some((owner, public_name)) = raw.split_once('.') else {
            anyhow::bail!("owner-local ability must use `<agent>.<ability>` form; got {raw:?}");
        };
        let owner = owner.trim();
        let public_name = public_name.trim();
        if owner.is_empty() || public_name.is_empty() {
            anyhow::bail!("owner-local ability must have non-empty owner and ability segments");
        }
        if owner.contains('.') {
            anyhow::bail!("owner-local ability owner segment must not contain `.`");
        }
        Ok(Self {
            registry_name: format!("{owner}.{public_name}"),
            owner: owner.to_string(),
            public_name: public_name.to_string(),
        })
    }

    pub fn registry_name(&self) -> &str {
        &self.registry_name
    }

    pub fn owner(&self) -> &str {
        &self.owner
    }

    pub fn public_name(&self) -> &str {
        &self.public_name
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
            // Hosted user agents and device-sponsored System Agents
            // (DEC-F048) both prefix owner-local names with their
            // agent_id; the two grammars expose it through different
            // accessors (`agent_ids` vs `device_agent_ids`).
            let Some(agent_id) = owner
                .agent_ids()
                .map(|(_, id)| id)
                .or_else(|| owner.device_agent_ids().map(|(_, id)| id))
            else {
                return name.to_string();
            };
            name.strip_prefix(&format!("{agent_id}."))
                .unwrap_or(name)
                .to_string()
        }
        URAKind::Device => name.strip_prefix("device.").unwrap_or(name).to_string(),
        URAKind::Authority => name.strip_prefix("hub.").unwrap_or(name).to_string(),
        _ => name.to_string(),
    }
}

/// Project a registry name into the public descriptor name for an authority.
///
/// User-owned Agents carry their Agent id in the Ability owner token, so their
/// descriptor name is owner-local. A device-sponsored Agent may expose a bare
/// verb such as `screenshot`; in that case its Agent id supplies the required
/// namespace (`terminal.screenshot`). Already-namespaced abilities such as
/// `consent.decide` remain unchanged—the Agent identity is already encoded in
/// the Ability owner token and must not be duplicated into the public name.
pub fn descriptor_public_ability_name(owner_ura: &str, ability_name: &str) -> String {
    let owner_local_name = owner_local_ability_name(owner_ura, ability_name);
    let Ok(owner) = parse_ura(owner_ura) else {
        return owner_local_name;
    };
    let Some((_device_id, agent_id)) = owner.device_agent_ids() else {
        return owner_local_name;
    };
    let prefix = format!("{agent_id}.");
    if owner_local_name.contains('.') || owner_local_name.starts_with(&prefix) {
        owner_local_name
    } else {
        format!("{prefix}{owner_local_name}")
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
            // Dual-grammar agent_id, same as owner_local_ability_name.
            let Some(agent_id) = target
                .agent_ids()
                .map(|(_, id)| id)
                .or_else(|| target.device_agent_ids().map(|(_, id)| id))
            else {
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
        URAKind::Device | URAKind::Authority => owner_local_ability_name(target_ura, name),
        _ => name.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_uses_axon_sdk_ura_builder() {
        assert_eq!(
            hub_ability_ura("localhost", "federation.resolve"),
            "easynet:///r/localhost/ability/authority.federation.resolve"
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
    fn realm_prefix_ura_is_derived_from_canonical_hub_builder() {
        assert_eq!(
            realm_prefix_ura("localhost").expect("realm prefix"),
            "easynet:///r/localhost/"
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
            owner_local_ability_name("easynet:///r/localhost/authority", "hub.openai.chat"),
            "openai.chat"
        );
        assert_eq!(
            owner_local_ability_name(
                "easynet:///r/localhost/authority",
                "authority.binding.grant"
            ),
            "authority.binding.grant"
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
    fn owner_local_ability_name_handles_device_sponsored_agents() {
        // DEC-F048 grammar: agent/device.<device-id>.<agent-id>.
        // The agent_id prefix strips exactly like a user-owned agent.
        assert_eq!(
            owner_local_ability_name(
                "easynet:///r/localhost/agent/device.dev-1.terminal",
                "terminal.screenshot"
            ),
            "screenshot"
        );
        assert_eq!(
            local_dispatch_ability_key(
                "easynet:///r/localhost/agent/device.dev-1.terminal",
                "screenshot"
            ),
            "terminal.screenshot"
        );
    }

    #[test]
    fn descriptor_public_name_qualifies_only_bare_device_agent_verbs() {
        let owner = "easynet:///r/localhost/agent/device.dev-1.terminal";
        assert_eq!(
            descriptor_public_ability_name(owner, "terminal.screenshot"),
            "terminal.screenshot"
        );
        assert_eq!(
            descriptor_public_ability_name(owner, "screenshot"),
            "terminal.screenshot"
        );
        assert_eq!(
            descriptor_public_ability_name(owner, "consent.decide"),
            "consent.decide",
            "an authored namespace must not gain a second Agent prefix"
        );
        assert_eq!(
            owner_ability_ura(
                owner,
                &descriptor_public_ability_name(owner, "terminal.screenshot")
            )
            .as_deref(),
            Some("easynet:///r/localhost/ability/device.dev-1.terminal.screenshot")
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
    fn owner_local_ability_name_keeps_dotted_public_ability() {
        let parsed =
            OwnerLocalAbilityName::parse("mentor.meta.acquire").expect("owner-local ability");

        assert_eq!(parsed.registry_name(), "mentor.meta.acquire");
        assert_eq!(parsed.owner(), "mentor");
        assert_eq!(parsed.public_name(), "meta.acquire");
    }

    #[test]
    fn owner_local_ability_name_rejects_missing_boundary() {
        let err = OwnerLocalAbilityName::parse("mentor").expect_err("missing dot must fail");
        assert!(err.to_string().contains("<agent>.<ability>"), "{err}");
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
