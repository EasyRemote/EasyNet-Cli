//! Descriptor-bound ability reference normalization.
//!
//! Axon protobuf transport code and daemon-local runtime code both need the
//! same descriptor reference rules, but only the former depends on generated
//! protobuf modules. This module owns the pure descriptor-reference portion so
//! feature-agnostic runtime paths never import `axon-pb`-gated transport code.

use std::sync::Arc;

use easynet_axon::invocation::{
    canonical_ability_descriptor_ref, AxonError, CallMode as AxonInvocationCallMode, LocalRuntime,
};

/// Failure to resolve the descriptor version a runtime registered for an
/// ability in a given call mode.
///
/// Carried as a typed error so every dispatch path (RPC/stream/bidi wire,
/// daemon-local invoker, kernel-internal dispatch) maps it into its own
/// domain error without re-deriving the canonical not-found tokens. There is
/// deliberately NO default fallback: an ability dispatched without a
/// registered, version-bound descriptor cannot be stamped with a truthful
/// version, so resolution fails closed.
#[derive(Debug)]
pub(crate) enum DescriptorVersionError {
    /// The ability is not registered in the local runtime. The message carries
    /// the canonical not-found tokens so a pre-dispatch miss classifies as
    /// NOT_FOUND (via `is_not_found_error` / `NOT_FOUND_REASON_FRAGMENTS`)
    /// instead of a generic failure.
    AbilityNotFound(String),
    /// The ability is registered but registration left the per-mode descriptor
    /// proof unbound, so no truthful version exists.
    VersionUnbound(String),
}

impl DescriptorVersionError {
    pub(crate) fn message(&self) -> &str {
        match self {
            Self::AbilityNotFound(message) | Self::VersionUnbound(message) => message,
        }
    }
}

impl std::fmt::Display for DescriptorVersionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

impl From<DescriptorVersionError> for AxonError {
    fn from(error: DescriptorVersionError) -> Self {
        AxonError::invalid_argument(error.message().to_string())
    }
}

pub(crate) fn ability_ura_from_descriptor_ref(descriptor_ref: &str) -> Result<String, AxonError> {
    let canonical = canonical_ability_descriptor_ref(descriptor_ref)?;
    let (ability_ura, _) = canonical.split_once('@').ok_or_else(|| {
        AxonError::invalid_argument("ability_descriptor_ref_malformed".to_string())
    })?;
    Ok(ability_ura.to_string())
}

pub(crate) fn descriptor_version_from_descriptor_ref(
    descriptor_ref: &str,
) -> Result<String, AxonError> {
    let canonical = canonical_ability_descriptor_ref(descriptor_ref)?;
    let (_, descriptor_version) = canonical.split_once('@').ok_or_else(|| {
        AxonError::invalid_argument("ability_descriptor_ref_malformed".to_string())
    })?;
    Ok(descriptor_version.to_string())
}

/// Read the descriptor version the runtime registered for `runtime_ability`
/// in `mode`.
///
/// The version is bound into the runtime's per-mode `AbilityProofBinding` at
/// registration time (see `AxonAbilityCatalog::bind_runtime_proof_for_mode`),
/// so the runtime is the single source of truth. Every descriptor-bound
/// dispatch path resolves through this one helper so the wire envelope and its
/// receipt proof facts carry the version actually admitted at registration
/// rather than a fabricated default.
pub(crate) async fn registered_descriptor_version(
    runtime: &Arc<LocalRuntime>,
    runtime_ability: &str,
    mode: AxonInvocationCallMode,
) -> Result<String, DescriptorVersionError> {
    let options = runtime
        .ability_options(runtime_ability)
        .await
        .ok_or_else(|| {
            DescriptorVersionError::AbilityNotFound(format!(
                "descriptor-bound dispatch of `{runtime_ability}` cannot resolve a descriptor \
             version: unknown_ability `{runtime_ability}` is not registered in the local \
             runtime (ability_not_found)"
            ))
        })?;
    let version = options.proof_for_mode(mode).descriptor_version;
    if version.trim().is_empty() {
        return Err(DescriptorVersionError::VersionUnbound(format!(
            "descriptor-bound dispatch of `{runtime_ability}` cannot resolve a descriptor \
             version: runtime registration left the {mode:?} descriptor proof unbound"
        )));
    }
    Ok(version)
}

pub(crate) fn ability_ura_for_wire(callee_ura: &str, ability: &str) -> Result<String, AxonError> {
    let callee_ura = callee_ura.trim();
    let ability = ability.trim();
    if callee_ura.is_empty() {
        return Err(AxonError::invalid_argument(
            "descriptor-bound ability callee URA is empty",
        ));
    }
    if ability.is_empty() {
        return Err(AxonError::invalid_argument(
            "descriptor-bound ability name is empty",
        ));
    }

    if let Ok(descriptor_ref) = canonical_ability_descriptor_ref(ability) {
        let ability_ura = ability_ura_from_descriptor_ref(&descriptor_ref)?;
        let selector = crate::ura::AbilitySelector::parse(&ability_ura).map_err(|err| {
            AxonError::invalid_argument(format!(
                "descriptor-bound ability ref carries invalid ability URA `{ability_ura}`: {err}"
            ))
        })?;
        ensure_ability_owner_matches_callee(callee_ura, &selector)?;
        return Ok(selector.ability_ura().to_string());
    }

    let ability_ura = match crate::ura::AbilitySelector::parse(ability) {
        Ok(selector) => {
            ensure_ability_owner_matches_callee(callee_ura, &selector)?;
            selector.ability_ura().to_string()
        }
        Err(_) => {
            let public_ability = public_ability_name_for_wire(callee_ura, ability);
            crate::ura::owner_ability_ura(callee_ura, &public_ability).ok_or_else(|| {
                AxonError::invalid_argument(format!(
                    "derive descriptor-bound ability URA for callee `{callee_ura}` ability `{ability}`"
                ))
            })?
        }
    };
    Ok(ability_ura)
}

pub(crate) fn ability_descriptor_ref_for_wire(
    callee_ura: &str,
    ability: &str,
    descriptor_version: &str,
) -> Result<String, AxonError> {
    let callee_ura = callee_ura.trim();
    let ability = ability.trim();
    if callee_ura.is_empty() {
        return Err(AxonError::invalid_argument(
            "descriptor-bound ability callee URA is empty",
        ));
    }
    if ability.is_empty() {
        return Err(AxonError::invalid_argument(
            "descriptor-bound ability name is empty",
        ));
    }

    if let Ok(descriptor_ref) = canonical_ability_descriptor_ref(ability) {
        let ability_ura = ability_ura_from_descriptor_ref(&descriptor_ref)?;
        crate::ura::AbilitySelector::parse(&ability_ura)
            .map_err(|err| {
                AxonError::invalid_argument(format!(
                    "descriptor-bound ability ref carries invalid ability URA `{ability_ura}`: {err}"
                ))
            })
            .and_then(|selector| ensure_ability_owner_matches_callee(callee_ura, &selector))?;
        return Ok(descriptor_ref);
    }

    let descriptor_version = descriptor_version.trim();
    if descriptor_version.is_empty() {
        return Err(AxonError::invalid_argument(
            "descriptor-bound ability descriptor version is empty",
        ));
    }
    let ability_ura = ability_ura_for_wire(callee_ura, ability)?;
    Ok(format!("{ability_ura}@{descriptor_version}"))
}

fn public_ability_name_for_wire(callee_ura: &str, ability: &str) -> String {
    let ability = ability.trim();
    let Ok(callee) = crate::ura::parse_ura(callee_ura) else {
        return ability.to_string();
    };
    match callee.kind {
        crate::ura::URAKind::Agent => crate::ura::owner_local_ability_name(callee_ura, ability),
        crate::ura::URAKind::Hub => ability.strip_prefix("hub.").unwrap_or(ability).to_string(),
        crate::ura::URAKind::Device => ability.to_string(),
        _ => ability.to_string(),
    }
}

pub(crate) fn require_descriptor_ref_for_wire(
    callee_ura: &str,
    descriptor_ref: &str,
) -> Result<String, AxonError> {
    let callee_ura = callee_ura.trim();
    let descriptor_ref = descriptor_ref.trim();
    if callee_ura.is_empty() {
        return Err(AxonError::invalid_argument(
            "descriptor-bound ability callee URA is empty",
        ));
    }
    if descriptor_ref.is_empty() {
        return Err(AxonError::invalid_argument(
            "descriptor-bound ability ref is empty",
        ));
    }
    let descriptor_ref = canonical_ability_descriptor_ref(descriptor_ref).map_err(|err| {
        AxonError::invalid_argument(format!(
            "descriptor-bound ability must be an explicit descriptor ref: {err}"
        ))
    })?;
    let ability_ura = ability_ura_from_descriptor_ref(&descriptor_ref)?;
    let selector = crate::ura::AbilitySelector::parse(&ability_ura).map_err(|err| {
        AxonError::invalid_argument(format!(
            "descriptor-bound ability ref carries invalid ability URA `{ability_ura}`: {err}"
        ))
    })?;
    ensure_ability_owner_matches_callee(callee_ura, &selector)?;
    Ok(descriptor_ref)
}

fn ensure_ability_owner_matches_callee(
    callee_ura: &str,
    selector: &crate::ura::AbilitySelector,
) -> Result<(), AxonError> {
    if selector.owner_ura() == callee_ura {
        return Ok(());
    }

    Err(AxonError::invalid_argument(format!(
        "descriptor-bound ability owner `{}` does not match callee `{callee_ura}`",
        selector.owner_ura()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_ref_requires_callee_to_own_ability() {
        let callee = crate::ura::device_ura("acme", "host-a");
        let other = crate::ura::device_ura("acme", "host-b");
        let ability_ura = crate::ura::owner_ability_ura(&other, "fs.read").unwrap();
        let ability_ref = format!(
            "{ability_ura}@{}",
            crate::runtime::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION
        );

        let err = ability_descriptor_ref_for_wire(
            &callee,
            &ability_ref,
            crate::runtime::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION,
        )
        .unwrap_err();

        assert!(err.to_string().contains("does not match callee"));
    }

    #[test]
    fn descriptor_ref_round_trips_when_callee_owns_ability() {
        let callee = crate::ura::device_ura("acme", "host-a");
        let ability_ura = crate::ura::owner_ability_ura(&callee, "fs.read").unwrap();
        let ability_ref = format!(
            "{ability_ura}@{}",
            crate::runtime::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION
        );

        let normalized = ability_descriptor_ref_for_wire(
            &callee,
            &ability_ref,
            crate::runtime::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION,
        )
        .unwrap();

        assert_eq!(normalized, ability_ref);
    }

    #[test]
    fn explicit_descriptor_ref_is_required_when_no_version_is_available() {
        let callee = crate::ura::device_ura("acme", "host-a");
        let err = require_descriptor_ref_for_wire(&callee, "fs.read").unwrap_err();

        assert!(err.to_string().contains("explicit descriptor ref"), "{err}");
    }

    /// Regression: the session prelude (`sign_descriptor_bound_prelude_request`)
    /// gets a BARE ability name (`federation.advertise_abilities`, no `@version`)
    /// and must CONSTRUCT a canonical descriptor ref from it — the builder
    /// accepts a bare name + version, whereas `require_descriptor_ref_for_wire`
    /// (the validator) rejects it with "explicit descriptor ref" (asserted
    /// above). Passing the bare name to the validator was the egress/ingress
    /// asymmetry that wedged session.open into an advertise_abilities reconnect
    /// loop (commit 22187b3f tightened ingress, left this egress site behind).
    #[test]
    fn builder_constructs_ref_from_bare_ability_name_where_validator_rejects() {
        let callee = crate::ura::device_ura("acme", "host-a");

        // Validator rejects the bare name (this is the failing path the prelude hit).
        assert!(require_descriptor_ref_for_wire(&callee, "fs.read").is_err());

        // Builder turns the SAME bare name into a canonical `<ability-ura>@<version>`.
        let built = ability_descriptor_ref_for_wire(
            &callee,
            "fs.read",
            crate::runtime::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION,
        )
        .expect("builder accepts a bare ability name plus an explicit version");

        let expected_ura = crate::ura::owner_ability_ura(&callee, "fs.read").unwrap();
        assert_eq!(
            built,
            format!(
                "{expected_ura}@{}",
                crate::runtime::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION
            )
        );
        // The built ref carries EXACTLY one `@`, which is what the hub ingress
        // (`ability_descriptor_ref_malformed` guard) requires.
        assert_eq!(built.matches('@').count(), 1);
    }

    #[test]
    fn bare_agent_prefixed_name_projects_to_owner_local_ability_ura() {
        let callee = crate::ura::agent_ura("localhost", "dev", "pages");
        let ability_ura = ability_ura_for_wire(&callee, "pages.list")
            .expect("agent-owned registry key should project to public ability URA");

        assert_eq!(ability_ura, "easynet:///r/localhost/ability/dev.pages.list");
    }

    #[test]
    fn bare_device_domain_name_is_preserved_in_ability_ura() {
        let callee = crate::ura::device_ura("localhost", "dev-a");
        let ability_ura = ability_ura_for_wire(&callee, "device.inspect")
            .expect("device-domain ability should remain explicit");

        assert_eq!(
            ability_ura,
            "easynet:///r/localhost/ability/device.dev-a.device.inspect"
        );
    }

    #[test]
    fn bare_hub_prefixed_name_projects_to_owner_local_ability_ura() {
        let callee = crate::ura::hub_ura("localhost");
        let ability_ura = ability_ura_for_wire(&callee, "hub.openai.list_models")
            .expect("hub-owned registry key should project to public ability URA");

        assert_eq!(
            ability_ura,
            "easynet:///r/localhost/ability/hub.openai.list_models"
        );
    }
}
