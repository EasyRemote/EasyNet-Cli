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
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tonic::Status;

use crate::daemon::invocation::admission::runtime_trust::{
    now_unix_ms, RuntimeTrust, RuntimeTrustContext,
};
use crate::daemon::persistence::file_lock::ExclusiveFileLock;
use crate::daemon::trust::anchor::TrustedAgentRole;

pub const ABILITY_PRINCIPAL_CREATE: &str = "principal.lifecycle.create";
pub const ABILITY_PRINCIPAL_BIND_FIRST_KEY: &str = "principal.lifecycle.bind_first_key";
pub const ABILITY_PRINCIPAL_ADD_KEY: &str = "principal.lifecycle.add_key";
pub const ABILITY_PRINCIPAL_ROTATE_KEY: &str = "principal.lifecycle.rotate_key";
pub const ABILITY_PRINCIPAL_REVOKE_KEY: &str = "principal.lifecycle.revoke_key";
pub const ABILITY_PRINCIPAL_CONFIGURE_RECOVERY: &str = "principal.lifecycle.configure_recovery";
pub const ABILITY_PRINCIPAL_RECOVER: &str = "principal.lifecycle.recover";
pub const ABILITY_PRINCIPAL_SUSPEND: &str = "principal.lifecycle.suspend";
pub const ABILITY_PRINCIPAL_REACTIVATE: &str = "principal.lifecycle.reactivate";
pub const ABILITY_PRINCIPAL_DELETE: &str = "principal.lifecycle.delete";
pub const ABILITY_PRINCIPAL_ISSUE_GRANT: &str = "principal.lifecycle.issue_grant";
pub const ABILITY_PRINCIPAL_REVOKE_GRANT: &str = "principal.lifecycle.revoke_grant";
pub const ABILITY_PRINCIPAL_GET: &str = "principal.lifecycle.get";

#[derive(Debug, Clone)]
pub(crate) struct PrincipalLifecycleContext {
    runtime_trust: RuntimeTrustContext,
    store_path: PathBuf,
}

impl PrincipalLifecycleContext {
    pub(crate) fn from_runtime_trust(runtime_trust: RuntimeTrustContext) -> Self {
        let store_path = runtime_trust
            .trust_anchor_path
            .with_file_name("principal-lifecycle.json");
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
        if is_first {
            require_proof_kind(ABILITY_PRINCIPAL_CREATE, &request.command, "bootstrap")?;
        } else {
            require_one_proof_kind(
                ABILITY_PRINCIPAL_CREATE,
                &request.command,
                &["grant", "enrollment"],
            )?;
        }
        let now = now_unix_ms() as i64;
        let mut principal = PrincipalRecord {
            principal_ura: request.principal_ura.clone(),
            state: PrincipalState::Pending,
            version: 1,
            bindings: Vec::new(),
            recovery: None,
            grants: Vec::new(),
            created_unix_ms: now,
            updated_unix_ms: now,
            command_log: BTreeMap::new(),
        };
        principal.record_command(&request.command)?;
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
        let principal =
            active_principal_mut(store, ABILITY_PRINCIPAL_ADD_KEY, &request.principal_ura)?;
        require_one_proof_kind(
            ABILITY_PRINCIPAL_ADD_KEY,
            &request.command,
            &["active_key", "grant", "enrollment"],
        )?;
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
        let principal =
            active_principal_mut(store, ABILITY_PRINCIPAL_ROTATE_KEY, &request.principal_ura)?;
        require_one_proof_kind(
            ABILITY_PRINCIPAL_ROTATE_KEY,
            &request.command,
            &["active_key", "grant"],
        )?;
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
        let principal =
            active_principal_mut(store, ABILITY_PRINCIPAL_REVOKE_KEY, &request.principal_ura)?;
        require_one_proof_kind(
            ABILITY_PRINCIPAL_REVOKE_KEY,
            &request.command,
            &["active_key", "grant"],
        )?;
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
        let principal = active_or_suspended_principal_mut(
            store,
            ABILITY_PRINCIPAL_CONFIGURE_RECOVERY,
            &request.principal_ura,
        )?;
        require_one_proof_kind(
            ABILITY_PRINCIPAL_CONFIGURE_RECOVERY,
            &request.command,
            &["active_key", "grant"],
        )?;
        if request.policy_ref.is_empty() {
            return Err(Status::invalid_argument(
                "principal.lifecycle.configure_recovery: policy_ref is required",
            ));
        }
        principal.recovery = Some(RecoveryPolicy {
            policy_ref: request.policy_ref,
            enabled: true,
            updated_unix_ms: now_unix_ms() as i64,
        });
        principal.bump(&request.command)?;
        Ok(principal.clone())
    }

    fn recover(
        &self,
        store: &mut PrincipalStore,
        request: RecoverRequest,
    ) -> Result<PrincipalRecord, Status> {
        let principal = active_or_suspended_principal_mut(
            store,
            ABILITY_PRINCIPAL_RECOVER,
            &request.principal_ura,
        )?;
        require_proof_kind(ABILITY_PRINCIPAL_RECOVER, &request.command, "recovery")?;
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
        let principal = principal_mut(store, ability, &request.principal_ura)?;
        if principal.state == PrincipalState::Deleted {
            return Err(Status::failed_precondition(format!(
                "{ability}: deleted principal is terminal"
            )));
        }
        match next {
            PrincipalState::Suspended => {
                if principal.state != PrincipalState::Active {
                    return Err(Status::failed_precondition(
                        "principal.lifecycle.suspend: principal must be active",
                    ));
                }
                require_one_proof_kind(ability, &request.command, &["active_key", "grant"])?;
            }
            PrincipalState::Active => {
                if principal.state != PrincipalState::Suspended {
                    return Err(Status::failed_precondition(
                        "principal.lifecycle.reactivate: principal must be suspended",
                    ));
                }
                require_one_proof_kind(ability, &request.command, &["grant", "recovery"])?;
            }
            PrincipalState::Deleted => {
                require_proof_kind(ability, &request.command, "grant")?;
            }
            PrincipalState::Pending => {}
        }
        principal.state = next;
        principal.bump(&request.command)?;
        Ok(principal.clone())
    }

    fn issue_grant(
        &self,
        store: &mut PrincipalStore,
        request: IssueGrantRequest,
    ) -> Result<PrincipalRecord, Status> {
        let principal =
            active_principal_mut(store, ABILITY_PRINCIPAL_ISSUE_GRANT, &request.principal_ura)?;
        require_one_proof_kind(
            ABILITY_PRINCIPAL_ISSUE_GRANT,
            &request.command,
            &["grant", "active_key"],
        )?;
        if request.actions.is_empty() {
            return Err(Status::invalid_argument(
                "principal.lifecycle.issue_grant: actions must not be empty",
            ));
        }
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
        let principal = active_or_suspended_principal_mut(
            store,
            ABILITY_PRINCIPAL_REVOKE_GRANT,
            &request.principal_ura,
        )?;
        require_one_proof_kind(
            ABILITY_PRINCIPAL_REVOKE_GRANT,
            &request.command,
            &["grant", "active_key"],
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
            TrustedAgentRole::User,
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
        ABILITY_PRINCIPAL_ISSUE_GRANT => Ok(ABILITY_PRINCIPAL_ISSUE_GRANT),
        ABILITY_PRINCIPAL_REVOKE_GRANT => Ok(ABILITY_PRINCIPAL_REVOKE_GRANT),
        ABILITY_PRINCIPAL_GET => Ok(ABILITY_PRINCIPAL_GET),
        _ => Err(Status::unimplemented(format!(
            "{ability}: principal lifecycle ability is not registered"
        ))),
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct PrincipalStore {
    #[serde(default)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PrincipalRecord {
    principal_ura: String,
    state: PrincipalState,
    version: u64,
    bindings: Vec<PublicKeyBinding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    recovery: Option<RecoveryPolicy>,
    #[serde(default)]
    grants: Vec<AuthorizationGrant>,
    created_unix_ms: i64,
    updated_unix_ms: i64,
    #[serde(default)]
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
struct RecoveryPolicy {
    policy_ref: String,
    enabled: bool,
    updated_unix_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Deserialize)]
struct RequestEnvelope {
    request: PrincipalRequest,
}

#[derive(Debug, Deserialize)]
struct GetArgs {
    principal_ura: String,
}

#[derive(Debug, Clone, Deserialize)]
struct PrincipalRequest {
    command: PrincipalCommand,
    principal_ura: String,
    #[serde(default)]
    key_id: String,
    #[serde(default)]
    public_key_b64: String,
    #[serde(default)]
    expires_unix_ms: Option<i64>,
    #[serde(default)]
    binding_id: String,
    #[serde(default)]
    replacement: Option<Box<PrincipalRequest>>,
    #[serde(default)]
    replacement_key: Option<Box<PrincipalRequest>>,
    #[serde(default)]
    policy_ref: String,
    #[serde(default)]
    actions: Vec<String>,
    #[serde(default)]
    grant_id: String,
}

impl PrincipalRequest {
    fn normalize(&mut self) {
        self.principal_ura = self.principal_ura.trim().to_string();
        self.key_id = self.key_id.trim().to_string();
        self.public_key_b64 = self.public_key_b64.trim().to_string();
        self.binding_id = self.binding_id.trim().to_string();
        self.policy_ref = self.policy_ref.trim().to_string();
        self.grant_id = self.grant_id.trim().to_string();
        self.command.normalize();
        if let Some(replacement) = self.replacement.as_mut() {
            replacement.normalize();
        }
        if let Some(replacement_key) = self.replacement_key.as_mut() {
            replacement_key.normalize();
        }
    }

    fn into_bind_key(self) -> Result<BindKeyRequest, Status> {
        if self.public_key_b64.is_empty() {
            return Err(Status::invalid_argument(
                "principal lifecycle key transition requires public_key_b64",
            ));
        }
        Ok(BindKeyRequest {
            command: self.command,
            principal_ura: self.principal_ura,
            key_id: self.key_id,
            public_key_b64: self.public_key_b64,
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
            policy_ref: self.policy_ref,
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
            actions: self.actions,
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

#[derive(Debug, Clone, Deserialize)]
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

#[derive(Debug, Clone, Deserialize)]
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

fn require_proof_kind(
    ability: &'static str,
    command: &PrincipalCommand,
    expected: &str,
) -> Result<(), Status> {
    if command.proof.kind != expected {
        return Err(Status::permission_denied(format!(
            "{ability}: proof kind `{}` is not sufficient; expected `{expected}`",
            command.proof.kind
        )));
    }
    Ok(())
}

fn require_one_proof_kind(
    ability: &'static str,
    command: &PrincipalCommand,
    expected: &[&str],
) -> Result<(), Status> {
    if expected.iter().any(|kind| *kind == command.proof.kind) {
        return Ok(());
    }
    Err(Status::permission_denied(format!(
        "{ability}: proof kind `{}` is not sufficient; expected one of {:?}",
        command.proof.kind, expected
    )))
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

fn required_text(
    ability: &'static str,
    field: &'static str,
    value: String,
) -> Result<String, Status> {
    if value.is_empty() {
        return Err(Status::invalid_argument(format!(
            "{ability}: {field} is required"
        )));
    }
    Ok(value)
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

    fn command(id: &str, kind: &str, expected_version: Option<u64>) -> serde_json::Value {
        let mut value = json!({
            "actor_ura": "easynet:///r/realm/user/admin",
            "idempotency_key": id,
            "proof": {"kind": kind, "reference": format!("proof:{id}")}
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
                "command": command("bind-1", "bootstrap", Some(1)),
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
    fn additional_user_requires_enrollment_or_grant_proof() {
        let (_dir, ctx) = context();
        invoke(
            &ctx,
            ABILITY_PRINCIPAL_CREATE,
            json!({
                "command": command("create-admin", "bootstrap", None),
                "principal_ura": "easynet:///r/realm/user/admin"
            }),
        );
        let err = ctx
            .handle(
                ABILITY_PRINCIPAL_CREATE,
                serde_json::to_vec(&json!({
                    "request": {
                        "command": command("create-bob", "active_key", None),
                        "principal_ura": "easynet:///r/realm/user/bob"
                    }
                }))
                .unwrap()
                .as_slice(),
            )
            .expect_err("wrong proof rejected");
        assert_eq!(err.code(), tonic::Code::PermissionDenied);

        invoke(
            &ctx,
            ABILITY_PRINCIPAL_CREATE,
            json!({
                "command": command("create-bob-2", "enrollment", None),
                "principal_ura": "easynet:///r/realm/user/bob"
            }),
        );
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
                "command": command("bind1", "bootstrap", Some(1)),
                "principal_ura": user,
                "public_key_b64": b64_pubkey(1)
            }),
        );
        invoke(
            &ctx,
            ABILITY_PRINCIPAL_ADD_KEY,
            json!({
                "command": command("bind2", "active_key", Some(2)),
                "principal_ura": user,
                "public_key_b64": b64_pubkey(2)
            }),
        );
        let binding_id = first["principal"]["bindings"][0]["binding_id"]
            .as_str()
            .unwrap();

        let out = invoke(
            &ctx,
            ABILITY_PRINCIPAL_REVOKE_KEY,
            json!({
                "command": command("revoke1", "active_key", Some(3)),
                "principal_ura": user,
                "binding_id": binding_id
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
                        "command": command("bind", "bootstrap", Some(99)),
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
        invoke(
            &ctx,
            ABILITY_PRINCIPAL_BIND_FIRST_KEY,
            json!({
                "command": command("bind1", "bootstrap", Some(1)),
                "principal_ura": user,
                "public_key_b64": b64_pubkey(1)
            }),
        );
        invoke(
            &ctx,
            ABILITY_PRINCIPAL_CONFIGURE_RECOVERY,
            json!({
                "command": command("recovery-policy", "active_key", Some(2)),
                "principal_ura": user,
                "policy_ref": "recovery-policy:test"
            }),
        );
        let out = invoke(
            &ctx,
            ABILITY_PRINCIPAL_RECOVER,
            json!({
                "command": command("recover", "recovery", Some(3)),
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
}
