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

pub mod crypto;
pub mod store;
pub mod handle;
pub mod abilities;
pub mod resolver;
pub mod forward;

pub use handle::KeyringHandle;
pub use store::{Entry, KeyRing, KeyStatus, MasterKeyKind, PeerEntry, PeerStatus};
pub use resolver::{ChainResolver, KeyResolveError, LocalKeyringResolver, PeerKeyringResolver};
