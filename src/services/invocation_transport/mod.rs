// EasyNet CLI — Services Layer — daemon Invocation transport
// ==========================================================
//
// File: src/services/invocation_transport/mod.rs
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
// - Transport policy gate semantics — this module owns daemon/product
//   routing checks, while descriptor-bound Axon runtime admission
//   remains inside `easynet-axon::invocation::LocalRuntime`
// - Ability dispatch — `runtime::ability_dispatch` continues to own
//   the AxonAbilityCatalog and the registered handler set; this
//   module routes inbound RPC calls into that runtime surface
// - Federation `<self>.session` / `<self>.invoke_remote` reverse-
//   channel liveness — that lives in `presence_registry` and the
//   session-specific modules below, not in the top-level service
//   namespace
//
// What it does own
// ----------------
// 1. The concrete `DaemonInvocationService` struct and its
//    `tonic` trait impl
// 2. The boundary error mapping from internal typed errors to
//    `tonic::Status`
// 3. The construction recipe that wires transport policy, presence
//    registry, Axon LocalRuntime, plugin runtime manager, invocation
//    ledger, federation dialers, and local session dispatch together
//    at daemon boot
//
// What it does NOT own
// --------------------
// This module is not a second Axon runtime and not a product policy
// singleton. It embeds Axon protocol/runtime primitives inside
// `easynet-daemon`, while EasyNet product policy remains daemon-owned
// and Axon protocol semantics remain Axon-owned.
//
// Why feature-gated
// -----------------
// `axon-pb` (declared in `Cargo.toml`) gates the SDK-owned generated
// `pb` module. It is enabled by default because this transport is the
// current product invocation plane. Specialist `--no-default-features`
// builds may still omit it, but those builds are not daemon/product
// builds and must not be used for live discovery or routing validation.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

// Tonic generated service traits and daemon-side dispatch helpers are
// `Result<_, tonic::Status>` boundaries. Boxing `Status` inside this
// module would only add unwrap/rebox noise before returning to the
// same generated trait surface. Keep the exception local to the
// transport boundary so the crate-level F-005 ratchet still catches
// large error regressions everywhere else.
#![allow(clippy::result_large_err)]

pub mod admission_facade;
pub(crate) mod bidi_dispatcher;
pub mod boot;
pub mod daemon_invocation_service;
pub(crate) mod deps;
pub(crate) mod descriptor_binding;
pub mod device_trust_sync;
#[cfg(feature = "axon-pb")]
pub mod federated_key_resolver;
/// CLI-side `federation.forward_invoke` helper (moved from `support`,
/// T4.1 pre-move b — it consumes this module's ProtoEnvelope).
#[cfg(feature = "axon-pb")]
pub(crate) mod federation_invoke;
pub mod federation_wrappers;
pub mod hub_resolver;
pub mod invocation_wire;
pub mod invoke_remote_initiator;
pub(crate) mod ledger_projection;
pub(crate) mod lifecycle_driver;
pub mod list_user_pubkeys;
pub mod local_session_dispatcher;
pub mod origin_caller;
pub(crate) mod peer_envelope_signer;
pub(crate) mod quota_meter;
pub mod register_device_pubkey;
pub mod revoke_user_pubkey;
pub mod route_resolver;
pub mod session_escalation;
pub mod session_initiator;
pub(crate) mod stream_dispatcher;
pub(crate) mod target_gate;
pub(crate) mod unary_dispatcher;

pub use admission_facade::AdmissionFacade;
pub use boot::{start_daemon_invocation_transport, SessionShutdown};
pub use daemon_invocation_service::DaemonInvocationService;
pub use invocation_wire::{ProtoEnvelope, DEFAULT_URA_PROFILE};
pub use invoke_remote_initiator::{
    invoke_remote, InvokeRemoteDown, InvokeRemoteFrame, InvokeRemoteUp, SessionDispatch,
    ABILITY_INVOKE_REMOTE, INVOKE_REMOTE_STREAM_ID,
};
pub use list_user_pubkeys::ABILITY_SELF_LIST_USER_PUBKEYS;
pub use register_device_pubkey::ABILITY_SELF_REGISTER_DEVICE_PUBKEY;
pub use revoke_user_pubkey::ABILITY_SELF_REVOKE_USER_PUBKEY;
