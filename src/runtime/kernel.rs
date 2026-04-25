// EasyNet CLI — Kernel (single execution entry point)
// =====================================================
//
// File: src/runtime/kernel.rs
// Description: The Kernel holds one handle per Execution sub-service
//              plus a Gateway handle; every KernelApi method routes
//              through the Kernel. Kernel::invoke is the single
//              execution entry that v10.3 C* pins as the only path
//              into the runtime.
//
// v1 state
// --------
// This is a skeleton. The constructor returns a Kernel holding
// `SessionService::default()` etc. — the methods that do real work
// ship in the feature PRs. The one piece that is wired today is
// `Kernel::invoke`: it performs the v1 admission phase (nonce
// generation + canonicalisation) and returns a placeholder
// Receipt. PR-INVOCATION-EXEC-UNITY will replace the placeholder
// with the full admission → dispatch → terminal flow.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::Arc;

use serde_json::json;

use crate::runtime::domain::{
    AgentId, DiscussRoom, LoopId, LoopInstance, NodeId, PermissionDecision, PermissionId,
    PermissionRequest, RoomId, ScheduleEntry, ScheduleId, Session, SessionId, TenantId,
};
use crate::runtime::execution::{
    discuss::DiscussService, loop_instance::LoopService, permission::PermissionService,
    schedule::ScheduleService, session::SessionService,
};
use crate::runtime::gateway_api::GatewayApi;
use crate::runtime::invocation::{invocation_id_of, Invocation, PriorChain, Receipt, TerminalState};
use crate::runtime::kernel_api::KernelApi;

/// The runtime kernel. Holds one sub-service per feature and one
/// Gateway handle for federation calls. Feature PRs extend the
/// sub-service handles; the Kernel itself stays thin — it is a
/// router, not a state owner.
///
/// Sub-services are held as `Arc` so the same handle can be
/// shared with the `runtime::system::*` ability handlers without
/// the Kernel having to re-vend through method delegation. The
/// daemon bin builds the registry off the same Arcs the Kernel
/// holds, so dispatch and direct KernelApi calls observe one
/// state.
pub struct Kernel {
    session: Arc<SessionService>,
    permission: Arc<PermissionService>,
    discuss: Arc<DiscussService>,
    schedule: Arc<ScheduleService>,
    loop_svc: Arc<LoopService>,
    #[allow(dead_code)]
    gateway: Arc<dyn GatewayApi>,
}

impl Kernel {
    /// Construct a Kernel backed by fresh sub-services and the
    /// provided Gateway.
    pub fn new(gateway: Arc<dyn GatewayApi>) -> Self {
        Self {
            session: Arc::new(SessionService::new()),
            permission: Arc::new(PermissionService::new()),
            discuss: Arc::new(DiscussService::new()),
            schedule: Arc::new(ScheduleService::new()),
            loop_svc: Arc::new(LoopService::new()),
            gateway,
        }
    }

    /// Borrow the SessionService handle. Used by the daemon bin's
    /// boot path to share the same Arc into the system ability
    /// registry.
    pub fn session_service(&self) -> Arc<SessionService> {
        Arc::clone(&self.session)
    }

    pub fn permission_service(&self) -> Arc<PermissionService> {
        Arc::clone(&self.permission)
    }

    pub fn discuss_service(&self) -> Arc<DiscussService> {
        Arc::clone(&self.discuss)
    }

    pub fn schedule_service(&self) -> Arc<ScheduleService> {
        Arc::clone(&self.schedule)
    }

    pub fn loop_service(&self) -> Arc<LoopService> {
        Arc::clone(&self.loop_svc)
    }

    /// Real agent-chat dispatch. Looks up `agent_name` in the local
    /// registry, calls `runtime::dispatch::send_external` (which
    /// shells out to the configured driver — claude / codex /
    /// codex-app-server). Streams driver progress as `kind: progress`
    /// session events so a Client subscribed to system.session.attach
    /// for this invocation_id sees the run unfold live.
    ///
    /// v1 streams progress in coarse grain only (a single
    /// `agent_response` event with the full markdown body) because
    /// `send_external` is synchronous. A finer-grained per-token
    /// stream would require teaching the driver layer about an
    /// SSE-style callback into SessionService — that lands when the
    /// driver crate gets its async surface.
    fn dispatch_agent_chat(
        &self,
        session_id: &SessionId,
        agent_name: &str,
        args: &serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let prompt = args
            .get("prompt")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                anyhow::anyhow!("agent.chat: `prompt` (string) required in args")
            })?;
        let context = args.get("context").and_then(serde_json::Value::as_str);

        let registry = crate::registry::agents::load_agents()
            .map_err(|e| anyhow::anyhow!("agent registry load failed: {e}"))?;
        let entry = registry
            .agents
            .get(agent_name)
            .cloned()
            .ok_or_else(|| {
                anyhow::anyhow!("agent {agent_name:?} not registered in this daemon")
            })?;

        // Surface a 200-char preview of the prompt so a Client UI
        // can see the rendered template in the timeline (cron with
        // a "Daily report on {{date}}" template renders here as
        // "Daily report on 2026-04-25", which is what the user
        // configured). Truncated to keep the event size bounded
        // for long prompts.
        let preview: String = prompt.chars().take(200).collect();
        let _ = self.session.emit_event(
            session_id,
            json!({
                "kind": "agent_dispatch_starting",
                "agent": agent_name,
                "prompt_len": prompt.len(),
                "prompt_preview": preview,
            }),
        );

        // send_external runs synchronously (subprocess + wait). The
        // Kernel is called from the proxy's tokio worker thread; we
        // don't want to block the entire worker for a 30-second
        // agent run. Defer the blocking call to a dedicated thread
        // via `tokio::task::block_in_place` if a runtime is in
        // scope; otherwise call directly.
        let response_result = if tokio::runtime::Handle::try_current().is_ok() {
            tokio::task::block_in_place(|| {
                crate::runtime::dispatch::send_external(agent_name, &entry, prompt, context)
            })
        } else {
            crate::runtime::dispatch::send_external(agent_name, &entry, prompt, context)
        };

        match response_result {
            Ok(resp) => {
                let usage = resp
                    .usage
                    .as_ref()
                    .map(|u| {
                        json!({
                            "input_tokens": u.input_tokens,
                            "output_tokens": u.output_tokens,
                            "num_turns": u.num_turns,
                            "total_cost_usd": u.total_cost_usd,
                        })
                    })
                    .unwrap_or(serde_json::Value::Null);
                let _ = self.session.emit_event(
                    session_id,
                    json!({
                        "kind": "agent_response",
                        "content": resp.content,
                        "model": resp.model,
                        "duration_ms": resp.duration_ms,
                        "truncated": resp.truncated,
                        "usage": usage,
                    }),
                );
                Ok(json!({
                    "agent": resp.agent,
                    "content": resp.content,
                    "duration_ms": resp.duration_ms,
                }))
            }
            Err(e) => {
                let _ = self.session.emit_event(
                    session_id,
                    json!({
                        "kind": "agent_error",
                        "error": format!("{e}"),
                    }),
                );
                Err(e)
            }
        }
    }
}

/// Parse `<agent>.chat` ability names. Returns `Some(<agent>)`
/// for the chat-style ability that Kernel::invoke routes through
/// `dispatch::send_external`; returns `None` for anything else
/// (system.*, future ability namespaces).
fn parse_agent_chat(ability: &str) -> Option<String> {
    if ability.starts_with("system.") {
        return None;
    }
    let (head, tail) = ability.rsplit_once('.')?;
    if tail != "chat" {
        return None;
    }
    if head.is_empty() {
        return None;
    }
    Some(head.to_string())
}

impl KernelApi for Kernel {
    fn invoke(&self, invocation: Invocation) -> anyhow::Result<Receipt> {
        // Plan v10.3 C* unity entry. Three phases:
        //   1. Admission — compute invocation_id, register a Session
        //      keyed by that id so live attachers see the run from
        //      its first frame.
        //   2. Dispatch — branch on the ability shape:
        //        * `<agent>.chat`        → real agent subprocess via
        //                                  runtime::dispatch::send_external
        //        * `system.*`            → reserved (system abilities
        //                                  are dispatched via the
        //                                  proxy's stage-2 executor;
        //                                  Kernel::invoke for them is
        //                                  redundant in v1)
        //        * anything else         → Failed(NotFound)
        //   3. Terminal — emit `kind: terminated`, mark the session
        //      ended, return the Receipt.
        let id = invocation_id_of(&invocation);
        let session_id = SessionId::new(id.clone());
        let agent_name = parse_agent_chat(&invocation.ability);
        let admit = Session {
            id: session_id.clone(),
            agent: AgentId::new(agent_name.clone().unwrap_or_else(|| "?".into())),
            node: NodeId::new("self"),
            tenant: TenantId::default_v1(),
            started_unix_ms: chrono::Utc::now().timestamp_millis(),
            ended_unix_ms: None,
        };
        // Idempotent admit: if a prior call admitted the same id
        // already (replay) we reuse the existing entry.
        let _ = self.session.admit(admit);
        let _ = self.session.emit_event(
            &session_id,
            json!({
                "kind": "invoke_admitted",
                "ability": invocation.ability,
                "caller": invocation.caller,
                "callee": invocation.callee,
            }),
        );

        let outcome: anyhow::Result<serde_json::Value> = match agent_name {
            Some(name) => self.dispatch_agent_chat(&session_id, &name, &invocation.args),
            None => {
                // System abilities are not dispatched through Kernel::invoke
                // in v1 — the proxy executes them directly. We accept
                // them here for "I just want a receipt without doing
                // anything" callers.
                Ok(json!({"note": "no-op kernel.invoke for non-agent ability"}))
            }
        };

        let now_ms = chrono::Utc::now().timestamp_millis();
        let terminal = match &outcome {
            Ok(_) => TerminalState::Succeeded,
            Err(e) => TerminalState::Failed { reason: format!("{e}") },
        };
        let _ = self.session.emit_event(
            &session_id,
            json!({
                "kind": "invoke_terminal",
                "outcome": match &outcome {
                    Ok(v) => json!({"ok": v}),
                    Err(e) => json!({"err": format!("{e}")}),
                },
            }),
        );
        let _ = self.session.terminate(&session_id, now_ms);

        Ok(Receipt {
            invocation_id: id,
            terminal,
            events: Vec::new(),
            prior: PriorChain::None,
            callee_signature: None,
        })
    }

    fn list_active_sessions(&self) -> anyhow::Result<Vec<Session>> {
        Ok(self.session.list_active())
    }

    fn get_session(&self, id: &SessionId) -> anyhow::Result<Option<Session>> {
        Ok(self.session.get(id))
    }

    fn pending_permission_requests(&self) -> anyhow::Result<Vec<PermissionRequest>> {
        Ok(self.permission.pending())
    }

    fn decide_permission(
        &self,
        id: &PermissionId,
        decision: PermissionDecision,
    ) -> anyhow::Result<()> {
        self.permission.decide(id, decision)?;
        Ok(())
    }

    fn list_schedules(&self) -> anyhow::Result<Vec<ScheduleEntry>> {
        Ok(self.schedule.list())
    }

    fn add_schedule(&self, entry: ScheduleEntry) -> anyhow::Result<ScheduleId> {
        self.schedule.add(entry)
    }

    fn remove_schedule(&self, id: &ScheduleId) -> anyhow::Result<()> {
        self.schedule.remove(id)
    }

    fn enable_schedule(&self, id: &ScheduleId, enabled: bool) -> anyhow::Result<()> {
        self.schedule.enable(id, enabled)
    }

    fn create_discuss_room(
        &self,
        participants: Vec<String>,
        topic: Option<String>,
    ) -> anyhow::Result<RoomId> {
        self.discuss.create(participants, topic)
    }

    fn list_discuss_rooms(&self) -> anyhow::Result<Vec<DiscussRoom>> {
        Ok((*self.discuss).list())
    }

    fn loop_status(&self, id: &LoopId) -> anyhow::Result<Option<LoopInstance>> {
        Ok(self.loop_svc.status(id))
    }

    fn cancel_loop(&self, id: &LoopId) -> anyhow::Result<()> {
        self.loop_svc.cancel(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::gateway_api::{PeerInfo, RemoteTarget};
    use crate::runtime::invocation::{CausalContext, Invocation};
    use serde_json::json;

    struct NoopGateway;

    impl GatewayApi for NoopGateway {
        fn publish_ability(
            &self,
            _name: &str,
            _description: &str,
            _schema: &serde_json::Value,
        ) -> anyhow::Result<()> {
            Ok(())
        }
        fn invoke_remote_ability(
            &self,
            _target: &RemoteTarget,
            _args: &serde_json::Value,
        ) -> anyhow::Result<serde_json::Value> {
            Ok(serde_json::Value::Null)
        }
        fn subscribe_remote_ability(
            &self,
            _target: &RemoteTarget,
            _args: &serde_json::Value,
            _on_frame: Box<dyn FnMut(serde_json::Value) + Send>,
        ) -> anyhow::Result<()> {
            Ok(())
        }
        fn list_peers(&self) -> anyhow::Result<Vec<PeerInfo>> {
            Ok(Vec::new())
        }
        fn send_heartbeat(&self) -> anyhow::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn kernel_invoke_returns_receipt_with_matching_id_for_same_invocation() {
        // The v1 stub must at minimum return a Receipt whose
        // invocation_id matches `invocation_id_of(&inv)`.
        let k = Kernel::new(Arc::new(NoopGateway));
        let inv = Invocation {
            caller: "easynet://nodes/a".into(),
            callee: "easynet://nodes/b".into(),
            ability: "system.ping".into(),
            subject: "easynet://nodes/b".into(),
            nonce_hex: "aa".repeat(16),
            causal_context: CausalContext::Null,
            args: json!({}),
            caller_signature: None,
        };
        let expected_id = invocation_id_of(&inv);
        let r = k.invoke(inv).unwrap();
        assert_eq!(r.invocation_id, expected_id);
        assert!(matches!(r.terminal, TerminalState::Succeeded));
    }

    #[test]
    fn parse_agent_chat_recognises_dot_chat_suffix() {
        assert_eq!(parse_agent_chat("alice.chat"), Some("alice".to_string()));
        assert_eq!(
            parse_agent_chat("claude-code.chat"),
            Some("claude-code".to_string())
        );
        assert_eq!(parse_agent_chat("a.b.chat"), Some("a.b".to_string()));
    }

    #[test]
    fn parse_agent_chat_rejects_system_and_non_chat() {
        // system.* abilities never go through the agent-dispatch
        // path; they are handled by the proxy's stage-2 executor.
        assert_eq!(parse_agent_chat("system.session.attach"), None);
        assert_eq!(parse_agent_chat("system.ping"), None);
        // Anything that doesn't end in `.chat` is not the agent-chat
        // shape Kernel::invoke knows about.
        assert_eq!(parse_agent_chat("alice.voice"), None);
        assert_eq!(parse_agent_chat("alice"), None);
        assert_eq!(parse_agent_chat(".chat"), None);
        assert_eq!(parse_agent_chat(""), None);
    }

    #[test]
    fn invoke_with_unknown_agent_returns_failed_receipt() {
        // An invocation against an agent the registry does not
        // know lands as Failed with a clear reason. This is the
        // contract a Client uses to render a "no such agent"
        // dialog rather than spinning forever.
        let k = Kernel::new(Arc::new(NoopGateway));
        // HomeGuard so the registry lookup hits an empty per-test
        // config dir, not whatever agents.json the developer has
        // installed locally.
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        let inv = Invocation {
            caller: "easynet://nodes/a".into(),
            callee: "easynet://nodes/a".into(),
            ability: "ghost-agent.chat".into(),
            subject: "easynet://nodes/a".into(),
            nonce_hex: "00".repeat(16),
            causal_context: CausalContext::Null,
            args: json!({"prompt": "hi"}),
            caller_signature: None,
        };
        let r = k.invoke(inv).unwrap();
        match r.terminal {
            TerminalState::Failed { reason } => {
                assert!(
                    reason.contains("ghost-agent") || reason.contains("not registered"),
                    "expected agent-not-registered reason; got {reason}"
                );
            }
            other => panic!("expected Failed receipt, got {other:?}"),
        }
    }
}
