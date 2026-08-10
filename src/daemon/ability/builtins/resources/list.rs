// EasyNet CLI — meta.list_resources ability handler
// =====================================================================
//
// File: src/daemon/ability/builtins/resources/list.rs
//
// Resource discovery surface for physical-channel abilities
// (mic.subscribe, camera.subscribe, screen.subscribe, camera.record_start,
// ...). A consumer wishing to record "Chrome" calls
// `meta.list_resources(types=["application"])`, picks the application's
// `resource_ura`, then invokes `screen.subscribe(subject=<that ura>)`.
//
// Wire shape:
//
//   args:    { "types"?: ["mic"|"camera"|"display"|"application"|
//                         "window"|"speaker"|"voice"|"asr_model"] }
//   receipt: {
//     "resources": [
//       { "resource_ura", "type", "display_name", "owner_agent",
//         "binding", "metadata" }
//     ]
//   }
//
// INV-META-SUBJECT-EXEMPT
// -----------------------
// Per the binding invariants in plan v3.2, `meta.*` abilities are
// the documented exception to INV-SUBJECT-ENVELOPE: their subject is
// the callee URA (degenerate). This handler reads no subject from
// args (and would reject one if seen, defending the rule for every
// non-meta sibling). The handler name is whitelisted for the
// degenerate subject by the dispatch-side validator (lands in PR2.c
// with the media handlers, which DO need INV-SUBJECT-ENVELOPE).
//
// Not stubbed — ships fully working in PR2 because resources.rs
// already provides everything the handler needs. PR3's real device
// backends (cpal/nokhwa/screen) will populate the file; this handler
// just reads it. Until PR3, the table is empty under fresh installs
// and the handler returns `{ "resources": [] }`.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet.

use std::str::FromStr;
use std::sync::Arc;

use serde_json::{json, Value};

use crate::daemon::ability::dispatch::AxonAbilityCatalog;
use crate::daemon::ability::dispatch::OwnerKind;
use crate::daemon::persistence::resources::{self, filter_by_kinds, ResourceType};
use crate::daemon::resources::projection::ResourceListResponse;

pub const ABILITY_META_LIST_RESOURCES: &str =
    crate::daemon::ability::names::resources::META_LIST_RESOURCES;

/// Register `meta.list_resources` on the registry.
pub fn register(reg: &mut AxonAbilityCatalog) {
    reg.register_rpc_with_owner(
        ABILITY_META_LIST_RESOURCES,
        OwnerKind::runtime_introspection_system(),
        Arc::new(handler),
    );
}

/// Handler: read the local resources table, optionally filter by
/// `args.types`, project to the wire shape.
///
/// Reading `~/.easynet/resources.json` on every call is
/// intentional: the file is small, hot in OS cache, and the
/// freshest read avoids a stale snapshot when the device-profile
/// boot scan lands a new resource (or the operator hot-plugs a
/// USB camera). The cost is trivial vs. the rate at which
/// `meta.list_resources` is invoked.
///
/// File-load failures (corrupted JSON, parse error, schema drift)
/// surface as terminal failures via `anyhow::Error` rather than
/// silently degrading to an empty list — an operator who edited
/// the file by hand and broke it MUST see the error, not "all
/// resources vanished" with no signal. The first-boot case
/// (file does not exist) is NOT a failure: `resources::load`
/// returns the default empty file there, and the handler returns
/// an empty array.
fn handler(args: Value) -> anyhow::Result<Value> {
    let kinds = parse_kinds(args.get("types"))?;
    let file = resources::load()?;
    let entries = filter_by_kinds(&file, &kinds);
    Ok(serde_json::to_value(ResourceListResponse::from_entries(
        entries,
    ))?)
}

/// Parse the optional `types` arg into a typed `ResourceType`
/// vector. Each unknown string rejects with the exact value that
/// failed (rather than silently being treated as "no filter",
/// which would turn a typo into a permission widening on the
/// result set). Three accepted shapes:
/// - missing or `null`            → no filter
/// - empty array                  → no filter
/// - array of known type strings  → filter
fn parse_kinds(raw: Option<&Value>) -> anyhow::Result<Vec<ResourceType>> {
    let Some(value) = raw else {
        return Ok(Vec::new());
    };
    match value {
        Value::Null => Ok(Vec::new()),
        Value::Array(arr) => arr
            .iter()
            .map(|v| {
                v.as_str()
                    .ok_or_else(|| anyhow::anyhow!("`types[]` entries must be strings"))
                    .and_then(ResourceType::from_str)
            })
            .collect(),
        other => anyhow::bail!("`types` must be an array of strings, got {other}"),
    }
}

/// JSON Schema for `args`. The `enum` for `types[]` derives from
/// `ResourceType::ALL` rather than a hand-typed string list so a
/// new variant in `persistence::resources` shows up here without
/// a second edit (single source of truth — the same drift-prevention
/// pattern used by descriptor call-mode rendering).
pub fn input_schema() -> Value {
    let enum_values: Vec<Value> = ResourceType::ALL
        .iter()
        .map(|t| Value::String(t.as_str().to_string()))
        .collect();
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "types": {
                "type": "array",
                "description": "Filter to these resource types. Absent or empty = all.",
                "items": {
                    "type": "string",
                    "enum": enum_values,
                }
            }
        }
    })
}

/// User-facing description (rendered into the TOML and surfaced
/// via MCP `tools/list`). Spec / RFC pointers belong in the
/// module preamble above, not in user-facing strings.
pub fn description() -> &'static str {
    "List physical and logical resources held by this Agent (mics, \
     cameras, displays, applications, windows, speakers, voice \
     profiles, ASR models). Each entry's `resource_ura` is the \
     canonical subject for media abilities (mic.subscribe, \
     camera.snapshot, ...). Optional `types` filter narrows the \
     result."
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_makes_meta_list_resources_dispatchable() {
        let mut reg = AxonAbilityCatalog::new_test_metadata_for_device_authority(
            "easynet:///r/test/device/resource-list",
        );
        register(&mut reg);
        assert!(reg.get_rpc(ABILITY_META_LIST_RESOURCES).is_some());
        assert_eq!(
            reg.control_plane_owner(ABILITY_META_LIST_RESOURCES),
            Some(OwnerKind::runtime_introspection_system())
        );
    }

    // ── parse_kinds (pure; no filesystem dependency) ──────────

    #[test]
    fn parse_kinds_accepts_missing_arg_as_no_filter() {
        assert_eq!(parse_kinds(None).unwrap(), Vec::<ResourceType>::new());
    }

    #[test]
    fn parse_kinds_accepts_null_as_no_filter() {
        assert_eq!(
            parse_kinds(Some(&Value::Null)).unwrap(),
            Vec::<ResourceType>::new()
        );
    }

    #[test]
    fn parse_kinds_accepts_empty_array_as_no_filter() {
        assert_eq!(
            parse_kinds(Some(&json!([]))).unwrap(),
            Vec::<ResourceType>::new()
        );
    }

    #[test]
    fn parse_kinds_parses_well_known_type_values() {
        assert_eq!(
            parse_kinds(Some(&json!(["mic", "camera"]))).unwrap(),
            vec![ResourceType::Mic, ResourceType::Camera]
        );
    }

    #[test]
    fn parse_kinds_rejects_unknown_type_string() {
        // Critical: a typo MUST surface, not silently match
        // nothing. Otherwise `meta.list_resources(types=["cammera"])`
        // returns [] and the operator concludes "no cameras" — when
        // the real cause is the typo. The error message must name
        // the offending value.
        let err = parse_kinds(Some(&json!(["cammera"])))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("cammera"),
            "error must name the bad value: {err}"
        );
    }

    #[test]
    fn parse_kinds_rejects_non_array_types() {
        let err = parse_kinds(Some(&json!("mic"))).unwrap_err().to_string();
        assert!(err.contains("must be an array"), "got {err}");
    }

    #[test]
    fn parse_kinds_rejects_non_string_entries() {
        let err = parse_kinds(Some(&json!([1, 2, 3])))
            .unwrap_err()
            .to_string();
        assert!(err.contains("must be strings"), "got {err}");
    }

    // ── handler (touches the filesystem; gated by the
    //    real-invoke harness in real_invoke_tests.rs) ──────────

    #[test]
    fn handler_with_no_args_returns_resources_field() {
        // The contract is: receipt body always has a `resources`
        // array. The pure-handler harness here doesn't seed a
        // HomeGuard, so any value is fine — the integration test
        // in real_invoke_tests.rs exercises the populated paths.
        // Here we just pin the field name + JSON shape.
        let resp = handler(Value::Null).unwrap();
        assert!(
            resp.get("resources").and_then(Value::as_array).is_some(),
            "receipt body must always carry a `resources` array; got {resp}"
        );
    }
}
