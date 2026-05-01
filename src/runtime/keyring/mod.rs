// EasyNet CLI — Keyring (RFC-002)
// =================================
//
// File: src/runtime/keyring/mod.rs
//
// The keyring is the local-authority store for an EasyNet device's
// cryptographic identity. RFC-002 §3. Five sub-modules:
//
//   crypto.rs    — AES-GCM encrypt/decrypt + Argon2id KDF + RNG
//   store.rs     — on-disk schema + load/save + master-key handling
//   handle.rs    — runtime handle wrapping the store + lock semantics
//   abilities.rs — 10 ability handlers (keyring.* surface)
//   resolver.rs  — KeyResolver implementations backed by the keyring
//
// Author: Silan.Hu
// Email:  silan.hu@u.nus.edu
// Copyright (c) 2026-2027 easynet. All rights reserved.

pub mod abilities;
/// RFC-002.2 production wiring: real CliForwardInvoker backed by
/// the daemon's DendriteBridge. Daemon boot constructs one and
/// installs it via forward::set_forward_invoker.
pub mod bridge_forward;
pub mod crypto;
pub mod forward;
pub mod handle;
pub mod resolver;
pub mod store;
/// Cross-realm user identity binding token (RFC-N PR-N4
/// commit 1/N). Builds on RFC-002 keyring primitives + PR-N2's
/// FederatedKeyResolver to let user U on realm A be recognised
/// as their realm B identity for federated `<self>.discover`.
/// Wire shape + canonical bytes contract + verify helper; the
/// `<self>.keyring.federate_user_identity_token` and
/// `<self>.keyring.consume_federate_user_token` ability handlers
/// land in commits 2/N + 3/N.
pub mod user_binding_chain;

/// On-disk + in-memory store for cross-realm user identity
/// bindings (RFC-N PR-N4 commit 3/N). Consumer-side counterpart
/// to `user_binding_chain.rs`: realm B's
/// `<self>.keyring.consume_federate_user_token` writes here
/// after the four-check verify chain passes; later
/// `<self>.discover` Tier-3 reads the bindings to filter
/// cross-realm devices by user identity.
pub mod federated_bindings;

pub use handle::KeyringHandle;
pub use resolver::{ChainResolver, KeyResolveError, LocalKeyringResolver, PeerKeyringResolver};
pub use store::{Entry, KeyRing, KeyStatus, MasterKeyKind, PeerEntry, PeerStatus};
