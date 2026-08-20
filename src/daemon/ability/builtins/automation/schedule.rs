// EasyNet CLI — schedule.{add,list,remove,enable} (PR-SCHED)
// ===================================================================
//
// File: src/daemon/ability/builtins/automation/schedule.rs
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

use serde_json::{json, Map, Value};

use crate::core::domain::{AgentId, MisfirePolicy, NodeId, ScheduleId};
use crate::daemon::ability::dispatch::{AxonAbilityCatalog, EnvelopeContext, OwnerKind};
use crate::daemon::execution::schedule::{ScheduleCreateSpec, ScheduleService};

pub const ABILITY_ADD: &str = crate::daemon::ability::names::automation::SCHEDULE_ADD;
pub const ABILITY_LIST: &str = crate::daemon::ability::names::automation::SCHEDULE_LIST;
pub const ABILITY_REMOVE: &str = crate::daemon::ability::names::automation::SCHEDULE_REMOVE;
pub const ABILITY_ENABLE: &str = crate::daemon::ability::names::automation::SCHEDULE_ENABLE;

pub fn register(reg: &mut AxonAbilityCatalog, svc: Arc<ScheduleService>) {
    let owner = OwnerKind::automation_system();
    let a = Arc::clone(&svc);
    reg.register_rpc_with_envelope_and_owner(
        "schedule.add",
        owner.clone(),
        Arc::new(move |envelope, args| add_handler_with_envelope(&a, &envelope, args)),
    );
    let b = Arc::clone(&svc);
    reg.register_rpc_with_envelope_and_owner(
        "schedule.list",
        owner.clone(),
        Arc::new(move |envelope, args| list_handler_with_envelope(&b, &envelope, args)),
    );
    let c = Arc::clone(&svc);
    reg.register_rpc_with_envelope_and_owner(
        "schedule.remove",
        owner.clone(),
        Arc::new(move |envelope, args| remove_handler_with_envelope(&c, &envelope, args)),
    );
    reg.register_rpc_with_envelope_and_owner(
        "schedule.enable",
        owner,
        Arc::new(move |envelope, args| enable_handler_with_envelope(&svc, &envelope, args)),
    );
}

#[derive(Debug)]
struct ScheduleAddInput {
    target_node: NodeId,
    target_agent: AgentId,
    cron_expr: String,
    misfire_policy: MisfirePolicy,
    catch_up_window_secs: Option<u64>,
    enabled: bool,
    prompt: String,
}

fn parse_add_input(args: &Value) -> anyhow::Result<ScheduleAddInput> {
    let args = schedule_args_object(
        "schedule.add",
        args,
        &[
            "target_node",
            "target_agent",
            "cron_expr",
            "misfire_policy",
            "catch_up_window_secs",
            "enabled",
            "prompt",
        ],
    )?;
    let target_node = schedule_required_string_arg("schedule.add", args, "target_node")?;
    let target_agent = schedule_required_string_arg("schedule.add", args, "target_agent")?;
    let cron_expr = schedule_required_string_arg("schedule.add", args, "cron_expr")?;
    let misfire_str = schedule_required_string_arg("schedule.add", args, "misfire_policy")?;
    let misfire_policy = match misfire_str.as_str() {
        "skip" => MisfirePolicy::Skip,
        "fire_once" => MisfirePolicy::FireOnce,
        "catch_up_windowed" => MisfirePolicy::CatchUpWindowed,
        other => anyhow::bail!(
            "schedule.add: misfire_policy must be skip|fire_once|catch_up_windowed, \
             got {other:?}"
        ),
    };
    let catch_up_window_secs =
        schedule_optional_u64_arg("schedule.add", args, "catch_up_window_secs")?;
    let enabled = schedule_optional_bool_arg("schedule.add", args, "enabled")?.unwrap_or(true);
    let prompt = schedule_required_string_arg("schedule.add", args, "prompt")?;
    Ok(ScheduleAddInput {
        target_node: NodeId::new(target_node),
        target_agent: AgentId::new(target_agent),
        cron_expr,
        misfire_policy,
        catch_up_window_secs,
        enabled,
        prompt,
    })
}

fn add_handler_with_authority(
    svc: &ScheduleService,
    input: ScheduleAddInput,
    authority: crate::core::domain::DeferredInvocationAuthority,
) -> anyhow::Result<Value> {
    let spec = ScheduleCreateSpec::new(
        input.target_node,
        input.target_agent,
        input.cron_expr,
        input.misfire_policy,
        authority,
    )
    .with_catch_up_window_secs(input.catch_up_window_secs)
    .with_enabled(input.enabled)
    .with_prompt(input.prompt);
    let id = svc.add_spec(spec)?;
    Ok(json!({ "schedule_id": id.as_str() }))
}

fn add_handler_with_envelope(
    svc: &ScheduleService,
    envelope: &EnvelopeContext,
    args: Value,
) -> anyhow::Result<Value> {
    let input = parse_add_input(&args)?;
    let identity = crate::daemon::execution::runtime_identity::LocalRuntimeInvocationIdentity::from_system_agent_callee(envelope.callee())?;
    let authority = identity.deferred_user_authority(
        envelope.caller(),
        envelope.invocation_id(),
        envelope.callee(),
        input.target_agent.as_str(),
        &input.target_node,
    )?;
    add_handler_with_authority(svc, input, authority)
}

#[cfg(test)]
fn add_handler(svc: &ScheduleService, args: Value) -> anyhow::Result<Value> {
    let input = parse_add_input(&args)?;
    let target_agent_ura =
        crate::core::ura::agent_ura("test", "user-1", input.target_agent.as_str());
    add_handler_with_authority(
        svc,
        input,
        crate::core::domain::DeferredInvocationAuthority {
            accountable_user_ura: crate::core::ura::user_ura("test", "user-1"),
            creator_invocation_id: "test-schedule-create".to_string(),
            controller_callee_ura: crate::core::ura::device_agent_ura(
                "test",
                "self",
                crate::daemon::ability::names::automation::AUTOMATION_SYSTEM_AGENT_ID,
            ),
            target_callee_ura: target_agent_ura,
            execution_host_ura: crate::core::ura::device_ura("test", "self"),
        },
    )
}

#[cfg(test)]
fn test_owner_envelope() -> EnvelopeContext {
    test_owner_envelope_for("user-1")
}

#[cfg(test)]
fn test_owner_envelope_for(user_id: &str) -> EnvelopeContext {
    EnvelopeContext::for_test_targeted_ability(
        crate::core::ura::user_ura("test", user_id),
        crate::core::ura::device_agent_ura(
            "test",
            "self",
            crate::daemon::ability::names::automation::AUTOMATION_SYSTEM_AGENT_ID,
        ),
        ABILITY_LIST,
        crate::core::ura::user_ura("test", user_id),
    )
}

fn accountable_user_from_envelope(
    ability: &str,
    envelope: &EnvelopeContext,
) -> anyhow::Result<String> {
    let caller = envelope.caller();
    let parsed = crate::core::ura::parse_ura(caller).map_err(|error| {
        anyhow::anyhow!("{ability}: caller must be a canonical User URA: {error}")
    })?;
    if parsed.kind != crate::core::ura::URAKind::User || parsed.user_id().is_none() {
        anyhow::bail!("{ability}: caller must be the accountable User Principal");
    }
    let canonical = crate::core::ura::user_ura(
        &parsed.realm,
        parsed
            .user_id()
            .expect("validated caller user id must exist"),
    );
    if canonical != caller {
        anyhow::bail!("{ability}: caller User URA must be canonical");
    }
    Ok(caller.to_string())
}

fn list_handler_with_envelope(
    svc: &ScheduleService,
    envelope: &EnvelopeContext,
    args: Value,
) -> anyhow::Result<Value> {
    schedule_args_object("schedule.list", &args, &[])?;
    let owner = accountable_user_from_envelope("schedule.list", envelope)?;
    let entries = svc.list_for_accountable_user(&owner)?;
    let arr: Vec<Value> = entries
        .into_iter()
        .map(serde_json::to_value)
        .collect::<Result<_, _>>()?;
    Ok(json!({ "schedules": arr }))
}

#[cfg(test)]
fn list_handler(svc: &ScheduleService, args: Value) -> anyhow::Result<Value> {
    list_handler_with_envelope(svc, &test_owner_envelope(), args)
}

fn remove_handler_with_envelope(
    svc: &ScheduleService,
    envelope: &EnvelopeContext,
    args: Value,
) -> anyhow::Result<Value> {
    let args = schedule_args_object("schedule.remove", &args, &["schedule_id"])?;
    let id = schedule_required_string_arg("schedule.remove", args, "schedule_id")?;
    let owner = accountable_user_from_envelope("schedule.remove", envelope)?;
    svc.remove_for_accountable_user(&ScheduleId::new(id), &owner)?;
    Ok(json!({ "ok": true }))
}

#[cfg(test)]
fn remove_handler(svc: &ScheduleService, args: Value) -> anyhow::Result<Value> {
    remove_handler_with_envelope(svc, &test_owner_envelope(), args)
}

fn enable_handler_with_envelope(
    svc: &ScheduleService,
    envelope: &EnvelopeContext,
    args: Value,
) -> anyhow::Result<Value> {
    let args = schedule_args_object("schedule.enable", &args, &["schedule_id", "enabled"])?;
    let id = schedule_required_string_arg("schedule.enable", args, "schedule_id")?;
    let enabled = schedule_required_bool_arg("schedule.enable", args, "enabled")?;
    let owner = accountable_user_from_envelope("schedule.enable", envelope)?;
    svc.enable_for_accountable_user(&ScheduleId::new(id), enabled, &owner)?;
    Ok(json!({ "ok": true }))
}

#[cfg(test)]
fn enable_handler(svc: &ScheduleService, args: Value) -> anyhow::Result<Value> {
    enable_handler_with_envelope(svc, &test_owner_envelope(), args)
}

fn schedule_args_object<'a>(
    ability: &str,
    args: &'a Value,
    allowed_fields: &[&str],
) -> anyhow::Result<&'a Map<String, Value>> {
    let object = args
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("{ability}: args must be a JSON object"))?;
    let mut unknown = object
        .keys()
        .filter(|key| !allowed_fields.contains(&key.as_str()))
        .map(String::as_str)
        .collect::<Vec<_>>();
    unknown.sort_unstable();
    if !unknown.is_empty() {
        anyhow::bail!("{ability}: unsupported field(s): {}", unknown.join(", "));
    }
    Ok(object)
}

fn schedule_required_string_arg(
    ability: &str,
    args: &Map<String, Value>,
    field: &str,
) -> anyhow::Result<String> {
    let value = args
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .ok_or_else(|| anyhow::anyhow!("{ability}: required field `{field}` (string) missing"))?;
    if value.is_empty() {
        anyhow::bail!("{ability}: {field} must be non-empty");
    }
    Ok(value.to_string())
}

fn schedule_optional_u64_arg(
    ability: &str,
    args: &Map<String, Value>,
    field: &str,
) -> anyhow::Result<Option<u64>> {
    let Some(value) = args.get(field) else {
        return Ok(None);
    };
    value
        .as_u64()
        .map(Some)
        .ok_or_else(|| anyhow::anyhow!("{ability}: `{field}` must be an unsigned integer"))
}

fn schedule_optional_bool_arg(
    ability: &str,
    args: &Map<String, Value>,
    field: &str,
) -> anyhow::Result<Option<bool>> {
    let Some(value) = args.get(field) else {
        return Ok(None);
    };
    value
        .as_bool()
        .map(Some)
        .ok_or_else(|| anyhow::anyhow!("{ability}: `{field}` must be a boolean"))
}

fn schedule_required_bool_arg(
    ability: &str,
    args: &Map<String, Value>,
    field: &str,
) -> anyhow::Result<bool> {
    args.get(field)
        .and_then(Value::as_bool)
        .ok_or_else(|| anyhow::anyhow!("{ability}: `{field}` (bool) required"))
}

pub fn add_input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["target_node", "target_agent", "cron_expr", "misfire_policy", "prompt"],
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
     misfire_policy (skip|fire_once|catch_up_windowed), prompt. Returns the assigned schedule_id."
}

pub fn list_description() -> &'static str {
    "List schedules owned by the accountable User Principal. Deterministic order by id."
}

pub fn remove_description() -> &'static str {
    "Delete a caller-owned schedule by id. Errors when the id is unknown or owned by another User."
}

pub fn enable_description() -> &'static str {
    "Toggle a caller-owned schedule's enabled flag. Disabled schedules stay in the registry but do not fire."
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::domain::TenantId;

    fn fresh() -> Arc<ScheduleService> {
        let svc = Arc::new(ScheduleService::new());
        svc.bind_memory_for_test(TenantId::new("tenant-a"));
        svc
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
                "misfire_policy": "skip",
                "prompt": "Daily task for {{target_agent}}"
            }),
        )
        .unwrap();
        assert!(resp["schedule_id"].as_str().unwrap().starts_with("sched-"));
        let listed = list_handler(&svc, json!({})).unwrap();
        assert_eq!(listed["schedules"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn list_remove_and_enable_are_scoped_to_accountable_user() {
        let svc = fresh();
        let alice = test_owner_envelope_for("user-1");
        let bob = test_owner_envelope_for("user-2");
        let add_for = |envelope: &EnvelopeContext, user_id: &str, agent: &str, hour: u8| {
            let args = json!({
                "target_node": "self",
                "target_agent": agent,
                "cron_expr": format!("0 {hour} * * *"),
                "misfire_policy": "skip",
                "prompt": "Daily task for {{target_agent}}"
            });
            add_handler_with_authority(
                &svc,
                parse_add_input(&args).unwrap(),
                crate::core::domain::DeferredInvocationAuthority {
                    accountable_user_ura: envelope.caller().to_string(),
                    creator_invocation_id: format!("test-create-{user_id}-{agent}"),
                    controller_callee_ura: crate::core::ura::device_agent_ura(
                        "test",
                        "self",
                        crate::daemon::ability::names::automation::AUTOMATION_SYSTEM_AGENT_ID,
                    ),
                    target_callee_ura: crate::core::ura::agent_ura("test", user_id, agent),
                    execution_host_ura: crate::core::ura::device_ura("test", "self"),
                },
            )
            .unwrap()["schedule_id"]
                .as_str()
                .unwrap()
                .to_string()
        };
        let alice_id = add_for(&alice, "user-1", "alice", 9);
        let bob_id = add_for(&bob, "user-2", "bob", 10);

        let alice_list = list_handler_with_envelope(&svc, &alice, json!({})).unwrap();
        let alice_rows = alice_list["schedules"].as_array().unwrap();
        assert_eq!(alice_rows.len(), 1);
        assert_eq!(alice_rows[0]["id"], alice_id);

        let bob_enable = enable_handler_with_envelope(
            &svc,
            &bob,
            json!({"schedule_id": alice_id, "enabled": false}),
        )
        .unwrap_err();
        assert!(format!("{bob_enable}").contains("not owned"));

        let bob_remove =
            remove_handler_with_envelope(&svc, &bob, json!({"schedule_id": alice_id})).unwrap_err();
        assert!(format!("{bob_remove}").contains("not owned"));

        remove_handler_with_envelope(&svc, &bob, json!({"schedule_id": bob_id})).unwrap();
        let bob_list = list_handler_with_envelope(&svc, &bob, json!({})).unwrap();
        assert!(bob_list["schedules"].as_array().unwrap().is_empty());
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
                "misfire_policy": "yolo",
                "prompt": "Daily task for {{target_agent}}"
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
                "misfire_policy": "fire_once",
                "prompt": "Daily task for {{target_agent}}"
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
                "misfire_policy": "skip",
                "prompt": "Daily task for {{target_agent}}"
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

    #[test]
    fn add_requires_explicit_non_empty_prompt() {
        let svc = fresh();
        let missing = add_handler(
            &svc,
            json!({
                "target_node": "self",
                "target_agent": "alice",
                "cron_expr": "0 9 * * *",
                "misfire_policy": "skip"
            }),
        )
        .unwrap_err();
        assert!(format!("{missing}").contains("prompt"));

        let blank = add_handler(
            &svc,
            json!({
                "target_node": "self",
                "target_agent": "alice",
                "cron_expr": "0 9 * * *",
                "misfire_policy": "skip",
                "prompt": "  "
            }),
        )
        .unwrap_err();
        assert!(format!("{blank}").contains("prompt must be non-empty"));
    }

    #[test]
    fn schedule_handlers_reject_non_object_args() {
        let svc = fresh();

        for (ability, err) in [
            (
                "schedule.add",
                add_handler(&svc, json!(["target_node", "self"])).unwrap_err(),
            ),
            (
                "schedule.list",
                list_handler(&svc, json!(null)).unwrap_err(),
            ),
            (
                "schedule.remove",
                remove_handler(&svc, json!("sched-1")).unwrap_err(),
            ),
            (
                "schedule.enable",
                enable_handler(&svc, json!(true)).unwrap_err(),
            ),
        ] {
            let message = format!("{err}");
            assert!(
                message.contains(ability) && message.contains("JSON object"),
                "{ability} must reject non-object args at the shared parser boundary: {message}"
            );
        }
    }

    #[test]
    fn schedule_handlers_reject_unknown_fields_before_service_dispatch() {
        let svc = fresh();

        for (ability, err) in [
            (
                "schedule.add",
                add_handler(
                    &svc,
                    json!({
                        "target_node": "self",
                        "target_agent": "alice",
                        "cron_expr": "0 9 * * *",
                        "misfire_policy": "skip",
                        "prompt": "Daily task",
                        "legacy_mode": true
                    }),
                )
                .unwrap_err(),
            ),
            (
                "schedule.list",
                list_handler(&svc, json!({"legacy_mode": true})).unwrap_err(),
            ),
            (
                "schedule.remove",
                remove_handler(
                    &svc,
                    json!({"schedule_id": "sched-missing", "legacy_mode": true}),
                )
                .unwrap_err(),
            ),
            (
                "schedule.enable",
                enable_handler(
                    &svc,
                    json!({"schedule_id": "sched-missing", "enabled": false, "legacy_mode": true}),
                )
                .unwrap_err(),
            ),
        ] {
            let message = format!("{err}");
            assert!(
                message.contains(ability)
                    && message.contains("unsupported field(s)")
                    && message.contains("legacy_mode"),
                "{ability} must fail closed on unknown fields: {message}"
            );
        }
        assert!(
            list_handler(&svc, json!({})).unwrap()["schedules"]
                .as_array()
                .unwrap()
                .is_empty(),
            "rejected schedule.add payload must not register a schedule"
        );
    }

    #[test]
    fn add_rejects_wrongly_typed_optional_fields_instead_of_defaulting() {
        let svc = fresh();
        for (field, value) in [
            ("enabled", json!("true")),
            ("catch_up_window_secs", json!("60")),
        ] {
            let mut args = json!({
                "target_node": "self",
                "target_agent": "alice",
                "cron_expr": "0 9 * * *",
                "misfire_policy": "skip",
                "prompt": "Daily task"
            });
            args.as_object_mut()
                .unwrap()
                .insert(field.to_string(), value);

            let err = add_handler(&svc, args).unwrap_err();
            let message = format!("{err}");
            assert!(
                message.contains(field),
                "wrongly typed optional field must be rejected, got {message}"
            );
        }
        assert!(
            list_handler(&svc, json!({})).unwrap()["schedules"]
                .as_array()
                .unwrap()
                .is_empty(),
            "wrongly typed optional fields must not register schedules"
        );
    }

    #[test]
    fn enable_rejects_wrongly_typed_enabled_before_service_dispatch() {
        let svc = fresh();
        let err = enable_handler(
            &svc,
            json!({"schedule_id": "sched-missing", "enabled": "false"}),
        )
        .unwrap_err();
        let message = format!("{err}");
        assert!(
            message.contains("enabled") && message.contains("bool"),
            "wrongly typed enabled must fail at parser boundary: {message}"
        );
    }

    #[test]
    fn add_schema_requires_prompt_to_match_handler_contract() {
        let required = add_input_schema()["required"].as_array().unwrap().clone();

        assert!(
            required.iter().any(|value| value == "prompt"),
            "schedule.add schema must require prompt because the handler has no canonical default"
        );
        assert!(add_description().contains("prompt"));
    }
}
