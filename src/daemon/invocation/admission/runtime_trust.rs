// EasyNet CLI — daemon RuntimeTrust aggregate
// ============================================
//
// File: src/daemon/invocation/runtime_trust.rs
// Description: Domain aggregate for daemon-owned runtime trust. It
//              centralizes the trust-anchor read/write transaction used
//              by identity.register_pubkey, identity.list_user_pubkeys,
//              and identity.revoke_user_pubkey.
//
// Protocol Responsibility
// -----------------------
// This module does not define Axon protocol semantics. URA parsing remains
// delegated to `crate::core::ura`; Invocation admission, receipts, signatures,
// stream, and bidi semantics remain Axon-owned. This aggregate owns the
// EasyNet daemon runtime admission for which trust rows may enter or leave the
// runtime trust anchor.
//
// Implementation Approach
// -----------------------
// Write methods snapshot the current `SharedTrustAnchor`, construct a fresh
// `RealmTrustAnchor`, apply one domain mutation, persist via the anchor's
// atomic `save`, and publish the replacement cell only after persistence
// succeeds. Query methods read the same shared cell consulted by admission.
//
// Usage Contract
// --------------
// Identity ability handlers may decode/encode their JSON wire shape, but they
// must not mutate or snapshot the trust anchor directly. Register and revoke
// go through `RuntimeTrust`; list goes through `RuntimeTrustReader`.
//
// Architectural Position
// ----------------------
// EasyNet-Cli daemon runtime policy. The backend remains a product wrapper;
// Axon remains the neutral protocol layer.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::prelude::*;
use ed25519_dalek::VerifyingKey;
use tonic::Status;

use crate::daemon::persistence::file_lock::ExclusiveFileLock;
use crate::daemon::trust::anchor::{
    RealmTrustAnchor, RealmTrustError, TrustedAgent, TrustedAgentRole, TrustedPrincipalOwner,
};
use crate::daemon::trust::cell::SharedTrustAnchor;

/// Stable daemon runtime trust context. This is threaded through the
/// identity plane once at boot and borrowed per invocation.
#[derive(Debug, Clone)]
pub(crate) struct RuntimeTrustContext {
    pub(crate) daemon_realm: String,
    pub(crate) trust_anchor_path: PathBuf,
    pub(crate) cell: SharedTrustAnchor,
}

impl RuntimeTrustContext {
    fn writer(&self) -> RuntimeTrust<'_> {
        RuntimeTrust::new(&self.daemon_realm, &self.trust_anchor_path, &self.cell)
    }

    pub(crate) fn register_user_pubkey(
        &self,
        user_ura: String,
        public_key_b64: String,
    ) -> Result<(), Status> {
        self.writer()
            .register_pubkey(user_ura, public_key_b64, TrustedAgentRole::User)
    }

    pub(crate) fn reader(&self) -> RuntimeTrustReader<'_> {
        RuntimeTrustReader::new(&self.cell)
    }
}

/// Query-side view of runtime trust. Keeping this in the aggregate module
/// prevents list handlers from becoming ad hoc cell readers.
pub(crate) struct RuntimeTrustReader<'a> {
    cell: &'a SharedTrustAnchor,
}

impl<'a> RuntimeTrustReader<'a> {
    pub(crate) fn new(cell: &'a SharedTrustAnchor) -> Self {
        Self { cell }
    }

    pub(crate) fn user_snapshot(&self, user_ura: &str) -> RuntimeTrustUserSnapshot {
        let anchor = self.cell.snapshot();
        let keys = anchor
            .lookup_user_all(user_ura)
            .iter()
            .map(|entry| RuntimeTrustUserKey {
                public_key_b64: entry.public_key_b64.clone(),
                added_at_unix_ms: entry.added_at_unix_ms,
            })
            .collect();
        RuntimeTrustUserSnapshot {
            user_ura: user_ura.to_string(),
            keys,
            rotation_epoch: anchor.user_rotation_epoch(user_ura),
            revoked_key_count: anchor.revoked_user_pubkey_count(user_ura),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuntimeTrustUserSnapshot {
    pub(crate) user_ura: String,
    pub(crate) keys: Vec<RuntimeTrustUserKey>,
    pub(crate) rotation_epoch: u64,
    pub(crate) revoked_key_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuntimeTrustUserKey {
    pub(crate) public_key_b64: String,
    pub(crate) added_at_unix_ms: u64,
}

/// Write-side runtime trust aggregate. Each mutation is a complete
/// snapshot -> validate -> save -> publish transaction.
pub(crate) struct RuntimeTrust<'a> {
    daemon_realm: &'a str,
    trust_anchor_path: &'a Path,
    cell: &'a SharedTrustAnchor,
}

impl<'a> RuntimeTrust<'a> {
    pub(crate) fn new(
        daemon_realm: &'a str,
        trust_anchor_path: &'a Path,
        cell: &'a SharedTrustAnchor,
    ) -> Self {
        Self {
            daemon_realm,
            trust_anchor_path,
            cell,
        }
    }

    pub(crate) fn register_pubkey(
        &self,
        principal_ura: String,
        public_key_b64: String,
        role: TrustedAgentRole,
    ) -> Result<(), Status> {
        self.register_pubkey_with_owner(principal_ura, public_key_b64, role, None)
    }

    pub(crate) fn register_pubkey_with_owner(
        &self,
        principal_ura: String,
        public_key_b64: String,
        role: TrustedAgentRole,
        owner: Option<TrustedPrincipalOwner>,
    ) -> Result<(), Status> {
        validate_public_key_b64("identity.register_pubkey", &public_key_b64)?;
        self.validate_register_realm(&principal_ura, role)?;
        if let Some(owner) = owner.as_ref() {
            if owner.principal_ura != principal_ura {
                return Err(Status::invalid_argument(format!(
                    "identity.register_pubkey: owner principal_ura `{}` must match principal_ura `{principal_ura}`",
                    owner.principal_ura
                )));
            }
        }

        let entry = TrustedAgent {
            agent_ura: principal_ura.clone(),
            public_key_b64: public_key_b64.clone(),
            role,
            added_at_unix_ms: now_unix_ms(),
            // Cross-hub federation fields are only operator-curated on
            // pre-existing Hub rows. Backend/hub upsert preserves them
            // through RealmTrustAnchor::upsert_singleton_agent.
            origin_realm: None,
            hub_endpoint: None,
            tls_ca_pem_path: None,
        };

        self.mutate_anchor_when_changed("identity.register_pubkey", |next_anchor| {
            let user_key_already_present = matches!(role, TrustedAgentRole::User)
                && next_anchor
                    .lookup_user_by_pubkey(&principal_ura, &public_key_b64)
                    .is_some();
            let mut changed = false;
            match role {
                TrustedAgentRole::Backend | TrustedAgentRole::Hub | TrustedAgentRole::Device => {
                    next_anchor.upsert_singleton_agent(entry)?;
                    changed = true;
                }
                TrustedAgentRole::User if !user_key_already_present => {
                    next_anchor.append_agent(entry)?;
                    changed = true;
                }
                TrustedAgentRole::User => {}
            }
            if let Some(owner) = owner {
                next_anchor.upsert_principal_owner(owner)?;
                changed = true;
            }
            Ok(((), changed))
        })
    }

    /// Persist one authenticated runtime-principal ownership fact.
    ///
    /// Hosted Agents do not own signing keys, so their owner binding is
    /// created by the already-admitted host publication rather than by
    /// `identity.register_pubkey`. Both paths still use this aggregate and
    /// the same conflict-safe trust-anchor transaction.
    pub(crate) fn bind_principal_owner(&self, owner: TrustedPrincipalOwner) -> Result<(), Status> {
        self.mutate_anchor("federation.advertise_agent", |next_anchor| {
            next_anchor.upsert_principal_owner(owner)
        })
    }

    pub(crate) fn revoke_user_pubkey(
        &self,
        user_ura: &str,
        public_key_b64: &str,
    ) -> Result<bool, Status> {
        validate_public_key_b64("identity.revoke_user_pubkey", public_key_b64)?;
        self.validate_revoke_realm(user_ura)?;

        let _cell_guard = self.cell.mutation_guard();
        let _file_guard = self.lock_store("identity.revoke_user_pubkey")?;
        let mut next_anchor = self.anchor_for_mutation("identity.revoke_user_pubkey")?;
        let revoked = next_anchor
            .revoke_user_pubkey(user_ura, public_key_b64, now_unix_ms())
            .map_err(|err| realm_error_to_status("identity.revoke_user_pubkey", err))?;
        if revoked.is_none() {
            return Ok(false);
        }
        self.persist_and_publish("identity.revoke_user_pubkey", next_anchor)?;
        Ok(true)
    }

    fn mutate_anchor<T>(
        &self,
        ability: &'static str,
        mutation: impl FnOnce(&mut RealmTrustAnchor) -> Result<T, RealmTrustError>,
    ) -> Result<T, Status> {
        self.mutate_anchor_when_changed(ability, |next_anchor| {
            mutation(next_anchor).map(|value| (value, true))
        })
    }

    fn mutate_anchor_when_changed<T>(
        &self,
        ability: &'static str,
        mutation: impl FnOnce(&mut RealmTrustAnchor) -> Result<(T, bool), RealmTrustError>,
    ) -> Result<T, Status> {
        let _cell_guard = self.cell.mutation_guard();
        let _file_guard = self.lock_store(ability)?;
        let mut next_anchor = self.anchor_for_mutation(ability)?;
        let result =
            mutation(&mut next_anchor).map_err(|err| realm_error_to_status(ability, err))?;
        let (result, changed) = result;

        if changed {
            self.persist_and_publish(ability, next_anchor)?;
        }
        Ok(result)
    }

    fn anchor_for_mutation(&self, ability: &'static str) -> Result<RealmTrustAnchor, Status> {
        if self.trust_anchor_path.exists() {
            return RealmTrustAnchor::try_load_strict(self.trust_anchor_path)
                .map_err(|err| realm_error_to_status(ability, err));
        }

        let snapshot = self.cell.snapshot();
        RealmTrustAnchor::from_parts_with_principal_owners(
            snapshot.entries_sorted(),
            snapshot.principal_owners_sorted(),
            snapshot.revoked_user_pubkeys_sorted(),
        )
        .map_err(|err| realm_error_to_status(ability, err))
    }

    fn lock_store(&self, ability: &'static str) -> Result<ExclusiveFileLock, Status> {
        ExclusiveFileLock::acquire_for_data_path(self.trust_anchor_path).map_err(|err| {
            Status::internal(format!(
                "{ability}: lock {}: {err}",
                self.trust_anchor_path.display()
            ))
        })
    }

    fn persist_and_publish(
        &self,
        ability: &'static str,
        next_anchor: RealmTrustAnchor,
    ) -> Result<(), Status> {
        next_anchor
            .save(self.trust_anchor_path)
            .map_err(|err| realm_error_to_status(ability, err))?;
        self.cell.replace(Arc::new(next_anchor));
        Ok(())
    }

    fn validate_register_realm(
        &self,
        principal_ura: &str,
        role: TrustedAgentRole,
    ) -> Result<(), Status> {
        let parsed_realm = crate::core::ura::realm_from_ura(principal_ura).ok_or_else(|| {
            Status::invalid_argument(format!(
                "identity.register_pubkey: principal_ura `{principal_ura}` does not match Axon's \
                 canonical URA grammar",
            ))
        })?;
        if parsed_realm != self.daemon_realm && !matches!(role, TrustedAgentRole::Device) {
            return Err(Status::permission_denied(format!(
                "identity.register_pubkey: role `{}` requires principal_ura realm `{parsed_realm}` \
                 to match daemon realm `{}` — cross-realm user pubkey resolution \
                 happens via federation.resolve_key, not via remote registration",
                role_wire_label(role),
                self.daemon_realm,
            )));
        }
        Ok(())
    }

    fn validate_revoke_realm(&self, user_ura: &str) -> Result<(), Status> {
        let parsed = crate::core::ura::parse_ura(user_ura).map_err(|_| {
            Status::invalid_argument(format!(
                "identity.revoke_user_pubkey: user_ura `{user_ura}` does not match the canonical user URA",
            ))
        })?;
        if parsed.kind != crate::core::ura::URAKind::User {
            return Err(Status::invalid_argument(
                "identity.revoke_user_pubkey: user_ura must identify a User",
            ));
        }
        let parsed_realm = parsed.realm;
        if parsed_realm != self.daemon_realm {
            return Err(Status::permission_denied(format!(
                "identity.revoke_user_pubkey: user_ura realm `{parsed_realm}` must match daemon \
                 realm `{}` (cross-realm user roaming is DEC-EU §multi-realm followup)",
                self.daemon_realm,
            )));
        }
        Ok(())
    }
}

fn validate_public_key_b64(ability: &'static str, raw: &str) -> Result<(), Status> {
    let decoded = BASE64_STANDARD.decode(raw).map_err(|err| {
        Status::invalid_argument(format!(
            "{ability}: public_key_b64 is not valid base64: {err}"
        ))
    })?;
    let bytes: [u8; 32] = decoded.as_slice().try_into().map_err(|_| {
        Status::invalid_argument(format!(
            "{ability}: public_key_b64 must decode to exactly 32 bytes, got {}",
            decoded.len()
        ))
    })?;
    VerifyingKey::from_bytes(&bytes).map_err(|err| {
        Status::invalid_argument(format!(
            "{ability}: public_key_b64 is not a valid Ed25519 verifying key: {err}"
        ))
    })?;
    Ok(())
}

pub(crate) fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn role_wire_label(role: TrustedAgentRole) -> &'static str {
    match role {
        TrustedAgentRole::Backend => "backend",
        TrustedAgentRole::Device => "device",
        TrustedAgentRole::Hub => "hub",
        TrustedAgentRole::User => "user",
    }
}

fn realm_error_to_status(ability: &'static str, err: RealmTrustError) -> Status {
    match err {
        RealmTrustError::DuplicateUra { agent_ura } => Status::already_exists(format!(
            "{ability}: agent_ura `{agent_ura}` already in trust set",
        )),
        RealmTrustError::DuplicateUserPubkey { agent_ura } => Status::already_exists(format!(
            "{ability}: user `{agent_ura}` already has this exact public key registered \
             (different pubkeys per device are expected; same pubkey twice is a no-op pairing retry)",
        )),
        RealmTrustError::DuplicateRevokedUserPubkey { agent_ura } => {
            Status::already_exists(format!(
                "{ability}: user `{agent_ura}` already has a revocation tombstone for this public key",
            ))
        }
        RealmTrustError::RevokedUserPubkey { agent_ura } => {
            Status::failed_precondition(format!(
                "{ability}: user `{agent_ura}` cannot register a public key that was already revoked; generate a fresh keypair",
            ))
        }
        RealmTrustError::ReadFailed { path, source } => {
            Status::internal(format!("{ability}: read {path:?}: {source}"))
        }
        RealmTrustError::ParseFailed { path, source } => {
            Status::internal(format!("{ability}: parse {path:?}: {source}"))
        }
        RealmTrustError::WriteFailed { path, source } => {
            Status::internal(format!("{ability}: write {path:?}: {source}"))
        }
        RealmTrustError::SerializeFailed { path, source } => {
            Status::internal(format!("{ability}: serialize {path:?}: {source}"))
        }
        RealmTrustError::InvalidUraForRole {
            agent_ura,
            role,
            detail,
        } => Status::invalid_argument(format!(
            "{ability}: trusted {role} URA `{agent_ura}` is invalid: {detail}"
        )),
        RealmTrustError::InvalidPrincipalOwner {
            principal_ura,
            detail,
        } => Status::invalid_argument(format!(
            "{ability}: trusted principal owner `{principal_ura}` is invalid: {detail}"
        )),
        RealmTrustError::PrincipalOwnerConflict { principal_ura } => {
            Status::failed_precondition(format!(
                "{ability}: trusted principal owner `{principal_ura}` conflicts with the existing canonical owner binding"
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    fn b64_pubkey(seed: u8) -> String {
        let signing = ed25519_dalek::SigningKey::from_bytes(&[seed; 32]);
        BASE64_STANDARD.encode(signing.verifying_key().to_bytes())
    }

    fn context() -> (tempfile::TempDir, RuntimeTrustContext) {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("realm-trust.toml");
        let ctx = RuntimeTrustContext {
            daemon_realm: "realm".to_string(),
            trust_anchor_path: path,
            cell: SharedTrustAnchor::default(),
        };
        (dir, ctx)
    }

    #[test]
    fn register_persists_and_publishes_through_one_aggregate() {
        let (_dir, ctx) = context();
        ctx.writer()
            .register_pubkey(
                "easynet:///r/realm/user/alice".to_string(),
                b64_pubkey(1),
                TrustedAgentRole::User,
            )
            .expect("register");

        assert_eq!(ctx.cell.cert_anchor_generation(), 1);
        assert_eq!(
            ctx.reader()
                .user_snapshot("easynet:///r/realm/user/alice")
                .keys
                .len(),
            1
        );
        let from_disk =
            RealmTrustAnchor::try_load_strict(&ctx.trust_anchor_path).expect("disk load");
        assert_eq!(from_disk.len(), 1);
    }

    #[test]
    fn register_user_same_pubkey_retry_is_idempotent_noop() {
        let (_dir, ctx) = context();
        let user_ura = "easynet:///r/realm/user/alice".to_string();
        let public_key = b64_pubkey(1);
        ctx.writer()
            .register_pubkey(user_ura.clone(), public_key.clone(), TrustedAgentRole::User)
            .expect("initial register");
        let generation = ctx.cell.cert_anchor_generation();

        ctx.writer()
            .register_pubkey(user_ura.clone(), public_key, TrustedAgentRole::User)
            .expect("same-key retry is idempotent");

        assert_eq!(ctx.cell.cert_anchor_generation(), generation);
        assert_eq!(ctx.reader().user_snapshot(&user_ura).keys.len(), 1);
        let from_disk =
            RealmTrustAnchor::try_load_strict(&ctx.trust_anchor_path).expect("disk load");
        assert_eq!(from_disk.lookup_user_all(&user_ura).len(), 1);
    }

    #[test]
    fn register_backfills_principal_owner_for_existing_device_key() {
        let (_dir, ctx) = context();
        let device_ura = "easynet:///r/realm/device/dev-1".to_string();
        let public_key = b64_pubkey(2);

        ctx.writer()
            .register_pubkey(
                device_ura.clone(),
                public_key.clone(),
                TrustedAgentRole::Device,
            )
            .expect("initial register");
        ctx.writer()
            .register_pubkey_with_owner(
                device_ura.clone(),
                public_key,
                TrustedAgentRole::Device,
                Some(TrustedPrincipalOwner {
                    principal_ura: device_ura.clone(),
                    owner_user_id: "alice".to_string(),
                    owner_ura: "easynet:///r/realm/user/alice".to_string(),
                    owner_username: Some("alice".to_string()),
                    added_at_unix_ms: 1,
                }),
            )
            .expect("idempotent owner backfill");

        let from_disk =
            RealmTrustAnchor::try_load_strict(&ctx.trust_anchor_path).expect("disk load");
        let owner = from_disk
            .lookup_principal_owner(&device_ura)
            .expect("owner fact");
        assert_eq!(owner.owner_user_id, "alice");
    }

    #[test]
    fn register_rejects_cross_realm_user() {
        let (_dir, ctx) = context();
        let err = ctx
            .writer()
            .register_pubkey(
                "easynet:///r/other/user/alice".to_string(),
                b64_pubkey(1),
                TrustedAgentRole::User,
            )
            .expect_err("reject cross-realm user");
        assert_eq!(err.code(), tonic::Code::PermissionDenied);
        assert_eq!(ctx.cell.cert_anchor_generation(), 0);
    }

    #[test]
    fn register_hub_role_uses_canonical_authority_identity() {
        let (_dir, ctx) = context();
        let hub_ura = crate::core::ura::hub_ura("realm");

        ctx.writer()
            .register_pubkey(hub_ura.clone(), b64_pubkey(1), TrustedAgentRole::Hub)
            .expect("canonical Authority identity admits Hub role");
        let err = ctx
            .writer()
            .register_pubkey(
                "easynet:///r/realm/authority/extra".to_string(),
                b64_pubkey(2),
                TrustedAgentRole::Hub,
            )
            .expect_err("Authority URA with tail must not be admitted as Hub");

        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(ctx.cell.snapshot().lookup(&hub_ura).is_some());
        assert_eq!(ctx.cell.cert_anchor_generation(), 1);
    }

    #[test]
    fn list_reads_same_anchor_without_bumping_generation() {
        let (_dir, ctx) = context();
        ctx.writer()
            .register_pubkey(
                "easynet:///r/realm/user/alice".to_string(),
                b64_pubkey(1),
                TrustedAgentRole::User,
            )
            .expect("first");
        ctx.writer()
            .register_pubkey(
                "easynet:///r/realm/user/alice".to_string(),
                b64_pubkey(2),
                TrustedAgentRole::User,
            )
            .expect("second");
        let before = ctx.cell.cert_anchor_generation();

        let snapshot = ctx.reader().user_snapshot("easynet:///r/realm/user/alice");

        assert_eq!(snapshot.keys.len(), 2);
        assert_eq!(snapshot.rotation_epoch, 0);
        assert_eq!(snapshot.revoked_key_count, 0);
        assert_eq!(ctx.cell.cert_anchor_generation(), before);
    }

    #[test]
    fn concurrent_registers_do_not_drop_trust_rows() {
        let (_dir, ctx) = context();
        let ctx = std::sync::Arc::new(ctx);
        let mut handles = Vec::new();

        for index in 1..=8u8 {
            let ctx = std::sync::Arc::clone(&ctx);
            handles.push(std::thread::spawn(move || {
                ctx.writer()
                    .register_pubkey(
                        crate::core::ura::user_ura("realm", &format!("user-{index}")),
                        b64_pubkey(index),
                        TrustedAgentRole::User,
                    )
                    .expect("register user key")
            }));
        }

        for handle in handles {
            handle.join().expect("writer thread must not panic");
        }

        assert_eq!(ctx.cell.snapshot().len(), 8);
        assert_eq!(ctx.cell.cert_anchor_generation(), 8);
        let from_disk =
            RealmTrustAnchor::try_load_strict(&ctx.trust_anchor_path).expect("disk load");
        assert_eq!(from_disk.len(), 8);
    }

    #[test]
    fn revoke_records_tombstone_and_repeated_revoke_is_noop() {
        let (_dir, ctx) = context();
        let key = b64_pubkey(1);
        ctx.writer()
            .register_pubkey(
                "easynet:///r/realm/user/alice".to_string(),
                key.clone(),
                TrustedAgentRole::User,
            )
            .expect("register");
        let removed = ctx
            .writer()
            .revoke_user_pubkey("easynet:///r/realm/user/alice", &key)
            .expect("revoke");
        assert!(removed);
        assert!(ctx
            .reader()
            .user_snapshot("easynet:///r/realm/user/alice")
            .keys
            .is_empty());
        let snapshot = ctx.reader().user_snapshot("easynet:///r/realm/user/alice");
        assert_eq!(snapshot.user_ura, "easynet:///r/realm/user/alice");
        assert_eq!(snapshot.rotation_epoch, 1);
        assert_eq!(snapshot.revoked_key_count, 1);

        let from_disk =
            RealmTrustAnchor::try_load_strict(&ctx.trust_anchor_path).expect("disk load");
        assert!(from_disk.is_user_pubkey_revoked("easynet:///r/realm/user/alice", &key));

        let missing = ctx
            .writer()
            .revoke_user_pubkey("easynet:///r/realm/user/alice", &key)
            .expect("idempotent retry");
        assert!(!missing);
        assert_eq!(ctx.cell.cert_anchor_generation(), 2);
        let after_retry =
            RealmTrustAnchor::try_load_strict(&ctx.trust_anchor_path).expect("disk reload");
        assert_eq!(
            after_retry.revoked_user_pubkey_count("easynet:///r/realm/user/alice"),
            1
        );
    }

    #[test]
    fn register_rejects_previously_revoked_user_key() {
        let (_dir, ctx) = context();
        let key = b64_pubkey(1);
        let user_ura = "easynet:///r/realm/user/alice";
        ctx.writer()
            .register_pubkey(user_ura.to_string(), key.clone(), TrustedAgentRole::User)
            .expect("register");
        assert!(ctx
            .writer()
            .revoke_user_pubkey(user_ura, &key)
            .expect("revoke"));

        let err = ctx
            .writer()
            .register_pubkey(user_ura.to_string(), key, TrustedAgentRole::User)
            .expect_err("tombstoned key rejected");
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
    }

    #[test]
    fn malformed_public_key_rejected_before_persistence() {
        let (_dir, ctx) = context();
        let err = ctx
            .writer()
            .register_pubkey(
                "easynet:///r/realm/user/alice".to_string(),
                json!("not-base64").as_str().unwrap().to_string(),
                TrustedAgentRole::User,
            )
            .expect_err("bad key");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert_eq!(ctx.cell.cert_anchor_generation(), 0);
        assert!(!ctx.trust_anchor_path.exists());
    }
}
