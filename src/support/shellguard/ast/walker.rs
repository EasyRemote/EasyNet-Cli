// EasyNet CLI — ShellGuard: fail-closed AST walker
// =================================================
//
// File: src/support/shellguard/ast/walker.rs
// Description: Translates a tree-sitter-bash `Tree` into a flat
//              list of `SimpleCommand` records, OR a `TooComplex`
//              verdict naming the offending node / pre-check.
//              Implements the AXIOM Tier 2.5 §"shell.run / 8-stage
//              pipeline / Stage 3" walker.
//
// Design philosophy: fail-closed allowlist
// ----------------------------------------
// Every node type the walker meets falls into one of three
// buckets:
//
//   STRUCTURAL — recurse into children, collect their commands.
//                Members: `program`, `list`, `pipeline`,
//                `redirected_statement`. Separator tokens
//                between siblings (`&&`, `||`, `;`, `|`, `&`,
//                `|&`, `\n`) are skipped.
//   LEAF       — extract one `SimpleCommand`. Members: `command`,
//                `declaration_command` (export/local/readonly/
//                declare/typeset). The walker reads argv tokens
//                from the leaf's children and stops; quotes have
//                already been removed by tree-sitter's
//                `string`/`raw_string`/`word` nodes' `text`.
//   IGNORE     — skip silently. Members: `comment`.
//
// Anything else — including any node tree-sitter-bash adds in a
// future grammar bump — falls into the wildcard arm and triggers
// `TooComplex` with the offending `node.kind()` recorded. This
// is the load-bearing safety property: a receiver upgrading
// tree-sitter-bash can NEVER end up silently accepting a new
// node type, because the wildcard arm catches it. AliveCode
// observed this property in production for ~18 months across
// three grammar bumps without a single false-accept.
//
// Pre-check rejections
// --------------------
// Five string-level patterns are checked before tree-sitter is
// even invoked. They cover known tree-sitter / bash differentials
// that are NOT discoverable from the tree alone:
//
//   1. Control characters (0x00-0x08, 0x0B-0x1F, 0x7F) — bash
//      drops them silently; tree-sitter splits on them.
//   2. Unicode whitespace (NBSP, ZW spaces, etc.) — invisible
//      to a reviewing operator; bash treats as word chars,
//      tree-sitter splits.
//   3. Backslash-escaped whitespace (`\ ` or `c\<NL>` adjacent
//      to a word) — bash joins, tree-sitter splits.
//   4. Zsh `~[name]` dynamic directory expansion — runs hook;
//      tree-sitter parses as literal.
//   5. Zsh `=cmd` equals expansion at word start — runs
//      `which cmd`; tree-sitter parses as literal word.
//
// (6, brace-with-quote obfuscation, is implemented in slice 4
// alongside the brace-expansion detector — it depends on a
// quote-aware brace-masker.)
//
// Out of scope (slice 4)
// ----------------------
// * `command_substitution` placeholder substitution ($())
// * `simple_expansion` variable tracking ($VAR with $VAR set
//   earlier in the same `&&`-chain via `VAR=val`)
// * `expansion` (${...}) handling of safe-env / special vars
//
// All three currently route to `TooComplex { node_type: ... }`.
// Future slice replaces those arms with substitution logic
// while keeping the same `ParseForSecurityResult` contract.
//
// Author: Silan.Hu
// Email: silan.hu@u.nus.edu
// Copyright (c) 2026-2027 easynet. All rights reserved.

use tree_sitter::Node;

use super::parser::{parse_root, ParseError};
use super::{EnvAssignment, ParseForSecurityResult, Redirect, SimpleCommand};

/// Public entry point. Returns:
///
///   * `Simple { commands }` if every node is on the allowlist
///     and every pre-check passes. `commands` may be empty —
///     for an empty input string, a comment-only string, or a
///     bare separator.
///   * `TooComplex { reason, node_type }` if any check fails.
///   * `ParseUnavailable` if tree-sitter itself failed to load
///     the grammar (build-time invariant violation).
pub fn parse_for_security(cmd: &str) -> ParseForSecurityResult {
    if cmd.is_empty() {
        return ParseForSecurityResult::Simple { commands: vec![] };
    }
    if let Some(reason) = pre_check(cmd) {
        return ParseForSecurityResult::too_complex_pre(reason);
    }

    // Trim ONLY for the empty check below — pre-checks above run
    // on the raw string so they can see the very Unicode whitespace
    // they're supposed to detect.
    if cmd.trim().is_empty() {
        return ParseForSecurityResult::Simple { commands: vec![] };
    }

    let tree = match parse_root(cmd) {
        Ok(t) => t,
        Err(ParseError::LanguageLoad(_)) => return ParseForSecurityResult::ParseUnavailable,
        Err(ParseError::ParseFailed) => return ParseForSecurityResult::ParseUnavailable,
    };
    let root = tree.root_node();

    if root.has_error() {
        return ParseForSecurityResult::too_complex_node(
            "tree-sitter reported a parse error in the command",
            "ERROR",
        );
    }

    let mut commands: Vec<SimpleCommand> = Vec::new();
    if let Some(err) = collect_commands(root, cmd, &mut commands) {
        return err;
    }
    ParseForSecurityResult::Simple { commands }
}

/// Placeholder string written into argv when a `$(inner)` is
/// extracted out of a double-quoted string. Mirrors AliveCode's
/// `__CMDSUB_OUTPUT__` so cross-implementation permission rules
/// match the same argv shape.
///
/// The placeholder is intentionally noisy — three letters, three
/// underscores, and an English word — so it cannot collide with
/// a path or flag a caller would legitimately type. A literal
/// occurrence in the *input* would still pass through, but later
/// stages that look for this exact constant treat any presence
/// as "this argv was synthesised, don't trust it as a path."
pub const CMDSUB_PLACEHOLDER: &str = "__CMDSUB_OUTPUT__";

// --- pre-checks -----------------------------------------------------------

fn pre_check(cmd: &str) -> Option<&'static str> {
    if has_control_char(cmd) {
        return Some("Contains control characters");
    }
    if has_unicode_whitespace(cmd) {
        return Some("Contains Unicode whitespace");
    }
    if has_backslash_whitespace(cmd) {
        return Some("Contains backslash-escaped whitespace");
    }
    if has_zsh_tilde_bracket(cmd) {
        return Some("Contains zsh ~[ dynamic directory syntax");
    }
    if has_zsh_equals_expansion(cmd) {
        return Some("Contains zsh =cmd equals expansion");
    }
    None
}

fn has_control_char(cmd: &str) -> bool {
    cmd.chars().any(|c| {
        let n = c as u32;
        (n <= 0x08) || (0x0B..=0x1F).contains(&n) || n == 0x7F
    })
}

fn has_unicode_whitespace(cmd: &str) -> bool {
    cmd.chars().any(|c| {
        matches!(
            c,
            '\u{00A0}'
                | '\u{1680}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202F}'
                | '\u{205F}'
                | '\u{3000}'
                | '\u{FEFF}'
        ) || ('\u{2000}'..='\u{200B}').contains(&c)
    })
}

/// `\ ` or `\<TAB>` anywhere, OR a `\<NL>` immediately preceded
/// by a non-whitespace, non-backslash char. Mirrors AliveCode's
/// `BACKSLASH_WHITESPACE_RE = /\\[ \t]|[^ \t\n\\]\\\n/`.
fn has_backslash_whitespace(cmd: &str) -> bool {
    let bytes = cmd.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            let next = bytes[i + 1];
            if next == b' ' || next == b'\t' {
                return true;
            }
        }
        // `[^ \t\n\\]\\\n` — char before the backslash is not
        // whitespace / not another backslash, AND backslash is
        // followed by newline.
        if i + 2 < bytes.len() && bytes[i + 1] == b'\\' && bytes[i + 2] == b'\n' {
            let prev = bytes[i];
            if prev != b' ' && prev != b'\t' && prev != b'\n' && prev != b'\\' {
                return true;
            }
        }
    }
    false
}

fn has_zsh_tilde_bracket(cmd: &str) -> bool {
    cmd.contains("~[")
}

/// Word-initial `=` followed by an ASCII letter or `_`. "Word-
/// initial" means start-of-string OR preceded by whitespace,
/// `;`, `&`, or `|`.
fn has_zsh_equals_expansion(cmd: &str) -> bool {
    let bytes = cmd.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] != b'=' {
            continue;
        }
        let word_initial = if i == 0 {
            true
        } else {
            matches!(bytes[i - 1], b' ' | b'\t' | b'\n' | b';' | b'&' | b'|')
        };
        if !word_initial {
            continue;
        }
        let Some(&next) = bytes.get(i + 1) else {
            continue;
        };
        if next.is_ascii_alphabetic() || next == b'_' {
            return true;
        }
    }
    false
}

// --- structural recursion -------------------------------------------------

fn collect_commands<'a>(
    node: Node<'a>,
    src: &str,
    commands: &mut Vec<SimpleCommand>,
) -> Option<ParseForSecurityResult> {
    match node.kind() {
        // Empty / structural roots — recurse.
        "program" | "list" | "pipeline" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if SEPARATORS.contains(&child.kind()) {
                    continue;
                }
                if let Some(err) = collect_commands(child, src, commands) {
                    return Some(err);
                }
            }
            None
        }

        "redirected_statement" => match walk_redirected_statement(node, src, commands) {
            Ok(cmd) => {
                commands.push(cmd);
                None
            }
            Err(err) => Some(err),
        },

        "command" => match walk_command(node, src, commands) {
            Ok(cmd) => {
                commands.push(cmd);
                None
            }
            Err(err) => Some(err),
        },

        "declaration_command" => match walk_declaration_command(node, src, commands) {
            Ok(cmd) => {
                commands.push(cmd);
                None
            }
            Err(err) => Some(err),
        },

        "negated_command" => {
            // `! cmd` — recurse into the wrapped command. tree-
            // sitter emits `!` as a `!` literal child plus the
            // wrapped command. Skip the `!`.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "!" {
                    continue;
                }
                if let Some(err) = collect_commands(child, src, commands) {
                    return Some(err);
                }
            }
            None
        }

        "comment" => None,

        // Fail-closed wildcard. Anything not on the allowlist
        // above — `subshell`, `command_substitution`, `expansion`,
        // `simple_expansion`, `for_statement`, `while_statement`,
        // `if_statement`, `case_statement`, `function_definition`,
        // `test_command`, `arithmetic_expansion`, `process_sub-
        // stitution`, `compound_statement`, future grammar
        // additions — rejects.
        other => Some(ParseForSecurityResult::too_complex_node(
            format!("Disallowed node type `{other}`"),
            other,
        )),
    }
}

/// Tokens that separate commands inside `program`/`list`/
/// `pipeline`. Tree-sitter emits them as anonymous (un-named)
/// children with `kind()` equal to the literal token. Skipped
/// during recursion.
const SEPARATORS: &[&str] = &["&&", "||", "|", ";", "&", "|&", "\n"];

// --- redirected_statement -------------------------------------------------

fn walk_redirected_statement<'a>(
    node: Node<'a>,
    src: &str,
    inner: &mut Vec<SimpleCommand>,
) -> Result<SimpleCommand, ParseForSecurityResult> {
    // Children: one `command` (or `declaration_command`) followed
    // by one or more redirect nodes. Comments and whitespace are
    // un-named.
    let mut inner_cmd: Option<SimpleCommand> = None;
    let mut redirects: Vec<Redirect> = Vec::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "command" => {
                if inner_cmd.is_some() {
                    return Err(ParseForSecurityResult::too_complex_node(
                        "redirected_statement with multiple inner commands",
                        "redirected_statement",
                    ));
                }
                inner_cmd = Some(walk_command(child, src, inner)?);
            }
            "declaration_command" => {
                if inner_cmd.is_some() {
                    return Err(ParseForSecurityResult::too_complex_node(
                        "redirected_statement with multiple inner commands",
                        "redirected_statement",
                    ));
                }
                inner_cmd = Some(walk_declaration_command(child, src, inner)?);
            }
            "file_redirect" => {
                redirects.push(parse_file_redirect(child, src)?);
            }
            "heredoc_redirect" | "herestring_redirect" => {
                return Err(ParseForSecurityResult::too_complex_node(
                    format!("Disallowed redirect type `{}`", child.kind()),
                    child.kind(),
                ));
            }
            "comment" => {}
            other => {
                return Err(ParseForSecurityResult::too_complex_node(
                    format!("Disallowed child of redirected_statement `{other}`"),
                    other,
                ));
            }
        }
    }
    let mut cmd = inner_cmd.ok_or_else(|| {
        ParseForSecurityResult::too_complex_node(
            "redirected_statement with no inner command",
            "redirected_statement",
        )
    })?;
    cmd.redirects = redirects;
    cmd.text = node_text(node, src).to_string();
    Ok(cmd)
}

fn parse_file_redirect<'a>(
    node: Node<'a>,
    src: &str,
) -> Result<Redirect, ParseForSecurityResult> {
    // file_redirect has a child token that is the operator and a
    // child word/string for the target. Optionally a leading
    // `file_descriptor` (a number node) for `2>`.
    let mut op: Option<String> = None;
    let mut target: Option<String> = None;
    let mut fd: Option<u32> = None;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let kind = child.kind();
        match kind {
            ">" | ">>" | "<" | "<<" | ">&" | "<&" | ">|" | "&>" | "&>>" | "<<<" => {
                op = Some(kind.to_string());
            }
            "file_descriptor" => {
                if let Ok(n) = node_text(child, src).parse::<u32>() {
                    fd = Some(n);
                }
            }
            "word" | "string" | "raw_string" | "concatenation" | "number" => {
                target = Some(unquote_redirect_target(child, src));
            }
            "$" | "&" => {
                // tree-sitter sometimes splits `&>` into `&` + `>`
                // (older grammar versions). Compose if seen.
                // Modern grammar emits `&>` as a single token, so
                // this branch is defensive.
            }
            "command_substitution" | "simple_expansion" | "expansion" => {
                return Err(ParseForSecurityResult::too_complex_node(
                    format!("Redirect target uses disallowed expansion `{kind}`"),
                    kind,
                ));
            }
            _ => {
                // Un-named whitespace etc.
            }
        }
    }
    let op = op.ok_or_else(|| {
        ParseForSecurityResult::too_complex_node(
            "file_redirect with no operator token",
            "file_redirect",
        )
    })?;
    let target = target.ok_or_else(|| {
        ParseForSecurityResult::too_complex_node(
            "file_redirect with no target word",
            "file_redirect",
        )
    })?;
    Ok(Redirect { op, target, fd })
}

fn unquote_redirect_target(node: Node<'_>, src: &str) -> String {
    let raw = node_text(node, src);
    // Strip a single layer of matching ASCII quotes if both ends
    // match. tree-sitter emits the raw source span; the path-
    // constraints stage (slice 6) makes the final policy decision.
    if raw.len() >= 2 {
        let bytes = raw.as_bytes();
        if (bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\'')
        {
            return raw[1..raw.len() - 1].to_string();
        }
    }
    raw.to_string()
}

// --- command --------------------------------------------------------------

fn walk_command<'a>(
    node: Node<'a>,
    src: &str,
    inner: &mut Vec<SimpleCommand>,
) -> Result<SimpleCommand, ParseForSecurityResult> {
    let mut argv: Vec<String> = Vec::new();
    let mut env_vars: Vec<EnvAssignment> = Vec::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "variable_assignment" => {
                env_vars.push(walk_variable_assignment(child, src, inner)?);
            }
            "command_name" => {
                // command_name wraps a single `word` (or `string`,
                // `raw_string`, `concatenation`).
                let mut nc = child.walk();
                let mut pushed = false;
                for n in child.named_children(&mut nc) {
                    let arg = walk_argument(n, src, inner)?;
                    argv.push(arg);
                    pushed = true;
                }
                if !pushed {
                    return Err(ParseForSecurityResult::too_complex_node(
                        "command_name with no inner word",
                        "command_name",
                    ));
                }
            }
            "word" | "string" | "raw_string" | "number" | "concatenation" => {
                argv.push(walk_argument(child, src, inner)?);
            }
            "comment" => {}
            other => {
                return Err(ParseForSecurityResult::too_complex_node(
                    format!("Disallowed child of command `{other}`"),
                    other,
                ));
            }
        }
    }
    if argv.is_empty() {
        return Err(ParseForSecurityResult::too_complex_node(
            "command with no argv",
            "command",
        ));
    }
    Ok(SimpleCommand {
        argv,
        env_vars,
        redirects: Vec::new(),
        text: node_text(node, src).to_string(),
    })
}

fn walk_variable_assignment<'a>(
    node: Node<'a>,
    src: &str,
    inner: &mut Vec<SimpleCommand>,
) -> Result<EnvAssignment, ParseForSecurityResult> {
    // variable_assignment has children: `variable_name` `=` then
    // a value node (word / string / raw_string / concatenation).
    let mut name: Option<String> = None;
    let mut value: Option<String> = None;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let kind = child.kind();
        match kind {
            "variable_name" => name = Some(node_text(child, src).to_string()),
            "=" => {}
            "word" | "string" | "raw_string" | "number" | "concatenation" => {
                value = Some(walk_argument(child, src, inner)?);
            }
            "command_substitution" | "simple_expansion" | "expansion"
            | "process_substitution" | "arithmetic_expansion" => {
                return Err(ParseForSecurityResult::too_complex_node(
                    format!("Variable assignment value uses `{kind}`"),
                    kind,
                ));
            }
            _ => {
                // `array` literal `(a b c)` and other rarer forms.
                if !kind.trim().is_empty() && !kind.chars().all(|c| !c.is_alphanumeric()) {
                    return Err(ParseForSecurityResult::too_complex_node(
                        format!("Variable assignment uses unsupported child `{kind}`"),
                        kind,
                    ));
                }
            }
        }
    }
    let name = name.ok_or_else(|| {
        ParseForSecurityResult::too_complex_node(
            "variable_assignment with no name",
            "variable_assignment",
        )
    })?;
    // Empty value is legal (`VAR=`).
    let value = value.unwrap_or_default();
    Ok(EnvAssignment { name, value })
}

fn walk_argument<'a>(
    node: Node<'a>,
    src: &str,
    inner: &mut Vec<SimpleCommand>,
) -> Result<String, ParseForSecurityResult> {
    match node.kind() {
        "word" | "number" => Ok(node_text(node, src).to_string()),
        "raw_string" => {
            // Single-quoted: strip outer quotes verbatim. No
            // escape processing inside single quotes in bash.
            let raw = node_text(node, src);
            if raw.len() >= 2 && raw.starts_with('\'') && raw.ends_with('\'') {
                Ok(raw[1..raw.len() - 1].to_string())
            } else {
                Ok(raw.to_string())
            }
        }
        "string" => walk_double_string(node, src, inner),
        "concatenation" => {
            // Concatenation of word + string + word + ...
            // tree-sitter splits at quote boundaries. We
            // concatenate child texts; if any child is itself a
            // disallowed expansion we surface that.
            let mut out = String::new();
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                out.push_str(&walk_argument(child, src, inner)?);
            }
            Ok(out)
        }
        // BARE command_substitution at arg position is intentionally
        // rejected — the output IS the entire argument and could
        // be a path/flag (`rm $(echo /etc)`). Only `$()` *inside*
        // a double-quoted string is extracted; that case is
        // handled by walk_double_string which gates on the
        // sawDynamicPlaceholder + sawLiteralContent invariant.
        "command_substitution" | "simple_expansion" | "expansion"
        | "process_substitution" | "arithmetic_expansion" | "subshell"
        | "brace_expression" | "ansi_c_string" | "translated_string" => {
            Err(ParseForSecurityResult::too_complex_node(
                format!("Argument uses `{}`", node.kind()),
                node.kind(),
            ))
        }
        other => Err(ParseForSecurityResult::too_complex_node(
            format!("Argument uses unsupported node `{other}`"),
            other,
        )),
    }
}

fn walk_double_string<'a>(
    node: Node<'a>,
    src: &str,
    inner: &mut Vec<SimpleCommand>,
) -> Result<String, ParseForSecurityResult> {
    // A `string` node wraps `"..."`. Children are `string_content`
    // (raw text), expansions, and the surrounding `"` delimiters.
    //
    // SECURITY (ported from AliveCode walkString):
    //
    //   * `command_substitution` inside `"..."` recurses into the
    //     inner command(s) — they get appended to `inner`, and
    //     `__CMDSUB_OUTPUT__` is written into the outer argv.
    //     Both outer + inner must pass downstream permission
    //     checks. `simple_expansion` ($VAR) inside strings stays
    //     rejected in slice 4a; slice 4b adds variable scope
    //     substitution.
    //
    //   * Solo-placeholder strings reject. `cd "$(echo /etc)"`
    //     would otherwise produce argv `["cd", "__CMDSUB_OUTPUT__"]`
    //     which downstream path validation would resolve as a
    //     relative filename within cwd, bypassing the real check.
    //     Rule: `saw_dynamic_placeholder && !saw_literal_content`
    //     → too_complex.
    //
    //   * Bash double-quote escape rules apply to `string_content`:
    //     `\$`, `\``, `\"`, `\\` are unescaped; other backslash
    //     sequences stay literal.
    let mut out = String::new();
    let mut cursor = node.walk();
    let mut saw_dynamic_placeholder = false;
    let mut saw_literal_content = false;
    for child in node.children(&mut cursor) {
        let kind = child.kind();
        match kind {
            "\"" => {}
            "string_content" => {
                out.push_str(&unescape_double_quoted(node_text(child, src)));
                saw_literal_content = true;
            }
            "command_substitution" => {
                let err = collect_command_substitution(child, src, inner);
                if let Some(e) = err {
                    return Err(e);
                }
                out.push_str(CMDSUB_PLACEHOLDER);
                saw_dynamic_placeholder = true;
            }
            "simple_expansion" | "expansion" | "process_substitution"
            | "arithmetic_expansion" => {
                return Err(ParseForSecurityResult::too_complex_node(
                    format!("Double-quoted string uses `{kind}`"),
                    kind,
                ));
            }
            _ => {
                // Un-named whitespace / escape sequences inside
                // the string. tree-sitter's `string` node may
                // include `\\` escape children; treat their text
                // as part of the content.
                let txt = node_text(child, src);
                if !txt.is_empty() {
                    out.push_str(txt);
                    saw_literal_content = true;
                }
            }
        }
    }
    // Reject solo-placeholder strings (see SECURITY note above).
    if saw_dynamic_placeholder && !saw_literal_content {
        return Err(ParseForSecurityResult::too_complex_node(
            "Double-quoted string contains only a $() placeholder",
            "string",
        ));
    }
    Ok(out)
}

/// Unescape bash double-quoted string content. Inside `"..."`,
/// only `\$`, `\``, `\"`, and `\\` are escapes; every other
/// `\X` stays literal (so `"a\nb"` is six characters, not five).
/// Mirrors AliveCode's `replace(/\\([$`"\\])/g, '$1')`.
fn unescape_double_quoted(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            let next = bytes[i + 1];
            if matches!(next, b'$' | b'`' | b'"' | b'\\') {
                out.push(next as char);
                i += 2;
                continue;
            }
        }
        // Push the byte (UTF-8 safe: we only special-case ASCII
        // backslash and ASCII `$ ` ` " \\`; multi-byte chars pass
        // through one byte at a time, reconstructed by `String`'s
        // own `from_utf8_lossy`-style invariants — but since `s`
        // is already valid UTF-8 and we only consume whole ASCII
        // byte pairs above, the remaining bytes are valid).
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// Recurse into a `command_substitution` node — `$(inner)` or
/// `` `inner` `` — and append every leaf command discovered to
/// `inner_commands`. The outer caller writes `CMDSUB_PLACEHOLDER`
/// into argv; this fn only exists to surface the inner
/// SimpleCommands.
///
/// Returns `Some(err)` if the inner command is itself too-complex
/// (`$(diff <(a) <(b))` for instance — process_substitution
/// rejects). The error text propagates up unchanged so callers
/// can show the operator the offending nested node.
fn collect_command_substitution<'a>(
    node: Node<'a>,
    src: &str,
    inner: &mut Vec<SimpleCommand>,
) -> Option<ParseForSecurityResult> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let kind = child.kind();
        if matches!(kind, "$(" | "`" | ")") {
            continue;
        }
        if let Some(err) = collect_commands(child, src, inner) {
            return Some(err);
        }
    }
    None
}

// --- declaration_command --------------------------------------------------

fn walk_declaration_command<'a>(
    node: Node<'a>,
    src: &str,
    inner: &mut Vec<SimpleCommand>,
) -> Result<SimpleCommand, ParseForSecurityResult> {
    // Children: builtin name (`export`/`local`/...), then a
    // sequence of `word`s and `variable_assignment`s.
    let mut argv: Vec<String> = Vec::new();
    let mut env_vars: Vec<EnvAssignment> = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let kind = child.kind();
        match kind {
            "export" | "local" | "readonly" | "declare" | "typeset" => {
                argv.push(kind.to_string());
            }
            "word" | "number" | "string" | "raw_string" | "concatenation" => {
                argv.push(walk_argument(child, src, inner)?);
            }
            "variable_assignment" => {
                let assign = walk_variable_assignment(child, src, inner)?;
                argv.push(format!("{}={}", assign.name, assign.value));
                env_vars.push(assign);
            }
            "comment" => {}
            other => {
                if !is_skip_unnamed(other) {
                    return Err(ParseForSecurityResult::too_complex_node(
                        format!("Disallowed child of declaration_command `{other}`"),
                        other,
                    ));
                }
            }
        }
    }
    if argv.is_empty() {
        return Err(ParseForSecurityResult::too_complex_node(
            "declaration_command with no argv",
            "declaration_command",
        ));
    }
    Ok(SimpleCommand {
        argv,
        env_vars,
        redirects: Vec::new(),
        text: node_text(node, src).to_string(),
    })
}

fn is_skip_unnamed(kind: &str) -> bool {
    // tree-sitter sometimes surfaces un-named flag tokens like
    // `-p`/`-x` for declare. They appear as `word` children for
    // most cases; this helper exists to make future grammar
    // tweaks adjustable without touching the main match.
    kind.is_empty() || kind == " " || kind == "\t" || kind == "\n"
}

// --- helpers --------------------------------------------------------------

fn node_text<'a>(node: Node<'_>, src: &'a str) -> &'a str {
    node.utf8_text(src.as_bytes()).unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn must_simple(cmd: &str) -> Vec<SimpleCommand> {
        match parse_for_security(cmd) {
            ParseForSecurityResult::Simple { commands } => commands,
            other => panic!("expected Simple for {cmd:?}, got {other:?}"),
        }
    }

    fn must_too_complex(cmd: &str) -> (String, Option<String>) {
        match parse_for_security(cmd) {
            ParseForSecurityResult::TooComplex { reason, node_type } => (reason, node_type),
            other => panic!("expected TooComplex for {cmd:?}, got {other:?}"),
        }
    }

    // ---- empty / trivial -------------------------------------------------

    #[test]
    fn empty_string_is_simple_with_no_commands() {
        let cmds = must_simple("");
        assert!(cmds.is_empty());
    }

    #[test]
    fn whitespace_only_is_simple_with_no_commands() {
        let cmds = must_simple("   \t  ");
        assert!(cmds.is_empty());
    }

    #[test]
    fn comment_only_is_simple_with_no_commands() {
        let cmds = must_simple("# nothing here");
        assert!(cmds.is_empty());
    }

    // ---- simple commands -------------------------------------------------

    #[test]
    fn bare_command() {
        let cmds = must_simple("ls");
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].argv, vec!["ls"]);
        assert!(cmds[0].env_vars.is_empty());
        assert!(cmds[0].redirects.is_empty());
    }

    #[test]
    fn command_with_args() {
        let cmds = must_simple("ls -la /tmp");
        assert_eq!(cmds[0].argv, vec!["ls", "-la", "/tmp"]);
    }

    #[test]
    fn single_quoted_argument_is_unquoted() {
        let cmds = must_simple("echo 'hello world'");
        assert_eq!(cmds[0].argv, vec!["echo", "hello world"]);
    }

    #[test]
    fn double_quoted_argument_is_unquoted() {
        let cmds = must_simple(r#"echo "hello world""#);
        assert_eq!(cmds[0].argv, vec!["echo", "hello world"]);
    }

    // ---- env assignments -------------------------------------------------

    #[test]
    fn leading_env_assignment() {
        let cmds = must_simple("FOO=bar make");
        assert_eq!(cmds[0].argv, vec!["make"]);
        assert_eq!(cmds[0].env_vars.len(), 1);
        assert_eq!(cmds[0].env_vars[0].name, "FOO");
        assert_eq!(cmds[0].env_vars[0].value, "bar");
    }

    #[test]
    fn multiple_env_assignments() {
        let cmds = must_simple("A=1 B=2 echo hi");
        assert_eq!(cmds[0].argv, vec!["echo", "hi"]);
        assert_eq!(cmds[0].env_vars.len(), 2);
        assert_eq!(cmds[0].env_vars[0].name, "A");
        assert_eq!(cmds[0].env_vars[1].name, "B");
    }

    #[test]
    fn env_with_quoted_value() {
        let cmds = must_simple(r#"FOO="some thing" cmd"#);
        assert_eq!(cmds[0].env_vars[0].value, "some thing");
    }

    // ---- pipelines & lists ----------------------------------------------

    #[test]
    fn pipeline_extracts_each_stage() {
        let cmds = must_simple("ls | wc -l");
        assert_eq!(cmds.len(), 2);
        assert_eq!(cmds[0].argv, vec!["ls"]);
        assert_eq!(cmds[1].argv, vec!["wc", "-l"]);
    }

    #[test]
    fn three_stage_pipeline() {
        let cmds = must_simple("a | b | c");
        assert_eq!(cmds.len(), 3);
    }

    #[test]
    fn logical_and_list() {
        let cmds = must_simple("true && echo ok");
        assert_eq!(cmds.len(), 2);
        assert_eq!(cmds[0].argv, vec!["true"]);
        assert_eq!(cmds[1].argv, vec!["echo", "ok"]);
    }

    #[test]
    fn logical_or_list() {
        let cmds = must_simple("false || echo nope");
        assert_eq!(cmds.len(), 2);
    }

    #[test]
    fn semicolon_list() {
        let cmds = must_simple("a; b; c");
        assert_eq!(cmds.len(), 3);
    }

    #[test]
    fn negated_command_is_unwrapped() {
        let cmds = must_simple("! grep err");
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].argv, vec!["grep", "err"]);
    }

    // ---- redirections ----------------------------------------------------

    #[test]
    fn write_redirect_to_file() {
        let cmds = must_simple("echo hi > out.txt");
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].argv, vec!["echo", "hi"]);
        assert_eq!(cmds[0].redirects.len(), 1);
        assert_eq!(cmds[0].redirects[0].op, ">");
        assert_eq!(cmds[0].redirects[0].target, "out.txt");
    }

    #[test]
    fn append_redirect_to_file() {
        let cmds = must_simple("echo hi >> log");
        assert_eq!(cmds[0].redirects[0].op, ">>");
        assert_eq!(cmds[0].redirects[0].target, "log");
    }

    #[test]
    fn read_redirect_from_file() {
        let cmds = must_simple("cat < input");
        assert_eq!(cmds[0].redirects[0].op, "<");
        assert_eq!(cmds[0].redirects[0].target, "input");
    }

    #[test]
    fn redirect_with_quoted_target() {
        let cmds = must_simple(r#"echo hi > "file with spaces""#);
        assert_eq!(cmds[0].redirects[0].target, "file with spaces");
    }

    #[test]
    fn stderr_redirect_carries_fd() {
        let cmds = must_simple("cmd 2> err.log");
        assert_eq!(cmds[0].redirects[0].op, ">");
        assert_eq!(cmds[0].redirects[0].fd, Some(2));
    }

    #[test]
    fn herestring_redirect_is_too_complex() {
        let (_, nt) = must_too_complex("cat <<< hi");
        assert_eq!(nt.as_deref(), Some("herestring_redirect"));
    }

    // ---- declaration_command --------------------------------------------

    #[test]
    fn export_with_assignment() {
        let cmds = must_simple("export FOO=bar");
        assert_eq!(cmds[0].argv[0], "export");
        // declaration_command surfaces NAME=value as argv[1]
        // because that's what bash will pass to the builtin.
        assert!(cmds[0].argv.iter().any(|a| a == "FOO=bar"));
    }

    // ---- pre-checks ------------------------------------------------------

    #[test]
    fn control_char_rejected_pre_check() {
        let (reason, nt) = must_too_complex("echo hi\x07there");
        assert_eq!(nt, None);
        assert!(reason.contains("control"));
    }

    #[test]
    fn nbsp_unicode_whitespace_rejected() {
        let (reason, _) = must_too_complex("echo hi\u{00A0}there");
        assert!(reason.contains("Unicode"));
    }

    #[test]
    fn zero_width_space_rejected() {
        let (reason, _) = must_too_complex("ls\u{200B}-la");
        assert!(reason.contains("Unicode"));
    }

    #[test]
    fn backslash_space_rejected() {
        let (reason, _) = must_too_complex("cat\\ test");
        assert!(reason.contains("backslash"));
    }

    #[test]
    fn zsh_tilde_bracket_rejected() {
        let (reason, _) = must_too_complex("ls ~[name]");
        assert!(reason.contains("zsh ~["));
    }

    #[test]
    fn zsh_equals_expansion_rejected() {
        let (reason, _) = must_too_complex("=curl example.com");
        assert!(reason.contains("zsh =cmd"));
    }

    #[test]
    fn zsh_equals_after_separator_rejected() {
        let (reason, _) = must_too_complex("true && =curl x");
        assert!(reason.contains("zsh =cmd"));
    }

    #[test]
    fn equals_inside_word_is_not_zsh_expansion() {
        // `--flag=value` and `VAR=val` have `=` mid-word — must
        // NOT trigger the pre-check.
        assert!(matches!(
            parse_for_security("cmd --flag=value"),
            ParseForSecurityResult::Simple { .. }
        ));
        assert!(matches!(
            parse_for_security("FOO=bar cmd"),
            ParseForSecurityResult::Simple { .. }
        ));
    }

    // ---- fail-closed rejections -----------------------------------------

    #[test]
    fn command_substitution_rejected() {
        let (_, nt) = must_too_complex("echo $(date)");
        assert_eq!(nt.as_deref(), Some("command_substitution"));
    }

    #[test]
    fn simple_expansion_rejected() {
        let (_, nt) = must_too_complex("echo $HOME");
        assert_eq!(nt.as_deref(), Some("simple_expansion"));
    }

    #[test]
    fn parameter_expansion_rejected() {
        let (_, nt) = must_too_complex("echo ${HOME}");
        assert_eq!(nt.as_deref(), Some("expansion"));
    }

    #[test]
    fn process_substitution_rejected() {
        let (_, nt) = must_too_complex("diff <(a) <(b)");
        assert_eq!(nt.as_deref(), Some("process_substitution"));
    }

    #[test]
    fn arithmetic_expansion_rejected() {
        let (_, nt) = must_too_complex("echo $((1+1))");
        assert_eq!(nt.as_deref(), Some("arithmetic_expansion"));
    }

    #[test]
    fn subshell_rejected() {
        let (_, nt) = must_too_complex("(echo hi)");
        assert_eq!(nt.as_deref(), Some("subshell"));
    }

    #[test]
    fn for_loop_rejected() {
        let (_, nt) = must_too_complex("for x in a b; do echo $x; done");
        // Could be `for_statement` or fall through earlier on $x.
        assert!(nt.is_some());
    }

    #[test]
    fn if_statement_rejected() {
        let (_, nt) = must_too_complex("if true; then echo a; fi");
        assert_eq!(nt.as_deref(), Some("if_statement"));
    }

    #[test]
    fn function_definition_rejected() {
        let (_, nt) = must_too_complex("foo() { echo hi; }");
        assert_eq!(nt.as_deref(), Some("function_definition"));
    }

    #[test]
    fn parse_error_is_too_complex_with_error_node() {
        // Unbalanced quote → tree-sitter ERROR node.
        let (_, nt) = must_too_complex(r#"echo "unterminated"#);
        assert_eq!(nt.as_deref(), Some("ERROR"));
    }

    // ---- text span ------------------------------------------------------

    #[test]
    fn simple_command_text_span_is_whole_command() {
        let cmds = must_simple("ls -la");
        assert_eq!(cmds[0].text, "ls -la");
    }

    #[test]
    fn redirected_command_text_includes_redirect() {
        let cmds = must_simple("echo hi > out.txt");
        assert_eq!(cmds[0].text, "echo hi > out.txt");
    }

    // ---- $() inside double-quoted strings (slice 4a) -------------------

    #[test]
    fn cmdsub_inside_string_with_literal_extracts_inner() {
        // Outer command + inner substitution both extracted; outer
        // argv carries the placeholder concatenated with the prefix.
        let cmds = must_simple(r#"echo "SHA: $(git rev-parse HEAD)""#);
        assert_eq!(cmds.len(), 2, "outer + inner");
        // Inner command pushed first (depth-first), outer pushed
        // after it returns.
        assert_eq!(cmds[0].argv, vec!["git", "rev-parse", "HEAD"]);
        assert_eq!(cmds[1].argv[0], "echo");
        assert!(
            cmds[1].argv[1].contains(CMDSUB_PLACEHOLDER),
            "outer argv must contain the placeholder, got {:?}",
            cmds[1].argv[1]
        );
        assert!(cmds[1].argv[1].starts_with("SHA: "));
    }

    #[test]
    fn solo_cmdsub_string_is_too_complex() {
        // `cd "$(echo /etc)"` — placeholder alone in argv would
        // bypass path validation. Reject.
        let (_, nt) = must_too_complex(r#"cd "$(echo /etc)""#);
        assert_eq!(nt.as_deref(), Some("string"));
    }

    #[test]
    fn nested_cmdsub_too_complex_propagates_inner_node() {
        // `$()` inside `$()` inside a string. The inner-inner is
        // process_substitution, which is rejected.
        let (_, nt) = must_too_complex(r#"echo "x: $(diff <(a) <(b))""#);
        assert_eq!(nt.as_deref(), Some("process_substitution"));
    }

    #[test]
    fn cmdsub_with_pipeline_inner_extracts_each_stage() {
        let cmds = must_simple(r#"echo "result: $(ls | wc -l)""#);
        // ls, wc, echo
        assert_eq!(cmds.len(), 3);
        assert_eq!(cmds[0].argv, vec!["ls"]);
        assert_eq!(cmds[1].argv, vec!["wc", "-l"]);
        assert_eq!(cmds[2].argv[0], "echo");
    }

    #[test]
    fn bare_cmdsub_at_arg_position_still_rejects() {
        // `rm $(echo /tmp/a)` — placeholder would BE the path arg.
        // AliveCode policy: bare $() at arg position rejects even
        // though inside-string $() recurses.
        let (_, nt) = must_too_complex("rm $(echo /tmp/a)");
        assert_eq!(nt.as_deref(), Some("command_substitution"));
    }

    #[test]
    fn double_quoted_escape_sequences_unescaped() {
        // `\$`, `\"`, `\\`, `\`` are escape sequences inside "...".
        let cmds = must_simple(r#"echo "fix \"bug\"""#);
        assert_eq!(cmds[0].argv[1], r#"fix "bug""#);
    }

    #[test]
    fn double_quoted_other_backslash_kept_literal() {
        // `\n` inside "..." is two characters in bash (backslash + n),
        // NOT a newline. Mirror that exactly.
        let cmds = must_simple(r#"echo "a\nb""#);
        assert_eq!(cmds[0].argv[1], r"a\nb");
    }

    #[test]
    fn cmdsub_placeholder_constant_value() {
        // Sanity: CMDSUB_PLACEHOLDER constant must match the
        // string AliveCode uses, so cross-impl audit events
        // correlate.
        assert_eq!(CMDSUB_PLACEHOLDER, "__CMDSUB_OUTPUT__");
    }
}
