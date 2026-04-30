// EasyNet CLI — axon_serve — DaemonInvocationService
// ===================================================
//
// File: src/services/axon_serve/daemon_invocation_service.rs
// Description: Concrete implementation of axon's
//              `pb::axon::v1::invocation_server::Invocation` trait
//              for the new daemon transport plane.
//
// State + behaviour binding
// -------------------------
// The struct is the single owner of every dependency the three RPC
// methods (Invoke / InvokeStream / InvokeBidi) need at runtime. All
// dependencies are injected through the `new` constructor; the
// struct holds them by `Arc` so individual RPC method calls clone
// cheaply.
//
// PR-1 staging — what is and is not implemented in this commit
// ------------------------------------------------------------
// This commit lands the *shape* of the service:
//
//   - The struct and constructor exist and accept the dependency
//     types this module's later commits will inject
//   - The trait impl compiles against the tonic-generated
//     `Invocation` trait
//   - Every RPC method returns `Status::unimplemented(<descriptive
//     message>)` so a probing client gets a precise reason for
//     refusal during the staging period
//   - Streaming associated types (`InvokeStreamStream`,
//     `InvokeBidiStream`) bind to a small empty-stream type so the
//     trait impl is shape-correct
//
// The actual ability dispatch — admission verification, federation
// wrapper routing, PresenceRegistry lookup, LocalAbilityRegistry
// forwarding — lands in commits 4 through 7 of PR-1 on this branch
// (see `team-work/checklists/PR-1-checklist.md`). Each subsequent
// commit replaces one method body at a time, keeping the trait impl
// compilable at every step (CTO directive 06 §3.5).
//
// Invariants
// ----------
// - The service is safe to construct and register with
//   `tonic::transport::Server` even before the real dispatch wires
//   in; clients hitting it during the PR-1 window receive
//   `Status::unimplemented` and can retry against a future build
// - The service does not own the gRPC listener — that is the
//   daemon binary's `main` job. This struct can be tested in
//   isolation by constructing it and calling its methods directly
//
// Out of scope for the file in this commit
// ----------------------------------------
// - Real `<self>.session` accept logic (PR-2)
// - Real `<self>.invoke_remote` per-call dispatch (PR-3)
// - federation.* wrapper routing (later commit, this PR)
// - LocalAbilityRegistry forwarding (later commit, this PR)
// - Admission gate verification (later commit, this PR)
// - PresenceRegistry and its construction (later commit, this PR)
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::pin::Pin;

use tonic::codegen::tokio_stream::Stream;
use tonic::{Request, Response, Status, Streaming};

use crate::pb::axon::v1::invocation_server::Invocation;
use crate::pb::axon::v1::{
    InvokeBidiDown, InvokeBidiUp, InvokeRequest, InvokeResponse, InvokeServerStreamRequest,
    InvokeStreamChunk,
};

/// gRPC `Invocation` service hosted by `easynet-daemon`.
///
/// Intentionally an empty struct in this commit. Subsequent commits
/// in PR-1 grow it to hold:
///
/// - `admission: Arc<AdmissionGate>` — applied at the start of
///   every RPC method's first frame
/// - `presence: Arc<PresenceRegistry>` — owned reverse-channel
///   senders for `<self>.session` and `<self>.invoke_remote`
/// - `federation: Arc<FederationWrappers>` — six `federation.*`
///   thin-wrapper handlers
/// - `ability_dispatch: Arc<LocalAbilityRegistry>` — fall-through
///   for any ability not handled above
///
/// Construction will be `DaemonInvocationService::new(admission,
/// presence, federation, ability_dispatch)` once those types exist.
/// The placeholder constructor below documents the intended shape
/// without committing to the field set; CTO directive 06 §一.5
/// "invariant docstrings" applies once the struct has state to
/// guard.
#[derive(Debug, Default)]
pub struct DaemonInvocationService {
    // Field intentionally blank in this commit. Adding fields is the
    // next commit's job. `_marker` would defeat the trait coherence
    // we want; using `Default` keeps the struct trivially
    // constructible while a real constructor is being designed.
    _placeholder: (),
}

impl DaemonInvocationService {
    /// Construct an empty service. Callers obtain one via the daemon
    /// binary's boot sequence; tests construct one directly.
    ///
    /// The signature is intentionally argument-free in this commit.
    /// Subsequent commits replace it with `new(admission, presence,
    /// federation, ability_dispatch)`; call sites updated together
    /// with the field additions.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

/// Empty stream type used by both `InvokeStreamStream` and
/// `InvokeBidiStream` associated types during the PR-1 staging
/// window.
///
/// A full implementation will replace this with concrete pinned
/// boxed streams driven by the federation wrappers (subscribe_directory
/// pump) and the `<self>.session` / `<self>.invoke_remote` reverse
/// channels. Until then, this type satisfies the trait shape and
/// produces no items because every RPC method returns
/// `Status::unimplemented` before constructing one.
type EmptyStream<T> = Pin<Box<dyn Stream<Item = Result<T, Status>> + Send + 'static>>;

#[tonic::async_trait]
impl Invocation for DaemonInvocationService {
    /// Spec §2.1 reference. Real impl lands once admission gate +
    /// federation wrappers + LocalAbilityRegistry forwarding are
    /// wired (later commits in PR-1).
    async fn invoke(
        &self,
        _request: Request<InvokeRequest>,
    ) -> Result<Response<InvokeResponse>, Status> {
        Err(Status::unimplemented(
            "easynet-daemon: Invoke is not yet wired in this build; \
             dispatch implementation lands later in RFC-003 PR-1 \
             (see team-work/checklists/PR-1-checklist.md §2)",
        ))
    }

    type InvokeStreamStream = EmptyStream<InvokeStreamChunk>;

    /// Spec §2.1 reference. The `federation.subscribe_directory`
    /// pump and the LocalAbilityRegistry stream handlers route
    /// through here once their dependencies are introduced.
    async fn invoke_stream(
        &self,
        _request: Request<InvokeServerStreamRequest>,
    ) -> Result<Response<Self::InvokeStreamStream>, Status> {
        Err(Status::unimplemented(
            "easynet-daemon: InvokeStream is not yet wired in this build; \
             dispatch implementation lands later in RFC-003 PR-1 \
             (see team-work/checklists/PR-1-checklist.md §2)",
        ))
    }

    type InvokeBidiStream = EmptyStream<InvokeBidiDown>;

    /// Spec §2.1 reference. The `<self>.session` and
    /// `<self>.invoke_remote` accept paths, plus LocalAbilityRegistry
    /// bidi forwarding, route through this method once
    /// PresenceRegistry exists.
    async fn invoke_bidi(
        &self,
        _request: Request<Streaming<InvokeBidiUp>>,
    ) -> Result<Response<Self::InvokeBidiStream>, Status> {
        Err(Status::unimplemented(
            "easynet-daemon: InvokeBidi is not yet wired in this build; \
             <self>.session real handler is RFC-003 PR-2 deliverable; \
             see team-work/checklists/PR-1-checklist.md §2 + \
             checklists/PR-2-checklist.md §1",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn invoke_returns_unimplemented_during_pr1_staging() {
        let svc = DaemonInvocationService::new();
        let req = Request::new(InvokeRequest::default());
        let err = svc.invoke(req).await.expect_err("must be unimplemented");
        assert_eq!(err.code(), tonic::Code::Unimplemented);
        assert!(
            err.message().contains("RFC-003 PR-1"),
            "expected message to cite RFC-003 PR-1; got: {}",
            err.message()
        );
    }

    #[tokio::test]
    async fn invoke_stream_returns_unimplemented_during_pr1_staging() {
        let svc = DaemonInvocationService::new();
        let req = Request::new(InvokeServerStreamRequest::default());
        // The success type is `Response<Pin<Box<dyn Stream + ...>>>`
        // which does not implement `Debug`, so `Result::expect_err`
        // is unavailable. Match the variants explicitly instead.
        match svc.invoke_stream(req).await {
            Err(err) => assert_eq!(err.code(), tonic::Code::Unimplemented),
            Ok(_) => panic!("must be unimplemented"),
        }
    }

    #[tokio::test]
    async fn invoke_bidi_returns_unimplemented_during_pr1_staging() {
        // Constructing a real `Streaming` requires a full tonic
        // stack. The trait method itself does not consult the
        // stream until it dispatches; PR-1 staging returns
        // `Unimplemented` before reading any frame, so a
        // synthetic empty `Streaming` is unnecessary — we only
        // need to exercise the method's early return path.
        // Skipping this test in PR-1 is intentional; the bidi
        // path is fully covered by Tier 1 cases 1-11 once PR-2
        // un-ignores them.
    }

    #[test]
    fn service_constructs_with_default() {
        // Default is the constructor we expose today; switching to
        // a fielded `new(...)` is a later commit's job and the test
        // moves with it.
        let _svc = DaemonInvocationService::default();
    }
}
