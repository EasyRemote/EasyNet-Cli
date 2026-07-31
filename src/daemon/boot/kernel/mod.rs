// EasyNet CLI — Kernel (single execution entry point)
// =====================================================
//
// File: src/daemon/kernel/mod.rs
// Description: The Kernel holds one handle per Execution sub-service;
//              every KernelApi method routes
//              through the Kernel. Kernel::invoke is the single
//              execution entry that v10.3 C* pins as the only daemon
//              kernel path into local execution.
//
// Invocation state
// ----------------
// `Kernel::invoke` owns the daemon product-permission gate around Axon's
// descriptor-bound LocalRuntime entry. Axon owns Invocation identity,
// lifecycle, and signed terminal receipts. Pre-admission failures return
// errors; the kernel never manufactures a terminal receipt.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

pub mod api;

use std::sync::{Arc, OnceLock};

use axon_sdk::invocation::{
    CallMode as AxonInvocationCallMode, CausalContext, DescriptorBoundInvocationRequest,
    FinalizedInvocation, InvocationState,
};
use serde_json::json;

use crate::core::domain::{
    AgentId, DiscussRoom, LoopId, LoopInstance, PermissionDecision, PermissionId,
    PermissionRequest, PermissionSensitivity, RoomId, ScheduleEntry, ScheduleId, Session,
    SessionId, TenantId,
};
use crate::daemon::axon_bridge::local_runtime_request::{
    LocalRuntimeRequestOptions, SystemInvocationIssuer,
};
use crate::daemon::boot::kernel::api::KernelApi;
use crate::daemon::execution::{
    loop_instance::LoopService,
    mission::discuss::DiscussService,
    permission::{AskContext, PermissionService},
    runtime_identity::LocalRuntimeSessionProjection,
    schedule::ScheduleService,
    session::SessionService,
};

/// The runtime kernel. Holds one sub-service per feature. Feature PRs extend the
/// sub-service handles; the Kernel itself stays thin — it is a
/// router, not a state owner.
///
/// Sub-services are held as `Arc` so the same handle can be
/// shared with the `daemon::ability::builtins::*` ability handlers without
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
    /// Shared Axon LocalRuntime, set by daemon boot after the runtime
    /// is constructed. `Kernel::invoke` dispatches through this handle
    /// so daemon-internal invocation sources (schedule ticks, loop
    /// iterations, future controllers) observe the same admission,
    /// state machine, and ledger sink as gRPC Invoke.
    local_runtime: OnceLock<Arc<axon_sdk::invocation::LocalRuntime>>,
}

impl Kernel {
    /// Construct a Kernel backed by fresh sub-services. Uses the
    /// headless permission policy, so Kernel admission does not
    /// block when no operator channel exists. Daemons that want
    /// interactive approval should use `new_interactive` instead
    /// so a Client subscribed to consent.subscribe sees pending
    /// requests.
    pub fn new() -> Self {
        Self {
            session: Arc::new(SessionService::new()),
            permission: Arc::new(PermissionService::new()),
            discuss: Arc::new(DiscussService::new()),
            schedule: Arc::new(ScheduleService::new()),
            loop_svc: Arc::new(LoopService::new()),
            local_runtime: OnceLock::new(),
        }
    }

    /// Construct a Kernel with the SubscriberBroker permission
    /// variant installed — every Kernel::invoke admission against
    /// a non-system ability publishes a PermissionRequest on the
    /// broker's broadcast channel, then blocks waiting for the
    /// matching `consent.decide` decision.
    ///
    /// When no subscriber is connected the broker uses the
    /// unobserved permission policy
    /// (per docs/rfc/permission-broker-v1.md §4 cross-machine
    /// advisory downgrade and §6 "no observer means no human in
    /// the loop"). The default `new()` should be preferred for
    /// tests and for daemons running without a Client; the
    /// daemon bin uses this constructor so the Permission tab
    /// in the GUI sees real pending requests.
    pub fn new_interactive() -> Self {
        Self {
            session: Arc::new(SessionService::new()),
            permission: Arc::new(PermissionService::interactive()),
            discuss: Arc::new(DiscussService::new()),
            schedule: Arc::new(ScheduleService::new()),
            loop_svc: Arc::new(LoopService::new()),
            local_runtime: OnceLock::new(),
        }
    }

    /// Wire the shared Axon runtime into the Kernel post-construction.
    /// Called by daemon boot once `LocalRuntime` exists; ability
    /// registrations can happen before or after this because the Arc
    /// points at the same runtime object.
    pub fn set_local_runtime(&self, runtime: Arc<axon_sdk::invocation::LocalRuntime>) {
        let _ = self.local_runtime.set(runtime);
    }

    fn require_local_runtime(
        &self,
        context: &str,
    ) -> anyhow::Result<Arc<axon_sdk::invocation::LocalRuntime>> {
        self.local_runtime.get().map(Arc::clone).ok_or_else(|| {
            anyhow::anyhow!(
                "{context} requires canonical daemon kernel runtime assembly: missing LocalRuntime"
            )
        })
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

    /// Product permission gate. The permission request keeps a correlation
    /// session supplied by the canonical request caller; LocalRuntime assigns
    /// the authoritative invocation id only after descriptor-bound admission.
    /// Allowed decisions are projected onto that canonical session after
    /// admission. A denial returns before runtime admission and therefore
    /// cannot produce a receipt or invocation session.
    ///
    /// Headless policy returns immediately. SubscriberBroker
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
        tenant: &TenantId,
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
        let ctx = AskContext {
            prompt: pending_prompt,
            sensitivity: PermissionSensitivity::Medium,
            session: session_id.clone(),
            tenant: tenant.clone(),
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
        decision
    }

    /// Dispatch an already-complete SDK request through LocalRuntime and
    /// project its canonical lifecycle into the daemon session read model.
    #[expect(
        clippy::too_many_arguments,
        reason = "the canonical request, tuple projection, payload, and permission decision are independent audited facts"
    )]
    fn dispatch_via_local_runtime(
        &self,
        request: DescriptorBoundInvocationRequest,
        caller: String,
        callee: String,
        ability: String,
        args: serde_json::Value,
        session_projection: LocalRuntimeSessionProjection,
        permission_decision: Option<PermissionDecision>,
    ) -> anyhow::Result<FinalizedInvocation> {
        let runtime = self.require_local_runtime("Kernel::invoke")?;
        let session = Arc::clone(&self.session);
        let preview: String = args
            .get("prompt")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .chars()
            .take(200)
            .collect();
        let admit_agent = agent_portion(&ability).to_string();

        block_on_axon_dispatch(move || async move {
            let (handle, _signed) = runtime
                .invoke_descriptor_bound_request_async(request)
                .await
                .map_err(|err| anyhow::anyhow!("{err}"))?;
            let invocation_id = handle.invocation_id().to_string();
            let session_id = SessionId::new(invocation_id.clone());
            let session_admitted = session
                .admit(Session {
                    id: session_id.clone(),
                    agent: AgentId::new(admit_agent.clone()),
                    node: session_projection.node().clone(),
                    tenant: session_projection.tenant().clone(),
                    started_unix_ms: chrono::Utc::now().timestamp_millis(),
                    ended_unix_ms: None,
                })
                .is_ok();
            if session_admitted {
                let _ = session.emit_event(
                    &session_id,
                    json!({
                        "kind": "invoke_admitted",
                        "ability": ability,
                        "caller": caller,
                        "callee": callee,
                    }),
                );
            }
            if session_admitted {
                if let Some(decision) = permission_decision {
                    let _ = session.emit_event(
                        &session_id,
                        json!({
                            "kind": "permission_pending",
                            "agent": admit_agent,
                            "sensitivity": "medium",
                        }),
                    );
                    let _ = session.emit_event(
                        &session_id,
                        json!({
                            "kind": "permission_decided",
                            "agent": admit_agent,
                            "decision": permission_decision_label(decision),
                        }),
                    );
                }
                let _ = session.emit_event(
                    &session_id,
                    json!({
                        "kind": "ability_dispatch_starting",
                        "ability": ability,
                        "prompt_preview": preview,
                    }),
                );
            }

            let finalized = handle
                .finalized()
                .await
                .map_err(|err| anyhow::anyhow!("Axon finalization: {err}"))?;
            if session_admitted {
                if finalized.terminal_state == InvocationState::Completed {
                    let _ = session.emit_event(
                        &session_id,
                        json!({
                            "kind": "ability_response",
                            "ability": ability,
                            "result": canonical_output_json(&finalized),
                        }),
                    );
                } else {
                    let _ = session.emit_event(
                        &session_id,
                        json!({
                            "kind": "ability_error",
                            "ability": ability,
                            "state": finalized.terminal_state.as_str(),
                            "reason": canonical_terminal_reason(&finalized),
                        }),
                    );
                }
                let _ = session.emit_event(
                    &session_id,
                    json!({
                        "state": finalized.terminal_state.as_str(),
                        "kind": "invoke_terminal",
                        "receipt_hash": hex::encode(finalized.terminal_receipt.self_hash()),
                    }),
                );
                let _ = session.terminate(&session_id, chrono::Utc::now().timestamp_millis());
            }
            Ok(finalized)
        })
    }
}

impl Default for Kernel {
    fn default() -> Self {
        Self::new()
    }
}

fn block_on_axon_dispatch<F, Fut>(f: F) -> anyhow::Result<FinalizedInvocation>
where
    F: FnOnce() -> Fut + Send,
    Fut: std::future::Future<Output = anyhow::Result<FinalizedInvocation>> + Send,
{
    crate::support::async_bridge::run_blocking(
        f(),
        crate::support::async_bridge::SyncBridgeRuntimePolicy::BuildCurrentThreadTokio,
    )
}

fn canonical_output_json(finalized: &FinalizedInvocation) -> serde_json::Value {
    if finalized.output().is_empty() {
        return serde_json::Value::Null;
    }
    serde_json::from_slice(finalized.output()).unwrap_or_else(|_| {
        use base64::Engine as _;
        json!({
            "payload_content_type": finalized.output_content_type(),
            "payload_base64": base64::engine::general_purpose::STANDARD.encode(finalized.output()),
        })
    })
}

fn canonical_terminal_reason(finalized: &FinalizedInvocation) -> String {
    finalized
        .failure
        .as_ref()
        .map(ToString::to_string)
        .filter(|reason| !reason.is_empty())
        .unwrap_or_else(|| finalized.terminal_receipt.reason().to_string())
}

fn permission_decision_label(decision: PermissionDecision) -> &'static str {
    match decision {
        PermissionDecision::Allow => "allow",
        PermissionDecision::Deny => "deny",
        PermissionDecision::AllowOnce => "allow_once",
    }
}

/// Decide whether an ability name should be routed through the
/// permission gate. Device-host/control-plane abilities run inside the
/// daemon authority; agent-shaped abilities require broker approval.
///
/// A future per-ability sensitivity config (`daemon::execution::mission::agent_ability_specs` or
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
    fn prepare_local_system_rpc(
        &self,
        callee_ura: &str,
        ability: &str,
        subject_ura: &str,
        payload: Vec<u8>,
    ) -> anyhow::Result<DescriptorBoundInvocationRequest> {
        let runtime = self.require_local_runtime("KernelApi::prepare_local_system_rpc")?;
        let callee_ura = callee_ura.to_string();
        let ability = ability.to_string();
        let subject_ura = subject_ura.to_string();
        crate::support::async_bridge::run_blocking(
            async move {
                let runtime_ability =
                    crate::daemon::axon_bridge::descriptor_ref::ability_ura_for_wire(
                        &callee_ura,
                        &ability,
                    )
                    .map_err(|err| anyhow::anyhow!("{err}"))?;
                let descriptor_binding =
                    crate::daemon::axon_bridge::descriptor_ref::registered_descriptor_binding(
                        &runtime,
                        &runtime_ability,
                        AxonInvocationCallMode::Rpc,
                    )
                    .await
                    .map_err(|err| anyhow::anyhow!("{err}"))?;
                let descriptor_ref =
                    crate::daemon::axon_bridge::descriptor_ref::ability_descriptor_ref_for_wire(
                        &callee_ura,
                        &ability,
                        &descriptor_binding,
                    )
                    .map_err(|err| anyhow::anyhow!("{err}"))?;
                SystemInvocationIssuer::request_for_descriptor_ref(
                    AxonInvocationCallMode::Rpc,
                    &callee_ura,
                    descriptor_ref,
                    &subject_ura,
                    payload,
                    CausalContext::None,
                    LocalRuntimeRequestOptions::default(),
                )
                .map_err(|err| anyhow::anyhow!("{err}"))
            },
            crate::support::async_bridge::SyncBridgeRuntimePolicy::BuildCurrentThreadTokio,
        )
    }

    fn invoke(
        &self,
        request: DescriptorBoundInvocationRequest,
    ) -> anyhow::Result<FinalizedInvocation> {
        if request.call_mode() != AxonInvocationCallMode::Rpc {
            anyhow::bail!(
                "KernelApi::invoke accepts RPC descriptor-bound requests, got {}",
                request.call_mode().as_str()
            );
        }
        let envelope = request.envelope().envelope();
        let caller = envelope.caller.ura.clone();
        let callee = envelope.callee.ura.clone();
        let ability_ura =
            crate::daemon::axon_bridge::descriptor_ref::ability_ura_from_descriptor_ref(
                &envelope.ability,
            )
            .map_err(|err| anyhow::anyhow!("invalid descriptor-bound ability: {err}"))?;
        let ability = crate::core::ura::AbilitySelector::parse(&ability_ura)
            .map_err(|err| anyhow::anyhow!("invalid descriptor-bound ability owner: {err}"))?
            .local_registry_ability()
            .to_string();
        let args = serde_json::from_slice(request.payload())
            .map_err(|err| anyhow::anyhow!("KernelApi::invoke requires a JSON payload: {err}"))?;
        let session_projection = LocalRuntimeSessionProjection::from_callee_ura(&callee)?;

        let permission_decision = if should_gate(&ability) {
            let permission_session_id = SessionId::new(format!(
                "permission-{}",
                hex::encode(envelope.invocation_nonce)
            ));
            let decision = self.gate_permission(
                &permission_session_id,
                agent_portion(&ability),
                &args,
                session_projection.tenant(),
            );
            if decision == PermissionDecision::Deny {
                anyhow::bail!("permission denied for {ability}");
            }
            Some(decision)
        } else {
            None
        };

        self.dispatch_via_local_runtime(
            request,
            caller,
            callee,
            ability,
            args,
            session_projection,
            permission_decision,
        )
    }

    fn list_active_sessions(&self) -> anyhow::Result<Vec<Session>> {
        self.session.list_active()
    }

    fn get_session(&self, id: &SessionId) -> anyhow::Result<Option<Session>> {
        self.session.get(id)
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
        self.permission.pending()
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
        self.schedule.list()
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
        self.discuss.list()
    }

    fn loop_status(&self, id: &LoopId) -> anyhow::Result<Option<LoopInstance>> {
        self.loop_svc.status(id)
    }

    fn cancel_loop(&self, id: &LoopId) -> anyhow::Result<()> {
        self.loop_svc.cancel(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::domain::NodeId;
    use axon_sdk::invocation::{make_ability, AbilityCallModes, AbilityOptions};
    use serde_json::json;

    fn test_system_request(
        callee_ura: &str,
        ability: &str,
        subject_ura: &str,
        payload: Vec<u8>,
    ) -> DescriptorBoundInvocationRequest {
        let descriptor_binding =
            crate::daemon::axon_bridge::descriptor_ref::descriptor_binding_for_wire(
                crate::daemon::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION,
                [0x33; 32],
                "invoke",
            )
            .expect("test descriptor binding");
        let descriptor_ref =
            crate::daemon::axon_bridge::descriptor_ref::ability_descriptor_ref_for_wire(
                callee_ura,
                ability,
                &descriptor_binding,
            )
            .expect("test descriptor ref");
        SystemInvocationIssuer::request_for_descriptor_ref(
            AxonInvocationCallMode::Rpc,
            callee_ura,
            descriptor_ref,
            subject_ura,
            payload,
            CausalContext::None,
            LocalRuntimeRequestOptions::default(),
        )
        .expect("test system request")
    }

    fn install_echo_runtime(kernel: &Kernel, callee_ura: &str, ability: &str) {
        let runtime = crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
            crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
            None,
        );
        let runtime_ability =
            crate::daemon::axon_bridge::descriptor_ref::ability_ura_for_wire(callee_ura, ability)
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
                                crate::daemon::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION,
                                "invoke",
                                [0x33; 32],
                                [0x11; 32],
                                [0x22; 32],
                            ),
                    )
                    .await
            },
            crate::support::async_bridge::SyncBridgeRuntimePolicy::BuildCurrentThreadTokio,
        )
        .expect("register descriptor-bound echo ability");
        kernel.set_local_runtime(runtime);
    }

    #[test]
    fn kernel_invoke_without_runtime_returns_error_without_receipt() {
        let k = Kernel::new();
        let callee = crate::core::ura::device_ura("localhost", "b");
        let request = test_system_request(
            &callee,
            "observe.health",
            &callee,
            serde_json::to_vec(&json!({})).unwrap(),
        );
        let err = k.invoke(request).expect_err("unwired runtime must reject");
        assert!(format!("{err}").contains("requires canonical daemon kernel runtime assembly"));
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
    async fn invoke_with_interactive_broker_publishes_pending_request_and_blocks_until_decision() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let k = Arc::new(Kernel::new_interactive());
        let device_ura = crate::core::ura::device_ura("localhost", "a");
        install_echo_runtime(&k, &device_ura, "ghost-agent.chat");
        let request = k
            .prepare_local_system_rpc(
                &device_ura,
                "ghost-agent.chat",
                &device_ura,
                serde_json::to_vec(&json!({"prompt": "do the thing"})).unwrap(),
            )
            .expect("canonical request");

        let perm_svc = k.permission_service();
        let sub = perm_svc
            .subscriber()
            .expect("interactive broker variant")
            .clone();
        let mut pending_rx = sub.subscribe();

        let k_clone = Arc::clone(&k);
        let invoke_task = tokio::task::spawn_blocking(move || k_clone.invoke(request));

        let pending = pending_rx.recv().await.expect("pending broadcast");
        assert!(pending.prompt.contains("ghost-agent"));

        sub.decide(&pending.id, PermissionDecision::Deny).unwrap();

        let err = invoke_task
            .await
            .unwrap()
            .expect_err("permission denial is pre-admission");
        assert!(format!("{err}").contains("permission denied"));
        assert!(
            k.list_active_sessions().unwrap().is_empty(),
            "pre-admission denial must not synthesize an invocation session"
        );
    }

    #[test]
    fn invoke_with_unknown_ability_returns_admission_error_without_receipt() {
        let k = Kernel::new();
        k.set_local_runtime(
            crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
                crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
                None,
            ),
        );
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let device_ura = crate::core::ura::device_ura("localhost", "a");
        let request = test_system_request(
            &device_ura,
            "ghost-agent.chat",
            &device_ura,
            serde_json::to_vec(&json!({"prompt": "hi"})).unwrap(),
        );
        let err = k.invoke(request).expect_err("unknown ability must reject");
        let message = format!("{err}");
        assert!(
            message.contains("ghost-agent.chat")
                || message.contains("unknown_ability")
                || message.contains("not registered"),
            "expected ability-not-registered reason; got {message}"
        );
    }

    #[test]
    fn invoke_success_returns_axon_finalization_and_indexes_canonical_session() {
        let k = Kernel::new();
        let device_ura = crate::core::ura::device_ura("tenant-a", "device-a");
        k.session_service()
            .bind_runtime(NodeId::new("device-a"), TenantId::new("tenant-a"))
            .expect("bind kernel test session runtime");
        install_echo_runtime(&k, &device_ura, "observe.health");
        let request = k
            .prepare_local_system_rpc(
                &device_ura,
                "observe.health",
                &device_ura,
                serde_json::to_vec(&json!({"ok": true})).unwrap(),
            )
            .expect("canonical request");

        let finalized = k.invoke(request).expect("canonical finalization");
        assert_eq!(finalized.terminal_state, InvocationState::Completed);
        assert_eq!(
            finalized.output(),
            serde_json::to_vec(&json!({"ok": true})).unwrap()
        );
        assert_eq!(
            finalized.admission_receipt.invocation_id(),
            finalized.terminal_receipt.invocation_id()
        );
        assert_ne!(finalized.terminal_receipt.self_hash(), [0u8; 32]);

        let session_id = SessionId::new(finalized.terminal_receipt.invocation_id().to_string());
        let indexed = k
            .get_session(&session_id)
            .expect("session lookup")
            .expect("canonical session row");
        assert_eq!(indexed.node, NodeId::new("device-a"));
        assert_eq!(indexed.tenant, TenantId::new("tenant-a"));
        let history = k.session_events(&session_id, 0).expect("canonical session");
        assert!(history
            .iter()
            .any(|event| event["kind"] == "ability_response"));
        assert!(history
            .iter()
            .any(|event| event["kind"] == "invoke_terminal"));
    }

    #[test]
    fn invoke_rejects_non_device_session_projection_without_admitting_row() {
        let k = Kernel::new();
        let hub_ura = crate::core::ura::hub_ura("tenant-a");
        let subject_ura = crate::core::ura::device_ura("tenant-a", "device-a");
        let request = test_system_request(
            &hub_ura,
            "observe.health",
            &subject_ura,
            serde_json::to_vec(&json!({"ok": true})).unwrap(),
        );

        let err = k
            .invoke(request)
            .expect_err("Kernel session read model must require Device callee");

        assert!(
            err.to_string().contains("requires Device callee URA"),
            "unexpected error: {err}"
        );
        assert!(
            k.list_active_sessions().unwrap().is_empty(),
            "rejected projection must not admit a synthetic session row"
        );
    }

    #[test]
    fn prepare_local_system_rpc_rejects_malformed_callee_before_admission() {
        let k = Kernel::new();
        k.set_local_runtime(
            crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
                crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
                None,
            ),
        );
        let subject = crate::core::ura::device_ura("localhost", "a");
        let err = match k.prepare_local_system_rpc(
            "easynet://nodes/a",
            "alice.chat",
            &subject,
            serde_json::to_vec(&json!({"prompt": "hi"})).unwrap(),
        ) {
            Ok(_) => panic!("malformed callee must reject"),
            Err(err) => err,
        };
        assert!(format!("{err}").contains("descriptor-bound ability"));
    }
}
