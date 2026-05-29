// EasyNet CLI — loop.{create,status,subscribe,cancel} (PR-LOOP)
// =====================================================================
//
// File: src/runtime/system/loop_ability.rs
// Description: Four abilities surfacing the loop primitive:
//
//   * `loop.create`    (RPC)    — register a new loop instance.
//   * `loop.status`    (RPC)    — fetch current state.
//   * `loop.subscribe` (Stream) — replay status snapshots.
//   * `loop.cancel`    (RPC)    — cancel an in-flight loop.
//
// Why suffix `_ability` and the file name `loop_ability.rs`
// ---------------------------------------------------------
// `loop` is a Rust keyword. Naming the module `loop` would force
// every importer into r#loop syntax. The suffix sidesteps the
// keyword while keeping the file's role obvious.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::runtime::ability_dispatch::OwnerKind;
use crate::runtime::ability_dispatch::{AxonAbilityCatalog, StreamSource};
use crate::runtime::domain::{AgentId, LoopId};
use crate::runtime::execution::loop_instance::LoopService;

pub const ABILITY_CREATE: &str = "device.loop.create";
pub const ABILITY_STATUS: &str = "device.loop.status";
pub const ABILITY_SUBSCRIBE: &str = "device.loop.subscribe";
pub const ABILITY_CANCEL: &str = "device.loop.cancel";

pub fn register(reg: &mut AxonAbilityCatalog, svc: Arc<LoopService>) {
    let a = Arc::clone(&svc);
    reg.register_rpc_with_owner(
        "device.loop.create",
        OwnerKind::Device,
        Arc::new(move |args| create_handler(&a, args)),
    );
    let b = Arc::clone(&svc);
    reg.register_rpc_with_owner(
        "device.loop.status",
        OwnerKind::Device,
        Arc::new(move |args| status_handler(&b, args)),
    );
    let c = Arc::clone(&svc);
    reg.register_stream_with_owner(
        "device.loop.subscribe",
        OwnerKind::Device,
        Arc::new(move |args| subscribe_handler(&c, args)),
    );
    reg.register_rpc_with_owner(
        "device.loop.cancel",
        OwnerKind::Device,
        Arc::new(move |args| cancel_handler(&svc, args)),
    );
}

fn create_handler(svc: &Arc<LoopService>, args: Value) -> anyhow::Result<Value> {
    let worker_agent = args
        .get("worker_agent")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("loop.create: `worker_agent` required"))?;
    let verify_expr = args
        .get("verify_expr")
        .and_then(Value::as_str)
        .unwrap_or("true");
    let max_iters = args
        .get("max_iters")
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow::anyhow!("loop.create: `max_iters` (positive integer) required"))?;
    let body_prompt = args
        .get("body_prompt")
        .and_then(Value::as_str)
        .unwrap_or("Continue the loop task and return the current result.");
    if max_iters > u32::MAX as u64 {
        anyhow::bail!("loop.create: max_iters too large (≤ u32::MAX)");
    }
    let id = svc.create(
        AgentId::new(worker_agent),
        verify_expr.to_owned(),
        max_iters as u32,
        body_prompt.to_owned(),
    )?;
    Ok(json!({ "loop_id": id.as_str() }))
}

fn status_handler(svc: &Arc<LoopService>, args: Value) -> anyhow::Result<Value> {
    let id = args
        .get("loop_id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("loop.status: `loop_id` required"))?;
    match svc.status(&LoopId::new(id)) {
        Some(inst) => Ok(serde_json::to_value(inst)?),
        None => anyhow::bail!("loop.status: loop {id} not found"),
    }
}

fn subscribe_handler(svc: &Arc<LoopService>, args: Value) -> anyhow::Result<StreamSource> {
    let id = args
        .get("loop_id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("loop.subscribe: `loop_id` required"))?;
    let loop_id = LoopId::new(id);
    let (snapshot, live) = svc.subscribe(&loop_id)?;
    match live {
        Some(rx) => Ok(StreamSource::SnapshotThenLive(snapshot, rx)),
        None => Ok(StreamSource::Snapshot(snapshot)),
    }
}

fn cancel_handler(svc: &Arc<LoopService>, args: Value) -> anyhow::Result<Value> {
    let id = args
        .get("loop_id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("loop.cancel: `loop_id` required"))?;
    svc.cancel(&LoopId::new(id))?;
    Ok(json!({ "ok": true }))
}

pub fn create_input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["worker_agent", "max_iters"],
        "properties": {
            "worker_agent": {"type": "string"},
            "verify_expr": {"type": "string"},
            "max_iters": {"type": "integer", "minimum": 1},
            "body_prompt": {"type": "string"}
        },
        "additionalProperties": false
    })
}

pub fn status_input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["loop_id"],
        "properties": {"loop_id": {"type": "string"}},
        "additionalProperties": false
    })
}

pub fn subscribe_input_schema() -> Value {
    status_input_schema()
}

pub fn cancel_input_schema() -> Value {
    status_input_schema()
}

pub fn create_description() -> &'static str {
    "Create a worker+verify loop instance bounded by max_iters. The daemon immediately starts the \
     per-loop controller, which drives body and verify Invocations through Kernel::invoke until a \
     terminal state is reached."
}

pub fn status_description() -> &'static str {
    "Fetch a loop instance's current state, current iteration, and metadata."
}

pub fn subscribe_description() -> &'static str {
    "Subscribe to a loop's status stream. Replays any buffered per-iteration frames, then tails \
     live IterStarted / BodyChunk / VerifyChunk / IterFinished / Terminal frames while the loop is running."
}

pub fn cancel_description() -> &'static str {
    "Cancel an in-flight loop. Already-terminal loops (Done / Exhausted / VerifyMalformed) are \
     untouched."
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> Arc<LoopService> {
        Arc::new(LoopService::new())
    }

    #[test]
    fn create_then_status_round_trip() {
        let svc = fresh();
        let resp = create_handler(&svc, json!({"worker_agent": "alice", "max_iters": 3})).unwrap();
        let id = resp["loop_id"].as_str().unwrap().to_string();
        let s = status_handler(&svc, json!({"loop_id": id})).unwrap();
        assert_eq!(s["max_iters"], 3);
        assert_eq!(s["verify_expr"], "true");
    }

    #[test]
    fn create_missing_max_iters_errors() {
        let svc = fresh();
        let err = create_handler(&svc, json!({"worker_agent": "alice"})).unwrap_err();
        assert!(format!("{err}").contains("max_iters"));
    }

    #[test]
    fn cancel_unknown_loop_errors() {
        let svc = fresh();
        let err = cancel_handler(&svc, json!({"loop_id": "nope"})).unwrap_err();
        assert!(format!("{err}").contains("not found"));
    }

    #[test]
    fn subscribe_known_loop_emits_snapshot_or_live_stream() {
        let svc = fresh();
        let id = create_handler(&svc, json!({"worker_agent": "alice", "max_iters": 2})).unwrap()
            ["loop_id"]
            .as_str()
            .unwrap()
            .to_string();
        let stream = subscribe_handler(&svc, json!({"loop_id": id})).unwrap();
        let frames = stream.into_snapshot();
        assert!(frames.is_empty() || frames[0]["kind"] == "iter_started");
    }

    #[test]
    fn subscribe_unknown_loop_yields_empty_stream() {
        let svc = fresh();
        let err = subscribe_handler(&svc, json!({"loop_id": "ghost"})).unwrap_err();
        assert!(format!("{err}").contains("not found"));
    }
}
