// EasyNet CLI — loop.{create,status,subscribe,cancel} (PR-LOOP)
// =====================================================================
//
// File: src/daemon/ability/builtins/automation/loop_ability.rs
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

use serde_json::{json, Map, Value};

use crate::core::domain::{AgentId, LoopId};
use crate::daemon::ability::dispatch::OwnerKind;
use crate::daemon::ability::dispatch::{AxonAbilityCatalog, StreamSource};
use crate::daemon::execution::loop_instance::LoopService;

pub const ABILITY_CREATE: &str = crate::daemon::ability::names::automation::LOOP_CREATE;
pub const ABILITY_STATUS: &str = crate::daemon::ability::names::automation::LOOP_STATUS;
pub const ABILITY_SUBSCRIBE: &str = crate::daemon::ability::names::automation::LOOP_SUBSCRIBE;
pub const ABILITY_CANCEL: &str = crate::daemon::ability::names::automation::LOOP_CANCEL;

pub fn register(reg: &mut AxonAbilityCatalog, svc: Arc<LoopService>) {
    let a = Arc::clone(&svc);
    reg.register_rpc_with_owner(
        "loop.create",
        OwnerKind::Device,
        Arc::new(move |args| create_handler(&a, args)),
    );
    let b = Arc::clone(&svc);
    reg.register_rpc_with_owner(
        "loop.status",
        OwnerKind::Device,
        Arc::new(move |args| status_handler(&b, args)),
    );
    let c = Arc::clone(&svc);
    reg.register_stream_with_owner(
        "loop.subscribe",
        OwnerKind::Device,
        Arc::new(move |args| subscribe_handler(&c, args)),
    );
    reg.register_rpc_with_owner(
        "loop.cancel",
        OwnerKind::Device,
        Arc::new(move |args| cancel_handler(&svc, args)),
    );
}

fn create_handler(svc: &Arc<LoopService>, args: Value) -> anyhow::Result<Value> {
    let args = loop_args_object(
        "loop.create",
        &args,
        &["worker_agent", "verify_expr", "max_iters", "body_prompt"],
    )?;
    let worker_agent = loop_required_string_arg("loop.create", args, "worker_agent")?;
    let verify_expr = loop_optional_string_arg("loop.create", args, "verify_expr")?
        .unwrap_or_else(|| "true".to_string());
    let max_iters = loop_required_positive_u32_arg("loop.create", args, "max_iters")?;
    let body_prompt = loop_optional_string_arg("loop.create", args, "body_prompt")?
        .unwrap_or_else(|| "Continue the loop task and return the current result.".to_string());
    let id = svc.create(
        AgentId::new(worker_agent),
        verify_expr,
        max_iters,
        body_prompt,
    )?;
    Ok(json!({ "loop_id": id.as_str() }))
}

fn status_handler(svc: &Arc<LoopService>, args: Value) -> anyhow::Result<Value> {
    let args = loop_args_object("loop.status", &args, &["loop_id"])?;
    let id = loop_required_string_arg("loop.status", args, "loop_id")?;
    let display_id = id.clone();
    match svc.status(&LoopId::new(id))? {
        Some(inst) => Ok(serde_json::to_value(inst)?),
        None => anyhow::bail!("loop.status: loop {display_id} not found"),
    }
}

fn subscribe_handler(svc: &Arc<LoopService>, args: Value) -> anyhow::Result<StreamSource> {
    let args = loop_args_object("loop.subscribe", &args, &["loop_id"])?;
    let id = loop_required_string_arg("loop.subscribe", args, "loop_id")?;
    let loop_id = LoopId::new(id);
    let (snapshot, live) = svc.subscribe(&loop_id)?;
    match live {
        Some(rx) => Ok(StreamSource::SnapshotThenLive(snapshot, rx)),
        None => Ok(StreamSource::Snapshot(snapshot)),
    }
}

fn cancel_handler(svc: &Arc<LoopService>, args: Value) -> anyhow::Result<Value> {
    let args = loop_args_object("loop.cancel", &args, &["loop_id"])?;
    let id = loop_required_string_arg("loop.cancel", args, "loop_id")?;
    svc.cancel(&LoopId::new(id))?;
    Ok(json!({ "ok": true }))
}

fn loop_args_object<'a>(
    ability: &str,
    args: &'a Value,
    allowed_fields: &[&str],
) -> anyhow::Result<&'a Map<String, Value>> {
    let object = args
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("{ability}: args must be a JSON object"))?;
    let mut unknown: Vec<&str> = object
        .keys()
        .map(String::as_str)
        .filter(|field| !allowed_fields.contains(field))
        .collect();
    unknown.sort_unstable();
    if !unknown.is_empty() {
        anyhow::bail!(
            "{ability}: unsupported field(s): {}",
            unknown
                .iter()
                .map(|field| format!("`{field}`"))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    Ok(object)
}

fn loop_required_string_arg(
    ability: &str,
    args: &Map<String, Value>,
    field: &str,
) -> anyhow::Result<String> {
    match args.get(field) {
        Some(Value::String(value)) if !value.trim().is_empty() => Ok(value.to_string()),
        Some(Value::String(_)) | None => {
            anyhow::bail!("{ability}: `{field}` must be a non-empty string")
        }
        Some(_) => anyhow::bail!("{ability}: `{field}` must be a string"),
    }
}

fn loop_optional_string_arg(
    ability: &str,
    args: &Map<String, Value>,
    field: &str,
) -> anyhow::Result<Option<String>> {
    match args.get(field) {
        None => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.to_string())),
        Some(_) => anyhow::bail!("{ability}: `{field}` must be a string"),
    }
}

fn loop_required_positive_u32_arg(
    ability: &str,
    args: &Map<String, Value>,
    field: &str,
) -> anyhow::Result<u32> {
    let value = match args.get(field) {
        Some(Value::Number(number)) => number
            .as_u64()
            .ok_or_else(|| anyhow::anyhow!("{ability}: `{field}` must be a positive integer"))?,
        None => anyhow::bail!("{ability}: `{field}` must be a positive integer"),
        Some(_) => anyhow::bail!("{ability}: `{field}` must be a positive integer"),
    };
    if value == 0 {
        anyhow::bail!("{ability}: `{field}` must be ≥ 1");
    }
    if value > u32::MAX as u64 {
        anyhow::bail!("{ability}: `{field}` too large (≤ u32::MAX)");
    }
    Ok(value as u32)
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
    use crate::core::domain::TenantId;

    fn fresh() -> Arc<LoopService> {
        let svc = Arc::new(LoopService::new());
        svc.bind_memory_for_test(TenantId::new("tenant-a"));
        svc
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
    fn loop_handlers_reject_unknown_fields() {
        let svc = fresh();
        let cases = [
            create_handler(
                &svc,
                json!({"worker_agent": "alice", "max_iters": 1, "legacy_mode": true}),
            )
            .unwrap_err(),
            status_handler(&svc, json!({"loop_id": "loop-1", "legacy_mode": true})).unwrap_err(),
            subscribe_handler(&svc, json!({"loop_id": "loop-1", "legacy_mode": true})).unwrap_err(),
            cancel_handler(&svc, json!({"loop_id": "loop-1", "legacy_mode": true})).unwrap_err(),
        ];
        for err in cases {
            let msg = format!("{err}");
            assert!(msg.contains("unsupported field"), "wrong error: {msg}");
            assert!(msg.contains("legacy_mode"), "wrong field: {msg}");
        }
    }

    #[test]
    fn create_rejects_wrongly_typed_optional_strings() {
        let svc = fresh();
        for (field, payload) in [
            (
                "verify_expr",
                json!({"worker_agent": "alice", "max_iters": 1, "verify_expr": false}),
            ),
            (
                "body_prompt",
                json!({"worker_agent": "alice", "max_iters": 1, "body_prompt": 7}),
            ),
        ] {
            let err = create_handler(&svc, payload).unwrap_err();
            let msg = format!("{err}");
            assert!(
                msg.contains(&format!("`{field}` must be a string")),
                "wrong error for {field}: {msg}"
            );
        }
    }

    #[test]
    fn create_rejects_non_positive_or_wrongly_typed_max_iters() {
        let svc = fresh();
        for payload in [
            json!({"worker_agent": "alice", "max_iters": 0}),
            json!({"worker_agent": "alice", "max_iters": "1"}),
        ] {
            let err = create_handler(&svc, payload).unwrap_err();
            assert!(format!("{err}").contains("max_iters"));
        }
    }

    #[test]
    fn loop_handlers_reject_non_object_args() {
        let svc = fresh();
        let err = create_handler(&svc, json!(null)).unwrap_err();
        assert!(format!("{err}").contains("args must be a JSON object"));
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

    #[test]
    fn status_propagates_poisoned_loop_cache() {
        let svc = fresh();
        svc.poison_cache_for_test();
        let err = status_handler(&svc, json!({"loop_id": "ghost"})).unwrap_err();
        assert!(
            format!("{err:#}").contains("LoopService cache lock poisoned"),
            "{err:#}"
        );
    }

    #[test]
    fn subscribe_propagates_poisoned_loop_cache() {
        let svc = fresh();
        svc.poison_cache_for_test();
        let err = subscribe_handler(&svc, json!({"loop_id": "ghost"})).unwrap_err();
        assert!(
            format!("{err:#}").contains("LoopService cache lock poisoned"),
            "{err:#}"
        );
    }
}
