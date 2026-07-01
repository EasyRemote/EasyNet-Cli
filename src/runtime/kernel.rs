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
// Invocation state
// ----------------
// `Kernel::invoke` owns the daemon-local admission → permission gate
// → Axon LocalRuntime dispatch → receipt projection flow. It must not
// manufacture success: if LocalRuntime is missing, unsigned external
// callers arrive, or Axon returns a terminal failure, the returned
// Receipt is Failed and carries only the events that actually exist.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::{Arc, OnceLock};

use easynet_axon::invocation::{CallMode as AxonInvocationCallMode, CallerSignature};
use serde_json::json;

use crate::core::domain::{
    AgentId, DiscussRoom, LoopId, LoopInstance, NodeId, PermissionDecision, PermissionId,
    PermissionRequest, PermissionSensitivity, RoomId, ScheduleEntry, ScheduleId, Session,
    SessionId, TenantId,
};
use crate::runtime::axon_bridge::local_runtime_request::{
    LocalRuntimeIngress, LocalRuntimeRequestFactory, LocalRuntimeRequestOptions,
};
use crate::runtime::execution::{
    discuss::DiscussService,
    loop_instance::LoopService,
    permission::{AskContext, PermissionService},
    schedule::ScheduleService,
    session::SessionService,
};
use crate::runtime::gateway_api::GatewayApi;
use crate::runtime::invocation::{
    runtime_invocation_id, PriorChain, Receipt, ReceiptEvent, RuntimeInvocation, TerminalState,
};
use crate::runtime::kernel_api::KernelApi;
use crate::runtime::local_invocation_identity::LOCAL_SYSTEM_AGENT_URA;

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
    /// Shared Axon LocalRuntime, set by daemon boot after the runtime
    /// is constructed. `Kernel::invoke` dispatches through this handle
    /// so daemon-internal invocation sources (schedule ticks, loop
    /// iterations, future controllers) observe the same admission,
    /// state machine, and ledger sink as gRPC Invoke.
    local_runtime: OnceLock<Arc<easynet_axon::invocation::LocalRuntime>>,
}

#[derive(Debug, Clone)]
enum KernelDispatchTerminal {
    Succeeded(serde_json::Value),
    Failed(String),
}

#[derive(Debug, Clone)]
struct KernelDispatchOutcome {
    terminal: KernelDispatchTerminal,
    events: Vec<ReceiptEvent>,
}

impl KernelDispatchOutcome {
    fn succeeded(value: serde_json::Value, events: Vec<ReceiptEvent>) -> Self {
        Self {
            terminal: KernelDispatchTerminal::Succeeded(value),
            events,
        }
    }

    fn failed(reason: impl Into<String>, events: Vec<ReceiptEvent>) -> Self {
        Self {
            terminal: KernelDispatchTerminal::Failed(reason.into()),
            events,
        }
    }

    fn terminal_state(&self) -> TerminalState {
        match &self.terminal {
            KernelDispatchTerminal::Succeeded(_) => TerminalState::from_axon_terminal(
                easynet_axon::invocation::InvocationState::Completed,
                None,
            )
            .expect("Completed is an Axon terminal state"),
            KernelDispatchTerminal::Failed(reason) => TerminalState::from_axon_terminal(
                easynet_axon::invocation::InvocationState::Failed,
                Some(reason.clone()),
            )
            .expect("Failed is an Axon terminal state"),
        }
    }

    fn terminal_event_value(&self) -> serde_json::Value {
        match &self.terminal {
            KernelDispatchTerminal::Succeeded(value) => json!({"ok": value}),
            KernelDispatchTerminal::Failed(reason) => json!({"err": reason}),
        }
    }
}

impl Kernel {
    /// Construct a Kernel backed by fresh sub-services and the
    /// provided Gateway. Uses the AllowAllBroker permission default
    /// — every Kernel::invoke admission auto-allows. Daemons that
    /// want interactive approval should use
    /// `new_with_subscriber_broker` instead so a Client subscribed
    /// to consent.subscribe sees pending requests.
    pub fn new(gateway: Arc<dyn GatewayApi>) -> Self {
        Self {
            session: Arc::new(SessionService::new()),
            permission: Arc::new(PermissionService::new()),
            discuss: Arc::new(DiscussService::new()),
            schedule: Arc::new(ScheduleService::new()),
            loop_svc: Arc::new(LoopService::new()),
            gateway,
            local_runtime: OnceLock::new(),
        }
    }

    /// Construct a Kernel with the SubscriberBroker permission
    /// variant installed — every Kernel::invoke admission against
    /// a non-system ability publishes a PermissionRequest on the
    /// broker's broadcast channel, then blocks waiting for the
    /// matching `consent.decide` decision.
    ///
    /// When no subscriber is connected the broker auto-allows
    /// (per docs/rfc/permission-broker-v1.md §4 cross-machine
    /// advisory downgrade and §6 "no observer means no human in
    /// the loop"). The default `new()` should be preferred for
    /// tests and for daemons running without a Client; the
    /// daemon bin uses this constructor so the Permission tab
    /// in the GUI sees real pending requests.
    pub fn new_with_subscriber_broker(gateway: Arc<dyn GatewayApi>) -> Self {
        Self {
            session: Arc::new(SessionService::new()),
            permission: Arc::new(PermissionService::with_subscriber_broker()),
            discuss: Arc::new(DiscussService::new()),
            schedule: Arc::new(ScheduleService::new()),
            loop_svc: Arc::new(LoopService::new()),
            gateway,
            local_runtime: OnceLock::new(),
        }
    }

    /// Wire the shared Axon runtime into the Kernel post-construction.
    /// Called by daemon boot once `LocalRuntime` exists; ability
    /// registrations can happen before or after this because the Arc
    /// points at the same runtime object.
    pub fn set_local_runtime(&self, runtime: Arc<easynet_axon::invocation::LocalRuntime>) {
        let _ = self.local_runtime.set(runtime);
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

    /// Permission admission gate. Asks the broker; emits a
    /// `permission_pending` event before the call and a
    /// `permission_decided` event after, so a Client subscribed to
    /// session.attach for this invocation_id sees admission
    /// was gated even when the broker auto-allows.
    ///
    /// AllowAllBroker returns immediately. SubscriberBroker
    /// publishes a PermissionRequest on its broadcast channel
    /// (which a Client connected to consent.subscribe
    /// receives live) and blocks `ask` until a matching
    /// `consent.decide` lands or the broker's internal
    /// timeout fires.
    ///
    /// v1 sensitivity is hardcoded to `Medium`. A future config
    /// layer (per-ability or per-agent) will let an operator pin
    /// "always ask" abilities at `High` and demote idempotent
    /// reads to `Low`.
    fn gate_permission(
        &self,
        session_id: &SessionId,
        agent_name: &str,
        args: &serde_json::Value,
    ) -> PermissionDecision {
        let prompt_preview: String = args
            .get("prompt")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .chars()
            .take(120)
            .collect();
        let pending_prompt = format!(
            "{} wants to chat (preview: {})",
            agent_name,
            if prompt_preview.is_empty() {
                "(no prompt)".into()
            } else {
                format!("\"{prompt_preview}\"")
            }
        );
        let _ = self.session.emit_event(
            session_id,
            json!({
                "kind": "permission_pending",
                "agent": agent_name,
                "sensitivity": "medium",
                "prompt_preview": prompt_preview,
            }),
        );
        let ctx = AskContext {
            prompt: pending_prompt,
            sensitivity: PermissionSensitivity::Medium,
            session: session_id.clone(),
            tenant: TenantId::default_v1(),
            capability_claim: None,
        };
        // The broker's `ask` is sync but blocks on a tokio oneshot
        // for the SubscriberBroker variant. block_in_place lets us
        // wait without freezing the tokio worker.
        let decision = if tokio::runtime::Handle::try_current().is_ok() {
            tokio::task::block_in_place(|| self.permission.broker().ask(ctx))
        } else {
            self.permission.broker().ask(ctx)
        };
        let _ = self.session.emit_event(
            session_id,
            json!({
                "kind": "permission_decided",
                "agent": agent_name,
                "decision": match decision {
                    PermissionDecision::Allow => "allow",
                    PermissionDecision::Deny => "deny",
                    PermissionDecision::AllowOnce => "allow_once",
                },
            }),
        );
        decision
    }

    /// Dispatch a daemon runtime invocation through Axon's public
    /// descriptor-bound LocalRuntime path.
    fn dispatch_via_local_runtime(
        &self,
        session_id: &SessionId,
        invocation: &RuntimeInvocation,
    ) -> KernelDispatchOutcome {
        let runtime = match self.local_runtime.get() {
            Some(runtime) => runtime,
            None => {
                return KernelDispatchOutcome::failed(
                    format!(
                        "kernel LocalRuntime is not wired; refusing to mark `{}` as succeeded",
                        invocation.ability
                    ),
                    Vec::new(),
                );
            }
        };
        // Surface a 200-char preview of the prompt for chat-style
        // calls so a Client UI can see the rendered template in the
        // timeline. The preview key is only emitted when the
        // arguments have a string `prompt`; for non-chat abilities
        // the event simply records the dispatch start without
        // peeking at the args' shape.
        let preview: String = invocation
            .args
            .get("prompt")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .chars()
            .take(200)
            .collect();
        let _ = self.session.emit_event(
            session_id,
            json!({
                "kind": "ability_dispatch_starting",
                "ability": invocation.ability,
                "prompt_preview": preview,
            }),
        );

        let caller_signature =
            invocation
                .caller_signature
                .clone()
                .map(|signature| CallerSignature {
                    algorithm: "ed25519".to_string(),
                    signature,
                    key_id_hint: invocation.caller.clone(),
                });
        if caller_signature.is_none() && invocation.caller.as_str() != LOCAL_SYSTEM_AGENT_URA {
            return KernelDispatchOutcome::failed(
                format!(
                    "runtime invocation from `{}` is missing caller_signature; only daemon-local `{}` calls may use the synthetic LocalRuntime signer",
                    invocation.caller,
                    LOCAL_SYSTEM_AGENT_URA
                ),
                Vec::new(),
            );
        }
        let ability_name = invocation.ability.to_string();
        let trace_id = session_id.as_ref().to_string();
        let runtime = Arc::clone(runtime);
        // The descriptor-bound envelope must carry the version the runtime
        // registered for this ability, not a fabricated default — otherwise a
        // kernel-originated call (schedule tick / loop iteration / KernelApi
        // dispatch) of an ability registered at a non-default version is
        // rejected at Axon admission with `proof_descriptor_version_mismatch`,
        // inconsistent with the same ability dispatched over the wire. Version
        // resolution reads the runtime (async), so it runs inside the dispatch
        // block alongside the rest of the descriptor-bound construction.
        let dispatch_invocation = invocation.clone();
        let result = block_on_axon_dispatch(move || async move {
            let runtime_ability =
                crate::runtime::axon_bridge::descriptor_ref::ability_ura_for_wire(
                    &dispatch_invocation.callee,
                    dispatch_invocation.ability.as_str(),
                )
                .map_err(|err| anyhow::anyhow!("{err}"))?;
            let descriptor_version =
                crate::runtime::axon_bridge::descriptor_ref::registered_descriptor_version(
                    &runtime,
                    &runtime_ability,
                    AxonInvocationCallMode::Rpc,
                )
                .await
                .map_err(|err| anyhow::anyhow!("{err}"))?;
            let (envelope, payload) =
                dispatch_invocation.axon_descriptor_bound_envelope(&descriptor_version)?;
            let ingress = match caller_signature {
                Some(signature) => LocalRuntimeIngress::ExternalSigned {
                    envelope,
                    signature,
                    payload,
                },
                None => LocalRuntimeIngress::LocalSystem { envelope, payload },
            };
            let request = LocalRuntimeRequestFactory::request_for(
                AxonInvocationCallMode::Rpc,
                ingress,
                LocalRuntimeRequestOptions::default().with_trace_id(trace_id),
            )
            .map_err(|err| anyhow::anyhow!("{err}"))?;
            let handle = runtime
                .invoke_descriptor_bound_request_async(request)
                .await
                .map(|(handle, _signed)| handle)
                .map_err(|err| anyhow::anyhow!("{err}"))?;
            let state = handle.wait().await;
            let events = handle.core().snapshot_events().await;
            let terminal = events.iter().rev().find(|e| e.state.is_terminal()).cloned();
            let receipt_events = receipt_events_from_axon(&events)?;
            match (state, terminal) {
                (easynet_axon::invocation::InvocationState::Completed, Some(ev)) => {
                    let value = if ev.payload.is_empty() {
                        serde_json::Value::Null
                    } else {
                        match serde_json::from_slice(&ev.payload) {
                            Ok(value) => value,
                            Err(err) => {
                                return Ok(KernelDispatchOutcome::failed(
                                    format!("decode {ability_name} result: {err}"),
                                    receipt_events,
                                ));
                            }
                        }
                    };
                    Ok(KernelDispatchOutcome::succeeded(value, receipt_events))
                }
                (_, Some(ev)) => Ok(KernelDispatchOutcome::failed(
                    terminal_reason(&ev, state),
                    receipt_events,
                )),
                (other, None) => Ok(KernelDispatchOutcome::failed(
                    format!(
                        "Axon invocation ended in {} without a terminal event",
                        other.as_str()
                    ),
                    receipt_events,
                )),
            }
        });

        let outcome = match result {
            Ok(outcome) => outcome,
            Err(error) => KernelDispatchOutcome::failed(format!("{error}"), Vec::new()),
        };

        match &outcome.terminal {
            KernelDispatchTerminal::Succeeded(value) => {
                let _ = self.session.emit_event(
                    session_id,
                    json!({
                        "kind": "ability_response",
                        "ability": invocation.ability,
                        "result": value,
                    }),
                );
            }
            KernelDispatchTerminal::Failed(reason) => {
                let _ = self.session.emit_event(
                    session_id,
                    json!({
                        "kind": "ability_error",
                        "ability": invocation.ability,
                        "error": reason,
                    }),
                );
            }
        }
        outcome
    }
}

fn block_on_axon_dispatch<F, Fut>(f: F) -> anyhow::Result<KernelDispatchOutcome>
where
    F: FnOnce() -> Fut + Send,
    Fut: std::future::Future<Output = anyhow::Result<KernelDispatchOutcome>> + Send,
{
    crate::support::async_bridge::run_blocking(
        f(),
        crate::support::async_bridge::NoRuntimeFallback::BuildCurrentThreadTokio,
    )
}

fn terminal_reason(
    event: &easynet_axon::invocation::InvocationEvent,
    state: easynet_axon::invocation::InvocationState,
) -> String {
    if event.reason.is_empty() {
        format!("Axon invocation ended as {}", state.as_str())
    } else {
        event.reason.clone()
    }
}

fn receipt_events_from_axon(
    events: &[easynet_axon::invocation::InvocationEvent],
) -> anyhow::Result<Vec<ReceiptEvent>> {
    events
        .iter()
        .map(|event| {
            let sequence = i64::try_from(event.sequence).map_err(|_| {
                anyhow::anyhow!(
                    "Axon event sequence {} does not fit EasyNet ReceiptEvent",
                    event.sequence
                )
            })?;
            Ok(ReceiptEvent {
                sequence,
                timestamp_unix_ms: event.timestamp_unix_ms,
                event_type: event.event_type.clone(),
                payload: receipt_event_payload(event),
            })
        })
        .collect()
}

fn receipt_event_payload(
    event: &easynet_axon::invocation::InvocationEvent,
) -> Option<serde_json::Value> {
    let payload = if event.payload.is_empty() {
        None
    } else {
        Some(
            match serde_json::from_slice::<serde_json::Value>(&event.payload) {
                Ok(value) => value,
                Err(_) => {
                    use base64::Engine as _;
                    json!({
                        "content_type": event.payload_content_type,
                        "data_base64": base64::engine::general_purpose::STANDARD.encode(&event.payload),
                    })
                }
            },
        )
    };

    if event.reason.is_empty() {
        payload
    } else {
        Some(json!({
            "payload": payload.unwrap_or(serde_json::Value::Null),
            "reason": event.reason,
        }))
    }
}

/// Decide whether an ability name should be routed through the
/// permission gate. Device-host/control-plane abilities run inside the
/// daemon authority; agent-shaped abilities require broker approval.
///
/// A future per-ability sensitivity config (`runtime::abilities` or
/// the manifest layer) would replace this name-prefix check with a
/// lookup; until then the prefix-based rule is exactly what was
/// there before, just generalised away from "is_chat" to "is_agent".
/// RFC-001 P2.2 retired the old namespace split; the non-gating set is
/// now the explicit list of in-host device/control-plane prefixes.
fn should_gate(ability: &str) -> bool {
    const NON_GATING_PREFIXES: &[&str] = &[
        "federation.",
        "a2a.",
        "ability.",
        "admin.",
        "agent.",
        "browser.",
        "camera.",
        "consent.",
        "discuss.",
        "fs.",
        "http.",
        "invocation.",
        "loop.",
        "mcp.bridge.",
        "mcp.client.",
        "meta.",
        "mic.",
        "mission.",
        "node.",
        "observe.",
        "openai.",
        "plugin.",
        "process.",
        "remote.",
        "schedule.",
        "screen.",
        "session.",
        "shell.",
        "skill.",
        "speaker.",
        "terminal.",
        "voice.",
        "device.keyring.",
        "identity.",
        "capability.",
        "state.",
        "stream.",
        "bridge.",
        "transport.relay.",
    ];
    !NON_GATING_PREFIXES.iter().any(|p| ability.starts_with(p))
}

/// Extract the agent name portion of an `<agent>.<verb>` ability
/// for use in permission events (`agent` field). Returns the full
/// ability name unchanged when there is no `.` — keeps the event
/// shape stable even for malformed inputs that should never reach
/// this far.
fn agent_portion(ability: &str) -> &str {
    ability
        .rsplit_once('.')
        .map(|(head, _)| head)
        .unwrap_or(ability)
}

impl KernelApi for Kernel {
    fn invoke(&self, invocation: RuntimeInvocation) -> anyhow::Result<Receipt> {
        // Plan v10.3 C* unity entry. Three phases:
        //   1. Admission — compute invocation_id, register a Session
        //      keyed by that id so live attachers see the run from
        //      its first frame.
        //   2. Permission gate — ask the broker for agent-shaped
        //      abilities; skip known in-host device/control-plane
        //      prefixes.
        //   3. Dispatch — invoke the ability through the daemon's
        //      Axon `LocalRuntime`.
        //      All admitted abilities flow through the same code path;
        //      the kernel does not special-case any one of them.
        //   4. Terminal — emit `invoke_terminal`, mark the session
        //      ended, return the Receipt.
        invocation.validate()?;
        let id = runtime_invocation_id(&invocation)?;
        let session_id = SessionId::new(id.clone());
        // The session's `agent` field is for observability only: it
        // carries the head of `<head>.<tail>`.
        let admit_agent = agent_portion(&invocation.ability).to_string();
        let admit = Session {
            id: session_id.clone(),
            agent: AgentId::new(admit_agent.clone()),
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

        // Permission admission gate. Device-host/control-plane abilities
        // skip the gate; agent-shaped abilities require broker approval.
        // AllowAllBroker (default) auto-allows; SubscriberBroker
        // publishes a PermissionRequest and blocks until a Client
        // decides.
        let outcome = if should_gate(&invocation.ability) {
            let agent_label = agent_portion(&invocation.ability);
            match self.gate_permission(&session_id, agent_label, &invocation.args) {
                PermissionDecision::Allow | PermissionDecision::AllowOnce => {
                    self.dispatch_via_local_runtime(&session_id, &invocation)
                }
                PermissionDecision::Deny => {
                    let _ = self.session.emit_event(
                        &session_id,
                        json!({
                            "kind": "permission_denied",
                            "agent": agent_label,
                            "ability": invocation.ability,
                        }),
                    );
                    KernelDispatchOutcome::failed(
                        format!("permission denied for {}", invocation.ability),
                        Vec::new(),
                    )
                }
            }
        } else {
            self.dispatch_via_local_runtime(&session_id, &invocation)
        };

        let now_ms = chrono::Utc::now().timestamp_millis();
        let terminal = outcome.terminal_state();
        let _ = self.session.emit_event(
            &session_id,
            json!({
                "kind": "invoke_terminal",
                "outcome": outcome.terminal_event_value(),
            }),
        );
        let _ = self.session.terminate(&session_id, now_ms);

        Ok(Receipt {
            invocation_id: id,
            terminal,
            events: outcome.events,
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

    fn session_events(
        &self,
        id: &SessionId,
        since_seq: usize,
    ) -> anyhow::Result<Vec<serde_json::Value>> {
        let (history, _rx) = self.session.subscribe_session(id, since_seq)?;
        Ok(history)
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
    use crate::runtime::gateway_api::PeerInfo;
    use crate::runtime::invocation::{RuntimeCausalContext, RuntimeInvocation};
    use easynet_axon::invocation::{make_ability, AbilityCallModes, AbilityOptions};
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
        fn list_peers(&self) -> anyhow::Result<Vec<PeerInfo>> {
            Ok(Vec::new())
        }
        fn send_heartbeat(&self) -> anyhow::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn kernel_invoke_without_runtime_keeps_receipt_id_and_fails_closed() {
        // Even when dispatch cannot run, the receipt must keep the
        // deterministic invocation id. What must not survive is the
        // old false success marker: an unwired LocalRuntime is a
        // failed invocation, not a successful no-op.
        let k = Kernel::new(Arc::new(NoopGateway));
        let caller = crate::ura::device_ura("localhost", "a");
        let callee = crate::ura::device_ura("localhost", "b");
        let inv = RuntimeInvocation {
            caller,
            callee: callee.clone(),
            ability: "observe.health".into(),
            subject: callee,
            nonce_hex: "aa".repeat(16),
            causal_context: RuntimeCausalContext::Null,
            args: json!({}),
            caller_signature: None,
        };
        let expected_id = runtime_invocation_id(&inv).unwrap();
        let r = k.invoke(inv).unwrap();
        assert_eq!(r.invocation_id, expected_id);
        match r.terminal {
            TerminalState::Failed { reason } => {
                assert!(
                    reason.contains("LocalRuntime is not wired"),
                    "expected missing LocalRuntime reason; got {reason}"
                );
            }
            other => panic!("expected Failed receipt, got {other:?}"),
        }
    }

    #[test]
    fn should_gate_passes_agent_abilities_and_skips_control_plane() {
        // Replaces the old `parse_agent_chat` tests after Phase 4
        // generalised invoke from "is_chat" to "is_agent". The
        // contract: in-host control-plane abilities do not gate;
        // agent-shaped abilities do.
        assert!(should_gate("alice.chat"));
        assert!(should_gate("claude-code.chat"));
        assert!(should_gate("alice.voice"));
        assert!(should_gate("a.b.chat"));
        // The narrower "<agent>.chat"-only behaviour is gone — voice,
        // exec, and any future verbs all gate the same way as chat.
        // Pin that explicitly so a reviewer notices the broader gate
        // rather than discovering it via a permission-prompt incident.
        assert!(should_gate("alice.exec"));
        // Control-plane prefixes must not gate.
        assert!(!should_gate("session.attach"));
        assert!(!should_gate("observe.health"));
        assert!(!should_gate("consent.subscribe"));
    }

    #[test]
    fn agent_portion_extracts_head_of_dotted_name() {
        // The session's `agent` field is for observability only;
        // the head of `<head>.<tail>` is what makes the timeline
        // readable ("alice did X" rather than "alice.chat did X").
        assert_eq!(agent_portion("alice.chat"), "alice");
        assert_eq!(agent_portion("a.b.chat"), "a.b");
        // RFC-001 P2.2: "observe.health" is now "observe.health" — the
        // head namespace is `observe`. Previous test asserted "system"
        // which was wrong even pre-rename (rsplit_once gives the head,
        // which was always "system" not "observe"); now the value is
        // genuinely "observe".
        assert_eq!(agent_portion("observe.health"), "observe");
        // Defensive: a malformed input with no `.` returns the whole
        // string rather than panicking — keeps invoke's event shape
        // stable even for shapes that should never reach this far.
        assert_eq!(agent_portion("noseparator"), "noseparator");
        assert_eq!(agent_portion(""), "");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn invoke_with_subscriber_broker_publishes_pending_request_and_blocks_until_decision() {
        // End-to-end: a daemon with SubscriberBroker installed
        // gates an agent dispatch; the test acts as the Client
        // by subscribing to the broker, pulling the pending
        // request, and calling decide(Deny). The Receipt comes
        // back as Failed("permission denied") and the session
        // timeline contains permission_pending → permission_denied
        // events the GUI needs to render the dialog.
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        let k = Arc::new(Kernel::new_with_subscriber_broker(Arc::new(NoopGateway)));

        // Subscribe to the broker BEFORE invoking — has_subscribers()
        // gates the SubscriberBroker fallback to Allow when empty.
        let perm_svc = k.permission_service();
        let sub = perm_svc
            .subscriber()
            .expect("with-subscriber-broker variant")
            .clone();
        let mut pending_rx = sub.subscribe();

        // Spawn the kernel.invoke call in a blocking task — broker.ask
        // blocks waiting for the decision.
        let k_clone = Arc::clone(&k);
        let invoke_task = tokio::task::spawn_blocking(move || {
            let device_ura = crate::ura::device_ura("localhost", "a");
            let inv = RuntimeInvocation {
                caller: device_ura.clone(),
                callee: device_ura.clone(),
                ability: "ghost-agent.chat".into(),
                subject: device_ura,
                nonce_hex: "11".repeat(16),
                causal_context: RuntimeCausalContext::Null,
                args: json!({"prompt": "do the thing"}),
                caller_signature: None,
            };
            k_clone.invoke(inv).unwrap()
        });

        // Pull the pending request off the broadcast.
        let pending = pending_rx.recv().await.expect("pending broadcast");
        assert!(pending.prompt.contains("ghost-agent"));

        // Decide Deny; the kernel's gate_permission returns Deny;
        // invoke returns a Failed receipt.
        sub.decide(&pending.id, PermissionDecision::Deny).unwrap();

        let receipt = invoke_task.await.unwrap();
        match receipt.terminal {
            TerminalState::Failed { reason } => {
                assert!(
                    reason.contains("permission denied"),
                    "expected denial reason; got {reason}"
                );
            }
            other => panic!("expected Failed(permission denied), got {other:?}"),
        }
    }

    #[test]
    fn invoke_with_unknown_ability_returns_failed_receipt() {
        // An invocation against an ability the unified registry does
        // not know lands as Failed with a clear reason. This is the
        // contract a Client uses to render a "no such ability" /
        // "no such agent" dialog rather than spinning forever.
        //
        // Kernel dispatch now enters Axon's LocalRuntime directly,
        // so we wire an empty runtime — same observable contract
        // through the daemon's current source of truth.
        let k = Kernel::new(Arc::new(NoopGateway));
        k.set_local_runtime(easynet_axon::invocation::LocalRuntime::new());
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        let device_ura = crate::ura::device_ura("localhost", "a");
        let inv = RuntimeInvocation {
            caller: LOCAL_SYSTEM_AGENT_URA.to_string(),
            callee: device_ura.clone(),
            ability: "ghost-agent.chat".into(),
            subject: device_ura,
            nonce_hex: "00".repeat(16),
            causal_context: RuntimeCausalContext::Null,
            args: json!({"prompt": "hi"}),
            caller_signature: None,
        };
        let r = k.invoke(inv).unwrap();
        match r.terminal {
            TerminalState::Failed { reason } => {
                assert!(
                    reason.contains("ghost-agent.chat") || reason.contains("no local handler"),
                    "expected ability-not-registered reason; got {reason}"
                );
            }
            other => panic!("expected Failed receipt, got {other:?}"),
        }
    }

    #[test]
    fn invoke_success_receipt_projects_axon_event_sequence() {
        let k = Kernel::new(Arc::new(NoopGateway));
        let runtime = easynet_axon::invocation::LocalRuntime::new();
        let device_ura = crate::ura::device_ura("localhost", "a");
        let runtime_ability = crate::runtime::axon_bridge::descriptor_ref::ability_ura_for_wire(
            &device_ura,
            "observe.health",
        )
        .expect("runtime ability URA");
        let runtime_for_register = Arc::clone(&runtime);
        crate::support::async_bridge::run_blocking(
            async move {
                runtime_for_register
                    .register_ability_with_options(
                        runtime_ability,
                        make_ability(|ctx| async move { Ok(ctx.payload.clone()) }),
                        AbilityOptions::default()
                            .with_modes(AbilityCallModes::RPC)
                            .with_descriptor_proof(
                                crate::runtime::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION,
                                [0x11; 32],
                                [0x22; 32],
                            ),
                    )
                    .await
            },
            crate::support::async_bridge::NoRuntimeFallback::BuildCurrentThreadTokio,
        )
        .expect("register descriptor-bound echo ability");
        k.set_local_runtime(runtime);

        let inv = RuntimeInvocation {
            caller: LOCAL_SYSTEM_AGENT_URA.to_string(),
            callee: device_ura.clone(),
            ability: "observe.health".into(),
            subject: device_ura,
            nonce_hex: "44".repeat(16),
            causal_context: RuntimeCausalContext::Null,
            args: json!({"ok": true}),
            caller_signature: None,
        };

        let receipt = k.invoke(inv).unwrap();
        assert_eq!(receipt.terminal, TerminalState::Succeeded);
        assert!(
            receipt
                .events
                .iter()
                .any(|event| event.event_type == "completed"),
            "receipt must preserve the terminal Axon event sequence"
        );
        for (index, event) in receipt.events.iter().enumerate() {
            assert_eq!(event.sequence, index as i64);
        }
        assert_eq!(
            receipt
                .events
                .iter()
                .rev()
                .find(|event| event.event_type == "completed")
                .and_then(|event| event.payload.as_ref()),
            Some(&json!({"ok": true}))
        );
    }

    #[test]
    fn invoke_user_without_signature_rejects_before_local_runtime_dispatch() {
        let k = Kernel::new(Arc::new(NoopGateway));
        k.set_local_runtime(easynet_axon::invocation::LocalRuntime::new());
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        let caller_ura = crate::ura::user_ura("localhost", "alice");
        let device_ura = crate::ura::device_ura("localhost", "a");
        let inv = RuntimeInvocation {
            caller: caller_ura,
            callee: device_ura.clone(),
            ability: "ghost-agent.chat".into(),
            subject: device_ura,
            nonce_hex: "22".repeat(16),
            causal_context: RuntimeCausalContext::Null,
            args: json!({"prompt": "hi"}),
            caller_signature: None,
        };
        let r = k.invoke(inv).unwrap();
        match r.terminal {
            TerminalState::Failed { reason } => {
                assert!(
                    reason.contains("missing caller_signature"),
                    "expected external unsigned rejection; got {reason}"
                );
            }
            other => panic!("expected Failed receipt, got {other:?}"),
        }
    }

    #[test]
    fn invoke_without_dispatcher_fails_closed() {
        // A Kernel built in isolation can still admit the session
        // and run the permission gate, but dispatch must fail closed.
        // Returning Succeeded here would turn a daemon boot wiring
        // regression into a false-positive invocation receipt.
        let k = Kernel::new(Arc::new(NoopGateway));
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        let device_ura = crate::ura::device_ura("localhost", "a");
        let inv = RuntimeInvocation {
            caller: device_ura.clone(),
            callee: device_ura.clone(),
            ability: "alice.chat".into(),
            subject: device_ura,
            nonce_hex: "ff".repeat(16),
            causal_context: RuntimeCausalContext::Null,
            args: json!({"prompt": "hi"}),
            caller_signature: None,
        };
        let r = k.invoke(inv).unwrap();
        match r.terminal {
            TerminalState::Failed { reason } => {
                assert!(
                    reason.contains("LocalRuntime is not wired"),
                    "expected missing LocalRuntime reason; got {reason}"
                );
            }
            other => panic!("expected Failed receipt, got {other:?}"),
        }
    }

    #[test]
    fn invoke_rejects_malformed_invocation_ura_before_admission() {
        let k = Kernel::new(Arc::new(NoopGateway));
        let inv = RuntimeInvocation {
            caller: "easynet://nodes/a".into(),
            callee: crate::ura::device_ura("localhost", "a"),
            ability: "alice.chat".into(),
            subject: crate::ura::device_ura("localhost", "a"),
            nonce_hex: "ff".repeat(16),
            causal_context: RuntimeCausalContext::Null,
            args: json!({"prompt": "hi"}),
            caller_signature: None,
        };
        let err = k.invoke(inv).unwrap_err();
        assert!(format!("{err}").contains("caller URA is invalid"));
    }
}
