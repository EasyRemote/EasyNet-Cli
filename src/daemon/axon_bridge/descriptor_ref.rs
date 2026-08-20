//! Descriptor-bound ability reference normalization.
//!
//! Axon protobuf transport code and daemon-local runtime code both need the
//! same descriptor reference rules, but only the former depends on generated
//! protobuf modules. This module owns the pure descriptor-reference portion so
//! feature-agnostic runtime paths never import `axon-pb`-gated transport code.

use std::sync::Arc;

use axon_sdk::invocation::{
    ability_ura_from_descriptor_ref as axon_ability_ura_from_descriptor_ref,
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
pub(crate) enum DescriptorBindingError {
    /// The ability is not registered in the local runtime. The message carries
    /// the canonical not-found tokens so a pre-dispatch miss classifies as
    /// NOT_FOUND (via `is_not_found_error` / `NOT_FOUND_REASON_FRAGMENTS`)
    /// instead of a generic failure.
    AbilityNotFound(String),
    /// The ability is registered but registration left the per-mode descriptor
    /// proof unbound, so no truthful version exists.
    VersionUnbound(String),
}

impl DescriptorBindingError {
    pub(crate) fn message(&self) -> &str {
        match self {
            Self::AbilityNotFound(message) | Self::VersionUnbound(message) => message,
        }
    }
}

impl std::fmt::Display for DescriptorBindingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

impl From<DescriptorBindingError> for AxonError {
    fn from(error: DescriptorBindingError) -> Self {
        AxonError::invalid_argument(error.message().to_string())
    }
}

pub(crate) fn ability_ura_from_descriptor_ref(descriptor_ref: &str) -> Result<String, AxonError> {
    let canonical = canonical_ability_descriptor_ref(descriptor_ref)?;
    axon_ability_ura_from_descriptor_ref(&canonical).map(str::to_string)
}

pub(crate) fn ability_selector_from_descriptor_ref(
    descriptor_ref: &str,
) -> Result<crate::core::ura::AbilitySelector, AxonError> {
    let ability_ura = ability_ura_from_descriptor_ref(descriptor_ref)?;
    crate::core::ura::AbilitySelector::parse(&ability_ura).map_err(|error| {
        AxonError::invalid_argument(format!(
            "descriptor_ref ability selector parse failed: {error}"
        ))
    })
}

pub(crate) fn descriptor_version_from_descriptor_ref(
    descriptor_ref: &str,
) -> Result<String, AxonError> {
    let canonical = canonical_ability_descriptor_ref(descriptor_ref)?;
    let Some((_ability_ura, version_and_digest)) = canonical.rsplit_once('@') else {
        return Err(AxonError::invalid_argument(
            "descriptor-bound ability ref is missing descriptor version",
        ));
    };
    let Some((version, _digest)) = version_and_digest.split_once('#') else {
        return Err(AxonError::invalid_argument(
            "descriptor-bound ability ref is missing descriptor digest",
        ));
    };
    let version = version.trim();
    if version.is_empty() {
        return Err(AxonError::invalid_argument(
            "descriptor-bound ability descriptor version is empty",
        ));
    }
    Ok(version.to_string())
}

/// Read the complete descriptor binding registered for `runtime_ability` in
/// `mode`: version, canonical descriptor digest, and admission action.
///
/// The version is bound into the runtime's per-mode `AbilityProofBinding` at
/// registration time (see `AxonAbilityCatalog::bind_runtime_proof_for_mode`),
/// so the runtime is the single source of truth. Every descriptor-bound
/// dispatch path resolves through this one helper so the wire envelope and its
/// receipt proof facts carry the version actually admitted at registration
/// rather than a fabricated default.
pub(crate) async fn registered_descriptor_binding(
    runtime: &Arc<LocalRuntime>,
    runtime_ability: &str,
    mode: AxonInvocationCallMode,
) -> Result<String, DescriptorBindingError> {
    let options = runtime
        .ability_options(runtime_ability)
        .await
        .ok_or_else(|| {
            DescriptorBindingError::AbilityNotFound(format!(
                "descriptor-bound dispatch of `{runtime_ability}` cannot resolve a descriptor \
             version: unknown_ability `{runtime_ability}` is not registered in the local \
             runtime (ability_not_found)"
            ))
        })?;
    let proof = options.proof_for_mode(mode).ok_or_else(|| {
        DescriptorBindingError::VersionUnbound(format!(
            "descriptor-bound dispatch of `{runtime_ability}` cannot resolve a descriptor \
             version: runtime registration has no {mode:?} descriptor proof"
        ))
    })?;
    let version = proof.descriptor_version;
    if version.trim().is_empty() {
        return Err(DescriptorBindingError::VersionUnbound(format!(
            "descriptor-bound dispatch of `{runtime_ability}` cannot resolve a descriptor \
             version: runtime registration left the {mode:?} descriptor proof unbound"
        )));
    }
    if proof.descriptor_hash == [0u8; 32] {
        return Err(DescriptorBindingError::VersionUnbound(format!(
            "descriptor-bound dispatch of `{runtime_ability}` cannot resolve a descriptor digest"
        )));
    }
    if proof.admission_action.trim().is_empty() {
        return Err(DescriptorBindingError::VersionUnbound(format!(
            "descriptor-bound dispatch of `{runtime_ability}` has no admission action"
        )));
    }
    descriptor_binding_for_wire(&version, proof.descriptor_hash, &proof.admission_action)
        .map_err(|error| DescriptorBindingError::VersionUnbound(error.to_string()))
}

pub(crate) fn descriptor_binding_for_wire(
    descriptor_version: &str,
    descriptor_hash: [u8; 32],
    admission_action: &str,
) -> Result<String, AxonError> {
    let descriptor_version = descriptor_version.trim();
    let admission_action = admission_action.trim();
    if descriptor_version.is_empty() {
        return Err(AxonError::invalid_argument(
            "descriptor-bound ability descriptor version is empty",
        ));
    }
    if descriptor_hash == [0u8; 32] {
        return Err(AxonError::invalid_argument(
            "descriptor-bound ability descriptor hash is empty",
        ));
    }
    if admission_action.is_empty() {
        return Err(AxonError::invalid_argument(
            "descriptor-bound ability admission action is empty",
        ));
    }
    Ok(format!(
        "{descriptor_version}#{}!{admission_action}",
        hex::encode(descriptor_hash)
    ))
}

/// Resolve a descriptor binding for a compile-time system protocol ability.
///
/// This is the bootstrap lane used before a live runtime catalog can be read.
/// Hosted and federated application abilities must carry an explicit
/// descriptor-bound Ability ref obtained from the live catalog.
pub(crate) fn system_protocol_descriptor_binding_for_wire(
    callee_ura: &str,
    ability: &str,
    call_mode: crate::daemon::ability::CallMode,
) -> Result<String, AxonError> {
    let ability_ura = ability_ura_for_wire(callee_ura, ability)?;
    let selector = crate::core::ura::AbilitySelector::parse(&ability_ura).map_err(|err| {
        AxonError::invalid_argument(format!(
            "descriptor-bound ability `{ability_ura}` is not a canonical Ability URA: {err}"
        ))
    })?;
    let catalog = crate::daemon::ability::catalog::build_system_registry();
    let owner = catalog_owner_kind_for_wire(selector.owner_ura())?;
    let descriptor = catalog
        .public_descriptor_for_mode(&owner, selector.public_name(), call_mode)
        .map_err(|error| {
            AxonError::invalid_argument(format!(
                "descriptor-bound ability `{}` cannot resolve a unique system catalog descriptor \
                 for owner `{}` in {:?}: {error}; callers must provide an explicit \
                 descriptor-bound Ability ref",
                selector.public_name(),
                selector.owner_ura(),
                call_mode
            ))
        })?
        .rebind_owner_ura(selector.owner_ura())
        .map_err(|err| {
            AxonError::invalid_argument(format!(
                "descriptor-bound ability `{}` cannot rebind descriptor owner to `{}`: {err}",
                selector.public_name(),
                selector.owner_ura()
            ))
        })?;
    descriptor_binding_for_wire(
        &descriptor.version,
        descriptor.descriptor_hash_bytes(),
        descriptor.admission_action().as_str(),
    )
}

pub(crate) fn system_protocol_descriptor_ref_for_wire(
    callee_ura: &str,
    ability: &str,
    call_mode: crate::daemon::ability::CallMode,
) -> Result<String, AxonError> {
    let descriptor_binding =
        system_protocol_descriptor_binding_for_wire(callee_ura, ability, call_mode)?;
    ability_descriptor_ref_for_wire(callee_ura, ability, &descriptor_binding)
}

/// Bind a remote call without treating the compile-time system registry as a
/// dynamic catalog. Application abilities must already be descriptor-bound;
/// only named system protocol abilities may use the bootstrap registry.
pub(crate) fn remote_descriptor_ref_for_wire(
    callee_ura: &str,
    ability: &str,
    call_mode: crate::daemon::ability::CallMode,
) -> Result<String, AxonError> {
    if let Ok(descriptor_ref) = canonical_ability_descriptor_ref(ability) {
        // `ability_ura_for_wire` proves that the explicit ref belongs to the
        // requested callee before it can enter the signed envelope.
        ability_ura_for_wire(callee_ura, &descriptor_ref)?;
        return Ok(descriptor_ref);
    }
    system_protocol_descriptor_ref_for_wire(callee_ura, ability, call_mode)
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
        let selector = crate::core::ura::AbilitySelector::parse(&ability_ura).map_err(|err| {
            AxonError::invalid_argument(format!(
                "descriptor-bound ability ref carries invalid ability URA `{ability_ura}`: {err}"
            ))
        })?;
        ensure_ability_owner_matches_callee(callee_ura, &selector)?;
        return Ok(selector.ability_ura().to_string());
    }

    let ability_ura = match crate::core::ura::AbilitySelector::parse(ability) {
        Ok(selector) => {
            ensure_ability_owner_matches_callee(callee_ura, &selector)?;
            selector.ability_ura().to_string()
        }
        Err(_) => {
            let callee = crate::core::ura::parse_ura(callee_ura).map_err(|err| {
                AxonError::invalid_argument(format!(
                    "descriptor-bound ability callee URA is invalid: {err}"
                ))
            })?;
            if callee.kind == crate::core::ura::URAKind::Authority && ability.starts_with("hub.") {
                return Err(AxonError::invalid_argument(
                    "Authority descriptor-bound ability must not use retired hub.* ability aliases",
                ));
            }
            let public_ability = public_ability_name_for_wire(callee_ura, ability);
            crate::core::ura::owner_ability_ura(callee_ura, &public_ability).ok_or_else(|| {
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
    descriptor_binding: &str,
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
        crate::core::ura::AbilitySelector::parse(&ability_ura)
            .map_err(|err| {
                AxonError::invalid_argument(format!(
                    "descriptor-bound ability ref carries invalid ability URA `{ability_ura}`: {err}"
                ))
            })
            .and_then(|selector| ensure_ability_owner_matches_callee(callee_ura, &selector))?;
        let expected = canonical_ability_descriptor_ref(&format!(
            "{ability_ura}@{}",
            descriptor_binding.trim()
        ))?;
        if descriptor_ref != expected {
            return Err(AxonError::invalid_argument(
                "explicit descriptor ref does not match the runtime-selected descriptor binding",
            ));
        }
        return Ok(descriptor_ref);
    }

    let descriptor_binding = descriptor_binding.trim();
    let Some((descriptor_version, descriptor_hash_and_action)) = descriptor_binding.split_once('#')
    else {
        return Err(AxonError::invalid_argument(
            "descriptor-bound ability descriptor digest is required",
        ));
    };
    if descriptor_version.is_empty() {
        return Err(AxonError::invalid_argument(
            "descriptor-bound ability descriptor version is empty",
        ));
    }
    let ability_ura = ability_ura_for_wire(callee_ura, ability)?;
    canonical_ability_descriptor_ref(&format!(
        "{ability_ura}@{descriptor_version}#{descriptor_hash_and_action}"
    ))
}

fn public_ability_name_for_wire(callee_ura: &str, ability: &str) -> String {
    crate::core::ura::descriptor_public_ability_name(callee_ura, ability)
}

pub(crate) fn catalog_owner_kind_for_wire(
    owner_ura: &str,
) -> Result<crate::daemon::ability::dispatch::OwnerKind, AxonError> {
    let parsed = crate::core::ura::parse_ura(owner_ura).map_err(|err| {
        AxonError::invalid_argument(format!(
            "descriptor-bound ability owner `{owner_ura}` is not a valid URA: {err}"
        ))
    })?;
    match crate::core::ura::ability_owner_from_identity(&parsed) {
        Some(crate::core::ura::AbilityOwner::Device { .. }) => {
            Ok(crate::daemon::ability::dispatch::OwnerKind::DeviceProfileProjection)
        }
        Some(crate::core::ura::AbilityOwner::Authority) => {
            Ok(crate::daemon::ability::dispatch::OwnerKind::RealmAuthority)
        }
        Some(crate::core::ura::AbilityOwner::SystemAgent { agent_id, .. }) => {
            if !crate::daemon::ability::catalog::profiles::is_declared_daemon_native_system_agent_id(
                &agent_id,
            ) {
                return Err(AxonError::invalid_argument(format!(
                    "descriptor-bound ability owner `{owner_ura}` is a device-scoped Agent that is not a declared daemon-native SystemAgent"
                )));
            }
            Ok(crate::daemon::ability::dispatch::OwnerKind::SystemAgent(
                agent_id,
            ))
        }
        Some(crate::core::ura::AbilityOwner::Agent { agent_id, .. }) => {
            Ok(crate::daemon::ability::dispatch::OwnerKind::Agent(agent_id))
        }
        Some(crate::core::ura::AbilityOwner::Service {
            principal_id,
            service_id,
        }) => Ok(crate::daemon::ability::dispatch::OwnerKind::Service {
            principal_id,
            service_id,
        }),
        None => Err(AxonError::invalid_argument(format!(
            "descriptor-bound ability owner `{owner_ura}` is not a catalog owner"
        ))),
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
    let selector = crate::core::ura::AbilitySelector::parse(&ability_ura).map_err(|err| {
        AxonError::invalid_argument(format!(
            "descriptor-bound ability ref carries invalid ability URA `{ability_ura}`: {err}"
        ))
    })?;
    ensure_ability_owner_matches_callee(callee_ura, &selector)?;
    Ok(descriptor_ref)
}

fn ensure_ability_owner_matches_callee(
    callee_ura: &str,
    selector: &crate::core::ura::AbilitySelector,
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

    const TEST_BINDING: &str =
        "1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!read";

    #[test]
    fn descriptor_ref_requires_callee_to_own_ability() {
        let callee = crate::core::ura::device_ura("acme", "host-a");
        let other = crate::core::ura::device_ura("acme", "host-b");
        let ability_ura = crate::core::ura::owner_ability_ura(&other, "fs.read").unwrap();
        let ability_ref = format!("{ability_ura}@{TEST_BINDING}");

        let err = ability_descriptor_ref_for_wire(&callee, &ability_ref, TEST_BINDING).unwrap_err();

        assert!(err.to_string().contains("does not match callee"));
    }

    #[test]
    fn descriptor_ref_round_trips_when_callee_owns_ability() {
        let callee = crate::core::ura::device_ura("acme", "host-a");
        let ability_ura = crate::core::ura::owner_ability_ura(&callee, "fs.read").unwrap();
        let ability_ref = format!("{ability_ura}@{TEST_BINDING}");

        let normalized =
            ability_descriptor_ref_for_wire(&callee, &ability_ref, TEST_BINDING).unwrap();

        assert_eq!(normalized, ability_ref);
    }

    #[test]
    fn system_agent_public_name_survives_owner_local_wire_projection() {
        let callee = crate::core::ura::device_agent_ura("acme", "host-a", "terminal");

        assert_eq!(
            ability_ura_for_wire(&callee, "terminal.create").expect("terminal Ability URA"),
            "easynet:///r/acme/ability/system-agent.host-a.terminal.terminal.create"
        );
    }

    #[test]
    fn explicit_descriptor_ref_rejects_a_different_runtime_digest() {
        let callee = crate::core::ura::device_ura("acme", "host-a");
        let ability_ura = crate::core::ura::owner_ability_ura(&callee, "fs.read").unwrap();
        let ability_ref = format!("{ability_ura}@1.0.0#{}!read", "11".repeat(32));

        let err = ability_descriptor_ref_for_wire(&callee, &ability_ref, TEST_BINDING)
            .expect_err("caller-selected digest must not replace the runtime binding");

        assert!(err.to_string().contains("runtime-selected"), "{err}");
    }

    #[test]
    fn explicit_descriptor_ref_is_required_when_no_version_is_available() {
        let callee = crate::core::ura::device_ura("acme", "host-a");
        let err = require_descriptor_ref_for_wire(&callee, "fs.read").unwrap_err();

        assert!(err.to_string().contains("explicit descriptor ref"), "{err}");
    }

    #[test]
    fn system_catalog_descriptor_lookup_is_owner_plane_aware() {
        let runtime_introspection = crate::core::ura::device_agent_ura(
            "localhost",
            "host-a",
            crate::daemon::ability::names::governance::RUNTIME_INTROSPECTION_SYSTEM_AGENT_ID,
        );
        let hub = crate::core::ura::hub_ura("localhost");
        let ability = crate::daemon::ability::names::governance::META_LIST_ABILITIES;

        let device_ref = system_protocol_descriptor_ref_for_wire(
            &runtime_introspection,
            ability,
            crate::daemon::ability::CallMode::Rpc,
        )
        .expect("runtime-introspection meta.list_abilities descriptor");
        let hub_ref = system_protocol_descriptor_ref_for_wire(
            &hub,
            ability,
            crate::daemon::ability::CallMode::Rpc,
        )
        .expect("Hub meta.list_abilities descriptor");

        assert!(
            device_ref.starts_with(&format!(
                "{}/ability/system-agent.host-a.runtime-introspection.{ability}@",
                "easynet:///r/localhost"
            )),
            "{device_ref}"
        );
        assert!(
            hub_ref.starts_with(&format!(
                "{}/ability/authority.{ability}@",
                "easynet:///r/localhost"
            )),
            "{hub_ref}"
        );
        assert_ne!(device_ref, hub_ref);
    }

    #[test]
    fn remote_application_ability_requires_and_preserves_explicit_descriptor_ref() {
        let callee = crate::core::ura::agent_ura("acme", "host-a", "worker");
        let ability_ura = crate::core::ura::owner_ability_ura(&callee, "custom.execute").unwrap();
        let explicit = format!("{ability_ura}@{TEST_BINDING}");

        let resolved = remote_descriptor_ref_for_wire(
            &callee,
            &explicit,
            crate::daemon::ability::CallMode::Rpc,
        )
        .expect("explicit live-catalog descriptor ref");

        assert_eq!(resolved, explicit);
        let error = remote_descriptor_ref_for_wire(
            &callee,
            "custom.execute",
            crate::daemon::ability::CallMode::Rpc,
        )
        .expect_err("application ability name must not resolve from the system bootstrap catalog");
        assert!(
            error.to_string().contains("system catalog descriptor"),
            "{error}"
        );
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
        let callee = crate::core::ura::device_ura("acme", "host-a");

        // Validator rejects the bare name (this is the failing path the prelude hit).
        assert!(require_descriptor_ref_for_wire(&callee, "fs.read").is_err());

        // Builder turns the SAME bare name into a canonical `<ability-ura>@<version>`.
        let built = ability_descriptor_ref_for_wire(&callee, "fs.read", TEST_BINDING)
            .expect("builder accepts a bare ability name plus an explicit version");

        let expected_ura = crate::core::ura::owner_ability_ura(&callee, "fs.read").unwrap();
        assert_eq!(built, format!("{expected_ura}@{}", TEST_BINDING));
        // The built ref carries EXACTLY one `@`, which is what the hub ingress
        // (`ability_descriptor_ref_malformed` guard) requires.
        assert_eq!(built.matches('@').count(), 1);
    }

    #[test]
    fn bare_agent_prefixed_name_projects_to_owner_local_ability_ura() {
        let callee = crate::core::ura::agent_ura("localhost", "dev", "worker");
        let ability_ura = ability_ura_for_wire(&callee, "task.list")
            .expect("agent-owned registry key should project to public ability URA");

        assert_eq!(
            ability_ura,
            "easynet:///r/localhost/ability/dev.worker.task.list"
        );
    }

    #[test]
    fn bare_service_prefixed_name_projects_to_owner_local_ability_ura() {
        let callee = crate::core::ura::service_ura("localhost", "dev", "pages");
        let ability_ura = ability_ura_for_wire(&callee, "project_list")
            .expect("service-owned registry key should project to public ability URA");

        assert_eq!(
            ability_ura,
            "easynet:///r/localhost/ability/service.dev.pages.project_list"
        );
    }

    #[test]
    fn catalog_owner_rejects_undeclared_device_scoped_agent() {
        let owner = crate::core::ura::device_agent_ura("acme", "dev-a", "random-agent");

        let error = catalog_owner_kind_for_wire(&owner)
            .expect_err("unknown device-scoped Agent must not become a SystemAgent owner");

        assert!(
            error
                .reason
                .contains("not a declared daemon-native SystemAgent"),
            "{}",
            error.reason
        );
    }

    #[test]
    fn catalog_owner_accepts_service_identity() {
        let owner = crate::core::ura::service_ura("acme", "user-alice", "pages");

        let kind = catalog_owner_kind_for_wire(&owner).expect("Service is a catalog owner");

        assert_eq!(
            kind,
            crate::daemon::ability::dispatch::OwnerKind::Service {
                principal_id: "user-alice".to_string(),
                service_id: "pages".to_string(),
            }
        );
    }

    #[test]
    fn catalog_owner_rejects_user_identity() {
        let owner = crate::core::ura::user_ura("acme", "user-alice");

        let error = catalog_owner_kind_for_wire(&owner)
            .expect_err("User principal must not become an executable catalog owner");

        assert!(
            error.reason.contains("not a catalog owner"),
            "{}",
            error.reason
        );
    }

    #[test]
    fn bare_device_domain_name_is_not_duplicated_in_ability_ura() {
        let callee = crate::core::ura::device_ura("localhost", "dev-a");
        let ability_ura = ability_ura_for_wire(&callee, "device.inspect")
            .expect("device-domain ability should remain explicit");

        assert_eq!(
            ability_ura,
            "easynet:///r/localhost/ability/device.dev-a.inspect"
        );
    }

    #[test]
    fn authority_bare_hub_prefixed_name_is_rejected() {
        let callee = crate::core::ura::hub_ura("localhost");
        let error = ability_ura_for_wire(&callee, "hub.openai.list_models")
            .expect_err("Authority bare names must not project through Hub aliases");

        assert!(
            error
                .reason
                .contains("must not use retired hub.* ability aliases"),
            "unexpected error: {}",
            error.reason
        );
    }
}
