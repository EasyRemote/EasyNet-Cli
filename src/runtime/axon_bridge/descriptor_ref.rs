//! Descriptor-bound ability reference normalization.
//!
//! Axon protobuf transport code and daemon-local runtime code both need the
//! same descriptor reference rules, but only the former depends on generated
//! protobuf modules. This module owns the pure descriptor-reference portion so
//! feature-agnostic runtime paths never import `axon-pb`-gated transport code.

use easynet_axon::invocation::{canonical_ability_descriptor_ref, AxonError};

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
