// EasyNet CLI — single source of truth for ability TOML descriptors
// =================================================================
//
// File: src/daemon/ability/catalog/ability_toml.rs
// Description: Renders an ability's `(name, description,
//              input_schema)` triple to the on-disk TOML form
//              resolved by `system_ability_descriptor_path`. Every ability
//              the dispatcher publishes round-trips through this
//              one renderer; the drift test compares its output
//              byte-for-byte against the file on disk, and the
//              `gen-ability-tomls` binary writes the same output.
//
// Why one renderer (and not handwritten TOMLs)
// --------------------------------------------
// We had a real correctness incident. Across slices 10-18 the
// code's `description_for(name)` and the on-disk
// the generated system descriptor description diverged:
// the runtime was telling MCP / discovery clients one story while
// the static descriptor said something else. 12 abilities were
// out of sync at the time the divergence was discovered. The
// fix is to make the TOML *generated*: a single function emits
// the canonical form, the drift test enforces byte equality, and
// regenerating after a code change is a one-liner.
//
// Why a function and not build.rs
// -------------------------------
// build.rs runs *before* the crate compiles, so it cannot call
// `daemon::ability::catalog::published_abilities()`. The generator
// therefore lives in the crate proper, and a tiny `bin/` driver
// invokes it from a `cargo run --bin gen-ability-tomls` after
// any change to ability metadata. The drift test in mod.rs
// compares the generator's output against on-disk files and
// fails CI if a maintainer forgot to regenerate.
//
// What this renderer does NOT support
// -----------------------------------
// * Deep schema composition with `$ref` or nested reusable
//   definitions. Simple combinators (`oneOf`, `anyOf`, `allOf`)
//   are rendered as inline tables when the schema carries them.
// * Comments inside the rendered TOML — TOML supports them, JSON
//   Schema doesn't carry them, and human-edited handwritten
//   comments would be erased on every regenerate. A future
//   per-ability "footer" extension to the input would let us
//   reintroduce comments alongside the generated body.
//
// Author: Silan.Hu
// Email: silan.hu@u.nus.edu
// Copyright (c) 2026-2027 easynet. All rights reserved.

use serde_json::Value;

use super::SystemAbilityContract;

pub const CONTRACT_SCHEMA_VERSION: &str = "2";

/// Render one ability descriptor to its canonical TOML form.
///
/// The output ends with exactly one trailing newline (POSIX
/// text-file convention) so a `git diff` after regenerate stays
/// minimal. Field order is fixed: `schema_version`, `name`,
/// `description`, blank line, `[input_schema]` block, then per-
/// property tables in the order they appear in `input_schema`'s
/// `properties` object.
pub fn render_ability_toml(name: &str, description: &str, input_schema: &Value) -> String {
    let mut out = String::with_capacity(512);
    out.push_str("schema_version = \"1\"\n");
    out.push_str(&format!("name = \"{}\"\n", escape_toml_basic(name)));
    out.push_str(&format!(
        "description = \"{}\"\n",
        escape_toml_basic(description)
    ));
    out.push('\n');
    render_schema(input_schema, "input_schema", &mut out);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Render the complete governed static contract. Every field used for
/// admission, authority conflict checks, or capability publication is
/// persisted so a semantic change is observable as descriptor drift.
pub fn render_ability_contract_toml(contract: &SystemAbilityContract) -> String {
    use crate::daemon::ability::descriptors::ReceiptSemantics;

    let mut out = String::with_capacity(1024);
    out.push_str(&format!("schema_version = \"{CONTRACT_SCHEMA_VERSION}\"\n"));
    out.push_str(&format!(
        "name = \"{}\"\n",
        escape_toml_basic(&contract.name)
    ));
    out.push_str(&format!(
        "descriptor_version = \"{}\"\n",
        escape_toml_basic(&contract.descriptor_version)
    ));
    out.push_str(&format!(
        "description = \"{}\"\n",
        escape_toml_basic(&contract.description)
    ));
    out.push_str(&format!(
        "call_mode = \"{}\"\n",
        contract.call_mode.as_str()
    ));
    out.push_str(&format!(
        "capability_state = {}\n",
        render_inline_value(
            &serde_json::to_value(contract.capability_state).expect("capability state serializes")
        )
    ));
    out.push_str(&format!(
        "admission_action = \"{}\"\n",
        contract.admission_action.as_str()
    ));
    out.push_str(&format!(
        "visibility = {}\n",
        render_inline_value(
            &serde_json::to_value(contract.visibility).expect("visibility serializes")
        )
    ));
    let (subject_kind, subject_uras) = scope_parts(&contract.scope_subjects);
    let (agent_kind, agent_uras) = scope_parts(&contract.scope_agents);
    out.push_str(&format!("scope_subjects_kind = \"{subject_kind}\"\n"));
    out.push_str(&format!(
        "scope_subjects_uras = {}\n",
        render_inline_value(&serde_json::json!(subject_uras))
    ));
    out.push_str(&format!("scope_agents_kind = \"{agent_kind}\"\n"));
    out.push_str(&format!(
        "scope_agents_uras = {}\n",
        render_inline_value(&serde_json::json!(agent_uras))
    ));
    out.push_str(&format!(
        "denied_agents = {}\n",
        render_inline_value(&serde_json::json!(contract.denied_agents))
    ));
    out.push_str(&format!(
        "output_receipt_schema_json = \"{}\"\n",
        escape_toml_basic(
            &serde_json::to_string(&contract.output_receipt_schema)
                .expect("output receipt schema serializes")
        )
    ));
    out.push_str(&format!(
        "hints_json = \"{}\"\n",
        escape_toml_basic(
            &serde_json::to_string(&contract.hints).expect("descriptor hints serialize")
        )
    ));
    match &contract.receipt_semantics {
        ReceiptSemantics::Operational => {
            out.push_str("receipt_semantics = \"operational\"\n");
        }
        ReceiptSemantics::StateTransition(transition) => {
            out.push_str("receipt_semantics = \"state_transition\"\n");
            out.push_str(&format!(
                "transition_id = \"{}\"\n",
                escape_toml_basic(transition.transition_id())
            ));
            out.push_str(&format!(
                "transition_class = \"{}\"\n",
                transition.transition_class().as_str()
            ));
        }
    }
    out.push('\n');
    render_schema(&contract.input_schema, "input_schema", &mut out);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn scope_parts(
    rule: &crate::daemon::ability::descriptors::ScopeRule,
) -> (&'static str, Vec<String>) {
    use crate::daemon::ability::descriptors::ScopeRule;
    match rule {
        ScopeRule::Any => ("any", Vec::new()),
        ScopeRule::None => ("none", Vec::new()),
        ScopeRule::OnlyMatching(uras) => ("only_matching", uras.clone()),
    }
}

#[derive(serde::Deserialize)]
struct RawAbilityContractToml {
    schema_version: String,
    name: String,
    descriptor_version: String,
    description: String,
    call_mode: String,
    capability_state: String,
    admission_action: String,
    visibility: String,
    scope_subjects_kind: String,
    #[serde(default)]
    scope_subjects_uras: Vec<String>,
    scope_agents_kind: String,
    #[serde(default)]
    scope_agents_uras: Vec<String>,
    #[serde(default)]
    denied_agents: Vec<String>,
    output_receipt_schema_json: String,
    hints_json: String,
    receipt_semantics: String,
    #[serde(default)]
    transition_id: Option<String>,
    #[serde(default)]
    transition_class: Option<String>,
    input_schema: toml::Value,
}

pub fn parse_ability_contract_toml(body: &str) -> anyhow::Result<SystemAbilityContract> {
    use crate::daemon::ability::conformance::CapabilityState;
    use crate::daemon::ability::descriptors::{
        AdmissionAction, ReceiptSemantics, StateTransition, TransitionClass, Visibility,
    };
    use crate::daemon::ability::CallMode;

    let raw: RawAbilityContractToml = toml::from_str(body)?;
    if raw.schema_version != CONTRACT_SCHEMA_VERSION {
        anyhow::bail!(
            "ability contract schema_version {:?} is unsupported; expected {:?}",
            raw.schema_version,
            CONTRACT_SCHEMA_VERSION
        );
    }
    let call_mode = parse_enum::<CallMode>(&raw.call_mode, "call_mode")?;
    let capability_state =
        parse_enum::<CapabilityState>(&raw.capability_state, "capability_state")?;
    let admission_action =
        parse_enum::<AdmissionAction>(&raw.admission_action, "admission_action")?;
    let visibility = parse_enum::<Visibility>(&raw.visibility, "visibility")?;
    let receipt_semantics = match raw.receipt_semantics.as_str() {
        "operational" if raw.transition_id.is_none() && raw.transition_class.is_none() => {
            ReceiptSemantics::Operational
        }
        "state_transition" => ReceiptSemantics::StateTransition(StateTransition::new(
            raw.transition_id
                .ok_or_else(|| anyhow::anyhow!("state_transition requires transition_id"))?,
            parse_enum::<TransitionClass>(
                raw.transition_class
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("state_transition requires transition_class"))?,
                "transition_class",
            )?,
        )?),
        other => anyhow::bail!("invalid receipt_semantics {other:?}"),
    };
    let denied_agents = canonical_policy_values(raw.denied_agents, "denied_agents")?;
    Ok(SystemAbilityContract {
        name: raw.name,
        descriptor_version: raw.descriptor_version,
        description: raw.description,
        input_schema: serde_json::to_value(raw.input_schema)?,
        output_receipt_schema: serde_json::from_str(&raw.output_receipt_schema_json)?,
        call_mode,
        admission_action,
        receipt_semantics,
        visibility,
        scope_subjects: parse_scope(raw.scope_subjects_kind, raw.scope_subjects_uras)?,
        scope_agents: parse_scope(raw.scope_agents_kind, raw.scope_agents_uras)?,
        denied_agents,
        hints: serde_json::from_str(&raw.hints_json)?,
        capability_state,
    })
}

fn parse_enum<T: serde::de::DeserializeOwned>(raw: &str, field: &str) -> anyhow::Result<T> {
    serde_json::from_value(Value::String(raw.to_string()))
        .map_err(|error| anyhow::anyhow!("invalid {field} {raw:?}: {error}"))
}

fn parse_scope(
    kind: String,
    uras: Vec<String>,
) -> anyhow::Result<crate::daemon::ability::descriptors::ScopeRule> {
    use crate::daemon::ability::descriptors::ScopeRule;
    let uras = canonical_policy_values(uras, "scope uras")?;
    match (kind.as_str(), uras.is_empty()) {
        ("any", true) => Ok(ScopeRule::Any),
        ("none", true) => Ok(ScopeRule::None),
        ("only_matching", false) => Ok(ScopeRule::OnlyMatching(uras)),
        _ => anyhow::bail!("invalid scope kind/uras combination {kind:?}"),
    }
}

fn canonical_policy_values(mut values: Vec<String>, field: &str) -> anyhow::Result<Vec<String>> {
    let original = values.clone();
    values.retain(|value| !value.trim().is_empty() && value.trim() == value);
    values.sort();
    values.dedup();
    if values != original {
        anyhow::bail!("{field} must be sorted, deduplicated, and non-empty");
    }
    Ok(values)
}

/// Render a JSON Schema object as a TOML table at the given
/// dotted-path. Each key becomes a TOML key on the table, with
/// `properties` and `additionalProperties` getting subtables of
/// their own.
fn render_schema(schema: &Value, path: &str, out: &mut String) {
    out.push_str(&format!("[{}]\n", path));
    let obj = match schema.as_object() {
        Some(o) => o,
        None => {
            // Schema isn't an object — emit nothing further.
            return;
        }
    };
    // Fixed key order: type, required, enum, additionalProperties,
    // items, plus the descriptive scalars (description, minimum,
    // maximum, minLength, maxLength, pattern, default). Anything
    // unrecognised falls into a trailing pass to keep the renderer
    // forward-compatible.
    let scalar_order: [&str; 10] = [
        "type",
        "required",
        "enum",
        "additionalProperties",
        "items",
        "description",
        "minimum",
        "maximum",
        "minLength",
        "maxLength",
    ];
    for key in scalar_order {
        if let Some(v) = obj.get(key) {
            if key == "properties" {
                continue; // handled below
            }
            render_inline_field(key, v, out);
        }
    }
    // Anything else (forward compat). Skip `properties`; it has
    // its own block.
    for (key, v) in obj {
        if scalar_order.contains(&key.as_str()) || key == "properties" {
            continue;
        }
        render_inline_field(key, v, out);
    }
    if let Some(props) = obj.get("properties").and_then(Value::as_object) {
        for (prop_name, prop_schema) in props {
            out.push('\n');
            render_schema(
                prop_schema,
                &format!("{}.properties.{}", path, prop_name),
                out,
            );
        }
    }
}

/// Render `key = <value>` as one TOML line, where `<value>` is the
/// inline form of the JSON value. Tables of the form
/// `additionalProperties = { type = "string" }` and arrays of
/// scalars stay inline.
fn render_inline_field(key: &str, v: &Value, out: &mut String) {
    out.push_str(key);
    out.push_str(" = ");
    out.push_str(&render_inline_value(v));
    out.push('\n');
}

/// Render a JSON value as TOML inline form. Handles strings,
/// integers, booleans, arrays of scalars, and inline tables.
fn render_inline_value(v: &Value) -> String {
    match v {
        Value::Null => "\"\"".to_string(), // shouldn't occur in our schemas
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => format!("\"{}\"", escape_toml_basic(s)),
        Value::Array(items) => {
            let parts: Vec<String> = items.iter().map(render_inline_value).collect();
            format!("[{}]", parts.join(", "))
        }
        Value::Object(obj) => {
            let parts: Vec<String> = obj
                .iter()
                .map(|(k, val)| format!("{} = {}", k, render_inline_value(val)))
                .collect();
            format!("{{ {} }}", parts.join(", "))
        }
    }
}

/// Escape a string for TOML basic-string (double-quoted) form.
/// Per the TOML spec, basic strings need: `\\` escape for the
/// backslash, `\"` for the inner double quote, `\n` / `\r` / `\t`
/// for the corresponding control chars. Other control bytes
/// `< 0x20` get `\uXXXX` escapes.
fn escape_toml_basic(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04X}", c as u32));
            }
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn renders_minimal_schema() {
        let toml = render_ability_toml(
            "x.y",
            "An ability.",
            &json!({"type":"object","additionalProperties":false}),
        );
        let expected = "\
schema_version = \"1\"
name = \"x.y\"
description = \"An ability.\"

[input_schema]
type = \"object\"
additionalProperties = false
";
        assert_eq!(toml, expected);
    }

    #[test]
    fn renders_required_array() {
        let toml = render_ability_toml(
            "x",
            "d",
            &json!({"type":"object","required":["a","b"],"additionalProperties":false}),
        );
        assert!(toml.contains("required = [\"a\", \"b\"]"));
    }

    #[test]
    fn renders_simple_schema_combinator() {
        let toml = render_ability_toml(
            "x",
            "d",
            &json!({
                "type":"object",
                "required":["name"],
                "additionalProperties":false,
                "anyOf":[
                    {"required":["agent_type"]},
                    {"required":["entry"]}
                ]
            }),
        );
        assert!(
            toml.contains("anyOf = [{ required = [\"agent_type\"] }, { required = [\"entry\"] }]")
        );
    }

    #[test]
    fn renders_property_with_description_and_min_length() {
        let toml = render_ability_toml(
            "x",
            "d",
            &json!({
                "type":"object",
                "properties":{
                    "path":{"type":"string","minLength":1,"description":"file path"}
                }
            }),
        );
        assert!(toml.contains("[input_schema.properties.path]"));
        assert!(toml.contains("type = \"string\""));
        assert!(toml.contains("minLength = 1"));
        assert!(toml.contains("description = \"file path\""));
    }

    #[test]
    fn renders_enum() {
        let toml = render_ability_toml(
            "x",
            "d",
            &json!({"type":"object","properties":{"e":{"type":"string","enum":["a","b","c"]}}}),
        );
        assert!(toml.contains("enum = [\"a\", \"b\", \"c\"]"));
    }

    #[test]
    fn renders_additional_properties_inline_table() {
        let toml = render_ability_toml(
            "x",
            "d",
            &json!({
                "type":"object",
                "properties":{
                    "env":{"type":"object","additionalProperties":{"type":"string"}}
                }
            }),
        );
        assert!(toml.contains("additionalProperties = { type = \"string\" }"));
    }

    #[test]
    fn renders_items_inline_table() {
        let toml = render_ability_toml(
            "x",
            "d",
            &json!({
                "type":"object",
                "properties":{
                    "args":{"type":"array","items":{"type":"string"}}
                }
            }),
        );
        assert!(toml.contains("items = { type = \"string\" }"));
    }

    #[test]
    fn escapes_quote_and_backslash_in_description() {
        let toml = render_ability_toml(
            "x",
            r#"He said "hi" with a backslash \"#,
            &json!({"type":"object"}),
        );
        // \\ for backslash, \" for quote.
        assert!(toml.contains(r#"description = "He said \"hi\" with a backslash \\""#));
    }

    #[test]
    fn escapes_newline_in_description() {
        let toml = render_ability_toml("x", "line1\nline2", &json!({"type":"object"}));
        assert!(toml.contains("description = \"line1\\nline2\""));
    }

    #[test]
    fn nested_property_renders_as_subtable() {
        let toml = render_ability_toml(
            "x",
            "d",
            &json!({
                "type":"object",
                "properties":{
                    "outer":{
                        "type":"object",
                        "properties":{
                            "inner":{"type":"string"}
                        }
                    }
                }
            }),
        );
        assert!(toml.contains("[input_schema.properties.outer]"));
        assert!(toml.contains("[input_schema.properties.outer.properties.inner]"));
    }

    #[test]
    fn output_round_trips_through_a_toml_parser() {
        // Use toml_edit (already in dev-deps) to confirm the
        // emitted bytes parse back to a structurally-equivalent
        // value. Catches any escaping or quoting we got wrong.
        let toml = render_ability_toml(
            "x.y",
            "Mixed: \"quotes\", \\backslash, and § symbol.",
            &json!({
                "type":"object",
                "required":["path"],
                "additionalProperties":false,
                "properties":{
                    "path":{"type":"string","minLength":1},
                    "max_bytes":{"type":"integer","minimum":0,"maximum":100},
                    "encoding":{"type":"string","enum":["binary","utf8"]},
                    "env":{"type":"object","additionalProperties":{"type":"string"}}
                }
            }),
        );
        let parsed: toml::Value = toml::from_str(&toml).expect("emitted TOML must parse");
        assert_eq!(parsed["name"].as_str().unwrap(), "x.y");
        assert_eq!(
            parsed["description"].as_str().unwrap(),
            "Mixed: \"quotes\", \\backslash, and § symbol."
        );
        let schema = &parsed["input_schema"];
        assert_eq!(schema["type"].as_str().unwrap(), "object");
        assert_eq!(schema["required"].as_array().unwrap().len(), 1);
        assert!(!schema["additionalProperties"].as_bool().unwrap());
        let props = schema.get("properties").unwrap();
        assert_eq!(
            props["encoding"]["enum"].as_array().unwrap()[0]
                .as_str()
                .unwrap(),
            "binary"
        );
    }

    #[test]
    fn governed_contract_round_trips_every_admission_and_capability_field() {
        let contract = super::super::system_ability_contract_inventory()
            .into_iter()
            .find(|contract| contract.name == "voice.report_metrics")
            .expect("voice report metrics contract");
        let body = render_ability_contract_toml(&contract);
        let parsed = parse_ability_contract_toml(&body).expect("parse governed contract");
        assert_eq!(parsed, contract);
        assert_eq!(
            parsed.admission_action,
            crate::daemon::ability::descriptors::AdmissionAction::Invoke
        );
        assert_eq!(
            parsed.capability_state,
            crate::daemon::ability::conformance::CapabilityState::Seam
        );
    }
}
