// EasyNet CLI — daemon Invocation
// ===============================
//
// File: src/daemon/invocation/mod.rs
// Description: Daemon-side implementation of the axon
//              `pb::axon::v1::invocation_server::Invocation` trait
//              (Invoke / InvokeStream / InvokeBidi).
//
// Responsibility boundary
// -----------------------
// This module owns the daemon's Invocation front door:
//
// - the SDK request builder used by `DaemonClient`;
// - the binding between `easynet-daemon` and the axon Invocation RPC
//   surface;
// - the daemon-side routing/admission/session scaffolding needed to
//   execute complete Axon Invocations.
//
// It does not own:
//
// - Configuration parsing — that lives in `persistence::daemon_config`
//   and is loaded in the daemon binary's `main` and threaded down
// - Transport policy gate semantics — this module owns daemon/product
//   routing checks, while descriptor-bound Axon runtime admission
//   remains inside `easynet-axon::invocation::LocalRuntime`
// - Ability dispatch — `daemon::ability::dispatch` continues to own
//   the AxonAbilityCatalog and the registered handler set; this
//   module routes inbound RPC calls into that runtime surface
// - `session.open` reverse-channel liveness — that lives in the presence
//   registry and the
//   session-specific modules below, not in the top-level service
//   namespace
//
// What it does own
// ----------------
// 1. The SDK `DaemonInvocation` request type and builder.
// 2. The concrete `DaemonInvocationService` struct and its
//    `tonic` trait impl
// 3. The boundary error mapping from internal typed errors to
//    `tonic::Status`
// 4. The construction recipe that wires transport policy, presence
//    registry, Axon LocalRuntime, plugin runtime manager, invocation
//    ledger, federation dialers, and local session dispatch together
//    at daemon boot
//
// What it does NOT own
// --------------------
// This module is not a second Axon runtime and not a runtime admission
// singleton. It embeds Axon protocol/runtime primitives inside
// `easynet-daemon`, while EasyNet runtime admission remains daemon-owned
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

pub mod admission;
#[cfg(feature = "axon-pb")]
pub mod bidi;
#[cfg(feature = "axon-pb")]
pub(crate) mod caller_signature;
pub(crate) mod causal_context_projection;
pub mod dispatch;
pub mod receipts;
pub mod routing;
#[cfg(feature = "axon-pb")]
pub mod streams;

#[cfg(feature = "axon-pb")]
pub use crate::daemon::boot::invocation::{
    build_invocation_federation_runtime, start_daemon_invocation_transport,
    InvocationFederationRuntime, InvocationTransportDependencies, SessionShutdown,
};
#[cfg(feature = "axon-pb")]
pub use admission::admission_facade::AdmissionFacade;
#[cfg(feature = "axon-pb")]
pub use admission::list_user_pubkeys::ABILITY_IDENTITY_LIST_USER_PUBKEYS;
#[cfg(feature = "axon-pb")]
pub use admission::register_device_pubkey::ABILITY_IDENTITY_REGISTER_PUBKEY;
#[cfg(feature = "axon-pb")]
pub use admission::revoke_user_pubkey::ABILITY_IDENTITY_REVOKE_USER_PUBKEY;
#[cfg(feature = "axon-pb")]
pub use bidi::session_wire::SessionDispatch;
#[cfg(feature = "axon-pb")]
pub use dispatch::daemon_invocation_service::DaemonInvocationService;
#[cfg(feature = "axon-pb")]
pub use dispatch::invocation_wire::{
    InvocationDerivationPolicy, ProtoEnvelope, RootInvocationDerivationIssuer, DEFAULT_URA_PROFILE,
};
#[cfg(feature = "axon-pb")]
pub use dispatch::{
    CallerSignatureMaterial, DaemonInvocation, DaemonInvocationBuilder, InvocationArgsSet,
    InvocationArgsUnset, InvocationDraft, InvocationTuple,
    KeyServiceProviderManagedInvocationSigner, PrepareOptions, PreparedInvocation,
    ProviderManagedInvocationSigner, SignedInvocation, SignerPolicy, SignerPolicyMode,
    SigningMaterial,
};
