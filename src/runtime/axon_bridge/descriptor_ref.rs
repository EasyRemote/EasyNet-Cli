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
        let (ability_ura, _) = descriptor_ref.rsplit_once('@').ok_or_else(|| {
            AxonError::invalid_argument("descriptor-bound ability ref missing version")
        })?;
        let selector = crate::ura::AbilitySelector::parse(ability_ura).map_err(|err| {
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
        Err(_) => crate::ura::owner_ability_ura(callee_ura, ability).ok_or_else(|| {
            AxonError::invalid_argument(format!(
                "derive descriptor-bound ability URA for callee `{callee_ura}` ability `{ability}`"
            ))
        })?,
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
        let (ability_ura, _) = descriptor_ref.rsplit_once('@').ok_or_else(|| {
            AxonError::invalid_argument("descriptor-bound ability ref missing version")
        })?;
        crate::ura::AbilitySelector::parse(ability_ura)
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
    let (ability_ura, _) = descriptor_ref.rsplit_once('@').ok_or_else(|| {
        AxonError::invalid_argument("descriptor-bound ability ref missing version")
    })?;
    let selector = crate::ura::AbilitySelector::parse(ability_ura).map_err(|err| {
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
}
