// EasyNet CLI — Domain Object Model
// ==================================
//
// File: src/runtime/domain.rs
// Description: Typed domain objects that live at the KernelApi
//              boundary. These are the shapes a future Client FFI
//              / planner / sub-service sees instead of raw
//              `args_json: Value` fragments.
//
// Why this file exists
// --------------------
// Plan v10.1 compels the KernelApi trait to speak in domain
// objects (Session / PermissionRequest / DiscussRoom /
// ScheduleEntry / LoopInstance / InvocationRecord), not
// `(ability_name: &str, args: Value)` tuples. Stuffing them all
// into one small module keeps the surface visible in one place:
// when a new feature PR lands, it adds its handle type here, not
// in five separate files.
//
// Relationship to the feature PRs
// -------------------------------
// Each feature PR adds methods / fields to the handle that
// belongs to it:
//   * PR-ATTACH   → Session
//   * PR-PERM     → PermissionRequest
//   * PR-DISCUSS  → DiscussRoom
//   * PR-SCHED    → ScheduleEntry
//   * PR-LOOP     → LoopInstance
// InvocationRecord is a cross-cutting trace handle used by every
// feature and by the future planner's audit surface.
//
// The v1 stubs below are intentionally skeletal. They pin the
// identity shapes (SessionId, RoomId, ScheduleId, LoopId) and the
// minimum fields a KernelApi method signature needs to declare.
// Feature PRs flesh them out without renaming the types, so every
// reviewer looking for "what is a Session" lands here first.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use serde::{Deserialize, Serialize};

/// Newtype wrappers over the string ids the runtime already uses,
/// so KernelApi method signatures are self-documenting. Conversion
/// to/from the underlying `String` is explicit via `into()` or
/// `AsRef<str>` rather than transparent, to catch "I passed the
/// wrong id type at the call site" at compile time.
macro_rules! string_newtype {
    ($name:ident) => {
        #[derive(
            Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
        )]
        pub struct $name(String);

        impl $name {
            pub fn new(s: impl Into<String>) -> Self {
                Self(s.into())
            }
            pub fn as_str(&self) -> &str {
                &self.0
            }
            pub fn into_inner(self) -> String {
                self.0
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

string_newtype!(SessionId);
string_newtype!(RoomId);
string_newtype!(ScheduleId);
string_newtype!(LoopId);
string_newtype!(PermissionId);
string_newtype!(NodeId);
string_newtype!(TenantId);
string_newtype!(AgentId);

impl TenantId {
    pub fn default_v1() -> Self {
        // v1 single-active-tenant: storage namespace is reserved
        // under `tenants/default/`. v2 will pass a real id in the
        // handshake. See src/persistence/tenant_paths.rs.
        Self("default".into())
    }
}

/// A live agent run. v1 equivalent of the handle the existing
/// `runtime::session::Session` already owns; the domain object here
/// is the public face that KernelApi hands to a subscriber (or to
/// PR-ATTACH's `fleet.attach_session` ability handler).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: SessionId,
    pub agent: AgentId,
    pub node: NodeId,
    pub tenant: TenantId,
    pub started_unix_ms: i64,
    /// None until the run terminates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_unix_ms: Option<i64>,
}

/// A pending or resolved permission approval. PR-PERM fills in the
/// `decision` field once the broker decides.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRequest {
    pub id: PermissionId,
    pub session: SessionId,
    pub tenant: TenantId,
    pub prompt: String,
    pub sensitivity: PermissionSensitivity,
    pub created_unix_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<PermissionDecision>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionSensitivity {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PermissionDecision {
    Allow,
    Deny,
    AllowOnce,
}

/// A multi-agent discussion room. PR-DISCUSS defines the membership
/// + turn-stream shape; v1 only names the handle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscussRoom {
    pub id: RoomId,
    pub origin_node: NodeId,
    pub tenant: TenantId,
    pub participants: Vec<AgentId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
    pub created_unix_ms: i64,
}

/// One cron schedule entry stored under
/// `~/.easynet/tenants/<tenant>/schedules/<id>.json`. PR-SCHED
/// populates the body; v1 only names the handle + misfire policy
/// enum (which is frozen pre-PR per plan §Misfire policy).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleEntry {
    pub id: ScheduleId,
    pub tenant: TenantId,
    pub target_node: NodeId,
    pub target_agent: AgentId,
    pub cron_expr: String,
    pub misfire_policy: MisfirePolicy,
    /// Optional catch-up window in seconds for
    /// `MisfirePolicy::CatchUpWindowed`. Default 86_400 (24h).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catch_up_window_secs: Option<u64>,
    pub enabled: bool,
    /// Optional prompt template the tick runner sends to the target
    /// agent at fire time. `None` means "fire as a heartbeat" — the
    /// tick runner uses a synthesised "Scheduled fire of …"
    /// placeholder. With a template the cron carries real work.
    ///
    /// Template variables (substituted before dispatch):
    ///   * `{{schedule_id}}`  — the schedule's id
    ///   * `{{fire_at_iso}}`  — ISO-8601 RFC3339 fire timestamp
    ///   * `{{catch_up}}`     — "true" / "false"
    ///   * `{{target_agent}}` — agent name verbatim
    ///
    /// Variables are case-sensitive. Unknown `{{...}}` tokens are
    /// left untouched (no template error) so a typo surfaces as
    /// odd-looking prompt text rather than a hard fail at fire time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
}

/// Misfire policy frozen at plan v10.1 so a production misread
/// ("it skipped our reports!" vs "it ran the report 200 times!")
/// cannot be attributed to an implementation oversight.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MisfirePolicy {
    /// Daemon downtime erases all missed triggers; next tick starts
    /// from "now". Default for v1 report/self-check workloads.
    Skip,
    /// Exactly one catch-up fire on resume, then normal cadence.
    /// Use when "we need to know we missed it" but not "we need to
    /// process every missed hour".
    FireOnce,
    /// Replay every missed trigger within
    /// `catch_up_window_secs` (default 24h). Never unbounded —
    /// `unbounded_catch_up` is explicitly rejected in v1 because
    /// the thundering-herd case is worse than the data-loss case.
    CatchUpWindowed,
}

/// A live Loop instance spawned via PR-LOOP's `loop.create`.
/// The EAL Stage 3 loop executor stays the underlying engine; this
/// handle is the Client-facing identity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopInstance {
    pub id: LoopId,
    pub tenant: TenantId,
    pub worker_agent: AgentId,
    pub verify_expr: String,
    pub body_prompt: String,
    pub max_iters: u32,
    pub current_iter: u32,
    pub state: LoopState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_body_output: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_verify_output: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LoopState {
    Pending,
    Running,
    Done,
    /// Max iterations exceeded without a passing verify.
    Exhausted,
    /// Verify expression returned a malformed value (not boolean-
    /// coercible, per EAL §verify).
    VerifyMalformed,
    Cancelled,
}

/// One audit trace entry for a past Invocation dispatch. Feature
/// PRs append to the trace log; the future planner consumes these
/// to learn "what happened when I tried X".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvocationRecord {
    pub invocation_id: String,
    pub tenant: TenantId,
    pub caller_node: NodeId,
    pub callee_node: NodeId,
    pub ability: String,
    pub started_unix_ms: i64,
    pub ended_unix_ms: Option<i64>,
    pub terminal_kind: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tenant_default_v1_is_literally_default() {
        // v1 pins the tenant string to the literal "default" so the
        // on-disk layout under `tenants/default/` is grep-stable.
        // v2 will route multi-tenant traffic via IPC handshake, but
        // that does not change this default for the active-tenant
        // fallback. Any refactor that renames this string must
        // update path_for_tenant() and every magic "default"
        // reference in the docs at the same time.
        assert_eq!(TenantId::default_v1().as_str(), "default");
    }

    #[test]
    fn id_newtypes_round_trip_through_display() {
        let s = SessionId::new("abc-123");
        assert_eq!(format!("{s}"), "abc-123");
        assert_eq!(s.as_str(), "abc-123");
    }

    #[test]
    fn misfire_policy_serializes_snake_case() {
        // Wire format is snake_case so the future proto enum
        // (SKIP / FIRE_ONCE / CATCH_UP_WINDOWED) lines up with the
        // JSON representation without a translation layer.
        let s = serde_json::to_string(&MisfirePolicy::CatchUpWindowed).unwrap();
        assert_eq!(s, "\"catch_up_windowed\"");
    }
}
