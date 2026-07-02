// EasyNet CLI — KernelApi (syscall boundary)
// ===========================================
//
// File: src/daemon/kernel/api.rs
// Description: The trait that is the *only* surface by which the
//              Control layer and daemon schedulers reach into the
//              daemon execution kernel. Every method is the daemon
//              analogue of a Linux syscall: a narrow,
//              typed, audited entry point.
//
// Layering rule
// -------------
// The plan v10.1–v10.3 pins two hard trait boundaries:
//   * KernelApi      — daemon control/schedulers ↔ daemon kernel
//   * GatewayApi     — daemon kernel ↔ federation/network adapter
//
// CI enforcement lives in `engineering/scripts/check-kernel-boundary.sh`:
// anything under `src/daemon/control/` may import
// `crate::daemon::kernel::api`, `crate::daemon::invocation::runtime_record`,
// `crate::core::domain`, and nothing else from `crate::runtime`.
//
// Why v1 KernelApi is still thin
// ------------------------------
// The trait itself is the immovable daemon-kernel surface. A reviewer
// checking "does this PR keep the Control boundary clean?" reads this
// file, counts the methods, confirms their domain-object signatures,
// and moves on.
//
// v10.3 C* unity: every execution entry point ultimately funnels
// through `invoke`. Schedule tick, loop iteration, permission
// admission, Client FFI — all construct a RuntimeInvocation adapter
// record and call
// `invoke`. No bypass paths.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use crate::core::domain::{
    DiscussRoom, LoopId, LoopInstance, PermissionDecision, PermissionId, PermissionRequest, RoomId,
    ScheduleEntry, ScheduleId, Session, SessionId,
};
use crate::daemon::invocation::runtime_record::{Receipt, RuntimeInvocation};
use serde_json::Value;

/// v1 KernelApi surface. Each method is the daemon-kernel analogue of a
/// Linux syscall; the feature PRs add implementations on the Kernel
/// struct that owns sub-service handles.
///
/// v1 methods are synchronous-returning `Result<_>` where blocking
/// I/O is expected; async versions will be added in v2 alongside
/// the planner.
///
/// ## Subscription / streaming note
/// v1 does not expose tokio-stream types across this trait to keep
/// the boundary simple; PR-ATTACH / PR-PERM / PR-DISCUSS / PR-LOOP
/// will each add their own `subscribe_*` method that returns a
/// feature-specific typed channel. The trait surface grows PR by
/// PR — the method *placeholders* listed here are the v1 floor.
pub trait KernelApi: Send + Sync {
    // ── RuntimeInvocation adapter (C* unity entry point) ─────────────

    /// The single execution entry point. Schedule tick, loop
    /// controller, permission admission, Client FFI all construct
    /// a RuntimeInvocation adapter record and call this method. Returns
    /// the terminal Receipt.
    ///
    /// v1 signature returns `Result<Receipt>` synchronously; v2
    /// async version returns `impl Future<Output = Receipt>`.
    fn invoke(&self, invocation: RuntimeInvocation) -> anyhow::Result<Receipt>;

    // ── Session (PR-ATTACH) ──────────────────────────────────────────

    /// List active sessions. v1 returns only sessions hosted on the
    /// local node; cross-node listing goes through a `session.list`
    /// ability invocation that itself calls back into `invoke`.
    fn list_active_sessions(&self) -> anyhow::Result<Vec<Session>>;

    /// Fetch a session handle by id, or None when not present.
    fn get_session(&self, id: &SessionId) -> anyhow::Result<Option<Session>>;

    /// Return stored timeline frames for one session. This is the
    /// kernel-mediated read path for execution sub-services that
    /// need to observe a session produced by `invoke` without
    /// importing the session sub-service directly.
    fn session_events(&self, _id: &SessionId, _since_seq: usize) -> anyhow::Result<Vec<Value>> {
        anyhow::bail!("KernelApi::session_events not implemented")
    }

    // ── Permission (PR-PERM) ─────────────────────────────────────────

    /// Return the pending queue of permission requests awaiting a
    /// decision.
    fn pending_permission_requests(&self) -> anyhow::Result<Vec<PermissionRequest>>;

    /// Deliver a decision to the permission broker.
    fn decide_permission(
        &self,
        id: &PermissionId,
        decision: PermissionDecision,
    ) -> anyhow::Result<()>;

    // ── Schedule (PR-SCHED) ──────────────────────────────────────────

    fn list_schedules(&self) -> anyhow::Result<Vec<ScheduleEntry>>;
    fn add_schedule(&self, entry: ScheduleEntry) -> anyhow::Result<ScheduleId>;
    fn remove_schedule(&self, id: &ScheduleId) -> anyhow::Result<()>;
    fn enable_schedule(&self, id: &ScheduleId, enabled: bool) -> anyhow::Result<()>;

    // ── Discuss (PR-DISCUSS) ─────────────────────────────────────────

    fn create_discuss_room(
        &self,
        participants: Vec<String>,
        topic: Option<String>,
    ) -> anyhow::Result<RoomId>;
    fn list_discuss_rooms(&self) -> anyhow::Result<Vec<DiscussRoom>>;

    // ── Loop (PR-LOOP) ───────────────────────────────────────────────

    fn loop_status(&self, id: &LoopId) -> anyhow::Result<Option<LoopInstance>>;
    fn cancel_loop(&self, id: &LoopId) -> anyhow::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke test: the trait is dyn-compatible (object-safe) so a
    /// future Control server can hold it behind `Arc<dyn KernelApi>`.
    /// A method that accidentally breaks object-safety (e.g. a generic
    /// type parameter on the trait) fails to compile this test.
    #[allow(dead_code)]
    fn _kernel_api_is_object_safe(_k: &dyn KernelApi) {}
}
