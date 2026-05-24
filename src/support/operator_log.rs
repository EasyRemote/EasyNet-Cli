// EasyNet CLI — Operator log macro
// =================================
//
// File: src/support/operator_log.rs
//
// Why this module exists
// ----------------------
// The codebase prints operator-visible events to stderr via `eprintln!`
// using a convention documented in `support/mod.rs`:
//
//     [<component>] kind=<event> key=val ...
//
// A documented convention is not enforced by the compiler. The audit
// of the 2026-05-24 streamable-HTTP / federation work found a fresh
// `eprintln!` that violated the convention inside the same PR that
// codified it (`forward_invoke.auto_route` used as `kind`, with a dot
// instead of underscore, and a `kind=` prefix missing entirely). That
// breakage is the cost of `eprintln!`-driven logging — there is no
// type to enforce the shape.
//
// [`op_event!`] is a thin macro that mechanises the convention.
// `component` and `kind` are required `ident` tokens, so the compiler
// rejects spaces, dots, and unquoted strings at the call site. Field
// names are `ident`s too; field values are any `Display`. Values that
// contain whitespace are auto-quoted at format time so a downstream
// `awk` / `cut` pipeline sees stable field boundaries.
//
// Migration to `tracing`
// ----------------------
// If/when the daemon adopts `tracing`, the macro body is the only
// place that needs to change — the call sites already declare
// component / kind / field=value in the structure `tracing::event!`
// wants. No call-site rewrite, no field-name churn.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

/// Format a single field value. Wraps in double quotes when the
/// value's `Display` form contains whitespace, so SRE pipelines that
/// split on `' '` see one field per token.
///
/// Not part of the public surface — only [`op_event!`] expansion
/// calls this. Public visibility is required because macro
/// expansion happens at the call-site crate position.
#[doc(hidden)]
pub fn fmt_field_value(value: &dyn std::fmt::Display) -> String {
    let rendered = format!("{value}");
    if rendered.chars().any(char::is_whitespace) {
        // Replace any embedded `"` with `\"` so the quote we add
        // doesn't terminate the value early. We do not escape
        // backslashes — operator log is not a shell-parseable format;
        // the only invariant we owe SRE is "one field per
        // whitespace-separated token".
        format!("\"{}\"", rendered.replace('"', "\\\""))
    } else if rendered.is_empty() {
        // An empty value still needs SOME token so `cut -d' ' -f3`
        // does not collapse two fields into one.
        "\"\"".to_string()
    } else {
        rendered
    }
}

/// Emit one operator-log line on stderr in the project's standard
/// format. Compile-time-enforced shape:
///
/// ```text
/// op_event!(
///     component = mcp_http_client,    // ident — no spaces, no dots
///     kind      = tls_insecure,       // ident — stable event class
///     host      = host,               // ident = Display
///     port      = port,
/// );
/// ```
///
/// Renders as (underscores in `component` become hyphens for the
/// `[bracket]` tag only, matching the existing `[axon-serve]` /
/// `[mcp-http-client]` convention; `kind` and field names are
/// emitted verbatim so `grep kind=tls_insecure` is stable):
///
/// ```text
/// [mcp-http-client] kind=tls_insecure host=example.com port=8443
/// ```
///
/// Values containing whitespace are auto-quoted. Empty values render
/// as `""` so field boundaries stay stable.
#[macro_export]
macro_rules! op_event {
    (
        component = $component:ident,
        kind = $kind:ident
        $(, $field:ident = $value:expr )*
        $(,)?
    ) => {{
        // The `replace('_', "-")` happens on a static string at format
        // time; the optimiser folds it on stable Rust. Doing it here
        // (rather than at the call site) means call sites read with
        // Rust-idiomatic underscores.
        let component_tag = stringify!($component).replace('_', "-");
        // `#[allow(unused_mut)]` because the zero-extra-fields call
        // shape never `push`es, but the macro's general form needs
        // mutability. Cleaner than splitting the macro into two arms.
        #[allow(unused_mut)]
        let mut line = format!("[{}] kind={}", component_tag, stringify!($kind));
        $(
            line.push(' ');
            line.push_str(stringify!($field));
            line.push('=');
            line.push_str(&$crate::support::operator_log::fmt_field_value(&$value));
        )*
        eprintln!("{line}");
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    /// We cannot easily capture `eprintln!` from a unit test without
    /// fd-redirection plumbing. Instead, we exercise the format
    /// helpers directly and assert the macro expands and runs without
    /// panic — the format invariants are unit-tested through
    /// [`fmt_field_value`].
    #[test]
    fn empty_value_renders_as_quoted_empty_string() {
        assert_eq!(fmt_field_value(&""), "\"\"");
    }

    #[test]
    fn plain_value_renders_verbatim() {
        assert_eq!(fmt_field_value(&"tls_insecure"), "tls_insecure");
        assert_eq!(fmt_field_value(&42_u32), "42");
    }

    #[test]
    fn value_with_space_is_quoted() {
        assert_eq!(fmt_field_value(&"hello world"), "\"hello world\"");
    }

    #[test]
    fn value_with_inner_double_quote_is_escaped() {
        assert_eq!(
            fmt_field_value(&"say \"hi\" friend"),
            "\"say \\\"hi\\\" friend\""
        );
    }

    #[test]
    fn value_with_tab_or_newline_is_quoted() {
        assert_eq!(fmt_field_value(&"a\tb"), "\"a\tb\"");
        assert_eq!(fmt_field_value(&"a\nb"), "\"a\nb\"");
    }

    #[test]
    fn macro_expands_and_runs_without_panic() {
        // No assertion possible without fd capture; the test passes
        // if expansion compiles and the runtime call returns.
        let host = "example.com";
        let port = 8443_u16;
        op_event!(
            component = mcp_http_client,
            kind = tls_insecure,
            host = host,
            port = port,
        );
    }

    #[test]
    fn macro_with_zero_extra_fields_expands() {
        op_event!(component = axon_serve, kind = boot_complete);
    }
}
