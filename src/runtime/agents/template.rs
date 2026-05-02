// EasyNet CLI — Shared `{{ var }}` template substitution
// =======================================================
//
// File: src/runtime/agents/template.rs
// Description: Substitute `{{ name }}` placeholders inside a string
//              against a JSON args object. Used by the shell
//              executor (per-argv-element render) and the EAL
//              executor (whole-program render). Pulled out so the
//              two executors share one substitution model and a
//              single set of failure-mode tests.
//
// Substitution model
// ------------------
//   * `{{ name }}` is replaced with the value of `args["name"]`.
//     Whitespace inside the braces is tolerated.
//   * String values substitute as their bare value (no JSON quoting).
//     Numbers / bools / null / arrays / objects substitute as their
//     `serde_json::Value::to_string()` (i.e. JSON-encoded). This is
//     the same convention `shell_executor` shipped with; tests below
//     pin it.
//   * A missing arg key raises `Err` BEFORE the consumer (subprocess
//     spawn, EAL parse, …) sees the half-rendered template. Catches
//     manifest typos at the call site instead of producing malformed
//     downstream input.
//   * `{{` without matching `}}` is `Err`. So is `{{ }}` with an
//     empty key.
//   * Inputs are byte-walked but only at ASCII `{` boundaries, so
//     multi-byte UTF-8 inside the template is preserved.
//
// What this module is NOT
// -----------------------
// A full templating engine. No conditionals, no loops, no filters,
// no nested expressions. The substitution surface is intentionally
// minimal so an ability author can read a `<verb>.ability.toml` and
// know exactly what will happen at call time.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use serde_json::Value;

/// Render one template string by substituting `{{ name }}`
/// placeholders against `args`. See module doc for the exact model.
///
/// `caller_label` is a short string identifying the caller (e.g.
/// "shell executor", "eal executor") that gets embedded in the
/// error messages so an operator reading the daemon log can tell
/// which executor blamed the manifest.
pub fn render_template(template: &str, args: &Value, caller_label: &str) -> anyhow::Result<String> {
    let bindings_holder;
    let bindings: Option<&serde_json::Map<String, Value>> = match args {
        Value::Object(map) => {
            bindings_holder = map;
            Some(bindings_holder)
        }
        Value::Null => None,
        other => anyhow::bail!(
            "{caller_label}: args must be a JSON object (got {})",
            short_kind(other)
        ),
    };
    render_with_bindings(template, bindings, caller_label)
}

/// Render multiple templates with one shared `args` resolution.
/// Used by the shell executor's argv loop — calling
/// `render_template` per element would re-classify args (Object /
/// Null / other) on every call; this hoists that out.
pub fn render_each(
    templates: &[String],
    args: &Value,
    caller_label: &str,
) -> anyhow::Result<Vec<String>> {
    let bindings_holder;
    let bindings: Option<&serde_json::Map<String, Value>> = match args {
        Value::Object(map) => {
            bindings_holder = map;
            Some(bindings_holder)
        }
        Value::Null => None,
        other => anyhow::bail!(
            "{caller_label}: args must be a JSON object (got {})",
            short_kind(other)
        ),
    };
    templates
        .iter()
        .map(|t| render_with_bindings(t, bindings, caller_label))
        .collect()
}

fn render_with_bindings(
    template: &str,
    bindings: Option<&serde_json::Map<String, Value>>,
    caller_label: &str,
) -> anyhow::Result<String> {
    let mut out = String::with_capacity(template.len());
    let mut cursor = 0usize;
    let bytes = template.as_bytes();
    while cursor < bytes.len() {
        if cursor + 1 < bytes.len() && bytes[cursor] == b'{' && bytes[cursor + 1] == b'{' {
            // Find the matching `}}`.
            let rest = &template[cursor + 2..];
            let end = rest.find("}}").ok_or_else(|| {
                anyhow::anyhow!(
                    "{caller_label}: template {:?} has an unclosed `{{{{`",
                    template
                )
            })?;
            let key = rest[..end].trim();
            if key.is_empty() {
                anyhow::bail!(
                    "{caller_label}: template {:?} contains an empty `{{{{ }}}}` placeholder",
                    template
                );
            }
            let bindings = bindings.ok_or_else(|| {
                anyhow::anyhow!(
                    "{caller_label}: template {:?} references arg `{}` but the call \
                     passed no args (the args field was null)",
                    template,
                    key
                )
            })?;
            let val = bindings.get(key).ok_or_else(|| {
                anyhow::anyhow!(
                    "{caller_label}: template {:?} references arg `{}` which is not \
                     present in the call's arguments (provided keys: {:?})",
                    template,
                    key,
                    bindings.keys().collect::<Vec<_>>()
                )
            })?;
            out.push_str(&stringify_arg(val));
            cursor += 2 + end + 2;
        } else {
            // Pass the byte through. Safe because we only matched
            // ASCII `{` boundaries; multi-byte UTF-8 chars start
            // with a non-`{` byte so they never get split here.
            let ch_end = next_char_boundary(template, cursor);
            out.push_str(&template[cursor..ch_end]);
            cursor = ch_end;
        }
    }
    Ok(out)
}

fn next_char_boundary(s: &str, from: usize) -> usize {
    let mut i = from + 1;
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i.min(s.len())
}

fn stringify_arg(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        // Numbers, bools, null, arrays, objects — JSON-encode so
        // the representation is unambiguous. A consumer that wants
        // a bare numeric will get `42`; one that wants a JSON
        // object gets `{...}`. Strings round-trip without quoting
        // because we special-cased them above (otherwise
        // `{{ name }}` would emit `"alice"` with the JSON quotes,
        // which is rarely what the ability author wants).
        other => other.to_string(),
    }
}

fn short_kind(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn substitutes_string_arg() {
        let out = render_template("hello {{ who }}", &json!({"who": "world"}), "test").unwrap();
        assert_eq!(out, "hello world");
    }

    #[test]
    fn substitutes_number_as_json() {
        let out = render_template("{{ count }}", &json!({"count": 42}), "test").unwrap();
        assert_eq!(out, "42");
    }

    #[test]
    fn substitutes_object_as_json() {
        let out = render_template("{{ obj }}", &json!({"obj": {"a": 1}}), "test").unwrap();
        assert_eq!(out, "{\"a\":1}");
    }

    #[test]
    fn whitespace_inside_braces_tolerated() {
        let out = render_template("{{name}} == {{ name }}", &json!({"name": "x"}), "test").unwrap();
        assert_eq!(out, "x == x");
    }

    #[test]
    fn missing_arg_errors_with_caller_label() {
        let err = render_template("{{ x }}", &json!({"y": 1}), "shell executor").unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("shell executor"));
        assert!(msg.contains("x"));
    }

    #[test]
    fn unclosed_braces_error() {
        let err = render_template("{{ broken", &json!({}), "test").unwrap_err();
        assert!(format!("{err}").contains("unclosed"));
    }

    #[test]
    fn empty_braces_error() {
        let err = render_template("{{ }}", &json!({}), "test").unwrap_err();
        assert!(format!("{err}").contains("empty"));
    }

    #[test]
    fn null_args_with_no_placeholders_passes_through() {
        let out = render_template("plain text", &Value::Null, "test").unwrap();
        assert_eq!(out, "plain text");
    }

    #[test]
    fn null_args_with_placeholder_errors() {
        let err = render_template("{{ x }}", &Value::Null, "test").unwrap_err();
        assert!(format!("{err}").contains("no args") || format!("{err}").contains("null"));
    }

    #[test]
    fn non_object_args_error() {
        let err = render_template("{{ x }}", &json!([1, 2]), "test").unwrap_err();
        assert!(format!("{err}").contains("must be a JSON object"));
    }

    #[test]
    fn render_each_processes_each_template_independently() {
        let out = render_each(
            &["a {{ x }}".to_string(), "b {{ y }}".to_string()],
            &json!({"x": "X", "y": "Y"}),
            "test",
        )
        .unwrap();
        assert_eq!(out, vec!["a X".to_string(), "b Y".to_string()]);
    }

    #[test]
    fn multibyte_utf8_passes_through() {
        let out = render_template("汉字 {{ x }} 字汉", &json!({"x": "中间"}), "test").unwrap();
        assert_eq!(out, "汉字 中间 字汉");
    }
}
