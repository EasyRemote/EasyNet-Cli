// EasyNet CLI — schedule.{add,list,remove,enable} (PR-SCHED)
// ===================================================================
//
// File: src/runtime/system/schedule_ability.rs
// Description: Four RPC abilities exposing the cron-driven
//              `ScheduleService` over the IPC plane.
//
//   * `schedule.add`     — register a new cron schedule.
//   * `schedule.list`    — snapshot every schedule.
//   * `schedule.remove`  — delete by id.
//   * `schedule.enable`  — toggle the enabled flag.
//
// The tick runner that actually fires schedules at their
// next-fire instant lives in
// `src/bin/easynet-daemon.rs` (the daemon main loop). This file
// only models the CRUD surface; PR-INVOCATION-EXEC-UNITY wires the
// runner's "tick → emit Invocation → Kernel::invoke" path.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::runtime::ability_dispatch::LocalAbilityRegistry;
use crate::runtime::ability_dispatch::OwnerKind;
use crate::runtime::domain::{AgentId, MisfirePolicy, NodeId, ScheduleEntry, ScheduleId, TenantId};
use crate::runtime::execution::schedule::ScheduleService;

pub const ABILITY_ADD: &str = "device.schedule.add";
pub const ABILITY_LIST: &str = "device.schedule.list";
pub const ABILITY_REMOVE: &str = "device.schedule.remove";
pub const ABILITY_ENABLE: &str = "device.schedule.enable";

pub fn register(reg: &mut LocalAbilityRegistry, svc: Arc<ScheduleService>) {
    let a = Arc::clone(&svc);
    reg.register_rpc_with_owner(
        "device.schedule.add",
        OwnerKind::Device,
        Arc::new(move |args| add_handler(&a, args)),
    );
    let b = Arc::clone(&svc);
    reg.register_rpc_with_owner(
        "device.schedule.list",
        OwnerKind::Device,
        Arc::new(move |args| list_handler(&b, args)),
    );
    let c = Arc::clone(&svc);
    reg.register_rpc_with_owner(
        "device.schedule.remove",
        OwnerKind::Device,
        Arc::new(move |args| remove_handler(&c, args)),
    );
    reg.register_rpc_with_owner(
        "device.schedule.enable",
        OwnerKind::Device,
        Arc::new(move |args| enable_handler(&svc, args)),
    );
}

fn add_handler(svc: &ScheduleService, args: Value) -> anyhow::Result<Value> {
    let target_node = required_str(&args, "target_node")?;
    let target_agent = required_str(&args, "target_agent")?;
    let cron_expr = required_str(&args, "cron_expr")?;
    let misfire_str = required_str(&args, "misfire_policy")?;
    let misfire_policy = match misfire_str {
        "skip" => MisfirePolicy::Skip,
        "fire_once" => MisfirePolicy::FireOnce,
        "catch_up_windowed" => MisfirePolicy::CatchUpWindowed,
        other => anyhow::bail!(
            "schedule.add: misfire_policy must be skip|fire_once|catch_up_windowed, \
             got {other:?}"
        ),
    };
    let catch_up_window_secs = args.get("catch_up_window_secs").and_then(Value::as_u64);
    let enabled = args.get("enabled").and_then(Value::as_bool).unwrap_or(true);
    // Optional prompt template — see ScheduleEntry::prompt for
    // supported template variables.
    let prompt = args.get("prompt").and_then(Value::as_str).map(String::from);
    let entry = ScheduleEntry {
        id: ScheduleId::new(""),
        tenant: TenantId::default_v1(),
        target_node: NodeId::new(target_node),
        target_agent: AgentId::new(target_agent),
        cron_expr: cron_expr.into(),
        misfire_policy,
        catch_up_window_secs,
        enabled,
        prompt,
    };
    let id = svc.add(entry)?;
    Ok(json!({ "schedule_id": id.as_str() }))
}

fn list_handler(svc: &ScheduleService, _args: Value) -> anyhow::Result<Value> {
    let entries = svc.list();
    let arr: Vec<Value> = entries
        .into_iter()
        .map(|e| serde_json::to_value(e).unwrap_or(Value::Null))
        .collect();
    Ok(json!({ "schedules": arr }))
}

fn remove_handler(svc: &ScheduleService, args: Value) -> anyhow::Result<Value> {
    let id = required_str(&args, "schedule_id")?;
    svc.remove(&ScheduleId::new(id))?;
    Ok(json!({ "ok": true }))
}

fn enable_handler(svc: &ScheduleService, args: Value) -> anyhow::Result<Value> {
    let id = required_str(&args, "schedule_id")?;
    let enabled = args
        .get("enabled")
        .and_then(Value::as_bool)
        .ok_or_else(|| anyhow::anyhow!("schedule.enable: `enabled` (bool) required"))?;
    svc.enable(&ScheduleId::new(id), enabled)?;
    Ok(json!({ "ok": true }))
}

fn required_str<'a>(args: &'a Value, key: &str) -> anyhow::Result<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("schedule: required field `{key}` (string) missing"))
}

pub fn add_input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["target_node", "target_agent", "cron_expr", "misfire_policy"],
        "properties": {
            "target_node": {"type": "string"},
            "target_agent": {"type": "string"},
            "cron_expr": {"type": "string"},
            "misfire_policy": {
                "type": "string",
                "enum": ["skip", "fire_once", "catch_up_windowed"]
            },
            "catch_up_window_secs": {"type": "integer", "minimum": 0},
            "enabled": {"type": "boolean"},
            "prompt": {
                "type": "string",
                "description": "Prompt template sent to target_agent at fire time. Supports {{schedule_id}}, {{fire_at_iso}}, {{catch_up}}, {{target_agent}}."
            }
        },
        "additionalProperties": false
    })
}

pub fn list_input_schema() -> Value {
    json!({"type": "object", "additionalProperties": false})
}

pub fn remove_input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["schedule_id"],
        "properties": {"schedule_id": {"type": "string"}},
        "additionalProperties": false
    })
}

pub fn enable_input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["schedule_id", "enabled"],
        "properties": {
            "schedule_id": {"type": "string"},
            "enabled": {"type": "boolean"}
        },
        "additionalProperties": false
    })
}

pub fn add_description() -> &'static str {
    "Register a new cron schedule. Required: target_node, target_agent, cron_expr (5- or 6-field), \
     misfire_policy (skip|fire_once|catch_up_windowed). Returns the assigned schedule_id."
}

pub fn list_description() -> &'static str {
    "List every schedule known to this daemon. Deterministic order by id."
}

pub fn remove_description() -> &'static str {
    "Delete a schedule by id. Errors when the id is unknown."
}

pub fn enable_description() -> &'static str {
    "Toggle a schedule's enabled flag. Disabled schedules stay in the registry but do not fire."
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> Arc<ScheduleService> {
        Arc::new(ScheduleService::new())
    }

    #[test]
    fn add_then_list_returns_the_schedule() {
        let svc = fresh();
        let resp = add_handler(
            &svc,
            json!({
                "target_node": "self",
                "target_agent": "alice",
                "cron_expr": "0 9 * * *",
                "misfire_policy": "skip"
            }),
        )
        .unwrap();
        assert!(resp["schedule_id"].as_str().unwrap().starts_with("sched-"));
        let listed = list_handler(&svc, json!({})).unwrap();
        assert_eq!(listed["schedules"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn add_with_unknown_misfire_string_rejects_at_parse() {
        let svc = fresh();
        let err = add_handler(
            &svc,
            json!({
                "target_node": "self",
                "target_agent": "alice",
                "cron_expr": "0 9 * * *",
                "misfire_policy": "yolo"
            }),
        )
        .unwrap_err();
        assert!(format!("{err}").contains("misfire_policy"));
    }

    #[test]
    fn add_then_remove_drops_entry() {
        let svc = fresh();
        let resp = add_handler(
            &svc,
            json!({
                "target_node": "self",
                "target_agent": "alice",
                "cron_expr": "0 9 * * *",
                "misfire_policy": "fire_once"
            }),
        )
        .unwrap();
        let id = resp["schedule_id"].as_str().unwrap().to_string();
        remove_handler(&svc, json!({"schedule_id": id})).unwrap();
        let listed = list_handler(&svc, json!({})).unwrap();
        assert!(listed["schedules"].as_array().unwrap().is_empty());
    }

    #[test]
    fn enable_toggles_flag() {
        let svc = fresh();
        let resp = add_handler(
            &svc,
            json!({
                "target_node": "self",
                "target_agent": "alice",
                "cron_expr": "0 9 * * *",
                "misfire_policy": "skip"
            }),
        )
        .unwrap();
        let id = resp["schedule_id"].as_str().unwrap().to_string();
        enable_handler(&svc, json!({"schedule_id": id, "enabled": false})).unwrap();
        let listed = list_handler(&svc, json!({})).unwrap();
        let arr = listed["schedules"].as_array().unwrap();
        assert!(!arr[0]["enabled"].as_bool().unwrap());
    }

    #[test]
    fn add_missing_required_field_errors_clearly() {
        let svc = fresh();
        let err = add_handler(&svc, json!({"target_node": "x"})).unwrap_err();
        assert!(format!("{err}").contains("target_agent"));
    }
}
