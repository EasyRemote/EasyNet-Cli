// EasyNet CLI — observe.health smoke ability
// ========================================
//
// File: src/runtime/system/ping.rs
// Description: Health probe used to confirm the dispatch path is wired
//              end-to-end. Returns the Axon observe.health contract
//              fields plus additive smoke-diagnostic fields.
//
// Why ping is the v1 system-namespace seed
// ----------------------------------------
// PR-SYS establishes the `system.<feature>` namespace, the
// stage-1 InvocationTarget resolver, and the stage-2 dispatch
// executor. Without at least one ability registered, none of those
// pieces would have a working call site to validate. `observe.health`
// is the simplest ability that exercises every layer:
//
//   1. The wire envelope reaches the IPC layer.
//   2. The resolver classifies it as Local or Remote.
//   3. The dispatcher routes it (loopback or via Gateway).
//   4. The handler returns; the response envelope flows back.
//
// A failing ping means one of those four stages is broken. A
// successful ping means the daemon is wired correctly and any
// PR-ATTACH/PR-PERM/PR-… handler can register the same way.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::daemon::ability::dispatch::AxonAbilityCatalog;

use crate::daemon::ability::dispatch::OwnerKind;
/// Wire name of the ability. Pinned so a future rename trips a
/// fixture-byte-stability test in `registry::a2a_labels`.
pub const ABILITY_NAME: &str = crate::daemon::ability::names::governance::OBSERVE_HEALTH;

/// Register the `observe.health` handler on the supplied registry.
/// Called from `daemon::ability::catalog::build_registry`.
pub fn register(reg: &mut AxonAbilityCatalog) {
    reg.register_rpc_with_owner(ABILITY_NAME, OwnerKind::Device, Arc::new(handler));
}

/// Health handler. Returns the Axon observe.health contract fields:
/// `{status, details, uptime_ms, version, components}`.
///
/// The body deliberately does no I/O and no global-state mutation.
/// A future change that adds either should land in a
/// purpose-built ability, not here, so `observe.health` stays a
/// reliable smoke target.
fn handler(args: Value) -> anyhow::Result<Value> {
    let ts = chrono::Utc::now().timestamp_millis();
    Ok(json!({
        "status": "healthy",
        "details": {
            "source": "easynet-cli",
            "replied_at_unix_ms": ts,
        },
        "uptime_ms": 0,
        "version": env!("CARGO_PKG_VERSION"),
        "components": {
            "dispatch": "healthy",
            "local_runtime": "healthy",
        },
        // Back-compat diagnostics. These fields are deliberately
        // additive; callers must key off `status` / `details`.
        "echo": args,
        "replied_at_unix_ms": ts,
    }))
}

/// JSON Schema for the ability's input. Empty-object schema
/// because ping accepts any payload (the echo carries it back).
/// Exposed so PR-SYS's `system_skills[]` discovery list can carry
/// a structured `input_schema` rather than a free-form string.
pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": true,
    })
}

/// Human-readable blurb for `system_skills[]` discovery JSON.
pub fn description() -> &'static str {
    "Local health probe. Returns Axon observe.health status fields plus smoke diagnostics."
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::invocation_target::{CallMode, InvocationTarget, TargetScope};

    #[test]
    fn handler_returns_health_contract_and_stamps_timestamp() {
        // Spirit of "verify every layer": call the handler in
        // isolation. Contract fields are present; legacy diagnostics
        // remain additive for existing smoke scripts.
        let resp = handler(json!({"k": "v"})).unwrap();
        assert_eq!(resp["status"], "healthy");
        assert!(resp["details"].is_object());
        assert_eq!(resp["components"]["dispatch"], "healthy");
        assert_eq!(resp["echo"], json!({"k": "v"}));
        let ts = resp["replied_at_unix_ms"].as_i64().unwrap();
        let now = chrono::Utc::now().timestamp_millis();
        // Stamp must be in the past 5 seconds. A regression that
        // hard-coded `0` or `i64::MAX` would fail this bound.
        assert!((now - ts).abs() < 5_000);
    }

    #[test]
    fn registration_makes_ability_dispatchable() {
        // End-to-end on the local path: register, build a
        // dispatcher, dispatch a Local target, observe the echo.
        // This is the smoke path the v1 daemon's startup hits.
        let mut reg = AxonAbilityCatalog::new();
        register(&mut reg);
        let dispatcher = Arc::new(reg);
        let target = InvocationTarget {
            scope: TargetScope::Local,
            ability: ABILITY_NAME.into(),
            normalized_args: json!({"hello": "world"}),
            call_mode: CallMode::Rpc,
            subject: None,
            causal_context: None,
        };
        let resp = dispatcher.execute_rpc(target).unwrap();
        assert_eq!(resp["status"], "healthy");
        assert!(resp["details"].is_object());
        assert_eq!(resp["echo"], json!({"hello": "world"}));
    }

    #[test]
    fn input_schema_is_a_json_object() {
        // The discovery contract requires a top-level JSON object
        // schema. Pin it here so a typo'd refactor (e.g. wrapping
        // in an array) trips the test.
        assert!(input_schema().is_object());
        assert_eq!(
            input_schema().get("type").and_then(Value::as_str),
            Some("object"),
        );
    }
}
