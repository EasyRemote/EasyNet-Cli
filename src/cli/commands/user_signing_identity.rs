//! Product lifecycle for a local user's canonical runtime signing identity.
//!
//! Authentication establishes who the user is. Runtime startup establishes
//! that the paired owner has one daemon-custodied, subject-bound signing key
//! whose public projection is admitted by the local runtime. Keeping both
//! transitions here prevents auth commands and boot orchestration from
//! independently assembling different identity states.

use anyhow::{anyhow, Context};

use crate::daemon::identity::self_identity::{
    ensure_user_runtime_signing_identity, KeyringClient, USER_SIGNING_CLI_PURPOSE,
};
use crate::daemon::keyring::{ManagedSigningKeyProjection, ManagedSigningStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UserSigningIdentityState {
    ExistingTrusted,
    ExistingRegistered,
    CreatedRegistered,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UserSigningIdentityOutcome {
    pub state: UserSigningIdentityState,
    pub key_id: String,
    pub public_key_b64: String,
}

trait UserSigningKeyInventory {
    fn ensure(&self, user_ura: &str) -> anyhow::Result<(ManagedSigningKeyProjection, bool)>;
}

trait UserPublicKeyRegistry {
    fn contains(&self, user_ura: &str, public_key_b64: &str) -> anyhow::Result<bool>;
    fn register(&self, user_ura: &str, public_key_b64: &str) -> anyhow::Result<()>;
}

struct KeyringUserSigningKeyInventory {
    client: KeyringClient,
}

impl Default for KeyringUserSigningKeyInventory {
    fn default() -> Self {
        Self {
            client: KeyringClient::default_path(),
        }
    }
}

impl UserSigningKeyInventory for KeyringUserSigningKeyInventory {
    fn ensure(&self, user_ura: &str) -> anyhow::Result<(ManagedSigningKeyProjection, bool)> {
        let ensured = ensure_user_runtime_signing_identity(&self.client, user_ura)?;
        Ok((ensured.projection, ensured.created))
    }
}

struct LocalUserPublicKeyRegistry;

impl UserPublicKeyRegistry for LocalUserPublicKeyRegistry {
    fn contains(&self, user_ura: &str, public_key_b64: &str) -> anyhow::Result<bool> {
        let response = crate::support::platform::local_invoke::LocalRuntimeIdentityReadIssuer::list_user_pubkeys(
            serde_json::json!({ "user_ura": user_ura }),
        )
        .context("invoke identity.list_user_pubkeys")?;
        let keys = response
            .get("keys")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| anyhow!("identity.list_user_pubkeys returned no keys array"))?;
        Ok(keys.iter().any(|key| {
            key.get("public_key_b64")
                .and_then(serde_json::Value::as_str)
                == Some(public_key_b64)
        }))
    }

    fn register(&self, user_ura: &str, public_key_b64: &str) -> anyhow::Result<()> {
        let local_device_ura = crate::daemon::identity::local_invocation::local_device_ura()?;
        let register_subject =
            crate::core::ura::owner_ability_ura(&local_device_ura, "identity.register_pubkey")
                .ok_or_else(|| anyhow!("derive identity.register_pubkey descriptor subject"))?;
        crate::support::platform::local_invoke::LocalDaemonSystemAbilityIssuer::invoke_root_for_subject(
            "identity.register_pubkey",
            serde_json::json!({
                "principal_ura": user_ura,
                "public_key_b64": public_key_b64,
                "role": "user",
            }),
            &register_subject,
        )
        .map(|_| ())
        .context("invoke identity.register_pubkey")
    }
}

struct UserSigningIdentityReconciler<'a, I, R> {
    inventory: &'a I,
    registry: &'a R,
}

impl<'a, I, R> UserSigningIdentityReconciler<'a, I, R>
where
    I: UserSigningKeyInventory,
    R: UserPublicKeyRegistry,
{
    fn new(inventory: &'a I, registry: &'a R) -> Self {
        Self {
            inventory,
            registry,
        }
    }

    fn reconcile(&self, user_ura: &str) -> anyhow::Result<UserSigningIdentityOutcome> {
        let user_ura = user_ura.trim();
        if user_ura.is_empty() {
            return Err(anyhow!(
                "user_ura is required for signing identity reconciliation"
            ));
        }

        let (key, created) = self.inventory.ensure(user_ura)?;
        validate_managed_user_key(&key, user_ura)?;

        if self.registry.contains(user_ura, &key.public_key_b64)? {
            return Ok(UserSigningIdentityOutcome {
                state: UserSigningIdentityState::ExistingTrusted,
                key_id: key.key_id,
                public_key_b64: key.public_key_b64,
            });
        }

        if let Err(register_error) = self.registry.register(user_ura, &key.public_key_b64) {
            if !self
                .registry
                .contains(user_ura, &key.public_key_b64)
                .unwrap_or(false)
            {
                return Err(register_error);
            }
        }

        Ok(UserSigningIdentityOutcome {
            state: if created {
                UserSigningIdentityState::CreatedRegistered
            } else {
                UserSigningIdentityState::ExistingRegistered
            },
            key_id: key.key_id,
            public_key_b64: key.public_key_b64,
        })
    }
}

fn validate_managed_user_key(
    key: &ManagedSigningKeyProjection,
    user_ura: &str,
) -> anyhow::Result<()> {
    if key.purpose != USER_SIGNING_CLI_PURPOSE
        || key.status != ManagedSigningStatus::Active
        || key.bound_subject.as_deref() != Some(user_ura)
        || key.key_id.trim().is_empty()
        || key.public_key_b64.trim().is_empty()
    {
        return Err(anyhow!(
            "key service returned a signing key outside the requested active subject binding"
        ));
    }
    Ok(())
}

pub(crate) fn reconcile_local_user_signing_identity(
    user_ura: &str,
) -> anyhow::Result<UserSigningIdentityOutcome> {
    let inventory = KeyringUserSigningKeyInventory::default();
    let registry = LocalUserPublicKeyRegistry;
    UserSigningIdentityReconciler::new(&inventory, &registry).reconcile(user_ura)
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use base64::{engine::general_purpose::STANDARD as B64, Engine as _};

    use super::*;

    struct FakeInventory {
        ensured: ManagedSigningKeyProjection,
        created: bool,
        ensure_calls: Cell<usize>,
    }

    impl UserSigningKeyInventory for FakeInventory {
        fn ensure(&self, _user_ura: &str) -> anyhow::Result<(ManagedSigningKeyProjection, bool)> {
            self.ensure_calls.set(self.ensure_calls.get() + 1);
            Ok((self.ensured.clone(), self.created))
        }
    }

    struct FakeRegistry {
        trusted: Cell<bool>,
        register_calls: Cell<usize>,
        registrations: RefCell<Vec<(String, String)>>,
        fail_register: bool,
    }

    impl UserPublicKeyRegistry for FakeRegistry {
        fn contains(&self, _user_ura: &str, _public_key_b64: &str) -> anyhow::Result<bool> {
            Ok(self.trusted.get())
        }

        fn register(&self, user_ura: &str, public_key_b64: &str) -> anyhow::Result<()> {
            self.register_calls.set(self.register_calls.get() + 1);
            self.registrations
                .borrow_mut()
                .push((user_ura.to_string(), public_key_b64.to_string()));
            if self.fail_register {
                return Err(anyhow!("register failed"));
            }
            self.trusted.set(true);
            Ok(())
        }
    }

    fn managed_key(key_id: &str, subject: &str, byte: u8) -> ManagedSigningKeyProjection {
        ManagedSigningKeyProjection {
            key_id: key_id.into(),
            purpose: USER_SIGNING_CLI_PURPOSE.into(),
            public_key_b64: B64.encode([byte; 32]),
            status: ManagedSigningStatus::Active,
            rotation_epoch: 0,
            bound_subject: Some(subject.into()),
            signer_policy_ref: Some("runtime.user-signing.v1".into()),
            rotated_from: None,
            created_unix_ms: 1,
            expires_unix_ms: None,
            revoked_unix_ms: None,
        }
    }

    fn fake_registry(trusted: bool) -> FakeRegistry {
        FakeRegistry {
            trusted: Cell::new(trusted),
            register_calls: Cell::new(0),
            registrations: RefCell::new(Vec::new()),
            fail_register: false,
        }
    }

    #[test]
    fn reuses_existing_trusted_identity_without_mutation() {
        let user = "easynet:///r/local/user/alice";
        let existing = managed_key("existing", user, 0x11);
        let inventory = FakeInventory {
            ensured: existing.clone(),
            created: false,
            ensure_calls: Cell::new(0),
        };
        let registry = fake_registry(true);

        let outcome = UserSigningIdentityReconciler::new(&inventory, &registry)
            .reconcile(user)
            .unwrap();

        assert_eq!(outcome.state, UserSigningIdentityState::ExistingTrusted);
        assert_eq!(outcome.key_id, existing.key_id);
        assert_eq!(inventory.ensure_calls.get(), 1);
        assert_eq!(registry.register_calls.get(), 0);
    }

    #[test]
    fn creates_and_registers_missing_identity() {
        let user = "easynet:///r/local/user/alice";
        let created = managed_key("created", user, 0x22);
        let inventory = FakeInventory {
            ensured: created.clone(),
            created: true,
            ensure_calls: Cell::new(0),
        };
        let registry = fake_registry(false);

        let outcome = UserSigningIdentityReconciler::new(&inventory, &registry)
            .reconcile(user)
            .unwrap();

        assert_eq!(outcome.state, UserSigningIdentityState::CreatedRegistered);
        assert_eq!(outcome.public_key_b64, created.public_key_b64);
        assert_eq!(inventory.ensure_calls.get(), 1);
        assert_eq!(registry.register_calls.get(), 1);
    }

    #[test]
    fn reports_existing_registration_when_inventory_reused_key() {
        let user = "easynet:///r/local/user/alice";
        let existing = managed_key("existing", user, 0x44);
        let inventory = FakeInventory {
            ensured: existing.clone(),
            created: false,
            ensure_calls: Cell::new(0),
        };
        let registry = fake_registry(false);

        let outcome = UserSigningIdentityReconciler::new(&inventory, &registry)
            .reconcile(user)
            .unwrap();

        assert_eq!(outcome.state, UserSigningIdentityState::ExistingRegistered);
        assert_eq!(outcome.key_id, existing.key_id);
        assert_eq!(inventory.ensure_calls.get(), 1);
    }

    #[test]
    fn accepts_concurrent_registration_only_after_observing_trust() {
        struct ConcurrentRegistry {
            trusted: Cell<bool>,
        }
        impl UserPublicKeyRegistry for ConcurrentRegistry {
            fn contains(&self, _user_ura: &str, _public_key_b64: &str) -> anyhow::Result<bool> {
                Ok(self.trusted.get())
            }

            fn register(&self, _user_ura: &str, _public_key_b64: &str) -> anyhow::Result<()> {
                self.trusted.set(true);
                Err(anyhow!("conflict"))
            }
        }

        let user = "easynet:///r/local/user/alice";
        let inventory = FakeInventory {
            ensured: managed_key("existing", user, 0x11),
            created: false,
            ensure_calls: Cell::new(0),
        };
        let registry = ConcurrentRegistry {
            trusted: Cell::new(false),
        };

        let outcome = UserSigningIdentityReconciler::new(&inventory, &registry)
            .reconcile(user)
            .unwrap();

        assert_eq!(outcome.state, UserSigningIdentityState::ExistingRegistered);
    }
}
