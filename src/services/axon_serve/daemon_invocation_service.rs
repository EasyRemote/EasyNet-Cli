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

use futures::Stream;
use tonic::{Request, Response, Status, Streaming};

use crate::pb::axon::v1::invocation_server::Invocation;
use crate::pb::axon::v1::{
    InvokeBidiDown, InvokeBidiUp, InvokeRequest, InvokeResponse, InvokeServerStreamRequest,
    InvokeStreamChunk,
};

/// gRPC `Invocation` service hosted by `easynet-daemon`.
///
/// Currently an empty struct. Fields land in commit 6/9 once
/// `PresenceRegistry`, the federation wrappers, and the admission
/// gate facade are all available to inject. Final shape:
///
/// ```ignore
/// pub struct DaemonInvocationService {
///     admission: Arc<AdmissionGate>,
///     presence: Arc<PresenceRegistry>,
///     ability_dispatch: Arc<LocalAbilityRegistry>,
/// }
/// ```
///
/// CTO directive 06 §一.5 "invariant docstrings" applies once the
/// struct has state to guard; until then, the only invariant is
/// "trivially constructible".
#[derive(Debug, Default)]
pub struct DaemonInvocationService;

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

    #[ignore = "PR-1 staging — bidi accept/dispatch covered by PR-2 Tier 1 cases 1-11 unignore"]
    #[tokio::test]
    async fn invoke_bidi_test_deferred_to_pr2_tier1() {
        // Constructing a real `tonic::Streaming<InvokeBidiUp>`
        // requires the full tonic codegen scaffolding. The
        // unimplemented path returns before reading any frame,
        // so a synthetic empty `Streaming` would not exercise
        // anything beyond the trait dispatch table — exactly
        // what PR-2 Tier 1 cases 1-11 cover end-to-end via real
        // gRPC roundtrip. Marking this `#[ignore]` so the test
        // result line surfaces the gap rather than passing
        // vacuously.
        unreachable!();
    }

    #[test]
    fn service_constructs_with_default() {
        // Default is the constructor we expose today; switching to
        // a fielded `new(...)` is a later commit's job and the test
        // moves with it.
        let _svc = DaemonInvocationService::default();
    }
}
