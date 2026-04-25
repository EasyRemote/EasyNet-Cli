// EasyNet CLI — system.loop.{create,status,subscribe,cancel} (PR-LOOP)
// =====================================================================
//
// File: src/runtime/system/loop_ability.rs
// Description: Four abilities surfacing the loop primitive:
//
//   * `system.loop.create`    (RPC)    — register a new loop instance.
//   * `system.loop.status`    (RPC)    — fetch current state.
//   * `system.loop.subscribe` (Stream) — replay status snapshots.
//   * `system.loop.cancel`    (RPC)    — cancel an in-flight loop.
//
// Why suffix `_ability` and the file name `loop_ability.rs`
// ---------------------------------------------------------
// `loop` is a Rust keyword. Naming the module `loop` would force
// every importer into r#loop syntax. The suffix sidesteps the
// keyword while keeping the file's role obvious.
//
// What this PR does NOT ship
// --------------------------
// The actual per-iteration execution (body Invocation → verify
// Invocation → state transition) lands in PR-INVOCATION-EXEC-UNITY.
// v1 here is the registry surface — Client can create / observe /
// cancel; the controller that drives iterations is the unity PR's
// job.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::runtime::ability_dispatch::LocalAbilityRegistry;
use crate::runtime::domain::{AgentId, LoopId};
use crate::runtime::execution::loop_instance::LoopService;

pub const ABILITY_CREATE: &str = "system.loop.create";
pub const ABILITY_STATUS: &str = "system.loop.status";
pub const ABILITY_SUBSCRIBE: &str = "system.loop.subscribe";
pub const ABILITY_CANCEL: &str = "system.loop.cancel";

pub fn register(reg: &mut LocalAbilityRegistry, svc: Arc<LoopService>) {
    let a = Arc::clone(&svc);
    reg.register_rpc(
        ABILITY_CREATE,
        Arc::new(move |args| create_handler(&a, args)),
    );
    let b = Arc::clone(&svc);
    reg.register_rpc(
        ABILITY_STATUS,
        Arc::new(move |args| status_handler(&b, args)),
    );
    let c = Arc::clone(&svc);
    reg.register_stream(
        ABILITY_SUBSCRIBE,
        Arc::new(move |args| subscribe_handler(&c, args)),
    );
    reg.register_rpc(
        ABILITY_CANCEL,
        Arc::new(move |args| cancel_handler(&svc, args)),
    );
}

fn create_handler(svc: &LoopService, args: Value) -> anyhow::Result<Value> {
    let worker_agent = args
        .get("worker_agent")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("loop.create: `worker_agent` required"))?;
    let max_iters = args
        .get("max_iters")
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow::anyhow!("loop.create: `max_iters` (positive integer) required"))?;
    if max_iters > u32::MAX as u64 {
        anyhow::bail!("loop.create: max_iters too large (≤ u32::MAX)");
    }
    let id = svc.create(AgentId::new(worker_agent), max_iters as u32)?;
    Ok(json!({ "loop_id": id.as_str() }))
}

fn status_handler(svc: &LoopService, args: Value) -> anyhow::Result<Value> {
    let id = args
        .get("loop_id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("loop.status: `loop_id` required"))?;
    match svc.status(&LoopId::new(id)) {
        Some(inst) => Ok(serde_json::to_value(inst)?),
        None => anyhow::bail!("loop.status: loop {id} not found"),
    }
}

fn subscribe_handler(svc: &LoopService, args: Value) -> anyhow::Result<Vec<Value>> {
    let id = args
        .get("loop_id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("loop.subscribe: `loop_id` required"))?;
    // v1 emits a one-shot snapshot. PR-INVOCATION-EXEC-UNITY adds
    // the live per-iteration frames (IterStarted / BodyFrame /
    // VerifyFrame / Terminal) when the controller fires.
    match svc.status(&LoopId::new(id)) {
        Some(inst) => Ok(vec![serde_json::to_value(inst)?]),
        None => Ok(Vec::new()),
    }
}

fn cancel_handler(svc: &LoopService, args: Value) -> anyhow::Result<Value> {
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
            "max_iters": {"type": "integer", "minimum": 1}
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
    "Create a worker+verify loop instance bounded by max_iters. v1 starts in Pending state; \
     PR-INVOCATION-EXEC-UNITY drives per-iteration body and verify Invocations through Kernel::invoke."
}

pub fn status_description() -> &'static str {
    "Fetch a loop instance's current state, current iteration, and metadata."
}

pub fn subscribe_description() -> &'static str {
    "Subscribe to a loop's status stream. v1 emits a one-shot snapshot; live per-iteration frames \
     land with PR-INVOCATION-EXEC-UNITY."
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
        let resp = create_handler(
            &svc,
            json!({"worker_agent": "alice", "max_iters": 3}),
        )
        .unwrap();
        let id = resp["loop_id"].as_str().unwrap().to_string();
        let s = status_handler(&svc, json!({"loop_id": id})).unwrap();
        assert_eq!(s["max_iters"], 3);
        assert_eq!(s["state"]["kind"], "pending");
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
    fn subscribe_known_loop_emits_one_snapshot_frame() {
        let svc = fresh();
        let id = create_handler(
            &svc,
            json!({"worker_agent": "alice", "max_iters": 2}),
        )
        .unwrap()["loop_id"]
            .as_str()
            .unwrap()
            .to_string();
        let frames = subscribe_handler(&svc, json!({"loop_id": id})).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0]["state"]["kind"], "pending");
    }

    #[test]
    fn subscribe_unknown_loop_yields_empty_stream() {
        // v1 contract: unknown id → empty stream (not an error).
        // Mirrors the system.session.attach behaviour so a Client
        // observing both gets a uniform "I just missed it" UX.
        let svc = fresh();
        let frames = subscribe_handler(&svc, json!({"loop_id": "ghost"})).unwrap();
        assert!(frames.is_empty());
    }
}
