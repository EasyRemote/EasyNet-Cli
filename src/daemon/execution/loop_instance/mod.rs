// EasyNet CLI — Execution / Loop sub-service
// ===========================================
//
// File: src/daemon/execution/loop_instance/mod.rs
// Description: Loop-instance registry + status store + controller
//              runner. `loop.create` persists a loop
//              instance; the daemon-side controller drives it from
//              `pending` through bounded body/verify iterations until
//              one terminal state is reached.

mod store;

pub use store::LoopStore;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, RwLock};

use anyhow::Context;
use axon_sdk::invocation::InvocationState;
use serde_json::{json, Value};
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::core::domain::{
    AgentId, DeferredInvocationAuthority, LoopId, LoopInstance, LoopInvocationKind,
    LoopInvocationRecord, LoopInvocationState, LoopState, TenantId,
};
use crate::daemon::boot::kernel::api::KernelApi;
use crate::daemon::execution::runtime_identity::LocalRuntimeInvocationIdentity;

impl LoopInvocationKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Body => "body",
            Self::Verify => "verify",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopInvocationOutcome {
    pub output: String,
    pub invocation_id: String,
    pub receipt_ura: String,
    pub receipt_hash: [u8; 32],
}

pub trait LoopInvocationDriver: Send + Sync {
    fn invoke(
        &self,
        loop_id: &LoopId,
        iter: u32,
        worker_agent: &AgentId,
        authority: &DeferredInvocationAuthority,
        prompt: &str,
        kind: LoopInvocationKind,
        causal_context: axon_sdk::invocation::CausalContext,
    ) -> anyhow::Result<LoopInvocationOutcome>;
}

pub struct KernelLoopInvocationDriver {
    kernel: Arc<dyn KernelApi>,
    identity: LocalRuntimeInvocationIdentity,
}

impl KernelLoopInvocationDriver {
    pub fn new(kernel: Arc<dyn KernelApi>, identity: LocalRuntimeInvocationIdentity) -> Self {
        Self { kernel, identity }
    }

    fn invocation_uras(
        &self,
        loop_id: &LoopId,
        iter: u32,
        worker_agent_ura: &str,
        kind: LoopInvocationKind,
    ) -> (String, String) {
        let loop_subject_ura = self.identity.resource_subject_ura(
            &format!("loop.{}", loop_id.as_str()),
            &format!("{}/{}", kind.as_str(), iter),
        );
        (worker_agent_ura.to_string(), loop_subject_ura)
    }
}

impl LoopInvocationDriver for KernelLoopInvocationDriver {
    fn invoke(
        &self,
        loop_id: &LoopId,
        iter: u32,
        worker_agent: &AgentId,
        authority: &DeferredInvocationAuthority,
        prompt: &str,
        kind: LoopInvocationKind,
        causal_context: axon_sdk::invocation::CausalContext,
    ) -> anyhow::Result<LoopInvocationOutcome> {
        if authority.execution_host_ura != self.identity.local_device_ura() {
            anyhow::bail!("loop authority execution host does not match this runtime");
        }
        let worker_agent_ura = authority.target_callee_ura.clone();
        let (worker_agent_ura, loop_subject_ura) =
            self.invocation_uras(loop_id, iter, &worker_agent_ura, kind);
        let payload = serde_json::to_vec(&json!({ "prompt": prompt }))
            .context("encode loop chat invocation payload")?;
        let request = self.kernel.prepare_accountable_user_rpc(
            &authority.accountable_user_ura,
            &worker_agent_ura,
            &format!("{}.chat", worker_agent.as_str()),
            &loop_subject_ura,
            payload,
            causal_context,
        )?;
        let finalized = self.kernel.invoke(request)?;
        let terminal_reason = || {
            finalized
                .failure
                .as_ref()
                .map(ToString::to_string)
                .filter(|reason| !reason.is_empty())
                .unwrap_or_else(|| finalized.terminal_receipt.reason().to_string())
        };
        match finalized.terminal_state {
            InvocationState::Completed => {}
            InvocationState::Cancelled => {
                anyhow::bail!("loop {} {} iter {} cancelled", loop_id, kind.as_str(), iter);
            }
            InvocationState::TimedOut => {
                anyhow::bail!(
                    "loop {} {} iter {} timed out: {}",
                    loop_id,
                    kind.as_str(),
                    iter,
                    terminal_reason()
                );
            }
            InvocationState::Failed => {
                anyhow::bail!(
                    "loop {} {} iter {} failed: {}",
                    loop_id,
                    kind.as_str(),
                    iter,
                    terminal_reason()
                );
            }
            state => anyhow::bail!(
                "loop {} {} iter {} returned non-terminal canonical state {}",
                loop_id,
                kind.as_str(),
                iter,
                state.as_str()
            ),
        }

        let response: Value = serde_json::from_slice(finalized.output()).with_context(|| {
            format!(
                "loop {} {} iter {}: decode canonical chat response",
                loop_id,
                kind.as_str(),
                iter
            )
        })?;
        let output = response
            .get("reply")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "loop {} {} iter {}: canonical chat response is missing reply",
                    loop_id,
                    kind.as_str(),
                    iter
                )
            })?;
        let invocation_id = finalized.terminal_receipt.invocation_id().to_string();
        let receipt_ura = format!(
            "{loop_subject_ura}/invocation/{invocation_id}/receipt/{}",
            finalized.terminal_receipt.index()
        );
        Ok(LoopInvocationOutcome {
            output,
            invocation_id,
            receipt_ura,
            receipt_hash: finalized.terminal_receipt.self_hash(),
        })
    }
}

#[derive(Debug)]
struct LoopStreamState {
    history: Vec<Value>,
    broadcast: broadcast::Sender<Value>,
}

impl LoopStreamState {
    fn new() -> Self {
        let (tx, _) = broadcast::channel(256);
        Self {
            history: Vec::new(),
            broadcast: tx,
        }
    }
}

#[derive(Default)]
pub struct LoopService {
    cache: RwLock<BTreeMap<LoopId, LoopInstance>>,
    tenant: RwLock<Option<TenantId>>,
    store: RwLock<Option<LoopStore>>,
    streams: RwLock<BTreeMap<LoopId, LoopStreamState>>,
    running: Mutex<BTreeSet<LoopId>>,
    driver: RwLock<Option<Arc<dyn LoopInvocationDriver>>>,
}

impl LoopService {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn install_driver(&self, driver: Arc<dyn LoopInvocationDriver>) -> anyhow::Result<()> {
        let mut guard = self
            .driver
            .write()
            .map_err(|_| anyhow::anyhow!("LoopService driver lock poisoned"))?;
        *guard = Some(driver);
        Ok(())
    }

    #[cfg(test)]
    pub fn bind_memory_for_test(&self, tenant: TenantId) {
        *self.tenant.write().expect("loop tenant") = Some(tenant);
    }

    pub fn bind(&self, tenant: &TenantId) -> anyhow::Result<()> {
        let store = LoopStore::open(tenant)?;
        let loaded = store.load_all()?;
        let mut cache = self
            .cache
            .write()
            .map_err(|_| anyhow::anyhow!("LoopService cache lock poisoned"))?;
        let mut streams = self
            .streams
            .write()
            .map_err(|_| anyhow::anyhow!("LoopService streams lock poisoned"))?;
        for inst in loaded {
            validate_loop_authority(&inst.worker_agent, &inst.authority)?;
            validate_loop_invocation_ledger(&inst)?;
            streams
                .entry(inst.id.clone())
                .or_insert_with(LoopStreamState::new);
            cache.insert(inst.id.clone(), inst);
        }
        let mut s = self
            .store
            .write()
            .map_err(|_| anyhow::anyhow!("LoopService store lock poisoned"))?;
        let mut bound_tenant = self
            .tenant
            .write()
            .map_err(|_| anyhow::anyhow!("LoopService tenant lock poisoned"))?;
        *bound_tenant = Some(tenant.clone());
        *s = Some(store);
        Ok(())
    }

    pub fn create(
        self: &Arc<Self>,
        worker_agent: AgentId,
        authority: DeferredInvocationAuthority,
        verify_expr: String,
        max_iters: u32,
        body_prompt: String,
    ) -> anyhow::Result<LoopId> {
        if max_iters == 0 {
            anyhow::bail!("loop.create: max_iters must be ≥ 1");
        }
        validate_loop_authority(&worker_agent, &authority)?;
        let tenant = self.bound_tenant()?;
        let id = LoopId::new(format!("loop-{}", Uuid::new_v4()));
        let instance = LoopInstance {
            id: id.clone(),
            tenant,
            worker_agent,
            authority,
            verify_expr,
            body_prompt,
            max_iters,
            current_iter: 0,
            state: LoopState::Pending,
            invocation_ledger: Vec::new(),
            last_body_output: None,
            last_verify_output: None,
        };
        self.ensure_stream_entry(&id)?;
        self.persist_instance(&instance)?;
        self.cache
            .write()
            .map_err(|_| anyhow::anyhow!("cache lock poisoned"))?
            .insert(id.clone(), instance);
        self.start_controller(&id)?;
        Ok(id)
    }

    fn bound_tenant(&self) -> anyhow::Result<TenantId> {
        self.tenant
            .read()
            .map_err(|_| anyhow::anyhow!("LoopService tenant lock poisoned"))?
            .clone()
            .ok_or_else(|| anyhow::anyhow!("LoopService is not bound to a runtime tenant"))
    }

    pub fn resume_inflight(self: &Arc<Self>) -> anyhow::Result<()> {
        let ids: Vec<LoopId> = self
            .list()?
            .into_iter()
            .filter(|inst| matches!(inst.state, LoopState::Pending | LoopState::Running))
            .map(|inst| inst.id)
            .collect();
        for id in ids {
            self.start_controller(&id)?;
        }
        Ok(())
    }

    pub fn update(&self, id: &LoopId, new_state: LoopState, new_iter: u32) -> anyhow::Result<()> {
        self.mutate(id, |inst| {
            inst.state = new_state;
            inst.current_iter = new_iter;
        })?;
        Ok(())
    }

    pub fn status(&self, id: &LoopId) -> anyhow::Result<Option<LoopInstance>> {
        Ok(self
            .cache
            .read()
            .map_err(|_| anyhow::anyhow!("LoopService cache lock poisoned"))?
            .get(id)
            .cloned())
    }

    pub fn status_for_accountable_user(
        &self,
        id: &LoopId,
        accountable_user_ura: &str,
    ) -> anyhow::Result<Option<LoopInstance>> {
        validate_accountable_user_scope(accountable_user_ura)?;
        let Some(instance) = self.status(id)? else {
            return Ok(None);
        };
        if instance.authority.accountable_user_ura != accountable_user_ura {
            anyhow::bail!("loop {id} is not owned by accountable User {accountable_user_ura}");
        }
        Ok(Some(instance))
    }

    pub fn list(&self) -> anyhow::Result<Vec<LoopInstance>> {
        let cache = self
            .cache
            .read()
            .map_err(|_| anyhow::anyhow!("LoopService cache lock poisoned"))?;
        Ok(cache.values().cloned().collect())
    }

    #[cfg(test)]
    pub fn poison_cache_for_test(&self) {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = self.cache.write().unwrap();
            panic!("poison loop cache");
        }));
    }

    pub fn subscribe(
        &self,
        id: &LoopId,
    ) -> anyhow::Result<(Vec<Value>, Option<broadcast::Receiver<Value>>)> {
        let inst = self
            .status(id)?
            .ok_or_else(|| anyhow::anyhow!("loop {id} not found"))?;
        self.ensure_stream_entry(id)?;
        let (history, rx) = {
            let streams = self
                .streams
                .read()
                .map_err(|_| anyhow::anyhow!("streams lock poisoned"))?;
            let state = streams
                .get(id)
                .ok_or_else(|| anyhow::anyhow!("loop stream {id} not found"))?;
            (state.history.clone(), state.broadcast.subscribe())
        };
        let snapshot = if history.is_empty() {
            synthesize_snapshot(&inst)
        } else {
            history
        };
        if is_terminal(&inst.state) {
            Ok((snapshot, None))
        } else {
            Ok((snapshot, Some(rx)))
        }
    }

    pub fn subscribe_for_accountable_user(
        &self,
        id: &LoopId,
        accountable_user_ura: &str,
    ) -> anyhow::Result<(Vec<Value>, Option<broadcast::Receiver<Value>>)> {
        self.status_for_accountable_user(id, accountable_user_ura)?
            .ok_or_else(|| anyhow::anyhow!("loop {id} not found"))?;
        self.subscribe(id)
    }

    pub fn cancel(&self, id: &LoopId) -> anyhow::Result<()> {
        let mut emit_terminal = false;
        let updated = {
            let mut cache = self
                .cache
                .write()
                .map_err(|_| anyhow::anyhow!("cache lock poisoned"))?;
            let inst = cache
                .get_mut(id)
                .ok_or_else(|| anyhow::anyhow!("loop {id} not found"))?;
            match inst.state {
                LoopState::Pending | LoopState::Running => {
                    inst.state = LoopState::Cancelled;
                    emit_terminal = true;
                }
                _ => {}
            }
            inst.clone()
        };
        self.persist_instance(&updated)?;
        if emit_terminal {
            self.record_frame(
                id,
                json!({
                    "kind": "terminal",
                    "loop_id": id.as_str(),
                    "state": LoopState::Cancelled,
                }),
            )?;
        }
        Ok(())
    }

    pub fn cancel_for_accountable_user(
        &self,
        id: &LoopId,
        accountable_user_ura: &str,
    ) -> anyhow::Result<()> {
        self.status_for_accountable_user(id, accountable_user_ura)?
            .ok_or_else(|| anyhow::anyhow!("loop {id} not found"))?;
        self.cancel(id)
    }

    fn driver(&self) -> anyhow::Result<Option<Arc<dyn LoopInvocationDriver>>> {
        Ok(self
            .driver
            .read()
            .map_err(|_| anyhow::anyhow!("LoopService driver lock poisoned"))?
            .clone())
    }

    fn start_controller(self: &Arc<Self>, id: &LoopId) -> anyhow::Result<()> {
        let Some(driver) = self.driver()? else {
            return Ok(());
        };
        let inst = self
            .status(id)?
            .ok_or_else(|| anyhow::anyhow!("loop {id} not found"))?;
        if is_terminal(&inst.state) {
            return Ok(());
        }
        {
            let mut running = self
                .running
                .lock()
                .map_err(|_| anyhow::anyhow!("LoopService running-set lock poisoned"))?;
            if !running.insert(id.clone()) {
                return Ok(());
            }
        }
        let svc = Arc::clone(self);
        let loop_id = id.clone();
        tokio::runtime::Handle::try_current()
            .map_err(|_| anyhow::anyhow!("loop controller requires a tokio runtime"))?
            .spawn(async move {
                let runner = Arc::clone(&svc);
                if let Err(e) = runner.drive_loop(loop_id.clone(), driver).await {
                    let detail = format!("{e:#}");
                    if let Ok(Some(inst)) = svc.status(&loop_id) {
                        if !is_terminal(&inst.state) {
                            let iter = inst.current_iter;
                            let _ = svc.mutate(&loop_id, |inst| {
                                inst.state = LoopState::Failed;
                                inst.last_verify_output = Some(detail.clone());
                            });
                            let _ = svc.record_frame(
                                &loop_id,
                                json!({
                                    "kind": "verify_chunk",
                                    "loop_id": loop_id.as_str(),
                                    "iter": iter,
                                    "text": detail,
                                }),
                            );
                            let _ = svc.record_terminal(&loop_id, LoopState::Failed);
                        }
                    }
                }
                if let Ok(mut running) = svc.running.lock() {
                    running.remove(&loop_id);
                }
            });
        Ok(())
    }

    async fn drive_loop(
        self: Arc<Self>,
        id: LoopId,
        driver: Arc<dyn LoopInvocationDriver>,
    ) -> anyhow::Result<()> {
        loop {
            let inst = self
                .status(&id)?
                .ok_or_else(|| anyhow::anyhow!("loop {id} vanished"))?;
            if is_terminal(&inst.state) {
                return Ok(());
            }
            let last = inst.invocation_ledger.last().cloned();
            if let Some(record) = &last {
                match record.state {
                    LoopInvocationState::Reserved => {
                        let error = format!(
                            "loop restart found ambiguous reserved {} invocation at iter {}; refusing duplicate dispatch",
                            record.kind.as_str(),
                            record.iter
                        );
                        self.fail_loop_invocation(&id, record.iter, record.kind, error.clone())?;
                        self.mutate(&id, |state| {
                            state.last_verify_output = Some(error.clone());
                        })?;
                        self.record_terminal(&id, LoopState::Failed)?;
                        return Ok(());
                    }
                    LoopInvocationState::Failed => {
                        self.record_terminal(&id, LoopState::Failed)?;
                        return Ok(());
                    }
                    LoopInvocationState::Completed => {}
                }
            }

            let (iter, kind, prompt, parent) = match last.as_ref() {
                None => (1, LoopInvocationKind::Body, inst.body_prompt.clone(), None),
                Some(record) if record.kind == LoopInvocationKind::Body => {
                    let body_output = record.output.clone().ok_or_else(|| {
                        anyhow::anyhow!("completed loop body record is missing output")
                    })?;
                    (
                        record.iter,
                        LoopInvocationKind::Verify,
                        render_verify_prompt(&inst.verify_expr, record.iter, &body_output),
                        Some(record),
                    )
                }
                Some(record) => {
                    let verify_output = record.output.as_deref().ok_or_else(|| {
                        anyhow::anyhow!("completed loop verify record is missing output")
                    })?;
                    let passed = match parse_verify_done(verify_output) {
                        Ok(passed) => passed,
                        Err(error) => {
                            self.mutate(&id, |state| {
                                state.last_verify_output =
                                    Some(format!("{verify_output} | parse error: {error}"));
                            })?;
                            self.record_terminal(&id, LoopState::VerifyMalformed)?;
                            return Ok(());
                        }
                    };
                    self.record_frame(
                        &id,
                        json!({
                            "kind": "iter_finished",
                            "loop_id": id.as_str(),
                            "iter": record.iter,
                            "verify_passed": passed,
                        }),
                    )?;
                    if passed {
                        self.record_terminal(&id, LoopState::Done)?;
                        return Ok(());
                    }
                    if record.iter >= inst.max_iters {
                        self.record_terminal(&id, LoopState::Exhausted)?;
                        return Ok(());
                    }
                    (
                        record.iter + 1,
                        LoopInvocationKind::Body,
                        inst.body_prompt.clone(),
                        Some(record),
                    )
                }
            };

            if self.is_cancelled(&id)? {
                return Ok(());
            }
            if kind == LoopInvocationKind::Body {
                self.mutate(&id, |state| {
                    state.state = LoopState::Running;
                    state.current_iter = iter;
                })?;
                self.record_frame(
                    &id,
                    json!({"kind": "iter_started", "loop_id": id.as_str(), "iter": iter}),
                )?;
            }
            let causal_context = loop_causal_context(parent)?;
            self.reserve_loop_invocation(&id, iter, kind, parent)?;
            match invoke_blocking(
                Arc::clone(&driver),
                id.clone(),
                iter,
                inst.worker_agent.clone(),
                inst.authority.clone(),
                prompt,
                kind,
                causal_context,
            )
            .await
            {
                Ok(outcome) => {
                    self.complete_loop_invocation(&id, iter, kind, &outcome)?;
                    self.mutate(&id, |state| match kind {
                        LoopInvocationKind::Body => {
                            state.last_body_output = Some(outcome.output.clone())
                        }
                        LoopInvocationKind::Verify => {
                            state.last_verify_output = Some(outcome.output.clone())
                        }
                    })?;
                    self.record_frame(
                        &id,
                        json!({
                            "kind": format!("{}_chunk", kind.as_str()),
                            "loop_id": id.as_str(),
                            "iter": iter,
                            "text": outcome.output,
                            "invocation_id": outcome.invocation_id,
                            "receipt_hash": hex::encode(outcome.receipt_hash),
                        }),
                    )?;
                }
                Err(error) => {
                    let detail = format!("[{}_error] {error:#}", kind.as_str());
                    self.fail_loop_invocation(&id, iter, kind, detail.clone())?;
                    self.mutate(&id, |state| match kind {
                        LoopInvocationKind::Body => state.last_body_output = Some(detail.clone()),
                        LoopInvocationKind::Verify => {
                            state.last_verify_output = Some(detail.clone())
                        }
                    })?;
                    self.record_frame(
                        &id,
                        json!({
                            "kind": format!("{}_chunk", kind.as_str()),
                            "loop_id": id.as_str(),
                            "iter": iter,
                            "text": detail,
                        }),
                    )?;
                    self.record_terminal(&id, LoopState::Failed)?;
                    return Ok(());
                }
            }
        }
    }

    fn reserve_loop_invocation(
        &self,
        id: &LoopId,
        iter: u32,
        kind: LoopInvocationKind,
        parent: Option<&LoopInvocationRecord>,
    ) -> anyhow::Result<()> {
        let inst = self
            .status(id)?
            .ok_or_else(|| anyhow::anyhow!("loop {id} not found"))?;
        if inst
            .invocation_ledger
            .iter()
            .any(|record| record.iter == iter && record.kind == kind)
        {
            anyhow::bail!(
                "loop {id} already has a {} record for iter {iter}",
                kind.as_str()
            );
        }
        self.mutate(id, |state| {
            state.invocation_ledger.push(LoopInvocationRecord {
                iter,
                kind,
                state: LoopInvocationState::Reserved,
                invocation_id: None,
                receipt_ura: None,
                receipt_hash: None,
                causal_parent_receipt_ura: parent.and_then(|record| record.receipt_ura.clone()),
                output: None,
                error: None,
            });
        })?;
        Ok(())
    }

    fn complete_loop_invocation(
        &self,
        id: &LoopId,
        iter: u32,
        kind: LoopInvocationKind,
        outcome: &LoopInvocationOutcome,
    ) -> anyhow::Result<()> {
        self.update_reserved_loop_invocation(id, iter, kind, |record| {
            record.state = LoopInvocationState::Completed;
            record.invocation_id = Some(outcome.invocation_id.clone());
            record.receipt_ura = Some(outcome.receipt_ura.clone());
            record.receipt_hash = Some(hex::encode(outcome.receipt_hash));
            record.output = Some(outcome.output.clone());
        })
    }

    fn fail_loop_invocation(
        &self,
        id: &LoopId,
        iter: u32,
        kind: LoopInvocationKind,
        error: String,
    ) -> anyhow::Result<()> {
        self.update_reserved_loop_invocation(id, iter, kind, |record| {
            record.state = LoopInvocationState::Failed;
            record.error = Some(error);
        })
    }

    fn update_reserved_loop_invocation(
        &self,
        id: &LoopId,
        iter: u32,
        kind: LoopInvocationKind,
        update: impl FnOnce(&mut LoopInvocationRecord),
    ) -> anyhow::Result<()> {
        let inst = self
            .status(id)?
            .ok_or_else(|| anyhow::anyhow!("loop {id} not found"))?;
        let record = inst
            .invocation_ledger
            .iter()
            .rev()
            .find(|record| record.iter == iter && record.kind == kind)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "loop {id} has no {} reservation for iter {iter}",
                    kind.as_str()
                )
            })?;
        if record.state != LoopInvocationState::Reserved {
            anyhow::bail!(
                "loop {id} {} iter {iter} cannot transition from {:?}",
                kind.as_str(),
                record.state
            );
        }
        self.mutate(id, |state| {
            let record = state
                .invocation_ledger
                .iter_mut()
                .rev()
                .find(|record| record.iter == iter && record.kind == kind)
                .expect("validated loop invocation record must remain present");
            update(record);
        })?;
        Ok(())
    }

    fn ensure_stream_entry(&self, id: &LoopId) -> anyhow::Result<()> {
        let mut streams = self
            .streams
            .write()
            .map_err(|_| anyhow::anyhow!("streams lock poisoned"))?;
        streams
            .entry(id.clone())
            .or_insert_with(LoopStreamState::new);
        Ok(())
    }

    fn persist_instance(&self, instance: &LoopInstance) -> anyhow::Result<()> {
        if let Some(store) = self
            .store
            .read()
            .map_err(|_| anyhow::anyhow!("store lock poisoned"))?
            .as_ref()
        {
            store.save(instance)?;
        }
        Ok(())
    }

    fn mutate<F>(&self, id: &LoopId, update: F) -> anyhow::Result<LoopInstance>
    where
        F: FnOnce(&mut LoopInstance),
    {
        let updated = {
            let mut cache = self
                .cache
                .write()
                .map_err(|_| anyhow::anyhow!("cache lock poisoned"))?;
            let inst = cache
                .get_mut(id)
                .ok_or_else(|| anyhow::anyhow!("loop {id} not found"))?;
            update(inst);
            inst.clone()
        };
        self.persist_instance(&updated)?;
        Ok(updated)
    }

    fn record_frame(&self, id: &LoopId, frame: Value) -> anyhow::Result<()> {
        self.ensure_stream_entry(id)?;
        let sender = {
            let mut streams = self
                .streams
                .write()
                .map_err(|_| anyhow::anyhow!("streams lock poisoned"))?;
            let state = streams
                .get_mut(id)
                .ok_or_else(|| anyhow::anyhow!("loop stream {id} not found"))?;
            state.history.push(frame.clone());
            state.broadcast.clone()
        };
        let _ = sender.send(frame);
        Ok(())
    }

    fn record_terminal(&self, id: &LoopId, state: LoopState) -> anyhow::Result<()> {
        let inst = self.mutate(id, |loop_inst| loop_inst.state = state.clone())?;
        self.record_frame(
            id,
            json!({
                "kind": "terminal",
                "loop_id": id.as_str(),
                "state": inst.state,
            }),
        )
    }

    fn is_cancelled(&self, id: &LoopId) -> anyhow::Result<bool> {
        Ok(matches!(
            self.status(id)?,
            Some(inst) if inst.state == LoopState::Cancelled
        ))
    }
}

fn validate_loop_authority(
    worker_agent: &AgentId,
    authority: &DeferredInvocationAuthority,
) -> anyhow::Result<()> {
    validate_accountable_user_scope(&authority.accountable_user_ura)?;
    let accountable_user = crate::core::ura::parse_ura(&authority.accountable_user_ura)
        .expect("validated accountable User URA must parse");
    let controller = crate::core::ura::parse_ura(&authority.controller_callee_ura)
        .map_err(|error| anyhow::anyhow!("loop controller is invalid: {error}"))?;
    let Some((controller_device, controller_agent)) = controller.device_agent_ids() else {
        anyhow::bail!("loop controller must be a device-sponsored SystemAgent");
    };
    if controller_agent != crate::daemon::ability::names::automation::AUTOMATION_SYSTEM_AGENT_ID {
        anyhow::bail!("loop controller must be the automation SystemAgent");
    }
    let host = crate::core::ura::parse_ura(&authority.execution_host_ura)
        .map_err(|error| anyhow::anyhow!("loop execution host is invalid: {error}"))?;
    if host.kind != crate::core::ura::URAKind::Device
        || host.device_id() != Some(controller_device)
        || host.realm != controller.realm
    {
        anyhow::bail!("loop controller sponsor and execution host must match");
    }
    let target = crate::core::ura::parse_ura(&authority.target_callee_ura)
        .map_err(|error| anyhow::anyhow!("loop target callee is invalid: {error}"))?;
    let Some((owner_user, target_agent)) = target.agent_ids() else {
        anyhow::bail!("loop target callee must be a hosted User Agent");
    };
    if target.realm != controller.realm
        || target.realm != accountable_user.realm
        || Some(owner_user) != accountable_user.user_id()
        || target_agent != worker_agent.as_str()
    {
        anyhow::bail!("loop worker identity does not match deferred target callee");
    }
    if authority.creator_invocation_id.trim().is_empty() {
        anyhow::bail!("loop authority requires its creator invocation id");
    }
    Ok(())
}

fn validate_accountable_user_scope(accountable_user_ura: &str) -> anyhow::Result<()> {
    let user = crate::core::ura::parse_ura(accountable_user_ura)
        .map_err(|error| anyhow::anyhow!("loop accountable User is invalid: {error}"))?;
    if user.kind != crate::core::ura::URAKind::User || user.user_id().is_none() {
        anyhow::bail!("loop accountable principal must be a canonical User URA");
    }
    let canonical = crate::core::ura::user_ura(
        &user.realm,
        user.user_id()
            .expect("validated accountable user id must exist"),
    );
    if canonical != accountable_user_ura {
        anyhow::bail!("loop accountable User URA must be canonical");
    }
    Ok(())
}

fn validate_loop_invocation_ledger(inst: &LoopInstance) -> anyhow::Result<()> {
    let mut previous: Option<&LoopInvocationRecord> = None;
    for (index, record) in inst.invocation_ledger.iter().enumerate() {
        let expected_kind = match previous {
            None => LoopInvocationKind::Body,
            Some(previous) if previous.kind == LoopInvocationKind::Body => {
                LoopInvocationKind::Verify
            }
            Some(_) => LoopInvocationKind::Body,
        };
        let expected_iter = match previous {
            None => 1,
            Some(previous) if expected_kind == LoopInvocationKind::Verify => previous.iter,
            Some(previous) => previous.iter + 1,
        };
        if record.kind != expected_kind || record.iter != expected_iter {
            anyhow::bail!(
                "loop {} invocation ledger expected {:?} iter {}, got {:?} iter {}",
                inst.id,
                expected_kind,
                expected_iter,
                record.kind,
                record.iter
            );
        }
        let expected_parent = previous.and_then(|previous| previous.receipt_ura.as_deref());
        if record.causal_parent_receipt_ura.as_deref() != expected_parent {
            anyhow::bail!(
                "loop {} invocation ledger has a broken causal parent link",
                inst.id
            );
        }
        match record.state {
            LoopInvocationState::Reserved => {
                if record.invocation_id.is_some()
                    || record.receipt_ura.is_some()
                    || record.receipt_hash.is_some()
                    || record.output.is_some()
                    || record.error.is_some()
                {
                    anyhow::bail!(
                        "loop {} reserved invocation carries terminal facts",
                        inst.id
                    );
                }
            }
            LoopInvocationState::Completed => {
                let hash = record.receipt_hash.as_deref().unwrap_or_default();
                if record.invocation_id.as_deref().is_none_or(str::is_empty)
                    || record.receipt_ura.as_deref().is_none_or(str::is_empty)
                    || record.output.is_none()
                    || hex::decode(hash).map_or(true, |bytes| bytes.len() != 32)
                    || record.error.is_some()
                {
                    anyhow::bail!(
                        "loop {} completed invocation has invalid receipt facts",
                        inst.id
                    );
                }
            }
            LoopInvocationState::Failed => {
                if record.error.as_deref().is_none_or(str::is_empty) {
                    anyhow::bail!("loop {} failed invocation is missing its error", inst.id);
                }
            }
        }
        if record.state != LoopInvocationState::Completed
            && index + 1 != inst.invocation_ledger.len()
        {
            anyhow::bail!(
                "loop {} has records after a non-completed invocation",
                inst.id
            );
        }
        previous = Some(record);
    }
    if let Some(last) = previous {
        if inst.current_iter != last.iter {
            anyhow::bail!(
                "loop {} current_iter does not match its invocation ledger",
                inst.id
            );
        }
    } else if inst.current_iter != 0 {
        anyhow::bail!(
            "loop {} has current_iter without invocation records",
            inst.id
        );
    }
    Ok(())
}

fn is_terminal(state: &LoopState) -> bool {
    matches!(
        state,
        LoopState::Done
            | LoopState::Exhausted
            | LoopState::VerifyMalformed
            | LoopState::Failed
            | LoopState::Cancelled
    )
}

fn synthesize_snapshot(inst: &LoopInstance) -> Vec<Value> {
    let mut frames = Vec::new();
    if matches!(inst.state, LoopState::Running) && inst.current_iter > 0 {
        frames.push(json!({
            "kind": "iter_started",
            "loop_id": inst.id.as_str(),
            "iter": inst.current_iter,
        }));
    }
    if let Some(body) = &inst.last_body_output {
        frames.push(json!({
            "kind": "body_chunk",
            "loop_id": inst.id.as_str(),
            "iter": inst.current_iter,
            "text": body,
        }));
    }
    if let Some(verify) = &inst.last_verify_output {
        frames.push(json!({
            "kind": "verify_chunk",
            "loop_id": inst.id.as_str(),
            "iter": inst.current_iter,
            "text": verify,
        }));
    }
    if is_terminal(&inst.state) {
        frames.push(json!({
            "kind": "terminal",
            "loop_id": inst.id.as_str(),
            "state": inst.state,
        }));
    }
    frames
}

fn loop_causal_context(
    parent: Option<&LoopInvocationRecord>,
) -> anyhow::Result<axon_sdk::invocation::CausalContext> {
    let Some(parent) = parent else {
        return Ok(axon_sdk::invocation::CausalContext::None);
    };
    if parent.state != LoopInvocationState::Completed {
        anyhow::bail!("loop causal parent must be a completed invocation record");
    }
    let receipt_ura = parent
        .receipt_ura
        .clone()
        .ok_or_else(|| anyhow::anyhow!("loop causal parent is missing receipt URA"))?;
    let receipt_hash = parent
        .receipt_hash
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("loop causal parent is missing receipt hash"))?;
    let bytes = hex::decode(receipt_hash).context("decode loop causal parent receipt hash")?;
    let receipt_hash: [u8; 32] = bytes.try_into().map_err(|bytes: Vec<u8>| {
        anyhow::anyhow!(
            "loop causal parent receipt hash must be 32 bytes, got {}",
            bytes.len()
        )
    })?;
    Ok(axon_sdk::invocation::CausalContext::Scalar(
        axon_sdk::invocation::ReceiptRef {
            receipt_hash,
            receipt_ura,
        },
    ))
}

fn render_verify_prompt(expr: &str, iter: u32, body_output: &str) -> String {
    format!(
        "Loop verify iteration {iter}.\n\n\
Verify expression:\n{expr}\n\n\
Body output:\n{body_output}\n\n\
Return ONLY valid JSON with a top-level boolean field named `done`, for example \
{{\"done\": true}} or {{\"done\": false}}."
    )
}

fn parse_verify_done(output: &str) -> anyhow::Result<bool> {
    let value: Value = serde_json::from_str(output).with_context(|| {
        format!("verify output must be valid JSON object with top-level done bool, got {output:?}")
    })?;
    value
        .get("done")
        .and_then(Value::as_bool)
        .ok_or_else(|| anyhow::anyhow!("verify output missing top-level `done: bool`: {output}"))
}

async fn invoke_blocking(
    driver: Arc<dyn LoopInvocationDriver>,
    loop_id: LoopId,
    iter: u32,
    worker_agent: AgentId,
    authority: DeferredInvocationAuthority,
    prompt: String,
    kind: LoopInvocationKind,
    causal_context: axon_sdk::invocation::CausalContext,
) -> anyhow::Result<LoopInvocationOutcome> {
    let join_loop_id = loop_id.clone();
    tokio::task::spawn_blocking(move || {
        driver.invoke(
            &join_loop_id,
            iter,
            &worker_agent,
            &authority,
            &prompt,
            kind,
            causal_context,
        )
    })
    .await
    .map_err(|e| {
        anyhow::anyhow!(
            "loop {} {} iter {} join error: {e}",
            loop_id,
            kind.as_str(),
            iter
        )
    })?
}

impl std::fmt::Debug for LoopService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.cache.read() {
            Ok(cache) => write!(f, "LoopService {{ loops: {} }}", cache.len()),
            Err(_) => write!(f, "LoopService {{ loops: unavailable }}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::domain::{
        DiscussRoom, NodeId, PermissionDecision, PermissionId, PermissionRequest, RoomId,
        ScheduleEntry, ScheduleId, Session, SessionId,
    };
    use std::collections::VecDeque;

    #[derive(Clone)]
    struct ScriptedDriver {
        body: Arc<Mutex<VecDeque<anyhow::Result<String>>>>,
        verify: Arc<Mutex<VecDeque<anyhow::Result<String>>>>,
    }

    impl ScriptedDriver {
        fn new(body: Vec<anyhow::Result<&str>>, verify: Vec<anyhow::Result<&str>>) -> Self {
            Self {
                body: Arc::new(Mutex::new(
                    body.into_iter().map(|r| r.map(str::to_owned)).collect(),
                )),
                verify: Arc::new(Mutex::new(
                    verify.into_iter().map(|r| r.map(str::to_owned)).collect(),
                )),
            }
        }
    }

    impl LoopInvocationDriver for ScriptedDriver {
        fn invoke(
            &self,
            _loop_id: &LoopId,
            _iter: u32,
            _worker_agent: &AgentId,
            _authority: &DeferredInvocationAuthority,
            _prompt: &str,
            kind: LoopInvocationKind,
            _causal_context: axon_sdk::invocation::CausalContext,
        ) -> anyhow::Result<LoopInvocationOutcome> {
            let queue = match kind {
                LoopInvocationKind::Body => &self.body,
                LoopInvocationKind::Verify => &self.verify,
            };
            let output = queue
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| Err(anyhow::anyhow!("no scripted response left")))?;
            let marker = match kind {
                LoopInvocationKind::Body => 1,
                LoopInvocationKind::Verify => 2,
            };
            Ok(LoopInvocationOutcome {
                output,
                invocation_id: format!("test-inv-{marker}"),
                receipt_ura: format!(
                    "easynet:///r/tenant-a/resource/loop.test/invocation/test-inv-{marker}/receipt/1"
                ),
                receipt_hash: [marker; 32],
            })
        }
    }

    async fn wait_for_terminal(svc: &LoopService, id: &LoopId) -> LoopInstance {
        for _ in 0..100 {
            if let Some(inst) = svc.status(id).expect("read loop status") {
                if is_terminal(&inst.state) {
                    return inst;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("loop {id} did not reach terminal state in time");
    }

    fn bound_service() -> Arc<LoopService> {
        let svc = Arc::new(LoopService::new());
        svc.bind_memory_for_test(TenantId::new("tenant-a"));
        svc
    }

    fn loop_authority(worker: &str) -> DeferredInvocationAuthority {
        DeferredInvocationAuthority {
            accountable_user_ura: crate::core::ura::user_ura("tenant-a", "user-1"),
            creator_invocation_id: "test-loop-create".to_string(),
            controller_callee_ura: crate::core::ura::device_agent_ura(
                "tenant-a",
                "node-a",
                crate::daemon::ability::names::automation::AUTOMATION_SYSTEM_AGENT_ID,
            ),
            target_callee_ura: crate::core::ura::agent_ura("tenant-a", "user-1", worker),
            execution_host_ura: crate::core::ura::device_ura("tenant-a", "node-a"),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn create_then_status_self_drives_to_done() {
        let svc = bound_service();
        svc.install_driver(Arc::new(ScriptedDriver::new(
            vec![Ok("body ok")],
            vec![Ok(r#"{"done": true}"#)],
        )))
        .unwrap();
        let id = svc
            .create(
                AgentId::new("alice"),
                loop_authority("alice"),
                "true".into(),
                3,
                "do work".into(),
            )
            .unwrap();
        let st = wait_for_terminal(&svc, &id).await;
        assert_eq!(st.state, LoopState::Done);
        assert_eq!(st.current_iter, 1);
        assert_eq!(st.last_body_output.as_deref(), Some("body ok"));
        assert_eq!(st.last_verify_output.as_deref(), Some(r#"{"done": true}"#));
        assert_eq!(st.invocation_ledger.len(), 2);
        let body = &st.invocation_ledger[0];
        let verify = &st.invocation_ledger[1];
        assert_eq!(body.kind, LoopInvocationKind::Body);
        assert_eq!(body.state, LoopInvocationState::Completed);
        assert_eq!(verify.kind, LoopInvocationKind::Verify);
        assert_eq!(verify.state, LoopInvocationState::Completed);
        assert_eq!(
            verify.causal_parent_receipt_ura.as_deref(),
            body.receipt_ura.as_deref()
        );
        assert!(body.invocation_id.is_some());
        assert!(verify.receipt_hash.is_some());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn reserved_invocation_on_restart_fails_without_duplicate_dispatch() {
        let svc = bound_service();
        let id = svc
            .create(
                AgentId::new("alice"),
                loop_authority("alice"),
                "true".into(),
                1,
                "do work".into(),
            )
            .unwrap();
        svc.update(&id, LoopState::Running, 1).unwrap();
        svc.reserve_loop_invocation(&id, 1, LoopInvocationKind::Body, None)
            .unwrap();
        svc.install_driver(Arc::new(ScriptedDriver::new(
            vec![Ok("must-not-dispatch")],
            vec![],
        )))
        .unwrap();

        svc.resume_inflight().unwrap();
        let st = wait_for_terminal(&svc, &id).await;
        assert_eq!(st.state, LoopState::Failed);
        assert_eq!(st.invocation_ledger.len(), 1);
        assert_eq!(st.invocation_ledger[0].state, LoopInvocationState::Failed);
        assert!(st.invocation_ledger[0]
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("ambiguous reserved"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn verify_false_exhausts_after_max_iters() {
        let svc = bound_service();
        svc.install_driver(Arc::new(ScriptedDriver::new(
            vec![Ok("body-1"), Ok("body-2")],
            vec![Ok(r#"{"done": false}"#), Ok(r#"{"done": false}"#)],
        )))
        .unwrap();
        let id = svc
            .create(
                AgentId::new("alice"),
                loop_authority("alice"),
                "done stays false".into(),
                2,
                "retry".into(),
            )
            .unwrap();
        let st = wait_for_terminal(&svc, &id).await;
        assert_eq!(st.state, LoopState::Exhausted);
        assert_eq!(st.current_iter, 2);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn malformed_verify_terminates_as_verify_malformed() {
        let svc = bound_service();
        svc.install_driver(Arc::new(ScriptedDriver::new(
            vec![Ok("body-1")],
            vec![Ok("not-json")],
        )))
        .unwrap();
        let id = svc
            .create(
                AgentId::new("alice"),
                loop_authority("alice"),
                "must return json".into(),
                1,
                "retry".into(),
            )
            .unwrap();
        let st = wait_for_terminal(&svc, &id).await;
        assert_eq!(st.state, LoopState::VerifyMalformed);
        assert!(st
            .last_verify_output
            .as_deref()
            .unwrap_or("")
            .contains("parse error"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn subscribe_replays_history_and_terminal_snapshot() {
        let svc = bound_service();
        svc.install_driver(Arc::new(ScriptedDriver::new(
            vec![Ok("body ok")],
            vec![Ok(r#"{"done": true}"#)],
        )))
        .unwrap();
        let id = svc
            .create(
                AgentId::new("alice"),
                loop_authority("alice"),
                "true".into(),
                1,
                "do work".into(),
            )
            .unwrap();
        let _ = wait_for_terminal(&svc, &id).await;
        let (snapshot, live) = svc.subscribe(&id).unwrap();
        assert!(live.is_none());
        assert!(snapshot.iter().any(|f| f["kind"] == "iter_started"));
        assert!(snapshot.iter().any(|f| f["kind"] == "body_chunk"));
        assert!(snapshot.iter().any(|f| f["kind"] == "verify_chunk"));
        assert!(snapshot.iter().any(|f| f["kind"] == "terminal"));
    }

    #[test]
    fn create_zero_iters_errors() {
        let svc = Arc::new(LoopService::new());
        let err = svc
            .create(
                AgentId::new("alice"),
                loop_authority("alice"),
                "true".into(),
                0,
                "x".into(),
            )
            .unwrap_err();
        assert!(format!("{err}").contains("max_iters"));
    }

    #[test]
    fn create_rejects_accountable_user_that_does_not_own_worker_agent() {
        let svc = bound_service();
        let mut authority = loop_authority("alice");
        authority.accountable_user_ura = crate::core::ura::user_ura("tenant-a", "other-user");
        let error = svc
            .create(
                AgentId::new("alice"),
                authority,
                "true".into(),
                1,
                "work".into(),
            )
            .expect_err("deferred User must own the worker Agent");
        assert!(
            error.to_string().contains("worker identity does not match"),
            "{error:#}"
        );
    }

    #[test]
    fn create_rejects_unbound_runtime_tenant() {
        let svc = Arc::new(LoopService::new());
        let err = svc
            .create(
                AgentId::new("alice"),
                loop_authority("alice"),
                "true".into(),
                1,
                "x".into(),
            )
            .unwrap_err();
        assert!(format!("{err}").contains("not bound to a runtime tenant"));
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
        for i in ["c", "a", "b"] {
            let mut cache = svc.cache.write().unwrap();
            let id = LoopId::new(format!("loop-{i}"));
            cache.insert(
                id.clone(),
                LoopInstance {
                    id,
                    authority: loop_authority("alice"),
                    tenant: TenantId::default_v1(),
                    worker_agent: AgentId::new("a"),
                    verify_expr: "true".into(),
                    body_prompt: "go".into(),
                    max_iters: 1,
                    current_iter: 0,
                    state: LoopState::Pending,
                    invocation_ledger: Vec::new(),
                    last_body_output: None,
                    last_verify_output: None,
                },
            );
        }
        let ids: Vec<_> = svc
            .list()
            .expect("list loops")
            .into_iter()
            .map(|i| i.id)
            .collect();
        assert_eq!(
            ids,
            vec![
                LoopId::new("loop-a"),
                LoopId::new("loop-b"),
                LoopId::new("loop-c"),
            ]
        );
    }

    #[test]
    fn status_rejects_poisoned_cache_instead_of_unknown_loop() {
        let svc = LoopService::new();
        svc.poison_cache_for_test();
        let err = svc
            .status(&LoopId::new("ghost"))
            .expect_err("poisoned cache must fail status");
        assert!(
            format!("{err:#}").contains("LoopService cache lock poisoned"),
            "{err:#}"
        );
    }

    #[test]
    fn list_rejects_poisoned_cache_instead_of_empty_loops() {
        let svc = LoopService::new();
        svc.poison_cache_for_test();
        let err = svc.list().expect_err("poisoned cache must fail list");
        assert!(
            format!("{err:#}").contains("LoopService cache lock poisoned"),
            "{err:#}"
        );
    }

    #[test]
    fn kernel_loop_driver_projects_configured_realm_into_invocation_tuple() {
        struct UnusedKernel;

        impl KernelApi for UnusedKernel {
            fn prepare_local_system_rpc(
                &self,
                _callee_ura: &str,
                _ability: &str,
                _subject_ura: &str,
                _payload: Vec<u8>,
            ) -> anyhow::Result<axon_sdk::invocation::DescriptorBoundInvocationRequest>
            {
                unreachable!("projection test must not enter KernelApi")
            }

            fn prepare_accountable_user_rpc(
                &self,
                _accountable_user_ura: &str,
                _callee_ura: &str,
                _ability: &str,
                _subject_ura: &str,
                _payload: Vec<u8>,
                _causal_context: axon_sdk::invocation::CausalContext,
            ) -> anyhow::Result<axon_sdk::invocation::DescriptorBoundInvocationRequest>
            {
                unreachable!("projection test must not enter KernelApi")
            }

            fn invoke(
                &self,
                _request: axon_sdk::invocation::DescriptorBoundInvocationRequest,
            ) -> anyhow::Result<axon_sdk::invocation::FinalizedInvocation> {
                unreachable!("projection test must not enter KernelApi")
            }

            fn list_active_sessions(&self) -> anyhow::Result<Vec<Session>> {
                unreachable!("projection test must not enter KernelApi")
            }

            fn get_session(&self, _id: &SessionId) -> anyhow::Result<Option<Session>> {
                unreachable!("projection test must not enter KernelApi")
            }

            fn pending_permission_requests(&self) -> anyhow::Result<Vec<PermissionRequest>> {
                unreachable!("projection test must not enter KernelApi")
            }

            fn decide_permission(
                &self,
                _id: &PermissionId,
                _decision: PermissionDecision,
            ) -> anyhow::Result<()> {
                unreachable!("projection test must not enter KernelApi")
            }

            fn list_schedules(&self) -> anyhow::Result<Vec<ScheduleEntry>> {
                unreachable!("projection test must not enter KernelApi")
            }

            fn add_schedule(&self, _entry: ScheduleEntry) -> anyhow::Result<ScheduleId> {
                unreachable!("projection test must not enter KernelApi")
            }

            fn remove_schedule(&self, _id: &ScheduleId) -> anyhow::Result<()> {
                unreachable!("projection test must not enter KernelApi")
            }

            fn enable_schedule(&self, _id: &ScheduleId, _enabled: bool) -> anyhow::Result<()> {
                unreachable!("projection test must not enter KernelApi")
            }

            fn create_discuss_room(
                &self,
                _participants: Vec<String>,
                _topic: Option<String>,
            ) -> anyhow::Result<RoomId> {
                unreachable!("projection test must not enter KernelApi")
            }

            fn list_discuss_rooms(&self) -> anyhow::Result<Vec<DiscussRoom>> {
                unreachable!("projection test must not enter KernelApi")
            }

            fn loop_status(&self, _id: &LoopId) -> anyhow::Result<Option<LoopInstance>> {
                unreachable!("projection test must not enter KernelApi")
            }

            fn cancel_loop(&self, _id: &LoopId) -> anyhow::Result<()> {
                unreachable!("projection test must not enter KernelApi")
            }
        }

        let identity =
            LocalRuntimeInvocationIdentity::new("tenant-a", NodeId::new("node-a")).unwrap();
        let driver = KernelLoopInvocationDriver::new(Arc::new(UnusedKernel), identity);
        let (callee, subject) = driver.invocation_uras(
            &LoopId::new("loop-a"),
            7,
            "easynet:///r/tenant-a/agent/user-a.worker-a",
            LoopInvocationKind::Verify,
        );

        assert_eq!(callee, "easynet:///r/tenant-a/agent/user-a.worker-a");
        assert_eq!(
            subject,
            "easynet:///r/tenant-a/resource/loop.loop-a/verify/7"
        );
        assert!(!callee.contains("/r/default/"));
        assert!(!subject.contains("/r/default/"));
    }

    #[test]
    fn subscribe_rejects_poisoned_cache_before_unknown_loop_projection() {
        let svc = LoopService::new();
        svc.poison_cache_for_test();
        let err = svc
            .subscribe(&LoopId::new("ghost"))
            .expect_err("poisoned cache must fail subscribe");
        assert!(
            format!("{err:#}").contains("LoopService cache lock poisoned"),
            "{err:#}"
        );
    }
}
