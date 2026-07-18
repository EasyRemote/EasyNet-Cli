//! CLI-owned runtime bootstrap identity ability and key providers.
//!
//! `runtime.bootstrap_self_identity` is product behavior. The Axon SDK owns
//! descriptor-bound admission and execution, while this module owns the
//! EasyNet bootstrap request, its bounded identity state, and the handler
//! registered into the canonical runtime.

use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex, RwLock,
};

use axon_sdk::invocation::{
    make_ability, AbilityCallModes, AbilityOptions, AxonError, KeyResolver, LocalRuntime,
};
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use ed25519_dalek::VerifyingKey;
use serde::{Deserialize, Serialize};

use crate::daemon::ability::conformance::ABILITY_RUNTIME_BOOTSTRAP_SELF_IDENTITY;
use crate::daemon::ability::dispatch::AxonAbilityCatalog;

const MAX_BOOTSTRAP_KEYS_PER_NODE: usize = 8;

#[derive(Debug, Clone, Deserialize)]
struct BootstrapSelfIdentityArgs {
    tenant_id: String,
    node_id: String,
    owner_id: String,
    #[serde(default)]
    display_name: String,
    public_key_b64: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct BootstrapSelfIdentityReceipt {
    ack: bool,
    replaced_prior: bool,
}

#[derive(Default)]
struct BootstrapIdentityState {
    keys_by_ura: HashMap<String, Vec<VerifyingKey>>,
    keys_by_node: HashMap<String, Vec<VerifyingKey>>,
    tenant_by_node: HashMap<String, String>,
    owner_by_node: HashMap<String, String>,
}

/// Product-owned key state installed by `runtime.bootstrap_self_identity`.
///
/// The provider is shared by the ability handler and
/// [`CanonicalAdmissionKeyResolver`](crate::daemon::identity::local_invocation::CanonicalAdmissionKeyResolver).
/// The runtime therefore verifies subsequent signatures through its ordinary
/// descriptor-bound admission path; the handler does not create an admission
/// bypass.
#[derive(Default)]
pub(crate) struct RuntimeBootstrapIdentityProvider {
    state: RwLock<BootstrapIdentityState>,
}

impl RuntimeBootstrapIdentityProvider {
    fn bootstrap(
        &self,
        args: BootstrapSelfIdentityArgs,
    ) -> Result<BootstrapSelfIdentityReceipt, AxonError> {
        validate_non_empty(&args.tenant_id, "tenant_id")?;
        validate_non_empty(&args.node_id, "node_id")?;
        validate_non_empty(&args.owner_id, "owner_id")?;
        validate_non_empty(&args.public_key_b64, "public_key_b64")?;
        let _ = args.display_name;

        let key = decode_public_key_b64(&args.public_key_b64)?;
        let mut state = self
            .state
            .write()
            .map_err(|_| AxonError::internal("bootstrap_identity_lock_poisoned"))?;
        if let Some(existing_tenant) = state.tenant_by_node.get(&args.node_id) {
            if existing_tenant != &args.tenant_id {
                return Err(AxonError::invalid_argument(format!(
                    "node_id_already_bootstrapped_for_tenant:{existing_tenant}"
                )));
            }
        }
        if let Some(existing_owner) = state.owner_by_node.get(&args.node_id) {
            if existing_owner != &args.owner_id {
                return Err(AxonError::invalid_argument(format!(
                    "node_id_already_bootstrapped_for_owner:{existing_owner}"
                )));
            }
        }

        let replaced_prior = state.keys_by_node.contains_key(&args.node_id);
        let node_keys = state.keys_by_node.entry(args.node_id.clone()).or_default();
        if !node_keys.contains(&key) {
            if node_keys.len() >= MAX_BOOTSTRAP_KEYS_PER_NODE {
                return Err(AxonError::invalid_argument(format!(
                    "node_id_bootstrap_key_limit:{MAX_BOOTSTRAP_KEYS_PER_NODE}"
                )));
            }
            node_keys.push(key);
        }
        if !replaced_prior {
            state
                .tenant_by_node
                .insert(args.node_id.clone(), args.tenant_id.clone());
            state
                .owner_by_node
                .insert(args.node_id.clone(), args.owner_id.clone());
        }
        for ura in bootstrap_aliases(&args.tenant_id, &args.node_id, &args.owner_id) {
            insert_unique_key(state.keys_by_ura.entry(ura).or_default(), key);
        }

        Ok(BootstrapSelfIdentityReceipt {
            ack: true,
            replaced_prior,
        })
    }

    pub(crate) fn keys_for(&self, agent_ura: &str) -> Result<Option<Vec<VerifyingKey>>, AxonError> {
        Ok(self
            .state
            .read()
            .map_err(|_| AxonError::internal("bootstrap_identity_lock_poisoned"))?
            .keys_by_ura
            .get(agent_ura)
            .cloned()
            .filter(|keys| !keys.is_empty()))
    }
}

impl KeyResolver for RuntimeBootstrapIdentityProvider {
    fn resolve(&self, agent_ura: &str) -> Result<VerifyingKey, AxonError> {
        self.resolve_all(agent_ura)?
            .into_iter()
            .next()
            .ok_or_else(|| unknown_bootstrap_key(agent_ura))
    }

    fn resolve_all(&self, agent_ura: &str) -> Result<Vec<VerifyingKey>, AxonError> {
        self.keys_for(agent_ura)?
            .ok_or_else(|| unknown_bootstrap_key(agent_ura))
    }
}

#[derive(Default)]
struct ProvisionalBootstrapState {
    candidates: HashMap<String, (u64, VerifyingKey)>,
}

/// Bounded, request-scoped key source for the first signed federation join.
///
/// A candidate can be installed only when its public-key digest is exactly the
/// provisional caller URA. The returned lease removes it after canonical Axon
/// admission has verified the descriptor-bound caller signature.
#[derive(Default)]
pub(crate) struct ProvisionalBootstrapKeyProvider {
    next_lease_id: AtomicU64,
    state: Mutex<ProvisionalBootstrapState>,
}

impl ProvisionalBootstrapKeyProvider {
    pub(crate) fn lease_candidate(
        self: &Arc<Self>,
        caller_ura: &str,
        key: VerifyingKey,
    ) -> Result<ProvisionalBootstrapKeyLease, AxonError> {
        let expected = crate::core::ura::provisional::provisional_ura_for_pubkey(&key.to_bytes());
        if caller_ura != expected {
            return Err(AxonError::permission_denied(
                "provisional_bootstrap_key_caller_digest_mismatch",
            ));
        }
        let lease_id = self.next_lease_id.fetch_add(1, Ordering::Relaxed);
        let mut state = self
            .state
            .lock()
            .map_err(|_| AxonError::internal("provisional_bootstrap_key_lock_poisoned"))?;
        if let Some((_, existing)) = state.candidates.get(caller_ura) {
            if existing != &key {
                return Err(AxonError::permission_denied(
                    "provisional_bootstrap_key_conflict",
                ));
            }
        }
        state
            .candidates
            .insert(caller_ura.to_string(), (lease_id, key));
        Ok(ProvisionalBootstrapKeyLease {
            caller_ura: caller_ura.to_string(),
            lease_id,
            provider: Arc::clone(self),
        })
    }

    pub(crate) fn key_for(&self, agent_ura: &str) -> Result<Option<VerifyingKey>, AxonError> {
        Ok(self
            .state
            .lock()
            .map_err(|_| AxonError::internal("provisional_bootstrap_key_lock_poisoned"))?
            .candidates
            .get(agent_ura)
            .map(|(_, key)| *key))
    }
}

impl KeyResolver for ProvisionalBootstrapKeyProvider {
    fn resolve(&self, agent_ura: &str) -> Result<VerifyingKey, AxonError> {
        self.key_for(agent_ura)?
            .ok_or_else(|| unknown_bootstrap_key(agent_ura))
    }
}

/// Lifetime capability for one provisional caller key.
pub(crate) struct ProvisionalBootstrapKeyLease {
    caller_ura: String,
    lease_id: u64,
    provider: Arc<ProvisionalBootstrapKeyProvider>,
}

impl std::fmt::Debug for ProvisionalBootstrapKeyLease {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProvisionalBootstrapKeyLease")
            .field("caller_ura", &self.caller_ura)
            .field("lease_id", &self.lease_id)
            .finish_non_exhaustive()
    }
}

impl Drop for ProvisionalBootstrapKeyLease {
    fn drop(&mut self) {
        let Ok(mut state) = self.provider.state.lock() else {
            return;
        };
        if state
            .candidates
            .get(&self.caller_ura)
            .is_some_and(|(lease_id, _)| *lease_id == self.lease_id)
        {
            state.candidates.remove(&self.caller_ura);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuntimeAdminRegistration {
    pub runtime_key: String,
}

/// Register the CLI-owned bootstrap handler directly under its governed
/// descriptor reference. No SDK product installer or bare-name rename path is
/// involved.
pub(crate) async fn register_runtime_bootstrap_identity_ability(
    runtime: &Arc<LocalRuntime>,
    catalog: &AxonAbilityCatalog,
    hub_ura: &str,
    provider: Arc<RuntimeBootstrapIdentityProvider>,
) -> anyhow::Result<RuntimeAdminRegistration> {
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
    runtime
        .register_ability_with_options(
            runtime_key.clone(),
            make_ability(move |context| {
                let provider = Arc::clone(&provider);
                async move {
                    let args =
                        serde_json::from_slice::<BootstrapSelfIdentityArgs>(&context.payload)
                            .map_err(|error| {
                                AxonError::invalid_argument(format!(
                                    "runtime_bootstrap_self_identity_args:{error}"
                                ))
                            })?;
                    let receipt = provider.bootstrap(args)?;
                    serde_json::to_vec(&receipt).map_err(|error| {
                        AxonError::internal(format!(
                            "runtime_bootstrap_self_identity_receipt:{error}"
                        ))
                    })
                }
            }),
            options,
        )
        .await
        .map_err(|error| {
            anyhow::anyhow!("register descriptor-bound runtime admin `{runtime_key}`: {error}")
        })?;

    Ok(RuntimeAdminRegistration { runtime_key })
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

fn validate_non_empty(value: &str, field: &str) -> Result<(), AxonError> {
    if value.trim().is_empty() {
        Err(AxonError::invalid_argument(format!("{field}_empty")))
    } else {
        Ok(())
    }
}

fn decode_public_key_b64(public_key_b64: &str) -> Result<VerifyingKey, AxonError> {
    let bytes = BASE64_STANDARD
        .decode(public_key_b64.as_bytes())
        .map_err(|error| AxonError::invalid_argument(format!("public_key_b64_base64:{error}")))?;
    let bytes: [u8; 32] = bytes.as_slice().try_into().map_err(|_| {
        AxonError::invalid_argument(format!(
            "ed25519_public_key_wrong_length:expected_32_got_{}",
            bytes.len()
        ))
    })?;
    VerifyingKey::from_bytes(&bytes)
        .map_err(|error| AxonError::invalid_argument(format!("public_key_parse_failed:{error}")))
}

fn bootstrap_aliases(tenant_id: &str, node_id: &str, owner_id: &str) -> [String; 2] {
    [
        crate::core::ura::device_ura(tenant_id, node_id),
        crate::core::ura::agent_ura(tenant_id, owner_id, node_id),
    ]
}

fn insert_unique_key(keys: &mut Vec<VerifyingKey>, key: VerifyingKey) {
    if !keys.contains(&key) {
        keys.push(key);
    }
}

fn unknown_bootstrap_key(agent_ura: &str) -> AxonError {
    AxonError::permission_denied(format!("bootstrap_identity_key_not_found:{agent_ura}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;

    fn args_for(
        tenant_id: &str,
        node_id: &str,
        owner_id: &str,
        signing_key: &SigningKey,
    ) -> BootstrapSelfIdentityArgs {
        BootstrapSelfIdentityArgs {
            tenant_id: tenant_id.to_string(),
            node_id: node_id.to_string(),
            owner_id: owner_id.to_string(),
            display_name: String::new(),
            public_key_b64: BASE64_STANDARD.encode(signing_key.verifying_key().to_bytes()),
        }
    }

    #[test]
    fn bootstrap_provider_is_bounded_and_partitions_aliases() {
        let provider = RuntimeBootstrapIdentityProvider::default();
        let first = SigningKey::from_bytes(&[1; 32]);
        let second = SigningKey::from_bytes(&[2; 32]);
        assert!(
            !provider
                .bootstrap(args_for("realm", "node", "owner", &first))
                .unwrap()
                .replaced_prior
        );
        assert!(
            provider
                .bootstrap(args_for("realm", "node", "owner", &second))
                .unwrap()
                .replaced_prior
        );
        assert_eq!(
            provider
                .resolve_all(&crate::core::ura::device_ura("realm", "node"))
                .unwrap(),
            vec![first.verifying_key(), second.verifying_key()]
        );
        assert!(provider
            .resolve(&crate::core::ura::hub_ura("realm"))
            .is_err());
    }

    #[test]
    fn provisional_key_lease_requires_digest_binding_and_removes_on_drop() {
        let provider = Arc::new(ProvisionalBootstrapKeyProvider::default());
        let key = SigningKey::from_bytes(&[7; 32]).verifying_key();
        let caller = crate::core::ura::provisional::provisional_ura_for_pubkey(&key.to_bytes());
        let lease = provider.lease_candidate(&caller, key).unwrap();
        assert_eq!(provider.resolve(&caller).unwrap(), key);
        drop(lease);
        assert!(provider.resolve(&caller).is_err());
    }
}
