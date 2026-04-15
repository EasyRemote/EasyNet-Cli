// EasyNet CLI
// ===========
//
// File: src/cli/ability_scaffold/templates.rs
// Description: Template layer for `easynet ability new`.
//
// Each output file has a matching `.tmpl` under `templates/` that holds the
// *exact* bytes of the generated artifact. Placeholders are enclosed in
// single braces (`{name}`, `{desc}`, `{handler}`) and substituted here —
// this explicit substitution is intentional: Rust's `format!` macro's
// `{{ }}` escaping gets catastrophic on shell/JSON heredocs, which is
// where Bug #1 (invalid-JSON handler.sh default) originally hid.
//
// Why separate files instead of `format!` in Rust:
//   - Editors syntax-highlight .tmpl files by extension.
//   - Templates read as what they are (shell, python, markdown), not as
//     quote-escaped Rust string literals.
//   - Golden tests can diff byte-for-byte against the .tmpl source.
//
// Why hand-rolled substitution instead of a template crate:
//   - Our placeholder set is closed (name/desc/handler).
//   - A template engine dependency would dwarf the logic.
//   - The substitution function is 10 lines and trivially testable.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use serde_json::{json, Value};

use super::AbilityLang;

/// Inputs shared by every templated file.
pub(super) struct ScaffoldCtx<'a> {
    pub name: &'a str,
    pub description: &'a str,
    pub lang: &'a AbilityLang,
}

const SKILL_MD: &str = include_str!("templates/SKILL.md.tmpl");
const INVOKE_SH: &str = include_str!("templates/invoke.sh.tmpl");
const INVOKE_SH_RUST: &str = include_str!("templates/invoke.rust.sh.tmpl");
const README_MD: &str = include_str!("templates/README.md.tmpl");
const HANDLER_SH: &str = include_str!("templates/handler.sh.tmpl");
const HANDLER_PY: &str = include_str!("templates/handler.py.tmpl");
const HANDLER_RS: &str = include_str!("templates/handler.rs.tmpl");

/// Substitute `{name}`, `{desc}`, `{handler}` placeholders in a template.
///
/// Takes the three concrete substitution values — not a `ScaffoldCtx` —
/// so the rendering layer doesn't know about `AbilityLang` or anything
/// higher-level. Each caller decides which fields to thread.
///
/// This is a **single-pass scan** (not chained `.replace()`) on purpose.
/// The chained form is not idempotent: given `name = "foo{desc}bar"`, the
/// second pass would substitute the description *inside the user's name*,
/// corrupting the output. A single pass guarantees that substitution
/// values are never re-scanned for later placeholders — the collision is
/// impossible regardless of input alphabet.
///
/// Unknown placeholders are left untouched: template files legitimately
/// contain literal `{` characters (JSON examples in README, shell
/// parameter expansion in handler.sh) and a renamed-but-forgotten
/// placeholder must not corrupt them silently. Golden tests guard drift.
fn render(template: &str, name: &str, desc: &str, handler: &str) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(brace) = rest.find('{') {
        out.push_str(&rest[..brace]);
        let after = &rest[brace..];
        // Try each known placeholder at this position; first match wins.
        if let Some(tail) = after.strip_prefix("{name}") {
            out.push_str(name);
            rest = tail;
        } else if let Some(tail) = after.strip_prefix("{desc}") {
            out.push_str(desc);
            rest = tail;
        } else if let Some(tail) = after.strip_prefix("{handler}") {
            out.push_str(handler);
            rest = tail;
        } else {
            // Unknown `{...}` — copy the `{` literally and advance past it.
            // This preserves `${foo}`, `{}`, `{ability_package_root}`, etc.
            out.push('{');
            rest = &after[1..];
        }
    }
    out.push_str(rest);
    out
}

/// Thin adapter that feeds a `ScaffoldCtx` through `render`. Keeps every
/// public entry point on this module a one-liner without leaking the
/// shape of `ScaffoldCtx` into the rendering primitive.
fn render_ctx(template: &str, ctx: &ScaffoldCtx<'_>) -> String {
    render(
        template,
        ctx.name,
        ctx.description,
        ctx.lang.handler_filename(),
    )
}

/// The `ability.json` manifest. Deliberately a superset of the MCP tool
/// manifest and the Agent Skills schema: one directory, three contracts.
///
/// We build this with `serde_json::json!` rather than a template because
/// the structure is a JSON object (not a text artifact) and pretty-printing
/// it through serde_json gives us canonical key ordering for free.
pub(super) fn ability_manifest(ctx: &ScaffoldCtx<'_>, tool_name: &str) -> Value {
    json!({
        "name": ctx.name,
        "version": "0.1.0",
        "tool_name": tool_name,
        "description": ctx.description,
        "command": "bash {ability_package_root}/scripts/invoke.sh",
        "input_schema": {
            "type": "object",
            "properties": {
                "message": {
                    "type": "string",
                    "description": "Example field — replace with your real input."
                }
            }
        },
        "output_schema": {
            "type": "object",
            "properties": {
                "ok":   { "type": "boolean" },
                "echo": {}
            },
            "required": ["ok"]
        },
        "tags": ["ability", "skill"],
        "category": "uncategorized",
        "read_only_hint":   false,
        "destructive_hint": false,
        "idempotent_hint":  true,
        "open_world_hint":  false,
        "prerequisites":    [],
        "instructions": "Fill in the handler logic under scripts/. The \
                         command template wires stdin/stdout to the runtime; \
                         you do not need to re-implement the transport."
    })
}

pub(super) fn skill_md(ctx: &ScaffoldCtx<'_>) -> String {
    render_ctx(SKILL_MD, ctx)
}

pub(super) fn invoke_sh(ctx: &ScaffoldCtx<'_>) -> String {
    match ctx.lang {
        AbilityLang::Rust => render_ctx(INVOKE_SH_RUST, ctx),
        _ => render_ctx(INVOKE_SH, ctx),
    }
}

pub(super) fn readme(ctx: &ScaffoldCtx<'_>) -> String {
    let mut out = render_ctx(README_MD, ctx);
    if matches!(ctx.lang, AbilityLang::Rust) {
        out.push_str(&format!(
            "\n## Rust build\n\n\
             `scripts/invoke.sh` executes `target/release/{}` (falls back to `target/debug/{}`).\n\
             If neither binary exists and `rustc` is available, it auto-compiles `scripts/handler.rs`.\n\
             For deterministic deploys, prebuild explicitly:\n\n\
             ```bash\n\
             rustc scripts/handler.rs -O -o target/release/{}\n\
             ```\n",
            ctx.name, ctx.name, ctx.name
        ));
    }
    out
}

pub(super) fn handler_source(ctx: &ScaffoldCtx<'_>) -> String {
    let tmpl = match ctx.lang {
        AbilityLang::Sh => HANDLER_SH,
        AbilityLang::Python => HANDLER_PY,
        AbilityLang::Rust => HANDLER_RS,
    };
    render_ctx(tmpl, ctx)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx<'a>(name: &'a str, lang: &'a AbilityLang) -> ScaffoldCtx<'a> {
        ScaffoldCtx {
            name,
            description: "a description",
            lang,
        }
    }

    #[test]
    fn render_substitutes_all_three_placeholders() {
        let out = render(
            "name={name} desc={desc} handler={handler}",
            "hello",
            "a description",
            "handler.py",
        );
        assert_eq!(out, "name=hello desc=a description handler=handler.py");
    }

    #[test]
    fn render_leaves_unknown_braces_untouched() {
        // Shell parameter expansion `${foo}` and JSON object literals `{}`
        // must survive substitution unchanged.
        let out = render("${foo} {} {name}", "x", "d", "handler.sh");
        assert_eq!(out, "${foo} {} x");
    }

    #[test]
    fn render_is_single_pass_and_cannot_collide() {
        // Bug #7 regression. Under the old chained-replace implementation,
        // a substitution value containing `{desc}` would be rescanned in
        // the second pass and corrupted. The single-pass scanner must
        // treat substituted text as literal output, never re-scannable.
        let out = render(
            "first={name} second={desc}",
            "user-supplied-{desc}-name",
            "DESC",
            "handler.sh",
        );
        assert_eq!(
            out, "first=user-supplied-{desc}-name second=DESC",
            "substitution values must never be rescanned for placeholders"
        );
    }

    #[test]
    fn render_preserves_placeholder_inside_substitution_value() {
        // Even more pointed: the substitution value itself contains every
        // known placeholder. Each must survive verbatim.
        let out = render("[{name}]", "{name}{desc}{handler}", "D", "H");
        assert_eq!(out, "[{name}{desc}{handler}]");
    }

    #[test]
    fn render_handles_adjacent_and_repeated_placeholders() {
        let out = render("{name}{name}{desc}", "A", "B", "H");
        assert_eq!(out, "AAB");
    }

    #[test]
    fn render_leaves_brace_at_end_of_string() {
        // Defensive: an unmatched `{` at EOF must be copied through, not
        // cause an infinite loop or panic.
        let out = render("trailing {", "n", "d", "h");
        assert_eq!(out, "trailing {");
    }

    #[test]
    fn render_ctx_is_a_thin_adapter_over_render() {
        // Sanity check the adapter: same inputs threaded through the
        // ScaffoldCtx path must produce the same output as calling render
        // directly with equivalent fields.
        let direct = render(
            "n={name} d={desc} h={handler}",
            "x",
            "a description",
            "handler.sh",
        );
        let via_ctx = render_ctx("n={name} d={desc} h={handler}", &ctx("x", &AbilityLang::Sh));
        assert_eq!(direct, via_ctx);
    }

    #[test]
    fn handler_sh_default_body_is_valid_json() {
        let body = handler_source(&ctx("demo", &AbilityLang::Sh));
        // Sanity: the template contains the fixed `{"ok":true}` literal
        // we ship as the safe default. Guards against a future edit that
        // reintroduces the unsafe `$input` splicing (Bug #1 regression test).
        assert!(
            body.contains(r#"printf '%s' '{"ok":true}'"#),
            "handler.sh must emit a fixed valid-JSON body, got:\n{body}"
        );
        assert!(
            !body.contains("\"echo\":$input"),
            "handler.sh must NOT splice raw stdin into JSON (Bug #1 regression)"
        );
    }

    #[test]
    fn handler_rs_default_body_is_valid_json() {
        let body = handler_source(&ctx("demo", &AbilityLang::Rust));
        assert!(
            body.contains(r##"br#"{"ok":true}"#"##),
            "handler.rs must emit a fixed valid-JSON byte string, got:\n{body}"
        );
        assert!(
            !body.contains(r#"\"echo\":{}"#),
            "handler.rs must NOT format! the raw stdin into JSON"
        );
    }

    #[test]
    fn handler_py_still_echoes_through_json_module() {
        // Python's json module escapes correctly, so the echo pattern is
        // safe there — keep it as a working example of the "right" way.
        let body = handler_source(&ctx("demo", &AbilityLang::Python));
        assert!(body.contains("json.dump({\"ok\": True, \"echo\": payload}"));
    }

    #[test]
    fn skill_md_contains_name_and_description() {
        let out = skill_md(&ctx("image-resize", &AbilityLang::Sh));
        assert!(out.contains("name: image-resize"));
        assert!(out.contains("description: a description"));
        assert!(out.contains("# image-resize"));
    }

    #[test]
    fn invoke_sh_points_at_language_specific_handler() {
        assert!(invoke_sh(&ctx("x", &AbilityLang::Sh)).contains("$HERE/handler.sh"));
        assert!(invoke_sh(&ctx("x", &AbilityLang::Python)).contains("$HERE/handler.py"));
        let rust_invoke = invoke_sh(&ctx("x", &AbilityLang::Rust));
        assert!(rust_invoke.contains("target/release/x"));
        assert!(rust_invoke.contains("target/debug/x"));
        assert!(!rust_invoke.contains("$HERE/handler.rs"));
    }

    #[test]
    fn readme_references_the_chosen_handler_file() {
        let out = readme(&ctx("x", &AbilityLang::Python));
        assert!(out.contains("scripts/handler.py"));
    }

    #[test]
    fn rust_readme_includes_build_instruction() {
        let out = readme(&ctx("x", &AbilityLang::Rust));
        assert!(out.contains("rustc scripts/handler.rs -O -o target/release/x"));
        assert!(out.contains("target/release/x"));
        assert!(out.contains("auto-compiles `scripts/handler.rs`"));
    }

    #[test]
    fn manifest_tool_name_defaults_to_normalized_name() {
        let m = ability_manifest(&ctx("hello-world", &AbilityLang::Sh), "hello-world");
        assert_eq!(m["tool_name"], "hello-world");
        assert_eq!(m["name"], "hello-world");
    }
}
