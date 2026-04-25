// EasyNet CLI — Execution / Loop sub-service
// ===========================================
//
// File: src/runtime/execution/loop_instance/mod.rs
// Description: Loop-instance registry + status store. PR-LOOP
//              surfaces this as four abilities so a Client can
//              create a "worker + verify + max_iters" closure,
//              query its status, subscribe to per-iteration frames,
//              and cancel.
//
// Loop boundary (per docs/rfc/loop-primitive-v1.md)
// -------------------------------------------------
// Loop is a local control primitive — a "worker + verify + retry"
// closure bounded by `max_iters`. It is NOT a planner, agent
// teaming, cost-aware routing, or cross-loop coordination. A
// future planner will consume `LoopInstance` as one primitive
// among several; that planner does not live in this module.
//
// Plan v10.3 C* unity: PR-INVOCATION-EXEC-UNITY collapses the
// loop controller's per-iteration body / verify steps onto
// Kernel::invoke. v1 here ships the registry + status store so
// the IPC surface is reachable; the per-iteration execution path
// is the unity-PR's job.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

mod store;

pub use store::LoopStore;

use std::collections::BTreeMap;
use std::sync::RwLock;

use uuid::Uuid;

use crate::runtime::domain::{AgentId, LoopId, LoopInstance, LoopState, TenantId};

#[derive(Default)]
pub struct LoopService {
    cache: RwLock<BTreeMap<LoopId, LoopInstance>>,
    store: RwLock<Option<LoopStore>>,
}

impl LoopService {
    pub fn new() -> Self {
        Self::default()
    }

    /// Bind to a tenant-scoped disk store. Loads existing loop
    /// instances into the cache. Daemon bin calls at boot.
    pub fn bind(&self, tenant: &TenantId) -> anyhow::Result<()> {
        let store = LoopStore::open(tenant)?;
        let loaded = store.load_all()?;
        let mut cache = self
            .cache
            .write()
            .map_err(|_| anyhow::anyhow!("LoopService cache lock poisoned"))?;
        for inst in loaded {
            cache.insert(inst.id.clone(), inst);
        }
        let mut s = self
            .store
            .write()
            .map_err(|_| anyhow::anyhow!("LoopService store lock poisoned"))?;
        *s = Some(store);
        Ok(())
    }

    /// Create a new loop instance. v1 generates the id, persists
    /// to disk if a store is bound, indexes in the cache. The
    /// instance starts in `Pending` state; PR-INVOCATION-EXEC-UNITY
    /// transitions it to `Running` when the controller fires the
    /// first body Invocation.
    pub fn create(
        &self,
        worker_agent: AgentId,
        max_iters: u32,
    ) -> anyhow::Result<LoopId> {
        if max_iters == 0 {
            anyhow::bail!("loop.create: max_iters must be ≥ 1");
        }
        let id = LoopId::new(format!("loop-{}", Uuid::new_v4()));
        let instance = LoopInstance {
            id: id.clone(),
            tenant: TenantId::default_v1(),
            worker_agent,
            max_iters,
            current_iter: 0,
            state: LoopState::Pending,
        };
        if let Some(store) = self
            .store
            .read()
            .map_err(|_| anyhow::anyhow!("store lock poisoned"))?
            .as_ref()
        {
            store.save(&instance)?;
        }
        let mut cache = self
            .cache
            .write()
            .map_err(|_| anyhow::anyhow!("cache lock poisoned"))?;
        cache.insert(id.clone(), instance);
        Ok(id)
    }

    /// Update the in-flight state of a loop. Used by the future
    /// controller (PR-INVOCATION-EXEC-UNITY) when a body or verify
    /// Invocation completes. Persists to disk if bound.
    pub fn update(
        &self,
        id: &LoopId,
        new_state: LoopState,
        new_iter: u32,
    ) -> anyhow::Result<()> {
        let updated = {
            let mut cache = self
                .cache
                .write()
                .map_err(|_| anyhow::anyhow!("cache lock poisoned"))?;
            let inst = cache
                .get_mut(id)
                .ok_or_else(|| anyhow::anyhow!("loop {id} not found"))?;
            inst.state = new_state;
            inst.current_iter = new_iter;
            inst.clone()
        };
        if let Some(store) = self
            .store
            .read()
            .map_err(|_| anyhow::anyhow!("store lock poisoned"))?
            .as_ref()
        {
            store.save(&updated)?;
        }
        Ok(())
    }

    pub fn status(&self, id: &LoopId) -> Option<LoopInstance> {
        self.cache.read().ok().and_then(|g| g.get(id).cloned())
    }

    pub fn list(&self) -> Vec<LoopInstance> {
        match self.cache.read() {
            Ok(g) => g.values().cloned().collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Cancel a loop instance. Transitions to `Cancelled` if the
    /// loop is still running; no-op if it already terminated.
    pub fn cancel(&self, id: &LoopId) -> anyhow::Result<()> {
        let updated = {
            let mut cache = self
                .cache
                .write()
                .map_err(|_| anyhow::anyhow!("cache lock poisoned"))?;
            let inst = cache
                .get_mut(id)
                .ok_or_else(|| anyhow::anyhow!("loop {id} not found"))?;
            // Only cancel if still in flight; preserve terminal
            // states (Done / Exhausted / VerifyMalformed) since
            // they are already terminal records.
            match inst.state {
                LoopState::Pending | LoopState::Running => {
                    inst.state = LoopState::Cancelled;
                }
                _ => {}
            }
            inst.clone()
        };
        if let Some(store) = self
            .store
            .read()
            .map_err(|_| anyhow::anyhow!("store lock poisoned"))?
            .as_ref()
        {
            store.save(&updated)?;
        }
        Ok(())
    }
}

impl std::fmt::Debug for LoopService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let n = self.cache.read().ok().map(|g| g.len()).unwrap_or(0);
        write!(f, "LoopService {{ loops: {n} }}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_then_status_returns_pending_instance() {
        let svc = LoopService::new();
        let id = svc.create(AgentId::new("alice"), 5).unwrap();
        let st = svc.status(&id).unwrap();
        assert_eq!(st.state, LoopState::Pending);
        assert_eq!(st.current_iter, 0);
        assert_eq!(st.max_iters, 5);
    }

    #[test]
    fn create_zero_iters_errors() {
        // A loop that runs zero iterations is by definition
        // useless; refusing at the boundary catches the misuse.
        let svc = LoopService::new();
        let err = svc.create(AgentId::new("alice"), 0).unwrap_err();
        assert!(format!("{err}").contains("max_iters"));
    }

    #[test]
    fn update_transitions_state_and_iter() {
        let svc = LoopService::new();
        let id = svc.create(AgentId::new("alice"), 3).unwrap();
        svc.update(&id, LoopState::Running, 1).unwrap();
        let st = svc.status(&id).unwrap();
        assert_eq!(st.state, LoopState::Running);
        assert_eq!(st.current_iter, 1);
        svc.update(&id, LoopState::Done, 2).unwrap();
        let st2 = svc.status(&id).unwrap();
        assert_eq!(st2.state, LoopState::Done);
        assert_eq!(st2.current_iter, 2);
    }

    #[test]
    fn cancel_in_flight_loop_marks_cancelled() {
        let svc = LoopService::new();
        let id = svc.create(AgentId::new("alice"), 3).unwrap();
        svc.update(&id, LoopState::Running, 1).unwrap();
        svc.cancel(&id).unwrap();
        assert_eq!(svc.status(&id).unwrap().state, LoopState::Cancelled);
    }

    #[test]
    fn cancel_already_terminal_loop_is_noop() {
        // Spirit: a loop that finished as Done should not be
        // re-tagged as Cancelled by a late cancel — that would
        // misrepresent the audit record.
        let svc = LoopService::new();
        let id = svc.create(AgentId::new("alice"), 1).unwrap();
        svc.update(&id, LoopState::Done, 1).unwrap();
        svc.cancel(&id).unwrap();
        assert_eq!(svc.status(&id).unwrap().state, LoopState::Done);
    }

    #[test]
    fn cancel_unknown_loop_errors() {
        let svc = LoopService::new();
        let err = svc.cancel(&LoopId::new("nope")).unwrap_err();
        assert!(format!("{err}").contains("not found"));
    }

    #[test]
    fn list_returns_deterministic_order_by_id() {
        let svc = LoopService::new();
        // Manually inject ids that sort lexically.
        for i in ["c", "a", "b"] {
            let mut cache = svc.cache.write().unwrap();
            let id = LoopId::new(format!("loop-{i}"));
            cache.insert(
                id.clone(),
                LoopInstance {
                    id,
                    tenant: TenantId::default_v1(),
                    worker_agent: AgentId::new("a"),
                    max_iters: 1,
                    current_iter: 0,
                    state: LoopState::Pending,
                },
            );
        }
        let ids: Vec<_> = svc.list().into_iter().map(|i| i.id).collect();
        assert_eq!(
            ids,
            vec![
                LoopId::new("loop-a"),
                LoopId::new("loop-b"),
                LoopId::new("loop-c"),
            ]
        );
    }
}
