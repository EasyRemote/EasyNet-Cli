// EasyNet CLI — Ability Proxy (Control-plane → KernelApi adapter)
// ================================================================
//
// File: src/services/control/ability_proxy.rs
// Description: Frame-level adapter between the Control plane's wire
//              messages (`RequestFrame` / `ResponseFrame`) and the
//              runtime's typed `KernelApi`. Every wire verb lands
//              on exactly one method here; every method routes to
//              `Kernel::invoke` or a Kernel query and produces a
//              response frame for the caller.
//
// Layering rule (enforced by scripts/check-kernel-boundary.sh)
// ------------------------------------------------------------
// This file is the *only* legal place in `src/services/control/`
// to import from `crate::runtime::*`. It must import:
//   * `crate::runtime::kernel_api::KernelApi`      (entry point)
//   * `crate::runtime::invocation::{Invocation, Receipt, ...}` (values)
//   * `crate::runtime::domain::{...}`              (typed ids)
//
// It must NOT import:
//   * `crate::runtime::gateway*` (Execution → Gateway boundary is
//     internal to the runtime; Control has no business reaching past
//     Kernel)
//   * `crate::runtime::execution::*` (sub-service internals — Control
//     talks through Kernel, not sub-services directly)
//
// v10.3 C* unity reminder
// -----------------------
// When a RequestFrame::Invoke arrives, the proxy builds an
// `Invocation` from the wire fields (filling in caller from the
// daemon's own node identity, nonce from `fresh_nonce_hex()`, and
// causal_context from the hint field if present, else `Null`) and
// passes it to `Kernel::invoke`. There are no other execution paths.
//
// v1 status — skeleton
// --------------------
// Signatures + stub bodies. Real wiring (admission dedup + proto-
// JSON canonicalisation + in-flight stream registry) lands in
// PR-INVOCATION-EXEC-UNITY and PR-SYS.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::Arc;

use crate::runtime::kernel_api::KernelApi;
use crate::services::control::frames::{codes, IncomingFrame, OutgoingFrame};

/// Stateless-on-construction adapter. Holds an `Arc<dyn KernelApi>`
/// so the server can spawn per-connection tasks that each clone the
/// Arc without owning the Kernel.
#[derive(Clone)]
pub struct AbilityProxy {
    kernel: Arc<dyn KernelApi>,
}

impl AbilityProxy {
    pub fn new(kernel: Arc<dyn KernelApi>) -> Self {
        Self { kernel }
    }

    /// Route one incoming frame through the Kernel and produce the
    /// corresponding outgoing frame. v1 is a skeleton: every frame
    /// returns an explicit "not yet wired" `Error` envelope so
    /// misuse is loud. Real dispatch (build Invocation →
    /// `kernel.invoke` → encode Receipt to `OutgoingFrame::Result`)
    /// lands in PR-INVOCATION-EXEC-UNITY.
    pub fn handle(&self, req: IncomingFrame) -> OutgoingFrame {
        let (request_id, subscription_id) = match &req {
            IncomingFrame::Invoke { request_id, .. } => (Some(request_id.clone()), None),
            IncomingFrame::Subscribe {
                subscription_id, ..
            }
            | IncomingFrame::Cancel { subscription_id } => (None, Some(subscription_id.clone())),
        };
        OutgoingFrame::Error {
            request_id,
            subscription_id,
            code: codes::ABILITY_FAILED.into(),
            message: "AbilityProxy::handle is a skeleton in v1 of PR-DAEMON; \
                      PR-INVOCATION-EXEC-UNITY lands the real dispatch"
                .into(),
        }
    }

    /// Accessor used by tests + the server accept-loop to borrow the
    /// held Kernel handle (for, e.g., pushing lifecycle events).
    #[allow(dead_code)]
    pub(crate) fn kernel(&self) -> &Arc<dyn KernelApi> {
        &self.kernel
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::domain::{
        DiscussRoom, LoopId, LoopInstance, PermissionDecision, PermissionId, PermissionRequest,
        RoomId, ScheduleEntry, ScheduleId, Session, SessionId,
    };
    use crate::runtime::invocation::{Invocation, Receipt};

    /// Minimum KernelApi impl for proxy-level tests; nothing reaches
    /// a real runtime because v1 proxy is a skeleton that does not
    /// call into the Kernel yet.
    struct StubKernel;

    impl KernelApi for StubKernel {
        fn invoke(&self, _inv: Invocation) -> anyhow::Result<Receipt> {
            anyhow::bail!("StubKernel: invoke not wired")
        }
        fn list_active_sessions(&self) -> anyhow::Result<Vec<Session>> {
            Ok(Vec::new())
        }
        fn get_session(&self, _id: &SessionId) -> anyhow::Result<Option<Session>> {
            Ok(None)
        }
        fn pending_permission_requests(&self) -> anyhow::Result<Vec<PermissionRequest>> {
            Ok(Vec::new())
        }
        fn decide_permission(
            &self,
            _id: &PermissionId,
            _decision: PermissionDecision,
        ) -> anyhow::Result<()> {
            Ok(())
        }
        fn list_schedules(&self) -> anyhow::Result<Vec<ScheduleEntry>> {
            Ok(Vec::new())
        }
        fn add_schedule(&self, _e: ScheduleEntry) -> anyhow::Result<ScheduleId> {
            anyhow::bail!("StubKernel: add_schedule not wired")
        }
        fn remove_schedule(&self, _id: &ScheduleId) -> anyhow::Result<()> {
            Ok(())
        }
        fn enable_schedule(&self, _id: &ScheduleId, _enabled: bool) -> anyhow::Result<()> {
            Ok(())
        }
        fn create_discuss_room(
            &self,
            _ps: Vec<String>,
            _topic: Option<String>,
        ) -> anyhow::Result<RoomId> {
            anyhow::bail!("StubKernel: create_discuss_room not wired")
        }
        fn list_discuss_rooms(&self) -> anyhow::Result<Vec<DiscussRoom>> {
            Ok(Vec::new())
        }
        fn loop_status(&self, _id: &LoopId) -> anyhow::Result<Option<LoopInstance>> {
            Ok(None)
        }
        fn cancel_loop(&self, _id: &LoopId) -> anyhow::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn handle_returns_error_frame_for_invoke_with_request_id_preserved() {
        let p = AbilityProxy::new(Arc::new(StubKernel));
        let resp = p.handle(IncomingFrame::Invoke {
            request_id: "abc".into(),
            ability: "system.ping".into(),
            args: serde_json::json!({}),
        });
        match resp {
            OutgoingFrame::Error {
                request_id,
                subscription_id,
                code,
                ..
            } => {
                assert_eq!(request_id.as_deref(), Some("abc"));
                assert!(subscription_id.is_none());
                assert_eq!(code, super::codes::ABILITY_FAILED);
            }
            other => panic!("expected Error frame, got {other:?}"),
        }
    }

    #[test]
    fn handle_preserves_subscription_id_for_subscribe_and_cancel() {
        // The `subscription_id` on the response frame must equal the
        // one the client sent. A regression that dropped the id or
        // filled in a fresh one would break stream correlation on
        // every streaming path.
        let p = AbilityProxy::new(Arc::new(StubKernel));
        for (req, expected) in [
            (
                IncomingFrame::Subscribe {
                    subscription_id: "sub-1".into(),
                    ability: "system.session.attach".into(),
                    args: serde_json::json!({}),
                },
                "sub-1",
            ),
            (
                IncomingFrame::Cancel {
                    subscription_id: "cancel-1".into(),
                },
                "cancel-1",
            ),
        ] {
            let resp = p.handle(req);
            match resp {
                OutgoingFrame::Error {
                    subscription_id,
                    request_id,
                    ..
                } => {
                    assert_eq!(subscription_id.as_deref(), Some(expected));
                    assert!(request_id.is_none());
                }
                other => panic!("expected Error frame, got {other:?}"),
            }
        }
    }
}
