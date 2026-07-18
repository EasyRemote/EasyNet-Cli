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

use crate::core::domain::{AgentId, LoopId, LoopInstance, LoopState, NodeId, TenantId};
use crate::daemon::boot::kernel::api::KernelApi;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopInvocationKind {
    Body,
    Verify,
}

impl LoopInvocationKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Body => "body",
            Self::Verify => "verify",
        }
    }
}

pub trait LoopInvocationDriver: Send + Sync {
    fn invoke(
        &self,
        loop_id: &LoopId,
        iter: u32,
        worker_agent: &AgentId,
        prompt: &str,
        kind: LoopInvocationKind,
    ) -> anyhow::Result<String>;
}

pub struct KernelLoopInvocationDriver {
    kernel: Arc<dyn KernelApi>,
    local_node: NodeId,
}

impl KernelLoopInvocationDriver {
    pub fn new(kernel: Arc<dyn KernelApi>, local_node: NodeId) -> Self {
        Self { kernel, local_node }
    }
}

impl LoopInvocationDriver for KernelLoopInvocationDriver {
    fn invoke(
        &self,
        loop_id: &LoopId,
        iter: u32,
        worker_agent: &AgentId,
        prompt: &str,
        kind: LoopInvocationKind,
    ) -> anyhow::Result<String> {
        let local_device_ura = crate::core::ura::device_ura("default", self.local_node.as_str());
        let loop_subject_ura = crate::core::ura::resource_dot_ura(
            "default",
            &format!("loop.{}", loop_id.as_str()),
            &format!("{}/{}", kind.as_str(), iter),
        );
        let payload = serde_json::to_vec(&json!({ "prompt": prompt }))
            .context("encode loop chat invocation payload")?;
        let request = self.kernel.prepare_local_system_rpc(
            &local_device_ura,
            &format!("{}.chat", worker_agent.as_str()),
            &loop_subject_ura,
            payload,
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
        response
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
            streams
                .entry(inst.id.clone())
                .or_insert_with(LoopStreamState::new);
            cache.insert(inst.id.clone(), inst);
        }
        let mut s = self
            .store
            .write()
            .map_err(|_| anyhow::anyhow!("LoopService store lock poisoned"))?;
        *s = Some(store);
        Ok(())
    }

    pub fn create(
        self: &Arc<Self>,
        worker_agent: AgentId,
        verify_expr: String,
        max_iters: u32,
        body_prompt: String,
    ) -> anyhow::Result<LoopId> {
        if max_iters == 0 {
            anyhow::bail!("loop.create: max_iters must be ≥ 1");
        }
        let id = LoopId::new(format!("loop-{}", Uuid::new_v4()));
        let instance = LoopInstance {
            id: id.clone(),
            tenant: TenantId::default_v1(),
            worker_agent,
            verify_expr,
            body_prompt,
            max_iters,
            current_iter: 0,
            state: LoopState::Pending,
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

    pub fn resume_inflight(self: &Arc<Self>) -> anyhow::Result<()> {
        let ids: Vec<LoopId> = self
            .list()
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

    pub fn status(&self, id: &LoopId) -> Option<LoopInstance> {
        self.cache.read().ok().and_then(|g| g.get(id).cloned())
    }

    pub fn list(&self) -> Vec<LoopInstance> {
        match self.cache.read() {
            Ok(g) => g.values().cloned().collect(),
            Err(_) => Vec::new(),
        }
    }

    pub fn subscribe(
        &self,
        id: &LoopId,
    ) -> anyhow::Result<(Vec<Value>, Option<broadcast::Receiver<Value>>)> {
        let inst = self
            .status(id)
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
        let inst = match self.status(id) {
            Some(inst) => inst,
            None => anyhow::bail!("loop {id} not found"),
        };
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
                    let already_terminal = svc
                        .status(&loop_id)
                        .map(|inst| is_terminal(&inst.state))
                        .unwrap_or(true);
                    if !already_terminal {
                        let _ = svc.mutate(&loop_id, |inst| {
                            inst.state = LoopState::VerifyMalformed;
                            inst.last_verify_output = Some(detail.clone());
                        });
                        let _ = svc.record_frame(
                            &loop_id,
                            json!({
                                "kind": "verify_chunk",
                                "loop_id": loop_id.as_str(),
                                "iter": svc.status(&loop_id).map(|i| i.current_iter).unwrap_or(0),
                                "text": detail,
                            }),
                        );
                        let _ = svc.record_terminal(&loop_id, LoopState::VerifyMalformed);
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
                .status(&id)
                .ok_or_else(|| anyhow::anyhow!("loop {id} vanished"))?;
            if is_terminal(&inst.state) {
                return Ok(());
            }
            let iter = inst.current_iter.saturating_add(1);
            if iter > inst.max_iters {
                self.record_terminal(&id, LoopState::Exhausted)?;
                return Ok(());
            }

            self.mutate(&id, |state| {
                state.state = LoopState::Running;
                state.current_iter = iter;
            })?;
            self.record_frame(
                &id,
                json!({"kind": "iter_started", "loop_id": id.as_str(), "iter": iter}),
            )?;

            if self.is_cancelled(&id)? {
                return Ok(());
            }

            let body_output = match invoke_blocking(
                Arc::clone(&driver),
                id.clone(),
                iter,
                inst.worker_agent.clone(),
                inst.body_prompt.clone(),
                LoopInvocationKind::Body,
            )
            .await
            {
                Ok(body_output) => {
                    self.mutate(&id, |state| {
                        state.last_body_output = Some(body_output.clone());
                    })?;
                    self.record_frame(
                        &id,
                        json!({
                            "kind": "body_chunk",
                            "loop_id": id.as_str(),
                            "iter": iter,
                            "text": body_output.clone(),
                        }),
                    )?;
                    body_output
                }
                Err(e) => {
                    let body_error = format!("[body_error] {e:#}");
                    self.mutate(&id, |state| {
                        state.last_body_output = Some(body_error.clone());
                    })?;
                    self.record_frame(
                        &id,
                        json!({
                            "kind": "body_chunk",
                            "loop_id": id.as_str(),
                            "iter": iter,
                            "text": body_error,
                        }),
                    )?;
                    self.record_frame(
                        &id,
                        json!({
                            "kind": "iter_finished",
                            "loop_id": id.as_str(),
                            "iter": iter,
                            "verify_passed": false,
                        }),
                    )?;
                    if iter >= inst.max_iters {
                        self.record_terminal(&id, LoopState::Exhausted)?;
                        return Ok(());
                    }
                    continue;
                }
            };

            if self.is_cancelled(&id)? {
                return Ok(());
            }

            let verify_prompt = render_verify_prompt(&inst.verify_expr, iter, &body_output);
            match invoke_blocking(
                Arc::clone(&driver),
                id.clone(),
                iter,
                inst.worker_agent.clone(),
                verify_prompt,
                LoopInvocationKind::Verify,
            )
            .await
            {
                Ok(verify_output) => {
                    self.mutate(&id, |state| {
                        state.last_verify_output = Some(verify_output.clone());
                    })?;
                    self.record_frame(
                        &id,
                        json!({
                            "kind": "verify_chunk",
                            "loop_id": id.as_str(),
                            "iter": iter,
                            "text": verify_output.clone(),
                        }),
                    )?;
                    match parse_verify_done(&verify_output) {
                        Ok(true) => {
                            self.record_frame(
                                &id,
                                json!({
                                    "kind": "iter_finished",
                                    "loop_id": id.as_str(),
                                    "iter": iter,
                                    "verify_passed": true,
                                }),
                            )?;
                            self.record_terminal(&id, LoopState::Done)?;
                            return Ok(());
                        }
                        Ok(false) => {
                            self.record_frame(
                                &id,
                                json!({
                                    "kind": "iter_finished",
                                    "loop_id": id.as_str(),
                                    "iter": iter,
                                    "verify_passed": false,
                                }),
                            )?;
                            if iter >= inst.max_iters {
                                self.record_terminal(&id, LoopState::Exhausted)?;
                                return Ok(());
                            }
                        }
                        Err(e) => {
                            self.mutate(&id, |state| {
                                state.last_verify_output =
                                    Some(format!("{} | parse error: {e}", verify_output));
                            })?;
                            self.record_terminal(&id, LoopState::VerifyMalformed)?;
                            return Ok(());
                        }
                    }
                }
                Err(e) => {
                    let verify_error = format!("[verify_error] {e:#}");
                    self.mutate(&id, |state| {
                        state.last_verify_output = Some(verify_error.clone());
                    })?;
                    self.record_frame(
                        &id,
                        json!({
                            "kind": "verify_chunk",
                            "loop_id": id.as_str(),
                            "iter": iter,
                            "text": verify_error,
                        }),
                    )?;
                    self.record_terminal(&id, LoopState::VerifyMalformed)?;
                    return Ok(());
                }
            }
        }
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
            self.status(id).map(|inst| inst.state),
            Some(LoopState::Cancelled)
        ))
    }
}

fn is_terminal(state: &LoopState) -> bool {
    matches!(
        state,
        LoopState::Done | LoopState::Exhausted | LoopState::VerifyMalformed | LoopState::Cancelled
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
    prompt: String,
    kind: LoopInvocationKind,
) -> anyhow::Result<String> {
    let join_loop_id = loop_id.clone();
    tokio::task::spawn_blocking(move || {
        driver.invoke(&join_loop_id, iter, &worker_agent, &prompt, kind)
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
        let n = self.cache.read().ok().map(|g| g.len()).unwrap_or(0);
        write!(f, "LoopService {{ loops: {n} }}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
            _prompt: &str,
            kind: LoopInvocationKind,
        ) -> anyhow::Result<String> {
            let queue = match kind {
                LoopInvocationKind::Body => &self.body,
                LoopInvocationKind::Verify => &self.verify,
            };
            queue
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| Err(anyhow::anyhow!("no scripted response left")))
        }
    }

    async fn wait_for_terminal(svc: &LoopService, id: &LoopId) -> LoopInstance {
        for _ in 0..100 {
            if let Some(inst) = svc.status(id) {
                if is_terminal(&inst.state) {
                    return inst;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("loop {id} did not reach terminal state in time");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn create_then_status_self_drives_to_done() {
        let svc = Arc::new(LoopService::new());
        svc.install_driver(Arc::new(ScriptedDriver::new(
            vec![Ok("body ok")],
            vec![Ok(r#"{"done": true}"#)],
        )))
        .unwrap();
        let id = svc
            .create(AgentId::new("alice"), "true".into(), 3, "do work".into())
            .unwrap();
        let st = wait_for_terminal(&svc, &id).await;
        assert_eq!(st.state, LoopState::Done);
        assert_eq!(st.current_iter, 1);
        assert_eq!(st.last_body_output.as_deref(), Some("body ok"));
        assert_eq!(st.last_verify_output.as_deref(), Some(r#"{"done": true}"#));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn verify_false_exhausts_after_max_iters() {
        let svc = Arc::new(LoopService::new());
        svc.install_driver(Arc::new(ScriptedDriver::new(
            vec![Ok("body-1"), Ok("body-2")],
            vec![Ok(r#"{"done": false}"#), Ok(r#"{"done": false}"#)],
        )))
        .unwrap();
        let id = svc
            .create(
                AgentId::new("alice"),
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
        let svc = Arc::new(LoopService::new());
        svc.install_driver(Arc::new(ScriptedDriver::new(
            vec![Ok("body-1")],
            vec![Ok("not-json")],
        )))
        .unwrap();
        let id = svc
            .create(
                AgentId::new("alice"),
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
        let svc = Arc::new(LoopService::new());
        svc.install_driver(Arc::new(ScriptedDriver::new(
            vec![Ok("body ok")],
            vec![Ok(r#"{"done": true}"#)],
        )))
        .unwrap();
        let id = svc
            .create(AgentId::new("alice"), "true".into(), 1, "do work".into())
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
            .create(AgentId::new("alice"), "true".into(), 0, "x".into())
            .unwrap_err();
        assert!(format!("{err}").contains("max_iters"));
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
                    tenant: TenantId::default_v1(),
                    worker_agent: AgentId::new("a"),
                    verify_expr: "true".into(),
                    body_prompt: "go".into(),
                    max_iters: 1,
                    current_iter: 0,
                    state: LoopState::Pending,
                    last_body_output: None,
                    last_verify_output: None,
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
