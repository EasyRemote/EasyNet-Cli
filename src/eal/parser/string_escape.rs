// EasyNet CLI — EAL string-literal escape contract (F-024)
// =========================================================
//
// File: src/eal/string_escape.rs
//
// THE CONTRACT (normative — the two halves):
//
//   1. The lexer preserves escapes VERBATIM. `\` only shields the next
//      byte from terminating the literal; both bytes stay in the
//      `StringLit`. What the author types is what gets carried.
//      (Pinned by `lexer::tests::tokenize_strings_preserves_escape_
//      sequences_verbatim`.)
//
//   2. A consumer that must MACHINE-PARSE a string-literal payload —
//      the `*_json` ability-argument convention is the canonical case —
//      unescapes through [`unescape_string_literal`] first, never with
//      ad-hoc `replace` calls. Opaque payloads (prompts, regexes,
//      shell snippets) are forwarded untouched: their escapes belong
//      to the destination, not to EAL.
//
// Why this split: EAL is an orchestration DSL whose strings are mostly
// opaque payloads. Decoding in the lexer would force `\\n` on every
// user who wants a literal `\n` in a regex. But an embedded JSON value
// can only be written as `"{\"k\":\"v\"}"` — the author HAD to escape
// the quotes to embed it, so the JSON consumer must peel exactly that
// authoring layer back off. This module is that single peel point.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

/// Peel the authoring-level escapes off a verbatim EAL string literal.
///
/// Decodes exactly the two sequences the author is FORCED to write to
/// embed the characters at all — `\"` → `"` and `\\` → `\` — and
/// preserves every other `\x` pair verbatim (those are payload, e.g.
/// `\n` in a regex argument, and the lexer's what-you-type-is-what-
/// gets-sent promise covers them).
///
/// Wrappers that accept `*_json` arguments MUST run the raw literal
/// through this function before `serde_json::from_str`.
//
// dead_code: today's consumers are ability wrappers outside this
// crate; the first in-crate caller lands with the ESO operator lane.
// Deliberately NOT auto-applied in the planner — wrappers that
// already unescape would double-decode.
#[allow(dead_code)]
pub(crate) fn unescape_string_literal(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            // Any other escape is payload: keep both bytes verbatim.
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            // Trailing lone backslash: keep it (lexer can only produce
            // this when the literal ended at input EOF guard).
            None => out.push('\\'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::unescape_string_literal;
    use crate::eal::parser::lexer::{Lexer, Token};

    #[test]
    fn peels_quote_and_backslash_escapes_only() {
        assert_eq!(unescape_string_literal(r#"{\"k\":\"v\"}"#), r#"{"k":"v"}"#);
        assert_eq!(unescape_string_literal(r"a\\b"), r"a\b");
        // Payload escapes stay verbatim — they belong to the destination.
        assert_eq!(unescape_string_literal(r"line\nbreak"), r"line\nbreak");
        assert_eq!(unescape_string_literal(r"\d+\.\d+"), r"\d+\.\d+");
    }

    /// F-024 acceptance: the `\"…\"` end-to-end round trip. An EAL
    /// author embeds a JSON object in a string literal; the lexer
    /// carries it verbatim; the shared helper peels the authoring
    /// escapes; the result is machine-parseable and value-identical.
    #[test]
    fn json_argument_round_trips_from_eal_source_to_parsed_value() {
        let src = r#"agent.run(config_json: "{\"retries\":2,\"name\":\"a \\\"b\\\" c\"}")"#;
        let tokens = Lexer::new(src).tokenize().expect("tokenize");
        let raw = tokens
            .iter()
            .find_map(|t| match t {
                Token::StringLit(s) => Some(s.clone()),
                _ => None,
            })
            .expect("string literal present");

        // Verbatim per the lexer contract: escapes still in place.
        assert!(raw.contains(r#"\""#), "lexer must not decode: {raw}");

        let peeled = unescape_string_literal(&raw);
        let value: serde_json::Value =
            serde_json::from_str(&peeled).expect("peeled literal is valid JSON");
        assert_eq!(value["retries"], 2);
        assert_eq!(value["name"], r#"a "b" c"#);
    }
}
