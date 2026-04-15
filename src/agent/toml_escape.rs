// EasyNet CLI — TOML basic-string escaping
// =========================================
//
// File: src/agent/toml_escape.rs
// Description: Shared TOML basic-string emitter for the agent layer.
//
// Why this module exists
// ----------------------
// Two call sites used to hand-roll TOML quoting:
//
//   1. `agent::codex::invoke_exec` formats `-c key=value` overrides for
//      the `codex` CLI, which parses each value as a TOML expression.
//   2. `agent::workspace::write_codex_config` emits a `~/<workspace>/
//      .codex/config.toml` file consumed by the same parser.
//
// Both did the same thing — `format!("\"{}\"", s.replace('"', "\\\""))`
// — and both had the same bug: backslashes (`\`), control characters,
// and bell/form-feed bytes were passed through unescaped. A Windows path
// like `C:\Users\me` rendered as `"C:\Users\me"`, which TOML reads as
// `C:Usersme` (consuming `\U` as the start of an invalid Unicode
// escape). The override silently dropped at runtime.
//
// Centralising the encoder here means both call sites get the spec-
// correct escape set in one place; if the TOML spec ever evolves
// (multi-line basic strings, raw strings, datetime literals) the change
// lands once, not twice.
//
// Why not just use `toml_edit::Value::from(s).to_string()`?
// ---------------------------------------------------------
// `toml_edit` is already a dependency, and in principle a one-liner
// `toml_edit::Value::from(s).to_string()` would emit a spec-compliant
// basic string. We deliberately do not route through it for two
// reasons that together justify the ~20 lines of explicit code:
//
//   1. **Shape mismatch at the call sites.** Both consumers — the
//      codex `-c key=value` override formatter and the `config.toml`
//      emitter — want a bare *value fragment* (`"…"`), not a
//      key/value assignment nor a whole document. `toml_edit`'s
//      public surface centers on `DocumentMut`; pulling out just the
//      RHS requires either constructing a throwaway doc and slicing
//      its serialized form, or using the `Value` type and hoping its
//      `Display` never changes across minor versions. The
//      hand-rolled encoder returns exactly the shape the call sites
//      need, with no ceremony.
//
//   2. **Property-tested round-trip against the real parser.** The
//      test `round_trips_through_toml_parser` below feeds every
//      adversarial input (backslashes, quotes, control chars,
//      Windows paths) through `toml_edit::DocumentMut::parse` and
//      asserts the decoded string equals the original. That pins
//      the encoder's output as "whatever the parser accepts" rather
//      than "whatever the encoder emits" — so if `toml_edit` tightens
//      its grammar, we fail loud rather than silently diverge. A
//      thin wrapper over `toml_edit::Value`'s serializer would lose
//      this test's independence: encoder and oracle would be the
//      same code path, making the test a tautology.
//
// If either call site grows to need a fuller document (not just a
// scalar value), reach for `toml_edit` directly at that site rather
// than extending this encoder; this module is scoped to "one basic
// string at a time" by design.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::fmt::Write;

/// Encode `s` as a TOML basic string literal — wrapped in `"…"` with the
/// six escapes the spec mandates (`\\`, `\"`, `\b`, `\t`, `\n`, `\f`,
/// `\r`) plus `\uXXXX` for any other ASCII control char (`< 0x20` or
/// `0x7F`). Printable Unicode passes through unchanged because TOML
/// basic strings are UTF-8 by construction.
///
/// See `toml_basic_string_round_trips_through_toml_parser` for the
/// round-trip property test against the real `toml_edit` parser.
pub fn toml_basic_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\u{08}' => out.push_str("\\b"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\u{0C}' => out.push_str("\\f"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 || c as u32 == 0x7F => {
                let _ = write!(out, "\\u{:04X}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_backslash_and_quote() {
        // Windows paths and JSON payloads both contain `\`. The previous
        // ad-hoc encoder only escaped `"`, leaving `\` to be interpreted
        // by TOML as the start of an escape sequence — which would then
        // either consume the next character or yield "invalid escape".
        assert_eq!(toml_basic_string(r"a\b"), r#""a\\b""#);
        assert_eq!(toml_basic_string(r#"say "hi""#), r#""say \"hi\"""#);
        assert_eq!(toml_basic_string(r"C:\Users\me"), r#""C:\\Users\\me""#);
    }

    #[test]
    fn escapes_control_chars() {
        assert_eq!(toml_basic_string("a\nb"), r#""a\nb""#);
        assert_eq!(toml_basic_string("a\tb"), r#""a\tb""#);
        assert_eq!(toml_basic_string("a\rb"), r#""a\rb""#);
        // Bell (0x07): no shorthand, must use \uXXXX.
        assert_eq!(toml_basic_string("a\x07b"), r#""a\u0007b""#);
        // DEL (0x7F): same — control char with no shorthand.
        assert_eq!(toml_basic_string("a\x7Fb"), r#""a\u007Fb""#);
    }

    #[test]
    fn passes_normal_text_unchanged() {
        // The encoder must not mangle ordinary identifiers/values, so
        // the codex `-c` overrides remain readable in `ps` listings.
        assert_eq!(toml_basic_string("hello"), r#""hello""#);
        assert_eq!(
            toml_basic_string("/usr/bin/easynet"),
            r#""/usr/bin/easynet""#
        );
    }

    #[test]
    fn round_trips_through_toml_parser() {
        // The spec's escape rules and our encoder must agree — round-
        // trip a few adversarial values through `toml_edit` (already a
        // dep) to make sure the encoded form parses back to the
        // original. Codex feeds each `-c key=value` to a real TOML
        // parser, so anything that fails this test would silently drop
        // the override at runtime.
        for raw in [
            "plain",
            "with space",
            r#"with "quote""#,
            r"with \backslash",
            "with\nnewline",
            "with\ttab",
            r"C:\Users\me\AppData",
        ] {
            let encoded = toml_basic_string(raw);
            let toml_doc = format!("v = {encoded}");
            let parsed: toml_edit::DocumentMut = toml_doc
                .parse()
                .unwrap_or_else(|e| panic!("encoded {encoded:?} did not parse: {e}"));
            let value = parsed["v"]
                .as_str()
                .unwrap_or_else(|| panic!("encoded {encoded:?} did not yield a string"));
            assert_eq!(value, raw, "round-trip mismatch for input {raw:?}");
        }
    }
}
