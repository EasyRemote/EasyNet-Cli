// EasyNet CLI — managed-signing provider boundary
// =================================================
//
// File: src/daemon/keyring/managed_signing_provider.rs
// Description: Provider trait boundary for daemon managed-signing operations.
//
// Protocol Responsibility
// -----------------------
// Define provider seams used by keyring runtime state machines and ability
// adapters. Production is backed by the daemon-local key-service endpoint;
// tests can use the narrow seam required by the state machine under test.
//
// Implementation Approach
// -----------------------
// `ManagedSigningProvider` exposes managed-signing inventory and peer
// operations without owning public response projection or ability registration.
// `ManagedSigningIssuerProvider` is the minimal read/sign seam required by
// token issuance state machines. `Arc<T>` delegates transparently so handlers
// and state machines can share provider instances.
//
// Usage Contract
// --------------
// Runtime state machines depend on narrow traits in this module rather than on
// `abilities.rs` or the full administration provider. Ability handlers may
// re-export the full trait for existing call sites, but they do not own the
// provider abstraction.
//
// Architectural Position
// ----------------------
// Keyring provider boundary. Depends on public managed-signing projections and
// the key-service client, but not on ability catalog registration or response
// DTO modules.

use anyhow::Result;
use std::sync::Arc;

use super::{ManagedPeer, ManagedSigningKeyProjection, ManagedSigningStatus};
use crate::daemon::identity::self_identity::KeyringClient;

pub trait ManagedSigningIssuerProvider: Send + Sync {
    fn public_key(&self, key_id: &str) -> Result<ManagedSigningKeyProjection>;
    fn sign(&self, key_id: &str, canonical_bytes: &[u8]) -> Result<ed25519_dalek::Signature>;
}

pub trait ManagedSigningProvider: Send + Sync {
    fn create(
        &self,
        purpose: String,
        bound_subject: Option<String>,
    ) -> Result<ManagedSigningKeyProjection>;
    fn list(
        &self,
        purpose: Option<String>,
        status: Option<ManagedSigningStatus>,
    ) -> Result<Vec<ManagedSigningKeyProjection>>;
    fn public_key(&self, key_id: &str) -> Result<ManagedSigningKeyProjection>;
    fn sign(&self, key_id: &str, canonical_bytes: &[u8]) -> Result<ed25519_dalek::Signature>;
    fn rotate(&self, key_id: &str) -> Result<ManagedSigningKeyProjection>;
    fn revoke(&self, key_id: &str) -> Result<i64>;
    fn set_expiry(&self, key_id: &str, expires_unix_ms: i64) -> Result<()>;
    fn bind_subject(&self, key_id: &str, subject_ura: &str) -> Result<()>;
    fn peer_add(
        &self,
        peer_ura: &str,
        public_key_b64: &str,
        via_hub: Option<String>,
    ) -> Result<bool>;
    fn peer_list(&self) -> Result<Vec<ManagedPeer>>;
}

impl<T: ManagedSigningProvider + ?Sized> ManagedSigningIssuerProvider for T {
    fn public_key(&self, key_id: &str) -> Result<ManagedSigningKeyProjection> {
        ManagedSigningProvider::public_key(self, key_id)
    }

    fn sign(&self, key_id: &str, canonical_bytes: &[u8]) -> Result<ed25519_dalek::Signature> {
        ManagedSigningProvider::sign(self, key_id, canonical_bytes)
    }
}

impl<T: ManagedSigningProvider + ?Sized> ManagedSigningProvider for Arc<T> {
    fn create(
        &self,
        purpose: String,
        bound_subject: Option<String>,
    ) -> Result<ManagedSigningKeyProjection> {
        (**self).create(purpose, bound_subject)
    }
    fn list(
        &self,
        purpose: Option<String>,
        status: Option<ManagedSigningStatus>,
    ) -> Result<Vec<ManagedSigningKeyProjection>> {
        (**self).list(purpose, status)
    }
    fn public_key(&self, key_id: &str) -> Result<ManagedSigningKeyProjection> {
        (**self).public_key(key_id)
    }
    fn sign(&self, key_id: &str, canonical_bytes: &[u8]) -> Result<ed25519_dalek::Signature> {
        (**self).sign(key_id, canonical_bytes)
    }
    fn rotate(&self, key_id: &str) -> Result<ManagedSigningKeyProjection> {
        (**self).rotate(key_id)
    }
    fn revoke(&self, key_id: &str) -> Result<i64> {
        (**self).revoke(key_id)
    }
    fn set_expiry(&self, key_id: &str, expires_unix_ms: i64) -> Result<()> {
        (**self).set_expiry(key_id, expires_unix_ms)
    }
    fn bind_subject(&self, key_id: &str, subject_ura: &str) -> Result<()> {
        (**self).bind_subject(key_id, subject_ura)
    }
    fn peer_add(
        &self,
        peer_ura: &str,
        public_key_b64: &str,
        via_hub: Option<String>,
    ) -> Result<bool> {
        (**self).peer_add(peer_ura, public_key_b64, via_hub)
    }
    fn peer_list(&self) -> Result<Vec<ManagedPeer>> {
        (**self).peer_list()
    }
}

impl ManagedSigningProvider for KeyringClient {
    fn create(
        &self,
        purpose: String,
        bound_subject: Option<String>,
    ) -> Result<ManagedSigningKeyProjection> {
        Ok(self.inventory_create(purpose, bound_subject)?)
    }
    fn list(
        &self,
        purpose: Option<String>,
        status: Option<ManagedSigningStatus>,
    ) -> Result<Vec<ManagedSigningKeyProjection>> {
        Ok(self.inventory_list(purpose, status)?)
    }
    fn public_key(&self, key_id: &str) -> Result<ManagedSigningKeyProjection> {
        Ok(self.inventory_public_key(key_id)?)
    }
    fn sign(&self, key_id: &str, canonical_bytes: &[u8]) -> Result<ed25519_dalek::Signature> {
        Ok(self.inventory_sign(key_id, canonical_bytes)?)
    }
    fn rotate(&self, key_id: &str) -> Result<ManagedSigningKeyProjection> {
        Ok(self.inventory_rotate(key_id)?)
    }
    fn revoke(&self, key_id: &str) -> Result<i64> {
        Ok(self.inventory_revoke(key_id)?)
    }
    fn set_expiry(&self, key_id: &str, expires_unix_ms: i64) -> Result<()> {
        Ok(self.inventory_set_expiry(key_id, expires_unix_ms)?)
    }
    fn bind_subject(&self, key_id: &str, subject_ura: &str) -> Result<()> {
        Ok(self.inventory_bind_subject(key_id, subject_ura)?)
    }
    fn peer_add(
        &self,
        peer_ura: &str,
        public_key_b64: &str,
        via_hub: Option<String>,
    ) -> Result<bool> {
        Ok(self.inventory_peer_add(peer_ura, public_key_b64, via_hub)?)
    }
    fn peer_list(&self) -> Result<Vec<ManagedPeer>> {
        Ok(self.inventory_peer_list()?)
    }
}
