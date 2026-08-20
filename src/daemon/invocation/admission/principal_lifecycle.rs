//! Daemon-owned product-neutral PrincipalLifecycle provider.
//!
//! This module owns the durable runtime principal aggregate required by
//! `docs/spec/daemon-sdk-requirements-v1.md` §14. It intentionally does not
//! model Backend accounts, OAuth sessions, EasyRemote workflows or private key
//! custody. Key admission facts are projected into the existing runtime trust
//! anchor; private material remains in the daemon key-service.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use base64::prelude::*;
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};
use tonic::Status;

use crate::daemon::invocation::admission::runtime_trust::{
    now_unix_ms, RuntimeTrust, RuntimeTrustContext,
};
use crate::daemon::persistence::file_lock::ExclusiveFileLock;
use crate::daemon::trust::anchor::TrustAnchorRole;

pub const ABILITY_PRINCIPAL_CREATE: &str =
    crate::daemon::ability::principal_routes_gen::ABILITY_PRINCIPAL_CREATE;
pub const ABILITY_PRINCIPAL_BIND_FIRST_KEY: &str =
    crate::daemon::ability::principal_routes_gen::ABILITY_PRINCIPAL_BIND_FIRST_KEY;
pub const ABILITY_PRINCIPAL_ADD_KEY: &str =
    crate::daemon::ability::principal_routes_gen::ABILITY_PRINCIPAL_ADD_KEY;
pub const ABILITY_PRINCIPAL_ROTATE_KEY: &str =
    crate::daemon::ability::principal_routes_gen::ABILITY_PRINCIPAL_ROTATE_KEY;
pub const ABILITY_PRINCIPAL_REVOKE_KEY: &str =
    crate::daemon::ability::principal_routes_gen::ABILITY_PRINCIPAL_REVOKE_KEY;
pub const ABILITY_PRINCIPAL_CONFIGURE_RECOVERY: &str =
    crate::daemon::ability::principal_routes_gen::ABILITY_PRINCIPAL_CONFIGURE_RECOVERY;
pub const ABILITY_PRINCIPAL_RECOVER: &str =
    crate::daemon::ability::principal_routes_gen::ABILITY_PRINCIPAL_RECOVER;
pub const ABILITY_PRINCIPAL_SUSPEND: &str =
    crate::daemon::ability::principal_routes_gen::ABILITY_PRINCIPAL_SUSPEND;
pub const ABILITY_PRINCIPAL_REACTIVATE: &str =
    crate::daemon::ability::principal_routes_gen::ABILITY_PRINCIPAL_REACTIVATE;
pub const ABILITY_PRINCIPAL_DELETE: &str =
    crate::daemon::ability::principal_routes_gen::ABILITY_PRINCIPAL_DELETE;
pub const ABILITY_PRINCIPAL_ISSUE_ENROLLMENT: &str =
    crate::daemon::ability::principal_routes_gen::ABILITY_PRINCIPAL_ISSUE_ENROLLMENT;
pub const ABILITY_PRINCIPAL_REVOKE_ENROLLMENT: &str =
    crate::daemon::ability::principal_routes_gen::ABILITY_PRINCIPAL_REVOKE_ENROLLMENT;
pub const ABILITY_PRINCIPAL_ISSUE_GRANT: &str =
    crate::daemon::ability::principal_routes_gen::ABILITY_PRINCIPAL_ISSUE_GRANT;
pub const ABILITY_PRINCIPAL_REVOKE_GRANT: &str =
    crate::daemon::ability::principal_routes_gen::ABILITY_PRINCIPAL_REVOKE_GRANT;
pub const ABILITY_PRINCIPAL_GET: &str =
    crate::daemon::ability::principal_routes_gen::ABILITY_PRINCIPAL_GET;

pub(crate) fn principal_lifecycle_store_path_for_trust_anchor(trust_anchor_path: &Path) -> PathBuf {
    trust_anchor_path.with_file_name("principal-lifecycle.json")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PrincipalAdmissionState {
    Missing,
    Pending,
    Active,
    Suspended,
    Deleted,
}

#[derive(Debug, Clone)]
pub(crate) struct PrincipalLifecycleReader {
    store_path: PathBuf,
}

impl PrincipalLifecycleReader {
    pub(crate) fn new(store_path: impl Into<PathBuf>) -> Self {
        Self {
            store_path: store_path.into(),
        }
    }

    pub(crate) fn admission_state(
        &self,
        principal_ura: &str,
    ) -> Result<PrincipalAdmissionState, Status> {
        let store = PrincipalStore::load(&self.store_path, "principal.lifecycle.admission_state")?;
        Ok(store
            .principals
            .get(principal_ura)
            .map(|record| match &record.state {
                PrincipalState::Pending => PrincipalAdmissionState::Pending,
                PrincipalState::Active => PrincipalAdmissionState::Active,
                PrincipalState::Suspended => PrincipalAdmissionState::Suspended,
                PrincipalState::Deleted => PrincipalAdmissionState::Deleted,
            })
            .unwrap_or(PrincipalAdmissionState::Missing))
    }

    pub(crate) fn active_public_keys_b64(
        &self,
        principal_ura: &str,
        presented_pubkey_b64: Option<&str>,
    ) -> Result<Vec<String>, Status> {
        let store =
            PrincipalStore::load(&self.store_path, "principal.lifecycle.active_public_keys")?;
        let Some(record) = store.principals.get(principal_ura) else {
            return Ok(Vec::new());
        };
        if record.state != PrincipalState::Active {
            return Ok(Vec::new());
        }
        let presented = presented_pubkey_b64
            .map(str::trim)
            .filter(|value| !value.is_empty());
        Ok(record
            .bindings
            .iter()
            .filter(|binding| binding.state == KeyBindingState::Active)
            .filter(|binding| {
                binding
                    .expires_unix_ms
                    .is_none_or(|expires| expires > now_unix_ms() as i64)
            })
            .map(|binding| binding.public_key_b64.trim())
            .filter(|public_key_b64| !public_key_b64.is_empty())
            .filter(|public_key_b64| presented.is_none_or(|presented| presented == *public_key_b64))
            .map(str::to_string)
            .collect())
    }

    pub(crate) fn verify_join_enrollment_proof(
        &self,
        principal_ura: &str,
        proof_kind: &str,
        proof_reference: &str,
    ) -> Result<(), Status> {
        let ability = "federation.join";
        let store = PrincipalStore::load(&self.store_path, ability)?;
        if let Some(principal) = store.principals.get(principal_ura) {
            if principal.state != PrincipalState::Active {
                return Err(Status::permission_denied(format!(
                    "{ability}: principal_ura `{principal_ura}` is {} and cannot bind a joining device",
                    principal_state_label(&principal.state)
                )));
            }
        }
        let command = PrincipalCommand {
            actor_ura: principal_ura.trim().to_string(),
            idempotency_key: "federation.join".to_string(),
            expected_version: None,
            proof: PrincipalProofRef {
                kind: proof_kind.trim().to_string(),
                reference: proof_reference.trim().to_string(),
            },
        };
        validate_command(ability, &command)?;
        PrincipalProofVerifier::new(&store).verify_any(
            ability,
            principal_ura,
            &command,
            &[
                PrincipalProofRequirement::EnrollmentCapability,
                PrincipalProofRequirement::ActiveKeyBinding,
                PrincipalProofRequirement::AuthorizationGrant,
                PrincipalProofRequirement::RecoveryPolicy,
            ],
        )
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PrincipalLifecycleContext {
    runtime_trust: RuntimeTrustContext,
    store_path: PathBuf,
}

impl PrincipalLifecycleContext {
    pub(crate) fn from_runtime_trust(runtime_trust: RuntimeTrustContext) -> Self {
        let store_path =
            principal_lifecycle_store_path_for_trust_anchor(&runtime_trust.trust_anchor_path);
        Self {
            runtime_trust,
            store_path,
        }
    }

    #[cfg(test)]
    fn new_for_test(runtime_trust: RuntimeTrustContext, store_path: PathBuf) -> Self {
        Self {
            runtime_trust,
            store_path,
        }
    }

    pub(crate) fn handle(&self, ability: &str, arguments: &[u8]) -> Result<Vec<u8>, Status> {
        PrincipalLifecycle::new(self).handle(ability, arguments)
    }

    pub(crate) fn reader(&self) -> PrincipalLifecycleReader {
        PrincipalLifecycleReader::new(self.store_path.clone())
    }
}

struct PrincipalLifecycle<'a> {
    ctx: &'a PrincipalLifecycleContext,
}

impl<'a> PrincipalLifecycle<'a> {
    fn new(ctx: &'a PrincipalLifecycleContext) -> Self {
        Self { ctx }
    }

    fn handle(&self, ability: &str, arguments: &[u8]) -> Result<Vec<u8>, Status> {
        let ability = canonical_lifecycle_ability(ability)?;
        if ability == ABILITY_PRINCIPAL_GET {
            let args: GetArgs = decode_args(ability, arguments)?;
            let store = PrincipalStore::load(&self.ctx.store_path, ability)?;
            let principal = store
                .principals
                .get(args.principal_ura.trim())
                .ok_or_else(|| {
                    Status::not_found(format!(
                        "{ability}: principal_ura `{}` is not registered",
                        args.principal_ura
                    ))
                })?;
            return encode_snapshot(ability, principal);
        }

        let args: RequestEnvelope = decode_args(ability, arguments)?;
        let mut request = args.request;
        request.normalize();
        validate_principal_ura(
            ability,
            &request.principal_ura,
            &self.ctx.runtime_trust.daemon_realm,
        )?;
        validate_command(ability, &request.command)?;

        let _store_guard = ExclusiveFileLock::acquire_for_data_path(&self.ctx.store_path)
            .map_err(|err| Status::internal(format!("{ability}: lock lifecycle store: {err}")))?;
        let mut store = PrincipalStore::load_unlocked(&self.ctx.store_path, ability)?;

        let snapshot = match ability {
            ABILITY_PRINCIPAL_CREATE => self.create(&mut store, &request)?,
            ABILITY_PRINCIPAL_BIND_FIRST_KEY => {
                self.bind_first_key(&mut store, request.into_bind_key()?)?
            }
            ABILITY_PRINCIPAL_ADD_KEY => self.add_key(&mut store, request.into_bind_key()?)?,
            ABILITY_PRINCIPAL_ROTATE_KEY => {
                self.rotate_key(&mut store, request.into_rotate_key()?)?
            }
            ABILITY_PRINCIPAL_REVOKE_KEY => {
                self.revoke_key(&mut store, request.into_revoke_key()?)?
            }
            ABILITY_PRINCIPAL_CONFIGURE_RECOVERY => {
                self.configure_recovery(&mut store, request.into_recovery_config()?)?
            }
            ABILITY_PRINCIPAL_RECOVER => self.recover(&mut store, request.into_recover()?)?,
            ABILITY_PRINCIPAL_SUSPEND => {
                self.change_state(&mut store, &request, PrincipalState::Suspended)?
            }
            ABILITY_PRINCIPAL_REACTIVATE => {
                self.change_state(&mut store, &request, PrincipalState::Active)?
            }
            ABILITY_PRINCIPAL_DELETE => {
                self.change_state(&mut store, &request, PrincipalState::Deleted)?
            }
            ABILITY_PRINCIPAL_ISSUE_ENROLLMENT => {
                self.issue_enrollment(&mut store, request.into_issue_enrollment()?)?
            }
            ABILITY_PRINCIPAL_REVOKE_ENROLLMENT => {
                self.revoke_enrollment(&mut store, request.into_revoke_enrollment()?)?
            }
            ABILITY_PRINCIPAL_ISSUE_GRANT => {
                self.issue_grant(&mut store, request.into_issue_grant()?)?
            }
            ABILITY_PRINCIPAL_REVOKE_GRANT => {
                self.revoke_grant(&mut store, request.into_revoke_grant()?)?
            }
            _ => {
                return Err(Status::unimplemented(format!(
                    "{ability}: principal lifecycle ability is not registered"
                )))
            }
        };

        PrincipalStore::save_unlocked(&self.ctx.store_path, &store, ability)?;
        encode_snapshot(ability, &snapshot)
    }

    fn create(
        &self,
        store: &mut PrincipalStore,
        request: &PrincipalRequest,
    ) -> Result<PrincipalRecord, Status> {
        if let Some(existing) = store.principals.get(&request.principal_ura) {
            return Ok(existing.clone());
        }
        let is_first = store.principals.is_empty();
        let verifier = PrincipalProofVerifier::new(store);
        if is_first {
            verifier.verify_any(
                ABILITY_PRINCIPAL_CREATE,
                &request.principal_ura,
                &request.command,
                &[PrincipalProofRequirement::Bootstrap],
            )?;
        } else {
            verifier.verify_any(
                ABILITY_PRINCIPAL_CREATE,
                &request.principal_ura,
                &request.command,
                &[
                    PrincipalProofRequirement::AuthorizationGrant,
                    PrincipalProofRequirement::EnrollmentCapability,
                ],
            )?;
        }
        let now = now_unix_ms() as i64;
        let mut principal = PrincipalRecord {
            principal_ura: request.principal_ura.clone(),
            state: PrincipalState::Pending,
            version: 1,
            bindings: Vec::new(),
            enrollment_proof: Some(request.command.proof.clone()),
            recovery: None,
            consumed_recovery_proofs: BTreeMap::new(),
            enrollments: Vec::new(),
            grants: Vec::new(),
            created_unix_ms: now,
            updated_unix_ms: now,
            command_log: BTreeMap::new(),
        };
        principal.record_command(&request.command)?;
        if !is_first && request.command.proof.kind == "enrollment" {
            consume_enrollment_capability(
                store,
                ABILITY_PRINCIPAL_CREATE,
                &request.command.proof.reference,
                &request.principal_ura,
                now,
            )?;
        }
        store
            .principals
            .insert(principal.principal_ura.clone(), principal.clone());
        Ok(principal)
    }

    fn bind_first_key(
        &self,
        store: &mut PrincipalStore,
        request: BindKeyRequest,
    ) -> Result<PrincipalRecord, Status> {
        PrincipalProofVerifier::new(store).verify_any(
            ABILITY_PRINCIPAL_BIND_FIRST_KEY,
            &request.principal_ura,
            &request.command,
            &[PrincipalProofRequirement::MatchingEnrollment],
        )?;
        let principal = principal_mut(
            store,
            ABILITY_PRINCIPAL_BIND_FIRST_KEY,
            &request.principal_ura,
        )?;
        if principal.state != PrincipalState::Pending {
            return Err(Status::failed_precondition(
                "principal.lifecycle.bind_first_key: principal must be pending",
            ));
        }
        if principal
            .bindings
            .iter()
            .any(|binding| binding.state == KeyBindingState::Active)
        {
            return Err(Status::failed_precondition(
                "principal.lifecycle.bind_first_key: active key already exists",
            ));
        }
        self.append_active_key(principal, &request, ABILITY_PRINCIPAL_BIND_FIRST_KEY)?;
        principal.state = PrincipalState::Active;
        principal.bump(&request.command)?;
        self.register_key(&request.principal_ura, &request.public_key_b64)?;
        Ok(principal.clone())
    }

    fn add_key(
        &self,
        store: &mut PrincipalStore,
        request: BindKeyRequest,
    ) -> Result<PrincipalRecord, Status> {
        PrincipalProofVerifier::new(store).verify_any(
            ABILITY_PRINCIPAL_ADD_KEY,
            &request.principal_ura,
            &request.command,
            &[
                PrincipalProofRequirement::ActiveKeyBinding,
                PrincipalProofRequirement::AuthorizationGrant,
                PrincipalProofRequirement::RecoveryPolicy,
            ],
        )?;
        let principal =
            active_principal_mut(store, ABILITY_PRINCIPAL_ADD_KEY, &request.principal_ura)?;
        self.append_active_key(principal, &request, ABILITY_PRINCIPAL_ADD_KEY)?;
        principal.bump(&request.command)?;
        self.register_key(&request.principal_ura, &request.public_key_b64)?;
        Ok(principal.clone())
    }

    fn rotate_key(
        &self,
        store: &mut PrincipalStore,
        request: RotateKeyRequest,
    ) -> Result<PrincipalRecord, Status> {
        PrincipalProofVerifier::new(store).verify_any(
            ABILITY_PRINCIPAL_ROTATE_KEY,
            &request.principal_ura,
            &request.command,
            &[
                PrincipalProofRequirement::ActiveKeyBinding,
                PrincipalProofRequirement::AuthorizationGrant,
            ],
        )?;
        let principal =
            active_principal_mut(store, ABILITY_PRINCIPAL_ROTATE_KEY, &request.principal_ura)?;
        let now = now_unix_ms() as i64;
        let old_public_key = {
            let binding =
                active_binding_mut(principal, ABILITY_PRINCIPAL_ROTATE_KEY, &request.binding_id)?;
            binding.state = KeyBindingState::Rotated;
            binding.rotated_unix_ms = Some(now);
            binding.public_key_b64.clone()
        };
        let new_binding = self.append_active_key(
            principal,
            &request.replacement,
            ABILITY_PRINCIPAL_ROTATE_KEY,
        )?;
        let rotated_to = new_binding.binding_id.clone();
        if let Some(binding) = principal
            .bindings
            .iter_mut()
            .find(|binding| binding.binding_id == request.binding_id)
        {
            binding.rotated_to = Some(rotated_to);
        }
        principal.bump(&request.command)?;
        self.revoke_key_from_trust(&request.principal_ura, &old_public_key)?;
        self.register_key(&request.principal_ura, &request.replacement.public_key_b64)?;
        Ok(principal.clone())
    }

    fn revoke_key(
        &self,
        store: &mut PrincipalStore,
        request: RevokeKeyRequest,
    ) -> Result<PrincipalRecord, Status> {
        PrincipalProofVerifier::new(store).verify_any(
            ABILITY_PRINCIPAL_REVOKE_KEY,
            &request.principal_ura,
            &request.command,
            &[
                PrincipalProofRequirement::ActiveKeyBinding,
                PrincipalProofRequirement::AuthorizationGrant,
            ],
        )?;
        let principal =
            active_principal_mut(store, ABILITY_PRINCIPAL_REVOKE_KEY, &request.principal_ura)?;
        let now = now_unix_ms() as i64;
        let public_key = {
            let binding =
                active_binding_mut(principal, ABILITY_PRINCIPAL_REVOKE_KEY, &request.binding_id)?;
            binding.state = KeyBindingState::Revoked;
            binding.revoked_unix_ms = Some(now);
            binding.public_key_b64.clone()
        };
        principal.bump(&request.command)?;
        self.revoke_key_from_trust(&request.principal_ura, &public_key)?;
        Ok(principal.clone())
    }

    fn configure_recovery(
        &self,
        store: &mut PrincipalStore,
        request: RecoveryConfigRequest,
    ) -> Result<PrincipalRecord, Status> {
        PrincipalProofVerifier::new(store).verify_any(
            ABILITY_PRINCIPAL_CONFIGURE_RECOVERY,
            &request.principal_ura,
            &request.command,
            &[
                PrincipalProofRequirement::ActiveKeyBinding,
                PrincipalProofRequirement::AuthorizationGrant,
            ],
        )?;
        let principal = active_or_suspended_principal_mut(
            store,
            ABILITY_PRINCIPAL_CONFIGURE_RECOVERY,
            &request.principal_ura,
        )?;
        principal.recovery = Some(RecoveryPolicy {
            policy_ref: request.policy_ref,
            enabled: true,
            updated_unix_ms: now_unix_ms() as i64,
        });
        principal.bump(&request.command)?;
        Ok(principal.clone())
    }

    fn issue_enrollment(
        &self,
        store: &mut PrincipalStore,
        request: IssueEnrollmentRequest,
    ) -> Result<PrincipalRecord, Status> {
        PrincipalProofVerifier::new(store).verify_any(
            ABILITY_PRINCIPAL_ISSUE_ENROLLMENT,
            &request.principal_ura,
            &request.command,
            &[
                PrincipalProofRequirement::ActiveKeyBinding,
                PrincipalProofRequirement::AuthorizationGrant,
            ],
        )?;
        validate_principal_ura(
            ABILITY_PRINCIPAL_ISSUE_ENROLLMENT,
            &request.subject_principal_ura,
            &self.ctx.runtime_trust.daemon_realm,
        )?;
        if request.subject_principal_ura == request.principal_ura {
            return Err(Status::invalid_argument(
                "principal.lifecycle.issue_enrollment: subject_principal_ura must name a different principal",
            ));
        }
        let now = now_unix_ms() as i64;
        if request
            .expires_unix_ms
            .is_some_and(|expires| expires <= now)
        {
            return Err(Status::invalid_argument(
                "principal.lifecycle.issue_enrollment: expires_unix_ms must be in the future",
            ));
        }
        let enrollment_id = enrollment_id(&request.principal_ura, &request.command.idempotency_key);
        let principal = active_principal_mut(
            store,
            ABILITY_PRINCIPAL_ISSUE_ENROLLMENT,
            &request.principal_ura,
        )?;
        if principal
            .enrollments
            .iter()
            .any(|enrollment| enrollment.enrollment_id == enrollment_id)
        {
            return Err(Status::already_exists(format!(
                "principal.lifecycle.issue_enrollment: enrollment_id `{enrollment_id}` already exists"
            )));
        }
        principal.enrollments.push(EnrollmentCapability {
            enrollment_id,
            issuer_ura: request.principal_ura.clone(),
            subject_principal_ura: request.subject_principal_ura,
            created_unix_ms: now,
            expires_unix_ms: request.expires_unix_ms,
            revoked_unix_ms: None,
            consumed_by_principal_ura: None,
            consumed_unix_ms: None,
        });
        principal.bump(&request.command)?;
        Ok(principal.clone())
    }

    fn revoke_enrollment(
        &self,
        store: &mut PrincipalStore,
        request: RevokeEnrollmentRequest,
    ) -> Result<PrincipalRecord, Status> {
        PrincipalProofVerifier::new(store).verify_any(
            ABILITY_PRINCIPAL_REVOKE_ENROLLMENT,
            &request.principal_ura,
            &request.command,
            &[
                PrincipalProofRequirement::ActiveKeyBinding,
                PrincipalProofRequirement::AuthorizationGrant,
            ],
        )?;
        let principal = active_or_suspended_principal_mut(
            store,
            ABILITY_PRINCIPAL_REVOKE_ENROLLMENT,
            &request.principal_ura,
        )?;
        let enrollment = principal
            .enrollments
            .iter_mut()
            .find(|enrollment| enrollment.enrollment_id == request.enrollment_id)
            .ok_or_else(|| {
                Status::not_found(format!(
                    "principal.lifecycle.revoke_enrollment: enrollment_id `{}` is not registered",
                    request.enrollment_id
                ))
            })?;
        if enrollment.revoked_unix_ms.is_none() {
            enrollment.revoked_unix_ms = Some(now_unix_ms() as i64);
        }
        principal.bump(&request.command)?;
        Ok(principal.clone())
    }

    fn recover(
        &self,
        store: &mut PrincipalStore,
        request: RecoverRequest,
    ) -> Result<PrincipalRecord, Status> {
        PrincipalProofVerifier::new(store).verify_any(
            ABILITY_PRINCIPAL_RECOVER,
            &request.principal_ura,
            &request.command,
            &[PrincipalProofRequirement::RecoveryPolicy],
        )?;
        let principal = active_or_suspended_principal_mut(
            store,
            ABILITY_PRINCIPAL_RECOVER,
            &request.principal_ura,
        )?;
        if !principal
            .recovery
            .as_ref()
            .is_some_and(|policy| policy.enabled)
        {
            return Err(Status::failed_precondition(
                "principal.lifecycle.recover: no enabled recovery policy",
            ));
        }
        self.append_active_key(
            principal,
            &request.replacement_key,
            ABILITY_PRINCIPAL_RECOVER,
        )?;
        principal.consume_recovery_proof(
            ABILITY_PRINCIPAL_RECOVER,
            &request.principal_ura,
            &request.command.proof.reference,
        )?;
        principal.state = PrincipalState::Active;
        principal.bump(&request.command)?;
        self.register_key(
            &request.principal_ura,
            &request.replacement_key.public_key_b64,
        )?;
        Ok(principal.clone())
    }

    fn change_state(
        &self,
        store: &mut PrincipalStore,
        request: &PrincipalRequest,
        next: PrincipalState,
    ) -> Result<PrincipalRecord, Status> {
        let ability = match next {
            PrincipalState::Suspended => ABILITY_PRINCIPAL_SUSPEND,
            PrincipalState::Active => ABILITY_PRINCIPAL_REACTIVATE,
            PrincipalState::Deleted => ABILITY_PRINCIPAL_DELETE,
            PrincipalState::Pending => unreachable!("pending is not a public change-state target"),
        };
        let current_state = principal_ref(store, ability, &request.principal_ura)?
            .state
            .clone();
        if current_state == PrincipalState::Deleted {
            return Err(Status::failed_precondition(format!(
                "{ability}: deleted principal is terminal"
            )));
        }
        match next {
            PrincipalState::Suspended => {
                if current_state != PrincipalState::Active {
                    return Err(Status::failed_precondition(
                        "principal.lifecycle.suspend: principal must be active",
                    ));
                }
                PrincipalProofVerifier::new(store).verify_any(
                    ability,
                    &request.principal_ura,
                    &request.command,
                    &[
                        PrincipalProofRequirement::ActiveKeyBinding,
                        PrincipalProofRequirement::AuthorizationGrant,
                    ],
                )?;
            }
            PrincipalState::Active => {
                if current_state != PrincipalState::Suspended {
                    return Err(Status::failed_precondition(
                        "principal.lifecycle.reactivate: principal must be suspended",
                    ));
                }
                PrincipalProofVerifier::new(store).verify_any(
                    ability,
                    &request.principal_ura,
                    &request.command,
                    &[
                        PrincipalProofRequirement::AuthorizationGrant,
                        PrincipalProofRequirement::RecoveryPolicy,
                    ],
                )?;
            }
            PrincipalState::Deleted => {
                PrincipalProofVerifier::new(store).verify_any(
                    ability,
                    &request.principal_ura,
                    &request.command,
                    &[PrincipalProofRequirement::AuthorizationGrant],
                )?;
            }
            PrincipalState::Pending => {}
        }
        let principal = principal_mut(store, ability, &request.principal_ura)?;
        principal.state = next;
        principal.bump(&request.command)?;
        Ok(principal.clone())
    }

    fn issue_grant(
        &self,
        store: &mut PrincipalStore,
        request: IssueGrantRequest,
    ) -> Result<PrincipalRecord, Status> {
        PrincipalProofVerifier::new(store).verify_any(
            ABILITY_PRINCIPAL_ISSUE_GRANT,
            &request.principal_ura,
            &request.command,
            &[
                PrincipalProofRequirement::ActiveKeyBinding,
                PrincipalProofRequirement::AuthorizationGrant,
            ],
        )?;
        let principal =
            active_principal_mut(store, ABILITY_PRINCIPAL_ISSUE_GRANT, &request.principal_ura)?;
        let now = now_unix_ms() as i64;
        principal.grants.push(AuthorizationGrant {
            grant_id: grant_id(&request.principal_ura, &request.command.idempotency_key),
            principal_ura: request.principal_ura,
            issuer_ura: request.command.actor_ura.clone(),
            actions: request.actions,
            created_unix_ms: now,
            expires_unix_ms: request.expires_unix_ms,
            revoked_unix_ms: None,
        });
        principal.bump(&request.command)?;
        Ok(principal.clone())
    }

    fn revoke_grant(
        &self,
        store: &mut PrincipalStore,
        request: RevokeGrantRequest,
    ) -> Result<PrincipalRecord, Status> {
        PrincipalProofVerifier::new(store).verify_any(
            ABILITY_PRINCIPAL_REVOKE_GRANT,
            &request.principal_ura,
            &request.command,
            &[
                PrincipalProofRequirement::ActiveKeyBinding,
                PrincipalProofRequirement::AuthorizationGrant,
            ],
        )?;
        let principal = active_or_suspended_principal_mut(
            store,
            ABILITY_PRINCIPAL_REVOKE_GRANT,
            &request.principal_ura,
        )?;
        let grant = principal
            .grants
            .iter_mut()
            .find(|grant| grant.grant_id == request.grant_id)
            .ok_or_else(|| {
                Status::not_found(format!(
                    "principal.lifecycle.revoke_grant: grant_id `{}` is not registered",
                    request.grant_id
                ))
            })?;
        if grant.revoked_unix_ms.is_none() {
            grant.revoked_unix_ms = Some(now_unix_ms() as i64);
        }
        principal.bump(&request.command)?;
        Ok(principal.clone())
    }

    fn append_active_key(
        &self,
        principal: &mut PrincipalRecord,
        request: &BindKeyRequest,
        ability: &'static str,
    ) -> Result<PublicKeyBinding, Status> {
        validate_public_key_b64(ability, &request.public_key_b64)?;
        if principal.bindings.iter().any(|binding| {
            binding.public_key_b64 == request.public_key_b64
                && binding.state == KeyBindingState::Active
        }) {
            return Err(Status::already_exists(format!(
                "{ability}: principal already has this active public key"
            )));
        }
        let now = now_unix_ms() as i64;
        let binding = PublicKeyBinding {
            binding_id: binding_id(&request.principal_ura, &request.public_key_b64),
            principal_ura: request.principal_ura.clone(),
            key_id: request.key_id.clone(),
            public_key_b64: request.public_key_b64.clone(),
            state: KeyBindingState::Active,
            created_unix_ms: now,
            expires_unix_ms: request.expires_unix_ms,
            rotated_unix_ms: None,
            revoked_unix_ms: None,
            rotated_to: None,
        };
        principal.bindings.push(binding.clone());
        Ok(binding)
    }

    fn register_key(&self, principal_ura: &str, public_key_b64: &str) -> Result<(), Status> {
        RuntimeTrust::new(
            &self.ctx.runtime_trust.daemon_realm,
            &self.ctx.runtime_trust.trust_anchor_path,
            &self.ctx.runtime_trust.cell,
        )
        .register_pubkey(
            principal_ura.to_string(),
            public_key_b64.to_string(),
            TrustAnchorRole::User,
        )
    }

    fn revoke_key_from_trust(
        &self,
        principal_ura: &str,
        public_key_b64: &str,
    ) -> Result<(), Status> {
        RuntimeTrust::new(
            &self.ctx.runtime_trust.daemon_realm,
            &self.ctx.runtime_trust.trust_anchor_path,
            &self.ctx.runtime_trust.cell,
        )
        .revoke_user_pubkey(principal_ura, public_key_b64)
        .map(|_| ())
    }
}

fn canonical_lifecycle_ability(ability: &str) -> Result<&'static str, Status> {
    match ability {
        ABILITY_PRINCIPAL_CREATE => Ok(ABILITY_PRINCIPAL_CREATE),
        ABILITY_PRINCIPAL_BIND_FIRST_KEY => Ok(ABILITY_PRINCIPAL_BIND_FIRST_KEY),
        ABILITY_PRINCIPAL_ADD_KEY => Ok(ABILITY_PRINCIPAL_ADD_KEY),
        ABILITY_PRINCIPAL_ROTATE_KEY => Ok(ABILITY_PRINCIPAL_ROTATE_KEY),
        ABILITY_PRINCIPAL_REVOKE_KEY => Ok(ABILITY_PRINCIPAL_REVOKE_KEY),
        ABILITY_PRINCIPAL_CONFIGURE_RECOVERY => Ok(ABILITY_PRINCIPAL_CONFIGURE_RECOVERY),
        ABILITY_PRINCIPAL_RECOVER => Ok(ABILITY_PRINCIPAL_RECOVER),
        ABILITY_PRINCIPAL_SUSPEND => Ok(ABILITY_PRINCIPAL_SUSPEND),
        ABILITY_PRINCIPAL_REACTIVATE => Ok(ABILITY_PRINCIPAL_REACTIVATE),
        ABILITY_PRINCIPAL_DELETE => Ok(ABILITY_PRINCIPAL_DELETE),
        ABILITY_PRINCIPAL_ISSUE_ENROLLMENT => Ok(ABILITY_PRINCIPAL_ISSUE_ENROLLMENT),
        ABILITY_PRINCIPAL_REVOKE_ENROLLMENT => Ok(ABILITY_PRINCIPAL_REVOKE_ENROLLMENT),
        ABILITY_PRINCIPAL_ISSUE_GRANT => Ok(ABILITY_PRINCIPAL_ISSUE_GRANT),
        ABILITY_PRINCIPAL_REVOKE_GRANT => Ok(ABILITY_PRINCIPAL_REVOKE_GRANT),
        ABILITY_PRINCIPAL_GET => Ok(ABILITY_PRINCIPAL_GET),
        _ => Err(Status::unimplemented(format!(
            "{ability}: principal lifecycle ability is not registered"
        ))),
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PrincipalStore {
    principals: BTreeMap<String, PrincipalRecord>,
}

impl PrincipalStore {
    fn load(path: &Path, ability: &'static str) -> Result<Self, Status> {
        let _guard = ExclusiveFileLock::acquire_for_data_path(path)
            .map_err(|err| Status::internal(format!("{ability}: lock lifecycle store: {err}")))?;
        Self::load_unlocked(path, ability)
    }

    fn load_unlocked(path: &Path, ability: &'static str) -> Result<Self, Status> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let bytes = fs::read(path).map_err(|err| {
            Status::internal(format!("{ability}: read {}: {err}", path.display()))
        })?;
        serde_json::from_slice(&bytes)
            .map_err(|err| Status::internal(format!("{ability}: parse {}: {err}", path.display())))
    }

    fn save_unlocked(path: &Path, store: &Self, ability: &'static str) -> Result<(), Status> {
        let parent = path.parent().ok_or_else(|| {
            Status::internal(format!("{ability}: lifecycle store path has no parent"))
        })?;
        fs::create_dir_all(parent).map_err(|err| {
            Status::internal(format!("{ability}: create {}: {err}", parent.display()))
        })?;
        let tmp_path = path.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(store)
            .map_err(|err| Status::internal(format!("{ability}: encode lifecycle store: {err}")))?;
        fs::write(&tmp_path, bytes).map_err(|err| {
            Status::internal(format!("{ability}: write {}: {err}", tmp_path.display()))
        })?;
        fs::rename(&tmp_path, path).map_err(|err| {
            Status::internal(format!(
                "{ability}: rename {} -> {}: {err}",
                tmp_path.display(),
                path.display()
            ))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum PrincipalState {
    Pending,
    Active,
    Suspended,
    Deleted,
}

fn principal_state_label(state: &PrincipalState) -> &'static str {
    match state {
        PrincipalState::Pending => "pending",
        PrincipalState::Active => "active",
        PrincipalState::Suspended => "suspended",
        PrincipalState::Deleted => "deleted",
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PrincipalRecord {
    principal_ura: String,
    state: PrincipalState,
    version: u64,
    bindings: Vec<PublicKeyBinding>,
    #[serde(deserialize_with = "deserialize_required_option")]
    enrollment_proof: Option<PrincipalProofRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    recovery: Option<RecoveryPolicy>,
    consumed_recovery_proofs: BTreeMap<String, i64>,
    enrollments: Vec<EnrollmentCapability>,
    grants: Vec<AuthorizationGrant>,
    created_unix_ms: i64,
    updated_unix_ms: i64,
    command_log: BTreeMap<String, u64>,
}

impl PrincipalRecord {
    fn bump(&mut self, command: &PrincipalCommand) -> Result<(), Status> {
        if let Some(expected) = command.expected_version {
            if expected != self.version {
                return Err(Status::failed_precondition(format!(
                    "principal lifecycle expected_version {expected} does not match current version {}",
                    self.version
                )));
            }
        }
        self.version += 1;
        self.updated_unix_ms = now_unix_ms() as i64;
        self.record_command(command)
    }

    fn record_command(&mut self, command: &PrincipalCommand) -> Result<(), Status> {
        if self
            .command_log
            .insert(command.idempotency_key.clone(), self.version)
            .is_some()
        {
            return Err(Status::already_exists(format!(
                "principal lifecycle idempotency_key `{}` was already used",
                command.idempotency_key
            )));
        }
        Ok(())
    }

    fn consume_recovery_proof(
        &mut self,
        ability: &'static str,
        principal_ura: &str,
        proof_reference: &str,
    ) -> Result<(), Status> {
        let proof_hash = recovery_proof_hash(principal_ura, proof_reference);
        if self
            .consumed_recovery_proofs
            .insert(proof_hash, now_unix_ms() as i64)
            .is_some()
        {
            return Err(Status::permission_denied(format!(
                "{ability}: recovery proof reference has already been consumed"
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PublicKeyBinding {
    binding_id: String,
    principal_ura: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    key_id: String,
    public_key_b64: String,
    state: KeyBindingState,
    created_unix_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    expires_unix_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rotated_unix_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    revoked_unix_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rotated_to: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum KeyBindingState {
    Active,
    Rotated,
    Revoked,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecoveryPolicy {
    policy_ref: String,
    enabled: bool,
    updated_unix_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthorizationGrant {
    grant_id: String,
    principal_ura: String,
    issuer_ura: String,
    actions: Vec<String>,
    created_unix_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    expires_unix_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    revoked_unix_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EnrollmentCapability {
    enrollment_id: String,
    issuer_ura: String,
    subject_principal_ura: String,
    created_unix_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    expires_unix_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    revoked_unix_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    consumed_by_principal_ura: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    consumed_unix_ms: Option<i64>,
}

impl EnrollmentCapability {
    fn is_active_for(&self, principal_ura: &str, now_unix_ms: i64) -> bool {
        self.subject_principal_ura == principal_ura
            && self.revoked_unix_ms.is_none()
            && self.consumed_unix_ms.is_none()
            && self
                .expires_unix_ms
                .is_none_or(|expires| expires > now_unix_ms)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestEnvelope {
    request: PrincipalRequest,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GetArgs {
    principal_ura: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct PrincipalRequest {
    command: PrincipalCommand,
    principal_ura: String,
    #[serde(default)]
    key_id: String,
    public_key_b64: Option<String>,
    #[serde(default)]
    expires_unix_ms: Option<i64>,
    binding_id: Option<String>,
    #[serde(default)]
    replacement: Option<Box<PrincipalRequest>>,
    #[serde(default)]
    replacement_key: Option<Box<PrincipalRequest>>,
    policy_ref: Option<String>,
    actions: Option<Vec<String>>,
    grant_id: Option<String>,
    subject_principal_ura: Option<String>,
    enrollment_id: Option<String>,
}

impl PrincipalRequest {
    fn normalize(&mut self) {
        self.principal_ura = self.principal_ura.trim().to_string();
        self.key_id = self.key_id.trim().to_string();
        trim_optional_text(&mut self.public_key_b64);
        trim_optional_text(&mut self.binding_id);
        trim_optional_text(&mut self.policy_ref);
        trim_optional_text(&mut self.grant_id);
        trim_optional_text(&mut self.subject_principal_ura);
        trim_optional_text(&mut self.enrollment_id);
        if let Some(actions) = self.actions.as_mut() {
            actions
                .iter_mut()
                .for_each(|action| *action = action.trim().to_string());
        }
        self.command.normalize();
        if let Some(replacement) = self.replacement.as_mut() {
            replacement.normalize();
        }
        if let Some(replacement_key) = self.replacement_key.as_mut() {
            replacement_key.normalize();
        }
    }

    fn into_bind_key(self) -> Result<BindKeyRequest, Status> {
        Ok(BindKeyRequest {
            command: self.command,
            principal_ura: self.principal_ura,
            key_id: self.key_id,
            public_key_b64: required_text(
                "principal lifecycle key transition",
                "public_key_b64",
                self.public_key_b64,
            )?,
            expires_unix_ms: self.expires_unix_ms,
        })
    }

    fn into_rotate_key(self) -> Result<RotateKeyRequest, Status> {
        let replacement = self
            .replacement
            .ok_or_else(|| {
                Status::invalid_argument("principal.lifecycle.rotate_key: replacement is required")
            })?
            .into_bind_key()?;
        Ok(RotateKeyRequest {
            command: self.command,
            principal_ura: self.principal_ura,
            binding_id: required_text(
                "principal.lifecycle.rotate_key",
                "binding_id",
                self.binding_id,
            )?,
            replacement,
        })
    }

    fn into_revoke_key(self) -> Result<RevokeKeyRequest, Status> {
        Ok(RevokeKeyRequest {
            command: self.command,
            principal_ura: self.principal_ura,
            binding_id: required_text(
                "principal.lifecycle.revoke_key",
                "binding_id",
                self.binding_id,
            )?,
        })
    }

    fn into_recovery_config(self) -> Result<RecoveryConfigRequest, Status> {
        Ok(RecoveryConfigRequest {
            command: self.command,
            principal_ura: self.principal_ura,
            policy_ref: required_text(
                "principal.lifecycle.configure_recovery",
                "policy_ref",
                self.policy_ref,
            )?,
        })
    }

    fn into_recover(self) -> Result<RecoverRequest, Status> {
        let replacement_key = self
            .replacement_key
            .ok_or_else(|| {
                Status::invalid_argument("principal.lifecycle.recover: replacement_key is required")
            })?
            .into_bind_key()?;
        Ok(RecoverRequest {
            command: self.command,
            principal_ura: self.principal_ura,
            replacement_key,
        })
    }

    fn into_issue_grant(self) -> Result<IssueGrantRequest, Status> {
        Ok(IssueGrantRequest {
            command: self.command,
            principal_ura: self.principal_ura,
            actions: required_non_empty_list(
                "principal.lifecycle.issue_grant",
                "actions",
                self.actions,
            )?,
            expires_unix_ms: self.expires_unix_ms,
        })
    }

    fn into_revoke_grant(self) -> Result<RevokeGrantRequest, Status> {
        Ok(RevokeGrantRequest {
            command: self.command,
            principal_ura: self.principal_ura,
            grant_id: required_text(
                "principal.lifecycle.revoke_grant",
                "grant_id",
                self.grant_id,
            )?,
        })
    }

    fn into_issue_enrollment(self) -> Result<IssueEnrollmentRequest, Status> {
        Ok(IssueEnrollmentRequest {
            command: self.command,
            principal_ura: self.principal_ura,
            subject_principal_ura: required_text(
                "principal.lifecycle.issue_enrollment",
                "subject_principal_ura",
                self.subject_principal_ura,
            )?,
            expires_unix_ms: self.expires_unix_ms,
        })
    }

    fn into_revoke_enrollment(self) -> Result<RevokeEnrollmentRequest, Status> {
        Ok(RevokeEnrollmentRequest {
            command: self.command,
            principal_ura: self.principal_ura,
            enrollment_id: required_text(
                "principal.lifecycle.revoke_enrollment",
                "enrollment_id",
                self.enrollment_id,
            )?,
        })
    }
}

#[derive(Debug, Clone)]
struct BindKeyRequest {
    command: PrincipalCommand,
    principal_ura: String,
    key_id: String,
    public_key_b64: String,
    expires_unix_ms: Option<i64>,
}

#[derive(Debug, Clone)]
struct RotateKeyRequest {
    command: PrincipalCommand,
    principal_ura: String,
    binding_id: String,
    replacement: BindKeyRequest,
}

#[derive(Debug, Clone)]
struct RevokeKeyRequest {
    command: PrincipalCommand,
    principal_ura: String,
    binding_id: String,
}

#[derive(Debug, Clone)]
struct RecoveryConfigRequest {
    command: PrincipalCommand,
    principal_ura: String,
    policy_ref: String,
}

#[derive(Debug, Clone)]
struct RecoverRequest {
    command: PrincipalCommand,
    principal_ura: String,
    replacement_key: BindKeyRequest,
}

#[derive(Debug, Clone)]
struct IssueGrantRequest {
    command: PrincipalCommand,
    principal_ura: String,
    actions: Vec<String>,
    expires_unix_ms: Option<i64>,
}

#[derive(Debug, Clone)]
struct RevokeGrantRequest {
    command: PrincipalCommand,
    principal_ura: String,
    grant_id: String,
}

#[derive(Debug, Clone)]
struct IssueEnrollmentRequest {
    command: PrincipalCommand,
    principal_ura: String,
    subject_principal_ura: String,
    expires_unix_ms: Option<i64>,
}

#[derive(Debug, Clone)]
struct RevokeEnrollmentRequest {
    command: PrincipalCommand,
    principal_ura: String,
    enrollment_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct PrincipalCommand {
    actor_ura: String,
    idempotency_key: String,
    #[serde(default)]
    expected_version: Option<u64>,
    proof: PrincipalProofRef,
}

impl PrincipalCommand {
    fn normalize(&mut self) {
        self.actor_ura = self.actor_ura.trim().to_string();
        self.idempotency_key = self.idempotency_key.trim().to_string();
        self.proof.kind = self.proof.kind.trim().to_string();
        self.proof.reference = self.proof.reference.trim().to_string();
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct PrincipalProofRef {
    kind: String,
    reference: String,
}

fn decode_args<T: for<'de> Deserialize<'de>>(
    ability: &'static str,
    arguments: &[u8],
) -> Result<T, Status> {
    serde_json::from_slice(arguments).map_err(|err| {
        Status::invalid_argument(format!("{ability}: arguments JSON decode failed: {err}"))
    })
}

fn encode_snapshot(ability: &'static str, principal: &PrincipalRecord) -> Result<Vec<u8>, Status> {
    serde_json::to_vec(&serde_json::json!({ "principal": principal }))
        .map_err(|err| Status::internal(format!("{ability}: response JSON encode failed: {err}")))
}

fn trim_optional_text(value: &mut Option<String>) {
    if let Some(raw) = value.as_mut() {
        *raw = raw.trim().to_string();
    }
}

fn deserialize_required_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

fn validate_principal_ura(
    ability: &'static str,
    ura: &str,
    daemon_realm: &str,
) -> Result<(), Status> {
    let parsed = crate::core::ura::parse_ura(ura).map_err(|err| {
        Status::invalid_argument(format!("{ability}: principal_ura must be canonical: {err}"))
    })?;
    if parsed.kind != crate::core::ura::URAKind::User {
        return Err(Status::invalid_argument(format!(
            "{ability}: principal_ura must be a User URA"
        )));
    }
    if parsed.realm != daemon_realm {
        return Err(Status::permission_denied(format!(
            "{ability}: principal realm `{}` must match daemon realm `{daemon_realm}`",
            parsed.realm
        )));
    }
    Ok(())
}

fn validate_command(ability: &'static str, command: &PrincipalCommand) -> Result<(), Status> {
    if command.actor_ura.is_empty() {
        return Err(Status::invalid_argument(format!(
            "{ability}: actor_ura is required"
        )));
    }
    if command.idempotency_key.is_empty() {
        return Err(Status::invalid_argument(format!(
            "{ability}: idempotency_key is required"
        )));
    }
    if command.proof.kind.is_empty() || command.proof.reference.is_empty() {
        return Err(Status::invalid_argument(format!(
            "{ability}: proof kind and reference are required"
        )));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrincipalProofRequirement {
    Bootstrap,
    EnrollmentCapability,
    MatchingEnrollment,
    ActiveKeyBinding,
    AuthorizationGrant,
    RecoveryPolicy,
}

impl PrincipalProofRequirement {
    fn label(self) -> &'static str {
        match self {
            Self::Bootstrap => "bootstrap",
            Self::EnrollmentCapability => "enrollment",
            Self::MatchingEnrollment => "matching_enrollment",
            Self::ActiveKeyBinding => "active_key",
            Self::AuthorizationGrant => "grant",
            Self::RecoveryPolicy => "recovery",
        }
    }
}

struct PrincipalProofVerifier<'a> {
    store: &'a PrincipalStore,
    now_unix_ms: i64,
}

impl<'a> PrincipalProofVerifier<'a> {
    fn new(store: &'a PrincipalStore) -> Self {
        Self {
            store,
            now_unix_ms: now_unix_ms() as i64,
        }
    }

    fn verify_any(
        &self,
        ability: &'static str,
        principal_ura: &str,
        command: &PrincipalCommand,
        accepted: &[PrincipalProofRequirement],
    ) -> Result<(), Status> {
        let mut direct_rejection = None;
        for requirement in accepted {
            match self.verify_one(*requirement, ability, principal_ura, command) {
                Ok(()) => return Ok(()),
                Err(status) if command.proof.kind == requirement.label() => {
                    direct_rejection = Some(status);
                }
                Err(_) => {}
            }
        }
        if let Some(status) = direct_rejection {
            return Err(status);
        }
        let expected = accepted
            .iter()
            .map(|requirement| requirement.label())
            .collect::<Vec<_>>()
            .join(", ");
        Err(Status::permission_denied(format!(
            "{ability}: proof kind/reference `{}`/`{}` is not sufficient; expected one of [{expected}]",
            command.proof.kind, command.proof.reference
        )))
    }

    fn verify_one(
        &self,
        requirement: PrincipalProofRequirement,
        ability: &'static str,
        principal_ura: &str,
        command: &PrincipalCommand,
    ) -> Result<(), Status> {
        match requirement {
            PrincipalProofRequirement::Bootstrap => self.verify_bootstrap(command),
            PrincipalProofRequirement::EnrollmentCapability => {
                self.verify_enrollment_capability(ability, principal_ura, command)
            }
            PrincipalProofRequirement::MatchingEnrollment => {
                self.verify_matching_enrollment(ability, principal_ura, command)
            }
            PrincipalProofRequirement::ActiveKeyBinding => {
                self.verify_active_key_binding(ability, principal_ura, command)
            }
            PrincipalProofRequirement::AuthorizationGrant => {
                self.verify_authorization_grant(command, ability)
            }
            PrincipalProofRequirement::RecoveryPolicy => {
                self.verify_recovery_policy(ability, principal_ura, command)
            }
        }
    }

    fn verify_bootstrap(&self, command: &PrincipalCommand) -> Result<(), Status> {
        if command.proof.kind == "bootstrap" {
            Ok(())
        } else {
            Err(Status::permission_denied("bootstrap proof required"))
        }
    }

    fn verify_enrollment_capability(
        &self,
        ability: &'static str,
        principal_ura: &str,
        command: &PrincipalCommand,
    ) -> Result<(), Status> {
        if command.proof.kind == "enrollment" {
            if self.store.principals.values().any(|issuer| {
                issuer.enrollments.iter().any(|enrollment| {
                    enrollment.enrollment_id == command.proof.reference
                        && enrollment.is_active_for(principal_ura, self.now_unix_ms)
                })
            }) {
                Ok(())
            } else {
                Err(Status::permission_denied(format!(
                    "{ability}: enrollment proof reference `{}` is not active for principal `{principal_ura}`",
                    command.proof.reference
                )))
            }
        } else {
            Err(Status::permission_denied("enrollment proof required"))
        }
    }

    fn verify_matching_enrollment(
        &self,
        ability: &'static str,
        principal_ura: &str,
        command: &PrincipalCommand,
    ) -> Result<(), Status> {
        let principal = principal_ref(self.store, ability, principal_ura)?;
        if principal.enrollment_proof.as_ref() == Some(&command.proof) {
            Ok(())
        } else {
            Err(Status::permission_denied(format!(
                "{ability}: bind_first_key proof must match the principal enrollment proof"
            )))
        }
    }

    fn verify_active_key_binding(
        &self,
        ability: &'static str,
        principal_ura: &str,
        command: &PrincipalCommand,
    ) -> Result<(), Status> {
        if command.proof.kind != "active_key" {
            return Err(Status::permission_denied("active_key proof required"));
        }
        if command.actor_ura != principal_ura {
            return Err(Status::permission_denied(format!(
                "{ability}: active_key proof actor `{}` must match principal `{principal_ura}`",
                command.actor_ura
            )));
        }
        let principal = principal_ref(self.store, ability, principal_ura)?;
        if principal.bindings.iter().any(|binding| {
            binding.binding_id == command.proof.reference
                && binding.state == KeyBindingState::Active
                && binding
                    .expires_unix_ms
                    .is_none_or(|expires| expires > self.now_unix_ms)
        }) {
            Ok(())
        } else {
            Err(Status::permission_denied(format!(
                "{ability}: active_key proof reference `{}` is not an active binding",
                command.proof.reference
            )))
        }
    }

    fn verify_authorization_grant(
        &self,
        command: &PrincipalCommand,
        ability: &'static str,
    ) -> Result<(), Status> {
        if command.proof.kind != "grant" {
            return Err(Status::permission_denied("grant proof required"));
        }
        if self
            .store
            .principals
            .values()
            .flat_map(|principal| principal.grants.iter())
            .any(|grant| {
                grant.grant_id == command.proof.reference
                    && grant.principal_ura == command.actor_ura
                    && grant.revoked_unix_ms.is_none()
                    && grant
                        .expires_unix_ms
                        .is_none_or(|expires| expires > self.now_unix_ms)
                    && grant_authorizes_ability(&grant.actions, ability)
            })
        {
            Ok(())
        } else {
            Err(Status::permission_denied(format!(
                "{ability}: grant proof reference `{}` is not active for actor `{}`",
                command.proof.reference, command.actor_ura
            )))
        }
    }

    fn verify_recovery_policy(
        &self,
        ability: &'static str,
        principal_ura: &str,
        command: &PrincipalCommand,
    ) -> Result<(), Status> {
        if command.proof.kind != "recovery" {
            return Err(Status::permission_denied("recovery proof required"));
        }
        let principal = principal_ref(self.store, ability, principal_ura)?;
        if principal
            .recovery
            .as_ref()
            .is_some_and(|policy| policy.enabled && policy.policy_ref == command.proof.reference)
        {
            if ability == ABILITY_PRINCIPAL_RECOVER
                && principal
                    .consumed_recovery_proofs
                    .contains_key(&recovery_proof_hash(
                        principal_ura,
                        &command.proof.reference,
                    ))
            {
                return Err(Status::permission_denied(format!(
                    "{ability}: recovery proof reference has already been consumed"
                )));
            }
            Ok(())
        } else {
            Err(Status::permission_denied(format!(
                "{ability}: recovery proof reference `{}` does not match an enabled recovery policy",
                command.proof.reference
            )))
        }
    }
}

fn grant_authorizes_ability(actions: &[String], ability: &str) -> bool {
    actions.iter().any(|action| {
        action == "*"
            || action == ability
            || action
                .strip_suffix('*')
                .is_some_and(|prefix| !prefix.is_empty() && ability.starts_with(prefix))
    })
}

fn consume_enrollment_capability(
    store: &mut PrincipalStore,
    ability: &'static str,
    enrollment_id: &str,
    principal_ura: &str,
    now_unix_ms: i64,
) -> Result<(), Status> {
    for issuer in store.principals.values_mut() {
        if let Some(enrollment) = issuer
            .enrollments
            .iter_mut()
            .find(|enrollment| enrollment.enrollment_id == enrollment_id)
        {
            if !enrollment.is_active_for(principal_ura, now_unix_ms) {
                return Err(Status::permission_denied(format!(
                    "{ability}: enrollment proof reference `{enrollment_id}` is not active for principal `{principal_ura}`"
                )));
            }
            enrollment.consumed_by_principal_ura = Some(principal_ura.to_string());
            enrollment.consumed_unix_ms = Some(now_unix_ms);
            issuer.updated_unix_ms = now_unix_ms;
            issuer.version += 1;
            return Ok(());
        }
    }
    Err(Status::permission_denied(format!(
        "{ability}: enrollment proof reference `{enrollment_id}` is not registered"
    )))
}

fn principal_ref<'a>(
    store: &'a PrincipalStore,
    ability: &'static str,
    principal_ura: &str,
) -> Result<&'a PrincipalRecord, Status> {
    store.principals.get(principal_ura).ok_or_else(|| {
        Status::not_found(format!(
            "{ability}: principal_ura `{principal_ura}` is not registered"
        ))
    })
}

fn principal_mut<'a>(
    store: &'a mut PrincipalStore,
    ability: &'static str,
    principal_ura: &str,
) -> Result<&'a mut PrincipalRecord, Status> {
    store.principals.get_mut(principal_ura).ok_or_else(|| {
        Status::not_found(format!(
            "{ability}: principal_ura `{principal_ura}` is not registered"
        ))
    })
}

fn active_principal_mut<'a>(
    store: &'a mut PrincipalStore,
    ability: &'static str,
    principal_ura: &str,
) -> Result<&'a mut PrincipalRecord, Status> {
    let principal = principal_mut(store, ability, principal_ura)?;
    if principal.state != PrincipalState::Active {
        return Err(Status::failed_precondition(format!(
            "{ability}: principal must be active"
        )));
    }
    Ok(principal)
}

fn active_or_suspended_principal_mut<'a>(
    store: &'a mut PrincipalStore,
    ability: &'static str,
    principal_ura: &str,
) -> Result<&'a mut PrincipalRecord, Status> {
    let principal = principal_mut(store, ability, principal_ura)?;
    if !matches!(
        principal.state,
        PrincipalState::Active | PrincipalState::Suspended
    ) {
        return Err(Status::failed_precondition(format!(
            "{ability}: principal must be active or suspended"
        )));
    }
    Ok(principal)
}

fn active_binding_mut<'a>(
    principal: &'a mut PrincipalRecord,
    ability: &'static str,
    binding_id: &str,
) -> Result<&'a mut PublicKeyBinding, Status> {
    let binding = principal
        .bindings
        .iter_mut()
        .find(|binding| binding.binding_id == binding_id)
        .ok_or_else(|| {
            Status::not_found(format!("{ability}: binding_id `{binding_id}` not found"))
        })?;
    if binding.state != KeyBindingState::Active {
        return Err(Status::failed_precondition(format!(
            "{ability}: binding_id `{binding_id}` is not active"
        )));
    }
    Ok(binding)
}

fn validate_public_key_b64(ability: &'static str, public_key_b64: &str) -> Result<(), Status> {
    let decoded = BASE64_STANDARD.decode(public_key_b64).map_err(|err| {
        Status::invalid_argument(format!(
            "{ability}: public_key_b64 is not valid base64: {err}"
        ))
    })?;
    if decoded.len() != 32 {
        return Err(Status::invalid_argument(format!(
            "{ability}: public_key_b64 must decode to exactly 32 bytes, got {}",
            decoded.len()
        )));
    }
    Ok(())
}

fn binding_id(principal_ura: &str, public_key_b64: &str) -> String {
    let digest = Sha256::digest(format!("{principal_ura}\0{public_key_b64}").as_bytes());
    format!("pk_{}", hex::encode(&digest[..16]))
}

fn grant_id(principal_ura: &str, idempotency_key: &str) -> String {
    let digest = Sha256::digest(format!("{principal_ura}\0{idempotency_key}").as_bytes());
    format!("grant_{}", hex::encode(&digest[..16]))
}

fn enrollment_id(principal_ura: &str, idempotency_key: &str) -> String {
    let digest = Sha256::digest(format!("{principal_ura}\0{idempotency_key}").as_bytes());
    format!("enroll_{}", hex::encode(&digest[..16]))
}

fn recovery_proof_hash(principal_ura: &str, proof_reference: &str) -> String {
    let digest = Sha256::digest(format!("{principal_ura}\0{proof_reference}").as_bytes());
    format!("recovery_{}", hex::encode(&digest[..16]))
}

fn required_text(
    ability: &'static str,
    field: &'static str,
    value: Option<String>,
) -> Result<String, Status> {
    match value {
        Some(value) if !value.is_empty() => Ok(value),
        _ => Err(Status::invalid_argument(format!(
            "{ability}: {field} is required"
        ))),
    }
}

fn required_non_empty_list(
    ability: &'static str,
    field: &'static str,
    value: Option<Vec<String>>,
) -> Result<Vec<String>, Status> {
    match value {
        Some(value) if !value.is_empty() && value.iter().all(|item| !item.is_empty()) => Ok(value),
        _ => Err(Status::invalid_argument(format!(
            "{ability}: {field} must contain at least one non-empty value"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::trust::anchor::RealmTrustAnchor;
    use crate::daemon::trust::cell::SharedTrustAnchor;
    use serde_json::json;
    use std::sync::Arc;
    use tempfile::tempdir;

    fn b64_pubkey(seed: u8) -> String {
        let signing = ed25519_dalek::SigningKey::from_bytes(&[seed; 32]);
        BASE64_STANDARD.encode(signing.verifying_key().to_bytes())
    }

    fn context() -> (tempfile::TempDir, PrincipalLifecycleContext) {
        let dir = tempdir().expect("tempdir");
        let runtime_trust = RuntimeTrustContext {
            daemon_realm: "realm".to_string(),
            trust_anchor_path: dir.path().join("realm-trust.toml"),
            cell: SharedTrustAnchor::new(Arc::new(RealmTrustAnchor::default())),
        };
        let store_path = dir.path().join("principal-lifecycle.json");
        (
            dir,
            PrincipalLifecycleContext::new_for_test(runtime_trust, store_path),
        )
    }

    fn canonical_principal_record_json() -> serde_json::Value {
        json!({
            "principal_ura": "easynet:///r/realm/user/alice",
            "state": "active",
            "version": 1,
            "bindings": [{
                "binding_id": "binding-1",
                "principal_ura": "easynet:///r/realm/user/alice",
                "key_id": "key-1",
                "public_key_b64": b64_pubkey(1),
                "state": "active",
                "created_unix_ms": 1
            }],
            "enrollment_proof": {
                "kind": "bootstrap",
                "reference": "proof:create"
            },
            "recovery": {
                "policy_ref": "policy:recovery",
                "enabled": true,
                "updated_unix_ms": 1
            },
            "consumed_recovery_proofs": {},
            "enrollments": [{
                "enrollment_id": "enrollment-1",
                "issuer_ura": "easynet:///r/realm/user/admin",
                "subject_principal_ura": "easynet:///r/realm/user/bob",
                "created_unix_ms": 1
            }],
            "grants": [{
                "grant_id": "grant-1",
                "principal_ura": "easynet:///r/realm/user/alice",
                "issuer_ura": "easynet:///r/realm/user/admin",
                "actions": ["principal.lifecycle.get"],
                "created_unix_ms": 1
            }],
            "created_unix_ms": 1,
            "updated_unix_ms": 1,
            "command_log": {"create": 1}
        })
    }

    #[test]
    fn principal_store_rejects_unknown_top_level_fields() {
        let dir = tempdir().expect("tempdir");
        let store_path = dir.path().join("principal-lifecycle.json");
        fs::write(
            &store_path,
            serde_json::to_vec_pretty(&json!({
                "principals": {},
                "legacy_principals": {}
            }))
            .expect("encode noncanonical store"),
        )
        .expect("write noncanonical store");

        let error = PrincipalStore::load_unlocked(&store_path, ABILITY_PRINCIPAL_GET)
            .expect_err("principal store with unknown top-level fields must fail closed");
        assert_eq!(error.code(), tonic::Code::Internal);
        assert!(
            error
                .message()
                .contains("unknown field `legacy_principals`"),
            "unexpected unknown store field error: {error}"
        );
    }

    #[test]
    fn principal_store_rejects_unknown_nested_lifecycle_fields() {
        let dir = tempdir().expect("tempdir");
        let store_path = dir.path().join("principal-lifecycle.json");
        let mut principal = canonical_principal_record_json();
        principal
            .as_object_mut()
            .expect("principal object")
            .insert("legacy_state".to_string(), json!("active"));
        fs::write(
            &store_path,
            serde_json::to_vec_pretty(&json!({
                "principals": {
                    "easynet:///r/realm/user/alice": principal
                }
            }))
            .expect("encode noncanonical principal store"),
        )
        .expect("write noncanonical principal store");

        let error = PrincipalStore::load_unlocked(&store_path, ABILITY_PRINCIPAL_GET)
            .expect_err("principal record with unknown fields must fail closed");
        assert_eq!(error.code(), tonic::Code::Internal);
        assert!(
            error.message().contains("unknown field `legacy_state`"),
            "unexpected unknown principal field error: {error}"
        );
    }

    #[test]
    fn principal_lifecycle_request_rejects_unknown_fields_before_execution() {
        let (_dir, ctx) = context();
        let error = ctx
            .handle(
                ABILITY_PRINCIPAL_CREATE,
                serde_json::to_vec(&json!({
                    "request": {
                        "command": command("create-1", "bootstrap", None),
                        "principal_ura": "easynet:///r/realm/user/alice",
                        "legacy_subject": "alice"
                    }
                }))
                .unwrap()
                .as_slice(),
            )
            .expect_err("principal request with unknown fields must fail closed");

        assert_eq!(error.code(), tonic::Code::InvalidArgument);
        assert!(
            error.message().contains("unknown field `legacy_subject`"),
            "unexpected unknown request field error: {error}"
        );
        assert!(
            !ctx.store_path.exists(),
            "malformed request must not create the lifecycle store"
        );
    }

    #[test]
    fn principal_lifecycle_request_keeps_absent_branch_facts_absent() {
        let envelope: RequestEnvelope = serde_json::from_value(json!({
            "request": {
                "command": command("create-absent-branch-facts", "bootstrap", None),
                "principal_ura": "easynet:///r/realm/user/alice"
            }
        }))
        .expect("principal request without branch-specific facts parses");

        let request = envelope.request;
        assert!(
            request.public_key_b64.is_none(),
            "missing public_key_b64 must stay absent, not become an empty string sentinel"
        );
        assert!(
            request.binding_id.is_none(),
            "missing binding_id must stay absent, not become an empty string sentinel"
        );
        assert!(
            request.policy_ref.is_none(),
            "missing policy_ref must stay absent, not become an empty string sentinel"
        );
        assert!(
            request.actions.is_none(),
            "missing actions must stay absent, not become an empty collection sentinel"
        );
        assert!(
            request.grant_id.is_none(),
            "missing grant_id must stay absent, not become an empty string sentinel"
        );
        assert!(
            request.subject_principal_ura.is_none(),
            "missing subject_principal_ura must stay absent, not become an empty string sentinel"
        );
        assert!(
            request.enrollment_id.is_none(),
            "missing enrollment_id must stay absent, not become an empty string sentinel"
        );
    }

    #[test]
    fn principal_lifecycle_issue_grant_rejects_empty_action_facts() {
        let envelope: RequestEnvelope = serde_json::from_value(json!({
            "request": {
                "command": command_with_ref("grant-empty-action", "active_key", "binding-1", None),
                "principal_ura": "easynet:///r/realm/user/alice",
                "actions": ["  "]
            }
        }))
        .expect("principal request with blank action parses");

        let mut request = envelope.request;
        request.normalize();
        let error = request
            .into_issue_grant()
            .expect_err("blank action facts must not become authorization facts");
        assert_eq!(error.code(), tonic::Code::InvalidArgument);
        assert!(
            error.message().contains(
                "principal.lifecycle.issue_grant: actions must contain at least one non-empty value"
            ),
            "unexpected error: {}",
            error.message()
        );
    }

    #[test]
    fn principal_record_requires_idempotency_command_log_fact() {
        let dir = tempdir().expect("tempdir");
        let store_path = dir.path().join("principal-lifecycle.json");
        fs::write(
            &store_path,
            serde_json::to_vec_pretty(&json!({
                "principals": {
                    "easynet:///r/realm/user/alice": {
                        "principal_ura": "easynet:///r/realm/user/alice",
                        "state": "active",
                        "version": 1,
                        "bindings": [],
                        "enrollment_proof": {
                            "kind": "bootstrap",
                            "reference": "proof:create"
                        },
                        "consumed_recovery_proofs": {},
                        "enrollments": [],
                        "grants": [],
                        "created_unix_ms": 1,
                        "updated_unix_ms": 1
                    }
                }
            }))
            .expect("encode legacy store"),
        )
        .expect("write legacy store");

        let error = PrincipalStore::load_unlocked(&store_path, ABILITY_PRINCIPAL_GET)
            .expect_err("principal record without command_log must fail closed");
        assert_eq!(error.code(), tonic::Code::Internal);
        assert!(
            error.message().contains("missing field `command_log`"),
            "unexpected missing command_log error: {error}"
        );
    }

    #[test]
    fn principal_record_requires_enrollment_proof_fact() {
        let dir = tempdir().expect("tempdir");
        let store_path = dir.path().join("principal-lifecycle.json");
        fs::write(
            &store_path,
            serde_json::to_vec_pretty(&json!({
                "principals": {
                    "easynet:///r/realm/user/alice": {
                        "principal_ura": "easynet:///r/realm/user/alice",
                        "state": "active",
                        "version": 1,
                        "bindings": [],
                        "consumed_recovery_proofs": {},
                        "enrollments": [],
                        "grants": [],
                        "created_unix_ms": 1,
                        "updated_unix_ms": 1,
                        "command_log": {"create": 1}
                    }
                }
            }))
            .expect("encode legacy store"),
        )
        .expect("write legacy store");

        let error = PrincipalStore::load_unlocked(&store_path, ABILITY_PRINCIPAL_GET)
            .expect_err("principal record without enrollment_proof must fail closed");
        assert_eq!(error.code(), tonic::Code::Internal);
        assert!(
            error.message().contains("missing field `enrollment_proof`"),
            "unexpected missing enrollment_proof error: {error}"
        );
    }

    #[test]
    fn principal_record_requires_lifecycle_collection_facts() {
        for (field, omitted) in [
            ("consumed_recovery_proofs", "consumed_recovery_proofs"),
            ("enrollments", "enrollments"),
            ("grants", "grants"),
        ] {
            let dir = tempdir().expect("tempdir");
            let store_path = dir.path().join(format!("{field}.json"));
            let mut principal = json!({
                "principal_ura": "easynet:///r/realm/user/alice",
                "state": "active",
                "version": 1,
                "bindings": [],
                "enrollment_proof": {
                    "kind": "bootstrap",
                    "reference": "proof:create"
                },
                "consumed_recovery_proofs": {},
                "enrollments": [],
                "grants": [],
                "created_unix_ms": 1,
                "updated_unix_ms": 1,
                "command_log": {"create": 1}
            });
            principal
                .as_object_mut()
                .expect("principal object")
                .remove(omitted);
            fs::write(
                &store_path,
                serde_json::to_vec_pretty(&json!({
                    "principals": {
                        "easynet:///r/realm/user/alice": principal
                    }
                }))
                .expect("encode malformed lifecycle collection store"),
            )
            .expect("write malformed lifecycle collection store");

            let error = PrincipalStore::load_unlocked(&store_path, ABILITY_PRINCIPAL_GET)
                .expect_err("principal record without lifecycle collections must fail closed");
            assert_eq!(error.code(), tonic::Code::Internal);
            assert!(
                error
                    .message()
                    .contains(&format!("missing field `{field}`")),
                "unexpected missing {field} error: {error}"
            );
        }
    }

    #[test]
    fn existing_principal_store_requires_principals_fact() {
        let dir = tempdir().expect("tempdir");
        let store_path = dir.path().join("principal-lifecycle.json");
        fs::write(
            &store_path,
            serde_json::to_vec_pretty(&json!({})).expect("encode malformed store"),
        )
        .expect("write malformed store");

        let error = PrincipalStore::load_unlocked(&store_path, ABILITY_PRINCIPAL_GET)
            .expect_err("existing principal store without principals must fail closed");
        assert_eq!(error.code(), tonic::Code::Internal);
        assert!(
            error.message().contains("missing field `principals`"),
            "unexpected missing principals error: {error}"
        );
    }

    fn command(id: &str, kind: &str, expected_version: Option<u64>) -> serde_json::Value {
        command_with_ref(id, kind, &format!("proof:{id}"), expected_version)
    }

    fn command_with_ref(
        id: &str,
        kind: &str,
        reference: &str,
        expected_version: Option<u64>,
    ) -> serde_json::Value {
        command_for_actor_with_ref(
            "easynet:///r/realm/user/admin",
            id,
            kind,
            reference,
            expected_version,
        )
    }

    fn command_for_actor_with_ref(
        actor_ura: &str,
        id: &str,
        kind: &str,
        reference: &str,
        expected_version: Option<u64>,
    ) -> serde_json::Value {
        let mut value = json!({
            "actor_ura": actor_ura,
            "idempotency_key": id,
            "proof": {"kind": kind, "reference": reference}
        });
        if let Some(version) = expected_version {
            value["expected_version"] = json!(version);
        }
        value
    }

    fn invoke(
        ctx: &PrincipalLifecycleContext,
        ability: &'static str,
        request: serde_json::Value,
    ) -> serde_json::Value {
        let body = ctx
            .handle(
                ability,
                serde_json::to_vec(&json!({"request": request}))
                    .unwrap()
                    .as_slice(),
            )
            .expect("invoke");
        serde_json::from_slice(&body).expect("json")
    }

    #[test]
    fn first_principal_bootstrap_binds_first_key_and_persists() {
        let (_dir, ctx) = context();
        let user = "easynet:///r/realm/user/alice";
        invoke(
            &ctx,
            ABILITY_PRINCIPAL_CREATE,
            json!({
                "command": command("create-1", "bootstrap", None),
                "principal_ura": user
            }),
        );
        let out = invoke(
            &ctx,
            ABILITY_PRINCIPAL_BIND_FIRST_KEY,
            json!({
                "command": command_with_ref("bind-1", "bootstrap", "proof:create-1", Some(1)),
                "principal_ura": user,
                "public_key_b64": b64_pubkey(1)
            }),
        );

        assert_eq!(out["principal"]["state"], "active");
        assert_eq!(out["principal"]["bindings"].as_array().unwrap().len(), 1);
        assert!(ctx
            .runtime_trust
            .cell
            .snapshot()
            .lookup_user_by_pubkey(user, &b64_pubkey(1))
            .is_some());

        let reloaded = PrincipalStore::load(&ctx.store_path, ABILITY_PRINCIPAL_GET).unwrap();
        assert_eq!(
            reloaded.principals.get(user).unwrap().state,
            PrincipalState::Active
        );
    }

    #[test]
    fn lifecycle_reader_resolves_only_active_unexpired_public_keys() {
        let (_dir, ctx) = context();
        let user = "easynet:///r/realm/user/alice";
        invoke(
            &ctx,
            ABILITY_PRINCIPAL_CREATE,
            json!({
                "command": command("create-1", "bootstrap", None),
                "principal_ura": user
            }),
        );
        let out = invoke(
            &ctx,
            ABILITY_PRINCIPAL_BIND_FIRST_KEY,
            json!({
                "command": command_with_ref("bind-1", "bootstrap", "proof:create-1", Some(1)),
                "principal_ura": user,
                "public_key_b64": b64_pubkey(1)
            }),
        );
        let binding_id = out["principal"]["bindings"][0]["binding_id"]
            .as_str()
            .expect("binding id");
        let reader = ctx.reader();

        assert_eq!(
            reader.active_public_keys_b64(user, None).unwrap(),
            vec![b64_pubkey(1)]
        );
        assert_eq!(
            reader
                .active_public_keys_b64(user, Some(&b64_pubkey(1)))
                .unwrap(),
            vec![b64_pubkey(1)]
        );
        assert!(reader
            .active_public_keys_b64(user, Some(&b64_pubkey(2)))
            .unwrap()
            .is_empty());

        invoke(
            &ctx,
            ABILITY_PRINCIPAL_SUSPEND,
            json!({
                "command": command_for_actor_with_ref(
                    user,
                    "suspend-1",
                    "active_key",
                    binding_id,
                    Some(2)
                ),
                "principal_ura": user
            }),
        );

        assert!(reader
            .active_public_keys_b64(user, None)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn additional_user_requires_enrollment_or_grant_proof() {
        let (_dir, ctx) = context();
        let admin = "easynet:///r/realm/user/admin";
        let bob = "easynet:///r/realm/user/bob";
        invoke(
            &ctx,
            ABILITY_PRINCIPAL_CREATE,
            json!({
                "command": command("create-admin", "bootstrap", None),
                "principal_ura": admin
            }),
        );
        let admin_snapshot = invoke(
            &ctx,
            ABILITY_PRINCIPAL_BIND_FIRST_KEY,
            json!({
                "command": command_with_ref("bind-admin", "bootstrap", "proof:create-admin", Some(1)),
                "principal_ura": admin,
                "public_key_b64": b64_pubkey(10)
            }),
        );
        let admin_binding_id = admin_snapshot["principal"]["bindings"][0]["binding_id"]
            .as_str()
            .unwrap();
        let err = ctx
            .handle(
                ABILITY_PRINCIPAL_CREATE,
                serde_json::to_vec(&json!({
                    "request": {
                        "command": command("create-bob", "active_key", None),
                        "principal_ura": bob
                    }
                }))
                .unwrap()
                .as_slice(),
            )
            .expect_err("wrong proof rejected");
        assert_eq!(err.code(), tonic::Code::PermissionDenied);

        let issued = invoke(
            &ctx,
            ABILITY_PRINCIPAL_ISSUE_ENROLLMENT,
            json!({
                "command": command_for_actor_with_ref(admin, "issue-bob", "active_key", admin_binding_id, Some(2)),
                "principal_ura": admin,
                "subject_principal_ura": bob
            }),
        );
        let enrollment_id = issued["principal"]["enrollments"][0]["enrollment_id"]
            .as_str()
            .unwrap();
        invoke(
            &ctx,
            ABILITY_PRINCIPAL_CREATE,
            json!({
                "command": command_for_actor_with_ref(bob, "create-bob-2", "enrollment", enrollment_id, None),
                "principal_ura": bob
            }),
        );
        let bob_snapshot = invoke(
            &ctx,
            ABILITY_PRINCIPAL_BIND_FIRST_KEY,
            json!({
                "command": command_for_actor_with_ref(bob, "bind-bob", "enrollment", enrollment_id, Some(1)),
                "principal_ura": bob,
                "public_key_b64": b64_pubkey(11)
            }),
        );
        assert_eq!(bob_snapshot["principal"]["state"], "active");

        let reloaded = PrincipalStore::load(&ctx.store_path, ABILITY_PRINCIPAL_GET).unwrap();
        let admin_record = reloaded.principals.get(admin).unwrap();
        assert_eq!(
            admin_record.enrollments[0]
                .consumed_by_principal_ura
                .as_deref(),
            Some(bob)
        );

        let err = ctx
            .handle(
                ABILITY_PRINCIPAL_CREATE,
                serde_json::to_vec(&json!({
                    "request": {
                        "command": command_for_actor_with_ref(
                            "easynet:///r/realm/user/carol",
                            "create-carol",
                            "enrollment",
                            enrollment_id,
                            None
                        ),
                        "principal_ura": "easynet:///r/realm/user/carol"
                    }
                }))
                .unwrap()
                .as_slice(),
            )
            .expect_err("consumed enrollment rejected");
        assert_eq!(err.code(), tonic::Code::PermissionDenied);
    }

    #[test]
    fn join_enrollment_reader_admits_active_key_and_rejects_suspended_principal() {
        let (_dir, ctx) = context();
        let user = "easynet:///r/realm/user/alice";
        invoke(
            &ctx,
            ABILITY_PRINCIPAL_CREATE,
            json!({
                "command": command("create", "bootstrap", None),
                "principal_ura": user
            }),
        );
        let first = invoke(
            &ctx,
            ABILITY_PRINCIPAL_BIND_FIRST_KEY,
            json!({
                "command": command_with_ref("bind", "bootstrap", "proof:create", Some(1)),
                "principal_ura": user,
                "public_key_b64": b64_pubkey(1)
            }),
        );
        let binding_id = binding_id_at(&first, 0);

        ctx.reader()
            .verify_join_enrollment_proof(user, "active_key", &binding_id)
            .expect("active user binding can authorize device join binding");

        invoke(
            &ctx,
            ABILITY_PRINCIPAL_SUSPEND,
            json!({
                "command": command_for_actor_with_ref(user, "suspend", "active_key", &binding_id, Some(2)),
                "principal_ura": user
            }),
        );

        let err = ctx
            .reader()
            .verify_join_enrollment_proof(user, "active_key", &binding_id)
            .expect_err("suspended principal cannot bind a joining device");
        assert_eq!(err.code(), tonic::Code::PermissionDenied);
    }

    #[test]
    fn revoke_key_updates_lifecycle_and_trust_without_removing_sibling_key() {
        let (_dir, ctx) = context();
        let user = "easynet:///r/realm/user/alice";
        invoke(
            &ctx,
            ABILITY_PRINCIPAL_CREATE,
            json!({
                "command": command("create", "bootstrap", None),
                "principal_ura": user
            }),
        );
        let first = invoke(
            &ctx,
            ABILITY_PRINCIPAL_BIND_FIRST_KEY,
            json!({
                "command": command_with_ref("bind1", "bootstrap", "proof:create", Some(1)),
                "principal_ura": user,
                "public_key_b64": b64_pubkey(1)
            }),
        );
        let first_binding_id = first["principal"]["bindings"][0]["binding_id"]
            .as_str()
            .unwrap();
        invoke(
            &ctx,
            ABILITY_PRINCIPAL_ADD_KEY,
            json!({
                "command": command_for_actor_with_ref(user, "bind2", "active_key", first_binding_id, Some(2)),
                "principal_ura": user,
                "public_key_b64": b64_pubkey(2)
            }),
        );

        let out = invoke(
            &ctx,
            ABILITY_PRINCIPAL_REVOKE_KEY,
            json!({
                "command": command_for_actor_with_ref(user, "revoke1", "active_key", first_binding_id, Some(3)),
                "principal_ura": user,
                "binding_id": first_binding_id
            }),
        );

        let bindings = out["principal"]["bindings"].as_array().unwrap();
        assert_eq!(
            bindings.iter().filter(|b| b["state"] == "active").count(),
            1
        );
        assert!(ctx
            .runtime_trust
            .cell
            .snapshot()
            .lookup_user_by_pubkey(user, &b64_pubkey(1))
            .is_none());
        assert!(ctx
            .runtime_trust
            .cell
            .snapshot()
            .lookup_user_by_pubkey(user, &b64_pubkey(2))
            .is_some());
    }

    #[test]
    fn active_key_proof_requires_active_binding_reference_and_does_not_mutate() {
        let (_dir, ctx) = context();
        let user = "easynet:///r/realm/user/alice";
        invoke(
            &ctx,
            ABILITY_PRINCIPAL_CREATE,
            json!({
                "command": command("create", "bootstrap", None),
                "principal_ura": user
            }),
        );
        invoke(
            &ctx,
            ABILITY_PRINCIPAL_BIND_FIRST_KEY,
            json!({
                "command": command_with_ref("bind1", "bootstrap", "proof:create", Some(1)),
                "principal_ura": user,
                "public_key_b64": b64_pubkey(1)
            }),
        );

        let err = ctx
            .handle(
                ABILITY_PRINCIPAL_ADD_KEY,
                serde_json::to_vec(&json!({
                    "request": {
                        "command": command_for_actor_with_ref(
                            user,
                            "bind2",
                            "active_key",
                            "missing-binding",
                            Some(2)
                        ),
                        "principal_ura": user,
                        "public_key_b64": b64_pubkey(2)
                    }
                }))
                .unwrap()
                .as_slice(),
            )
            .expect_err("missing active binding must reject");
        assert_eq!(err.code(), tonic::Code::PermissionDenied);
        assert!(ctx
            .runtime_trust
            .cell
            .snapshot()
            .lookup_user_by_pubkey(user, &b64_pubkey(2))
            .is_none());
        let loaded = PrincipalStore::load(&ctx.store_path, ABILITY_PRINCIPAL_GET).unwrap();
        assert_eq!(loaded.principals.get(user).unwrap().bindings.len(), 1);
    }

    #[test]
    fn grant_proof_requires_active_authorizing_grant_and_does_not_mutate() {
        let (_dir, ctx) = context();
        let admin = "easynet:///r/realm/user/admin";
        let bob = "easynet:///r/realm/user/bob";
        invoke(
            &ctx,
            ABILITY_PRINCIPAL_CREATE,
            json!({
                "command": command("create-admin", "bootstrap", None),
                "principal_ura": admin
            }),
        );
        invoke(
            &ctx,
            ABILITY_PRINCIPAL_BIND_FIRST_KEY,
            json!({
                "command": command_with_ref(
                    "bind-admin",
                    "bootstrap",
                    "proof:create-admin",
                    Some(1)
                ),
                "principal_ura": admin,
                "public_key_b64": b64_pubkey(1)
            }),
        );

        let err = ctx
            .handle(
                ABILITY_PRINCIPAL_CREATE,
                serde_json::to_vec(&json!({
                    "request": {
                        "command": command_for_actor_with_ref(
                            admin,
                            "create-bob",
                            "grant",
                            "missing-grant",
                            None
                        ),
                        "principal_ura": bob
                    }
                }))
                .unwrap()
                .as_slice(),
            )
            .expect_err("missing grant must reject");
        assert_eq!(err.code(), tonic::Code::PermissionDenied);
        let loaded = PrincipalStore::load(&ctx.store_path, ABILITY_PRINCIPAL_GET).unwrap();
        assert!(!loaded.principals.contains_key(bob));
    }

    #[test]
    fn expected_version_mismatch_does_not_mutate_store() {
        let (_dir, ctx) = context();
        let user = "easynet:///r/realm/user/alice";
        invoke(
            &ctx,
            ABILITY_PRINCIPAL_CREATE,
            json!({
                "command": command("create", "bootstrap", None),
                "principal_ura": user
            }),
        );
        let err = ctx
            .handle(
                ABILITY_PRINCIPAL_BIND_FIRST_KEY,
                serde_json::to_vec(&json!({
                    "request": {
                        "command": command_with_ref("bind", "bootstrap", "proof:create", Some(99)),
                        "principal_ura": user,
                        "public_key_b64": b64_pubkey(1)
                    }
                }))
                .unwrap()
                .as_slice(),
            )
            .expect_err("version mismatch");
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);

        let loaded = PrincipalStore::load(&ctx.store_path, ABILITY_PRINCIPAL_GET).unwrap();
        let principal = loaded.principals.get(user).unwrap();
        assert_eq!(principal.version, 1);
        assert!(principal.bindings.is_empty());
    }

    #[test]
    fn recovery_adds_key_without_silently_replacing_existing_keys() {
        let (_dir, ctx) = context();
        let user = "easynet:///r/realm/user/alice";
        invoke(
            &ctx,
            ABILITY_PRINCIPAL_CREATE,
            json!({
                "command": command("create", "bootstrap", None),
                "principal_ura": user
            }),
        );
        let first = invoke(
            &ctx,
            ABILITY_PRINCIPAL_BIND_FIRST_KEY,
            json!({
                "command": command_with_ref("bind1", "bootstrap", "proof:create", Some(1)),
                "principal_ura": user,
                "public_key_b64": b64_pubkey(1)
            }),
        );
        let first_binding_id = first["principal"]["bindings"][0]["binding_id"]
            .as_str()
            .unwrap();
        invoke(
            &ctx,
            ABILITY_PRINCIPAL_CONFIGURE_RECOVERY,
            json!({
                "command": command_for_actor_with_ref(user, "recovery-policy", "active_key", first_binding_id, Some(2)),
                "principal_ura": user,
                "policy_ref": "recovery-policy:test"
            }),
        );
        let out = invoke(
            &ctx,
            ABILITY_PRINCIPAL_RECOVER,
            json!({
                "command": command_with_ref("recover", "recovery", "recovery-policy:test", Some(3)),
                "principal_ura": user,
                "replacement_key": {
                    "command": command("ignored-child", "recovery", None),
                    "principal_ura": user,
                    "public_key_b64": b64_pubkey(2)
                }
            }),
        );
        assert_eq!(
            out["principal"]["bindings"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|binding| binding["state"] == "active")
                .count(),
            2
        );
    }

    #[test]
    fn recovery_proof_reference_is_single_use() {
        let (_dir, ctx) = context();
        let user = "easynet:///r/realm/user/alice";
        invoke(
            &ctx,
            ABILITY_PRINCIPAL_CREATE,
            json!({
                "command": command("create", "bootstrap", None),
                "principal_ura": user
            }),
        );
        let first = invoke(
            &ctx,
            ABILITY_PRINCIPAL_BIND_FIRST_KEY,
            json!({
                "command": command_with_ref("bind1", "bootstrap", "proof:create", Some(1)),
                "principal_ura": user,
                "public_key_b64": b64_pubkey(1)
            }),
        );
        let first_binding_id = binding_id_at(&first, 0);
        invoke(
            &ctx,
            ABILITY_PRINCIPAL_CONFIGURE_RECOVERY,
            json!({
                "command": command_for_actor_with_ref(
                    user,
                    "recovery-policy",
                    "active_key",
                    &first_binding_id,
                    Some(2)
                ),
                "principal_ura": user,
                "policy_ref": "recovery-policy:single-use"
            }),
        );
        invoke(
            &ctx,
            ABILITY_PRINCIPAL_RECOVER,
            json!({
                "command": command_for_actor_with_ref(
                    user,
                    "recover-once",
                    "recovery",
                    "recovery-policy:single-use",
                    Some(3)
                ),
                "principal_ura": user,
                "replacement_key": {
                    "command": command("ignored-child", "recovery", None),
                    "principal_ura": user,
                    "public_key_b64": b64_pubkey(2)
                }
            }),
        );

        let err = ctx
            .handle(
                ABILITY_PRINCIPAL_RECOVER,
                serde_json::to_vec(&json!({
                    "request": {
                        "command": command_for_actor_with_ref(
                            user,
                            "recover-replay",
                            "recovery",
                            "recovery-policy:single-use",
                            Some(4)
                        ),
                        "principal_ura": user,
                        "replacement_key": {
                            "command": command("ignored-replay-child", "recovery", None),
                            "principal_ura": user,
                            "public_key_b64": b64_pubkey(3)
                        }
                    }
                }))
                .unwrap()
                .as_slice(),
            )
            .expect_err("recovery proof replay must reject");
        assert_eq!(err.code(), tonic::Code::PermissionDenied);
        assert!(err.message().contains("already been consumed"));

        let loaded = PrincipalStore::load(&ctx.store_path, ABILITY_PRINCIPAL_GET).unwrap();
        let principal = loaded.principals.get(user).unwrap();
        assert_eq!(
            principal
                .bindings
                .iter()
                .filter(|binding| binding.state == KeyBindingState::Active)
                .count(),
            2
        );
        assert_eq!(principal.consumed_recovery_proofs.len(), 1);
        assert!(principal
            .consumed_recovery_proofs
            .contains_key(&recovery_proof_hash(user, "recovery-policy:single-use")));
        assert!(ctx
            .runtime_trust
            .cell
            .snapshot()
            .lookup_user_by_pubkey(user, &b64_pubkey(3))
            .is_none());
    }

    #[test]
    fn recovery_restores_suspended_principal_and_deleted_state_is_terminal() {
        let (_dir, ctx) = context();
        let user = "easynet:///r/realm/user/alice";
        invoke(
            &ctx,
            ABILITY_PRINCIPAL_CREATE,
            json!({
                "command": command("create", "bootstrap", None),
                "principal_ura": user
            }),
        );
        let first = invoke(
            &ctx,
            ABILITY_PRINCIPAL_BIND_FIRST_KEY,
            json!({
                "command": command_with_ref("bind", "bootstrap", "proof:create", Some(1)),
                "principal_ura": user,
                "public_key_b64": b64_pubkey(4)
            }),
        );
        let first_binding_id = binding_id_at(&first, 0);
        invoke(
            &ctx,
            ABILITY_PRINCIPAL_CONFIGURE_RECOVERY,
            json!({
                "command": command_for_actor_with_ref(
                    user,
                    "configure-recovery",
                    "active_key",
                    &first_binding_id,
                    Some(2)
                ),
                "principal_ura": user,
                "policy_ref": "recovery-policy:suspended"
            }),
        );
        let suspended = invoke(
            &ctx,
            ABILITY_PRINCIPAL_SUSPEND,
            json!({
                "command": command_for_actor_with_ref(
                    user,
                    "suspend",
                    "active_key",
                    &first_binding_id,
                    Some(3)
                ),
                "principal_ura": user
            }),
        );
        assert_eq!(suspended["principal"]["state"], "suspended");

        let recovered = invoke(
            &ctx,
            ABILITY_PRINCIPAL_RECOVER,
            json!({
                "command": command_for_actor_with_ref(
                    user,
                    "recover-from-suspended",
                    "recovery",
                    "recovery-policy:suspended",
                    Some(4)
                ),
                "principal_ura": user,
                "replacement_key": {
                    "command": command("ignored-recovery-child", "recovery", None),
                    "principal_ura": user,
                    "public_key_b64": b64_pubkey(5)
                }
            }),
        );
        assert_eq!(recovered["principal"]["state"], "active");
        assert!(ctx
            .runtime_trust
            .cell
            .snapshot()
            .lookup_user_by_pubkey(user, &b64_pubkey(5))
            .is_some());

        invoke(
            &ctx,
            ABILITY_PRINCIPAL_CONFIGURE_RECOVERY,
            json!({
                "command": command_for_actor_with_ref(
                    user,
                    "configure-deleted-recovery",
                    "active_key",
                    &first_binding_id,
                    Some(5)
                ),
                "principal_ura": user,
                "policy_ref": "recovery-policy:deleted"
            }),
        );

        let delete_grant = invoke(
            &ctx,
            ABILITY_PRINCIPAL_ISSUE_GRANT,
            json!({
                "command": command_for_actor_with_ref(
                    user,
                    "issue-delete-grant",
                    "active_key",
                    &first_binding_id,
                    Some(6)
                ),
                "principal_ura": user,
                "actions": [ABILITY_PRINCIPAL_DELETE]
            }),
        );
        let delete_grant_id = delete_grant["principal"]["grants"][0]["grant_id"]
            .as_str()
            .unwrap();
        let deleted = invoke(
            &ctx,
            ABILITY_PRINCIPAL_DELETE,
            json!({
                "command": command_for_actor_with_ref(
                    user,
                    "delete",
                    "grant",
                    delete_grant_id,
                    Some(7)
                ),
                "principal_ura": user
            }),
        );
        assert_eq!(deleted["principal"]["state"], "deleted");

        let err = ctx
            .handle(
                ABILITY_PRINCIPAL_RECOVER,
                serde_json::to_vec(&json!({
                    "request": {
                        "command": command_for_actor_with_ref(
                            user,
                            "recover-deleted",
                            "recovery",
                            "recovery-policy:deleted",
                            Some(8)
                        ),
                        "principal_ura": user,
                        "replacement_key": {
                            "command": command("ignored-deleted-recovery-child", "recovery", None),
                            "principal_ura": user,
                            "public_key_b64": b64_pubkey(6)
                        }
                    }
                }))
                .unwrap()
                .as_slice(),
            )
            .expect_err("deleted principal recovery must reject");
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
        assert!(err
            .message()
            .contains("principal must be active or suspended"));
        assert!(ctx
            .runtime_trust
            .cell
            .snapshot()
            .lookup_user_by_pubkey(user, &b64_pubkey(6))
            .is_none());
    }

    #[test]
    fn backend_free_principal_lifecycle_scenario_persists_multi_user_state() {
        let (_dir, ctx) = context();
        let admin = "easynet:///r/realm/user/admin";
        let bob = "easynet:///r/realm/user/bob";

        invoke(
            &ctx,
            ABILITY_PRINCIPAL_CREATE,
            json!({
                "command": command("admin-create", "bootstrap", None),
                "principal_ura": admin
            }),
        );
        let admin_bound = invoke(
            &ctx,
            ABILITY_PRINCIPAL_BIND_FIRST_KEY,
            json!({
                "command": command_with_ref(
                    "admin-bind",
                    "bootstrap",
                    "proof:admin-create",
                    Some(1)
                ),
                "principal_ura": admin,
                "public_key_b64": b64_pubkey(20)
            }),
        );
        let admin_binding_id = binding_id_at(&admin_bound, 0);

        invoke(
            &ctx,
            ABILITY_PRINCIPAL_ADD_KEY,
            json!({
                "command": command_for_actor_with_ref(
                    admin,
                    "admin-backup-key",
                    "active_key",
                    &admin_binding_id,
                    Some(2)
                ),
                "principal_ura": admin,
                "public_key_b64": b64_pubkey(25)
            }),
        );

        let issued_enrollment = invoke(
            &ctx,
            ABILITY_PRINCIPAL_ISSUE_ENROLLMENT,
            json!({
                "command": command_for_actor_with_ref(
                    admin,
                    "bob-enrollment",
                    "active_key",
                    &admin_binding_id,
                    Some(3)
                ),
                "principal_ura": admin,
                "subject_principal_ura": bob
            }),
        );
        let enrollment_id = issued_enrollment["principal"]["enrollments"][0]["enrollment_id"]
            .as_str()
            .unwrap()
            .to_string();

        invoke(
            &ctx,
            ABILITY_PRINCIPAL_CREATE,
            json!({
                "command": command_for_actor_with_ref(
                    bob,
                    "bob-create",
                    "enrollment",
                    &enrollment_id,
                    None
                ),
                "principal_ura": bob
            }),
        );
        let bob_bound = invoke(
            &ctx,
            ABILITY_PRINCIPAL_BIND_FIRST_KEY,
            json!({
                "command": command_for_actor_with_ref(
                    bob,
                    "bob-laptop",
                    "enrollment",
                    &enrollment_id,
                    Some(1)
                ),
                "principal_ura": bob,
                "public_key_b64": b64_pubkey(21)
            }),
        );
        let bob_laptop_binding_id = binding_id_at(&bob_bound, 0);

        let bob_with_phone = invoke(
            &ctx,
            ABILITY_PRINCIPAL_ADD_KEY,
            json!({
                "command": command_for_actor_with_ref(
                    bob,
                    "bob-phone",
                    "active_key",
                    &bob_laptop_binding_id,
                    Some(2)
                ),
                "principal_ura": bob,
                "public_key_b64": b64_pubkey(23)
            }),
        );
        let bob_phone_binding_id = binding_id_at(&bob_with_phone, 1);

        let bob_rotated = invoke(
            &ctx,
            ABILITY_PRINCIPAL_ROTATE_KEY,
            json!({
                "command": command_for_actor_with_ref(
                    bob,
                    "bob-laptop-rotate",
                    "active_key",
                    &bob_phone_binding_id,
                    Some(3)
                ),
                "principal_ura": bob,
                "binding_id": bob_laptop_binding_id,
                "replacement": {
                    "command": command_for_actor_with_ref(
                        bob,
                        "ignored-replacement-command",
                        "active_key",
                        &bob_phone_binding_id,
                        None
                    ),
                    "principal_ura": bob,
                    "public_key_b64": b64_pubkey(22)
                }
            }),
        );
        let bob_rotated_laptop_binding_id = binding_id_at(&bob_rotated, 2);

        invoke(
            &ctx,
            ABILITY_PRINCIPAL_REVOKE_KEY,
            json!({
                "command": command_for_actor_with_ref(
                    bob,
                    "bob-phone-revoke",
                    "active_key",
                    &bob_rotated_laptop_binding_id,
                    Some(4)
                ),
                "principal_ura": bob,
                "binding_id": bob_phone_binding_id
            }),
        );

        invoke(
            &ctx,
            ABILITY_PRINCIPAL_CONFIGURE_RECOVERY,
            json!({
                "command": command_for_actor_with_ref(
                    bob,
                    "bob-recovery-policy",
                    "active_key",
                    &bob_rotated_laptop_binding_id,
                    Some(5)
                ),
                "principal_ura": bob,
                "policy_ref": "recovery-policy:bob"
            }),
        );

        invoke(
            &ctx,
            ABILITY_PRINCIPAL_RECOVER,
            json!({
                "command": command_for_actor_with_ref(
                    bob,
                    "bob-recover",
                    "recovery",
                    "recovery-policy:bob",
                    Some(6)
                ),
                "principal_ura": bob,
                "replacement_key": {
                    "command": command_for_actor_with_ref(
                        bob,
                        "ignored-recovery-child-command",
                        "recovery",
                        "recovery-policy:bob",
                        None
                    ),
                    "principal_ura": bob,
                    "public_key_b64": b64_pubkey(24)
                }
            }),
        );

        let bob_suspended = invoke(
            &ctx,
            ABILITY_PRINCIPAL_SUSPEND,
            json!({
                "command": command_for_actor_with_ref(
                    bob,
                    "bob-suspend",
                    "active_key",
                    &bob_rotated_laptop_binding_id,
                    Some(7)
                ),
                "principal_ura": bob
            }),
        );
        assert_eq!(bob_suspended["principal"]["state"], "suspended");

        let bob_reactivated = invoke(
            &ctx,
            ABILITY_PRINCIPAL_REACTIVATE,
            json!({
                "command": command_for_actor_with_ref(
                    bob,
                    "bob-reactivate",
                    "recovery",
                    "recovery-policy:bob",
                    Some(8)
                ),
                "principal_ura": bob
            }),
        );
        assert_eq!(bob_reactivated["principal"]["state"], "active");

        let admin_grant = invoke(
            &ctx,
            ABILITY_PRINCIPAL_ISSUE_GRANT,
            json!({
                "command": command_for_actor_with_ref(
                    admin,
                    "admin-delete-grant",
                    "active_key",
                    &admin_binding_id,
                    Some(5)
                ),
                "principal_ura": admin,
                "actions": [ABILITY_PRINCIPAL_DELETE]
            }),
        );
        let delete_grant_id = admin_grant["principal"]["grants"][0]["grant_id"]
            .as_str()
            .unwrap()
            .to_string();

        let bob_deleted = invoke(
            &ctx,
            ABILITY_PRINCIPAL_DELETE,
            json!({
                "command": command_for_actor_with_ref(
                    admin,
                    "bob-delete",
                    "grant",
                    &delete_grant_id,
                    Some(9)
                ),
                "principal_ura": bob
            }),
        );
        assert_eq!(bob_deleted["principal"]["state"], "deleted");

        let reloaded = PrincipalStore::load(&ctx.store_path, ABILITY_PRINCIPAL_GET).unwrap();
        let admin_record = reloaded.principals.get(admin).unwrap();
        let bob_record = reloaded.principals.get(bob).unwrap();
        assert_eq!(admin_record.state, PrincipalState::Active);
        assert_eq!(
            admin_record
                .bindings
                .iter()
                .filter(|binding| binding.state == KeyBindingState::Active)
                .count(),
            2
        );
        assert_eq!(bob_record.state, PrincipalState::Deleted);
        assert_eq!(bob_record.version, 10);
        assert_eq!(
            admin_record.enrollments[0]
                .consumed_by_principal_ura
                .as_deref(),
            Some(bob)
        );
        assert_eq!(
            bob_record
                .bindings
                .iter()
                .filter(|binding| binding.state == KeyBindingState::Rotated)
                .count(),
            1
        );
        assert_eq!(
            bob_record
                .bindings
                .iter()
                .filter(|binding| binding.state == KeyBindingState::Revoked)
                .count(),
            1
        );
        assert_eq!(
            bob_record
                .bindings
                .iter()
                .filter(|binding| binding.state == KeyBindingState::Active)
                .count(),
            2
        );
        assert_eq!(
            PrincipalLifecycleReader::new(ctx.store_path.clone())
                .admission_state(bob)
                .unwrap(),
            PrincipalAdmissionState::Deleted
        );

        let persisted_trust =
            RealmTrustAnchor::try_load_strict(&ctx.runtime_trust.trust_anchor_path).unwrap();
        assert!(persisted_trust
            .lookup_user_by_pubkey(bob, &b64_pubkey(21))
            .is_none());
        assert!(persisted_trust
            .lookup_user_by_pubkey(bob, &b64_pubkey(23))
            .is_none());
        assert!(persisted_trust
            .lookup_user_by_pubkey(bob, &b64_pubkey(22))
            .is_some());
        assert!(persisted_trust
            .lookup_user_by_pubkey(bob, &b64_pubkey(24))
            .is_some());
        assert_eq!(persisted_trust.revoked_user_pubkey_count(bob), 2);
    }

    fn binding_id_at(snapshot: &serde_json::Value, index: usize) -> String {
        snapshot["principal"]["bindings"][index]["binding_id"]
            .as_str()
            .expect("binding id")
            .to_string()
    }
}
