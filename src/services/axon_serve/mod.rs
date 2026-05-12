// EasyNet CLI — Services Layer — Axon Invocation gRPC server
// ===========================================================
//
// File: src/services/axon_serve/mod.rs
// Description: Daemon-side implementation of the axon
//              `pb::axon::v1::invocation_server::Invocation` trait
//              (Invoke / InvokeStream / InvokeBidi).
//
// Responsibility boundary
// -----------------------
// This module owns the *binding* between the new daemon binary and
// the axon Invocation RPC surface. It does not own:
//
// - Configuration parsing — that lives in `persistence::daemon_config`
//   and is loaded in the daemon binary's `main` and threaded down
// - Admission gate semantics — those live in `easynet-axon`'s
//   `domain::admission` helpers; this module *invokes* them at the
//   start of every RPC method but never re-implements them
// - Ability dispatch — `runtime::ability_dispatch` continues to own
//   the LocalAbilityRegistry and the registered handler set; this
//   module routes inbound RPC calls into that registry
// - Federation `<self>.session` / `<self>.invoke_remote` reverse-
//   channel state — that lives in the future `presence_registry`
//   module which a follow-up commit on this branch introduces
//
// What it does own
// ----------------
// 1. The concrete `DaemonInvocationService` struct and its
//    `tonic` trait impl
// 2. The boundary error mapping from internal typed errors to
//    `tonic::Status`
// 3. The construction recipe that wires admission gate + presence
//    registry + ability dispatcher together at daemon boot
//
// PR-1 staging
// ------------
// This commit lands the module structure + a service struct + a
// `tonic` impl that returns `Status::unimplemented` for every
// method. It compiles, registers, and refuses every call. The real
// dispatch logic (PresenceRegistry wiring, federation.* wrappers,
// LocalAbilityRegistry forwarding) lands in subsequent commits on
// the same `rfc-001-impl` branch — each as a self-contained logical
// change per CTO directive 06 §3.5.
//
// Why feature-gated
// -----------------
// `axon-pb` (declared in `Cargo.toml`) gates the entire generated
// `pb` module. Building without it lets developers without `protoc`
// on PATH compile the existing CLI; turning it on opts in to the
// new daemon transport plane. RFC-003 PR-10 (production canary)
// flips the default; until then, builds for production take the
// feature explicitly.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

pub mod admission_facade;
pub mod boot;
pub mod daemon_invocation_service;
#[cfg(feature = "axon-pb")]
pub mod federated_key_resolver;
pub mod federation_wrappers;
pub mod invoke_remote_initiator;
pub mod local_ability_dispatcher;
pub mod list_user_pubkeys;
pub mod pinned_user_key_resolver;
pub mod register_device_pubkey;
pub mod revoke_user_pubkey;
pub mod session_escalation;
pub mod session_initiator;

pub use admission_facade::AdmissionFacade;
pub use boot::start_axon_serve_sidecar;
pub use daemon_invocation_service::DaemonInvocationService;
pub use invoke_remote_initiator::{
    invoke_remote, InvokeRemoteDown, InvokeRemoteFrame, InvokeRemoteUp, SessionDispatch,
    ABILITY_INVOKE_REMOTE, INVOKE_REMOTE_STREAM_ID,
};
pub use list_user_pubkeys::ABILITY_SELF_LIST_USER_PUBKEYS;
pub use register_device_pubkey::ABILITY_SELF_REGISTER_DEVICE_PUBKEY;
pub use revoke_user_pubkey::ABILITY_SELF_REVOKE_USER_PUBKEY;
