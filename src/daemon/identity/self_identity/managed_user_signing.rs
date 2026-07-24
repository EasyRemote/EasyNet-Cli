use std::sync::Arc;

use ed25519_dalek::{Signature, VerifyingKey};

use super::{
    decode_public_key, validate_canonical_signing_bytes, CanonicalSigner, KeyringClient,
    SelfIdentityError,
};
use crate::daemon::keyring::{
    managed_signer_policy_ref, ManagedSigningKeyProjection, ManagedSigningStatus,
};

pub const USER_SIGNING_CLI_PURPOSE: &str = "user_signing.cli";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnsuredUserRuntimeSigningIdentity {
    pub projection: ManagedSigningKeyProjection,
    pub created: bool,
}

/// Ensure the paired user has one active daemon-custodied signing key.
///
/// Device and Hub runtime owners are provisioned through `KeyringClient::ensure`.
/// User callers are a 1:N domain under DEC-EU, so they must be represented as
/// managed, subject-bound signing keys instead of singleton owner keys. Daemon
/// boot calls this before publishing Invocation readiness; user-as-caller
/// descriptor resolution must never depend on a CLI post-start compatibility
/// repair.
pub fn ensure_user_runtime_signing_identity(
    client: &KeyringClient,
    user_ura: &str,
) -> Result<EnsuredUserRuntimeSigningIdentity, SelfIdentityError> {
    let user_ura = user_ura.trim();
    let (projection, created) = match active_user_runtime_signing_identity(client, user_ura)? {
        Some(existing) => (existing, false),
        None => (
            client.inventory_create(USER_SIGNING_CLI_PURPOSE, Some(user_ura.to_string()))?,
            true,
        ),
    };
    validate_user_runtime_signing_projection(&projection, user_ura)?;
    Ok(EnsuredUserRuntimeSigningIdentity {
        projection,
        created,
    })
}

fn active_user_runtime_signing_identity(
    client: &KeyringClient,
    user_ura: &str,
) -> Result<Option<ManagedSigningKeyProjection>, SelfIdentityError> {
    let user_ura = user_ura.trim();
    validate_user_ura(user_ura)?;
    let mut matching = client
        .inventory_list(
            Some(USER_SIGNING_CLI_PURPOSE.to_string()),
            Some(ManagedSigningStatus::Active),
        )?
        .into_iter()
        .filter(|entry| entry.bound_subject.as_deref() == Some(user_ura))
        .collect::<Vec<_>>();
    matching.sort_by(|left, right| left.key_id.cmp(&right.key_id));
    match matching.into_iter().next() {
        Some(projection) => {
            validate_user_runtime_signing_projection(&projection, user_ura)?;
            Ok(Some(projection))
        }
        None => Ok(None),
    }
}

fn validate_user_ura(user_ura: &str) -> Result<(), SelfIdentityError> {
    let identity =
        crate::core::identity::RuntimeIdentityUra::parse(user_ura).map_err(
            |error| match error {
                crate::core::identity::RuntimeIdentityUraError::Empty => {
                    SelfIdentityError::InvalidOwner
                }
                error => SelfIdentityError::Rejected {
                    kind: "invalid_argument".into(),
                    message: format!("managed user signing identity User URA is invalid: {error}"),
                },
            },
        )?;
    if identity.kind() != crate::core::ura::URAKind::User {
        return Err(SelfIdentityError::Rejected {
            kind: "invalid_argument".into(),
            message: format!(
                "managed user signing identity requires a User URA, got `{}`",
                identity.as_str()
            ),
        });
    }
    Ok(())
}

fn validate_user_runtime_signing_projection(
    projection: &ManagedSigningKeyProjection,
    user_ura: &str,
) -> Result<(), SelfIdentityError> {
    if projection.purpose != USER_SIGNING_CLI_PURPOSE {
        return Err(SelfIdentityError::Rejected {
            kind: "policy".into(),
            message: "managed user signing key has the wrong purpose".into(),
        });
    }
    if projection.status != ManagedSigningStatus::Active {
        return Err(SelfIdentityError::Rejected {
            kind: "policy".into(),
            message: "managed user signing key is not active".into(),
        });
    }
    if projection.bound_subject.as_deref() != Some(user_ura) {
        return Err(SelfIdentityError::Rejected {
            kind: "policy".into(),
            message: "managed user signing key is not bound to the requested User URA".into(),
        });
    }
    if projection.key_id.trim().is_empty() {
        return Err(SelfIdentityError::Rejected {
            kind: "policy".into(),
            message: "managed user signing key has no key id".into(),
        });
    }
    decode_public_key(projection.public_key_b64.clone())?;
    let expected_policy_ref = managed_signer_policy_ref(
        &projection.purpose,
        user_ura,
        &projection.key_id,
        &projection.public_key_b64,
    );
    if projection.signer_policy_ref.as_deref() != Some(expected_policy_ref.as_str()) {
        return Err(SelfIdentityError::Rejected {
            kind: "policy".into(),
            message: "managed user signing key has a non-canonical signer policy reference".into(),
        });
    }
    Ok(())
}

#[derive(Clone)]
pub(super) struct ManagedRuntimeSigningIdentity {
    owner_ura: Arc<str>,
    projection: ManagedSigningKeyProjection,
    public_key: VerifyingKey,
    provider: Arc<KeyringClient>,
}

impl ManagedRuntimeSigningIdentity {
    pub(super) fn load_user(
        owner_ura: impl Into<String>,
        provider: Arc<KeyringClient>,
    ) -> Result<Self, SelfIdentityError> {
        let owner_ura = owner_ura.into();
        let owner_ura = owner_ura.trim();
        if owner_ura.is_empty() {
            return Err(SelfIdentityError::InvalidOwner);
        }
        let projection =
            active_user_runtime_signing_identity(&provider, owner_ura)?.ok_or_else(|| {
                SelfIdentityError::Rejected {
                    kind: "not_found".into(),
                    message: format!("managed user signing key not found for `{owner_ura}`"),
                }
            })?;
        let public_key = decode_public_key(projection.public_key_b64.clone())?;
        Ok(Self {
            owner_ura: Arc::from(owner_ura.to_string()),
            projection,
            public_key,
            provider,
        })
    }
}

#[async_trait::async_trait]
impl CanonicalSigner for ManagedRuntimeSigningIdentity {
    fn owner_ura(&self) -> &str {
        &self.owner_ura
    }

    async fn sign_canonical(&self, canonical_bytes: &[u8]) -> Result<Signature, SelfIdentityError> {
        validate_canonical_signing_bytes(canonical_bytes)?;
        let provider = Arc::clone(&self.provider);
        let projection = self.projection.clone();
        let canonical_bytes = canonical_bytes.to_vec();
        tokio::task::spawn_blocking(move || {
            provider.inventory_sign_bound(&projection, &canonical_bytes)
        })
        .await
        .map_err(|error| {
            SelfIdentityError::Transport(format!(
                "managed key-service signing worker terminated unexpectedly: {error}"
            ))
        })?
    }

    fn signing_public_key(&self) -> Result<VerifyingKey, SelfIdentityError> {
        Ok(self.public_key)
    }
}
