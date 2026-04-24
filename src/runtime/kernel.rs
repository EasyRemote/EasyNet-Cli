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

use crate::runtime::domain::{
    DiscussRoom, LoopId, LoopInstance, PermissionDecision, PermissionId, PermissionRequest,
    RoomId, ScheduleEntry, ScheduleId, Session, SessionId,
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
pub struct Kernel {
    session: SessionService,
    permission: PermissionService,
    discuss: DiscussService,
    schedule: ScheduleService,
    loop_svc: LoopService,
    #[allow(dead_code)]
    gateway: Arc<dyn GatewayApi>,
}

impl Kernel {
    /// Construct a Kernel backed by fresh sub-services and the
    /// provided Gateway.
    pub fn new(gateway: Arc<dyn GatewayApi>) -> Self {
        Self {
            session: SessionService::new(),
            permission: PermissionService::new(),
            discuss: DiscussService::new(),
            schedule: ScheduleService::new(),
            loop_svc: LoopService::new(),
            gateway,
        }
    }
}

impl KernelApi for Kernel {
    fn invoke(&self, invocation: Invocation) -> anyhow::Result<Receipt> {
        // v1 admission stub: compute the invocation_id, build a
        // Succeeded Receipt with no events, and return. This is the
        // minimum behaviour required for the trait to be instantiable.
        // PR-INVOCATION-EXEC-UNITY replaces this stub with the full
        // admission (nonce dedup → permission broker → schedule
        // conflict → dispatch) + terminal pipeline.
        let id = invocation_id_of(&invocation);
        Ok(Receipt {
            invocation_id: id,
            terminal: TerminalState::Succeeded,
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
        self.permission.decide(id, decision)
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
        Ok(self.discuss.list())
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
        // invocation_id matches `invocation_id_of(&inv)`. This is
        // the anchor contract the future PR-INVOCATION-EXEC-UNITY
        // fleshes out.
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
}
