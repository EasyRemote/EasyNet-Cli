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

use std::collections::HashMap;

use tree_sitter::Node;

use super::parser::{parse_root, ParseError};
use super::{EnvAssignment, ParseForSecurityResult, Redirect, SimpleCommand};

/// Per-traversal variable scope. Maps a variable name to its
/// stored value (a literal like `"/tmp"` or the
/// [`VAR_PLACEHOLDER`] sentinel for runtime-unknown).
///
/// Scope is mutated linearly along `&&` and `;` chains and
/// inside the value of a `command` node (so `A=1 B=$A cmd`
/// resolves `$A` to `"1"`). It is reset at `||`/`|`/`|&`/`&`
/// boundaries to model bash's actual semantics there:
///
///   * `||` RHS runs only when LHS fails → vars set on LHS
///     may not be set when RHS executes.
///   * `|` and `|&` stages run in subshells → vars set inside
///     a stage are NEVER visible to later stages.
///   * `&` LHS runs in a background subshell → ditto.
///
/// Linear scope crossing those separators would let a
/// flag-omission attack pass:
/// `true || FLAG=--dry-run && cmd $FLAG` — bash skips the
/// `||` RHS so `cmd` runs WITHOUT `--dry-run`. With linear
/// scope, our argv would carry `["cmd","--dry-run"]` (looks
/// safe). The pre-scan + reset in [`collect_commands`]
/// prevents that.
type VarScope = HashMap<String, String>;

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
    let mut scope: VarScope = HashMap::new();
    if let Some(err) = collect_commands(root, cmd, &mut commands, &mut scope) {
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

/// Placeholder for `$VAR` references where the variable's value
/// is runtime-unknown — set from a `$()` substitution, from a
/// SAFE_ENV_VARS reference (`$HOME`, `$PWD`, …), from a special
/// variable (`$?`, `$$`, `$1`, …), or from a tracked composite
/// like `VAR="prefix$(cmd)"`.
///
/// `__TRACKED_VAR__` only flows into argv positions that are
/// *inside* a double-quoted string. As a bare argument, the
/// resolver returns too_complex instead — bare-arg word-splitting
/// would change argv shape at runtime in ways that static
/// permission checks cannot model.
///
/// Same string AliveCode emits, again so cross-impl audit
/// events match.
pub const VAR_PLACEHOLDER: &str = "__TRACKED_VAR__";

/// Returns true when `value` carries either placeholder string,
/// either standalone or embedded (`"prefix__CMDSUB_OUTPUT__"`).
/// Used by [`apply_assignment_to_scope`] to mark a composite
/// value as runtime-non-literal — a future `$VAR` deref of that
/// scope entry must reject as a bare arg.
fn contains_any_placeholder(value: &str) -> bool {
    value.contains(CMDSUB_PLACEHOLDER) || value.contains(VAR_PLACEHOLDER)
}

/// Variables that bash sets automatically; their values are
/// shell/OS-controlled, not arbitrary user input. Referencing
/// them via `$VAR` is safe inside double-quoted strings — the
/// expansion is deterministic. As a bare argument they remain
/// rejected, since `$HOME` could still be a path the caller
/// shouldn't be reading without an explicit allow.
const SAFE_ENV_VARS: &[&str] = &[
    "HOME",
    "PWD",
    "OLDPWD",
    "USER",
    "LOGNAME",
    "SHELL",
    "PATH",
    "HOSTNAME",
    "UID",
    "EUID",
    "PPID",
    "RANDOM",
    "SECONDS",
    "LINENO",
    "TMPDIR",
    "BASH_VERSION",
    "BASHPID",
    "SHLVL",
    "HISTFILE",
    "IFS",
];

/// Bash special variable names — `$?`, `$$`, `$!`, `$#`, `$0`,
/// `$-`. Not numeric positional ($1..$9) — those match a
/// numeric-only check separately. Same caveat as SAFE_ENV_VARS:
/// safe inside strings, rejected as bare args.
///
/// `@` and `*` are deliberately excluded. Inside `"..."` they
/// expand to the positional params, which are EMPTY in any
/// shell shellguard would spawn — returning VAR_PLACEHOLDER
/// would lie to permission rules. Static analysis can't reason
/// about them, so they fall through to too_complex.
const SPECIAL_VAR_NAMES: &[&str] = &["?", "$", "!", "#", "0", "-"];

/// Characters that change argv shape at runtime when an unquoted
/// `$VAR` expands to a value containing them: bash's default IFS
/// (space / tab / newline) word-splits, and `*`, `?`, `[` glob.
/// `VAR="-rf /" && rm $VAR` → bash runs `rm -rf /` (two args);
/// `VAR="/etc/*" && cat $VAR` → bash expands to every /etc file.
///
/// Inside `"$VAR"`, neither splitting nor globbing applies, so
/// the resolver only checks this regex when `inside_string` is
/// false. Mirrors AliveCode's `BARE_VAR_UNSAFE_RE`.
fn bare_var_unsafe(value: &str) -> bool {
    value
        .chars()
        .any(|c| matches!(c, ' ' | '\t' | '\n' | '*' | '?' | '['))
}

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
    if has_brace_with_quote(cmd) {
        return Some("Contains brace with quote character (expansion obfuscation)");
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

/// Brace expansion combined with a quote character inside. Bash
/// constructions like `{a'}',b}` use quoted braces inside brace
/// expansion context to obfuscate the expansion from regex-based
/// detection. They expand at runtime to non-trivial token sets
/// and have no legitimate use in commands a receiver would want
/// to auto-allow. This detector runs against a quote-masked copy
/// of the command (so JSON like `'{"k":"v"}'` does NOT trigger),
/// then looks for `{...quote...}` patterns where the opening `{`
/// is at unquoted position.
///
/// Two-step:
///   1. mask_braces_in_quoted_contexts replaces `{` characters
///      that appear inside single- or double-quoted spans with
///      a space, leaving the surrounding quote chars in place.
///   2. Search the masked string for `\{[^}]*['"]` — an opening
///      brace, any non-`}` chars, then a quote. Brace expansion
///      is impossible inside any quote in bash, so a `{` that
///      reaches a quote within the same `{...}` span is the
///      tell.
fn has_brace_with_quote(cmd: &str) -> bool {
    if !cmd.contains('{') {
        return false;
    }
    let masked = mask_braces_in_quoted_contexts(cmd);
    let bytes = masked.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'{' {
            i += 1;
            continue;
        }
        // Scan from `{` looking for a quote before the matching `}`.
        let mut j = i + 1;
        while j < bytes.len() && bytes[j] != b'}' {
            if bytes[j] == b'\'' || bytes[j] == b'"' {
                return true;
            }
            j += 1;
        }
        i = j + 1;
    }
    false
}

/// Replace every `{` byte inside a single- or double-quoted span
/// with a space, leaving every other character (including the
/// surrounding quote chars) untouched. Quote-state scanner so
/// JSON payloads like `curl -d '{"k":"v"}'` don't false-trigger
/// the brace-with-quote check while still letting truly unquoted
/// `{a'}',b}` patterns reach it. Mirrors AliveCode's
/// `maskBracesInQuotedContexts`.
fn mask_braces_in_quoted_contexts(cmd: &str) -> String {
    if !cmd.contains('{') {
        return cmd.to_string();
    }
    let mut out = String::with_capacity(cmd.len());
    let bytes = cmd.as_bytes();
    let mut in_single = false;
    let mut in_double = false;
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if in_single {
            // Bash single-quote: no escapes, `'` always closes.
            if c == b'\'' {
                in_single = false;
            }
            out.push(if c == b'{' { ' ' } else { c as char });
            i += 1;
        } else if in_double {
            // Bash double-quote: `\` escapes `"` and `\\`.
            if c == b'\\' && i + 1 < bytes.len() {
                let next = bytes[i + 1];
                if next == b'"' || next == b'\\' {
                    out.push(c as char);
                    out.push(next as char);
                    i += 2;
                    continue;
                }
            }
            if c == b'"' {
                in_double = false;
            }
            out.push(if c == b'{' { ' ' } else { c as char });
            i += 1;
        } else {
            // Unquoted: `\` escapes any single next char.
            if c == b'\\' && i + 1 < bytes.len() {
                out.push(c as char);
                out.push(bytes[i + 1] as char);
                i += 2;
                continue;
            }
            if c == b'\'' {
                in_single = true;
            } else if c == b'"' {
                in_double = true;
            }
            out.push(c as char);
            i += 1;
        }
    }
    out
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
    scope: &mut VarScope,
) -> Option<ParseForSecurityResult> {
    match node.kind() {
        // Empty / structural roots — recurse with scope discipline.
        // See VarScope doc-comment for the linear-vs-reset rationale.
        "program" | "list" | "pipeline" => {
            let is_pipeline = node.kind() == "pipeline";
            // Pre-scan: any `||`/`&` separators that should reset
            // scope? `|`/`|&` only appear under pipeline (where
            // we already fork). For list/program we walk children
            // looking for the linear-break tokens.
            let mut needs_snapshot = false;
            if !is_pipeline {
                let mut c = node.walk();
                for ch in node.children(&mut c) {
                    if matches!(ch.kind(), "||" | "&") {
                        needs_snapshot = true;
                        break;
                    }
                }
            }
            // For pipelines, every stage runs in its own subshell —
            // start with a fork of the entry scope so inner
            // assignments don't leak. For list/program, the
            // `&&`/`;` chain mutates the caller's scope (sequential
            // semantics); reset only at `||`/`&` boundaries.
            let entry: VarScope = scope.clone();
            let snapshot: VarScope = if needs_snapshot {
                entry.clone()
            } else {
                VarScope::new()
            };
            let mut local: VarScope = if is_pipeline {
                entry.clone()
            } else {
                VarScope::new()
            };

            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                let kind = child.kind();
                if SEPARATORS.contains(&kind) {
                    if matches!(kind, "||" | "|" | "|&" | "&") {
                        if is_pipeline {
                            local = entry.clone();
                        } else {
                            *scope = snapshot.clone();
                        }
                    }
                    continue;
                }
                let active: &mut VarScope = if is_pipeline { &mut local } else { scope };
                if let Some(err) = collect_commands(child, src, commands, active) {
                    return Some(err);
                }
            }
            None
        }

        "redirected_statement" => match walk_redirected_statement(node, src, commands, scope) {
            Ok(cmd) => {
                commands.push(cmd);
                None
            }
            Err(err) => Some(err),
        },

        "command" => match walk_command(node, src, commands, scope) {
            Ok(cmd) => {
                commands.push(cmd);
                None
            }
            Err(err) => Some(err),
        },

        "declaration_command" => match walk_declaration_command(node, src, commands, scope) {
            Ok(cmd) => {
                commands.push(cmd);
                None
            }
            Err(err) => Some(err),
        },

        "negated_command" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "!" {
                    continue;
                }
                if let Some(err) = collect_commands(child, src, commands, scope) {
                    return Some(err);
                }
            }
            None
        }

        // A bare `VAR=value` statement (no following command) is
        // valid bash and just sets the variable in the current
        // shell. tree-sitter emits it as a top-level
        // variable_assignment under `program` / `list`. We
        // mutate scope but emit no SimpleCommand.
        "variable_assignment" => match walk_variable_assignment(node, src, commands, scope) {
            Ok(assign) => {
                apply_assignment_to_scope(scope, &assign);
                None
            }
            Err(err) => Some(err),
        },

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
    scope: &mut VarScope,
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
                inner_cmd = Some(walk_command(child, src, inner, scope)?);
            }
            "declaration_command" => {
                if inner_cmd.is_some() {
                    return Err(ParseForSecurityResult::too_complex_node(
                        "redirected_statement with multiple inner commands",
                        "redirected_statement",
                    ));
                }
                inner_cmd = Some(walk_declaration_command(child, src, inner, scope)?);
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

fn parse_file_redirect<'a>(node: Node<'a>, src: &str) -> Result<Redirect, ParseForSecurityResult> {
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
    scope: &mut VarScope,
) -> Result<SimpleCommand, ParseForSecurityResult> {
    let mut argv: Vec<String> = Vec::new();
    let mut env_vars: Vec<EnvAssignment> = Vec::new();
    // A `command` node's leading `variable_assignment`s create
    // env vars for THIS command and only this command — bash's
    // `A=1 B=2 cmd` semantics. They flow into `env_vars` AND are
    // visible to expansions in subsequent argv tokens of the
    // same command (`A=1 echo "$A"`). We therefore track them in
    // a per-command scope layered over the chain scope.
    let mut local_scope: VarScope = scope.clone();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "variable_assignment" => {
                let assign = walk_variable_assignment(child, src, inner, &mut local_scope)?;
                apply_assignment_to_scope(&mut local_scope, &assign);
                env_vars.push(assign);
            }
            "command_name" => {
                let mut nc = child.walk();
                let mut pushed = false;
                for n in child.named_children(&mut nc) {
                    let arg = walk_argument(n, src, inner, &mut local_scope)?;
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
            "word" | "string" | "raw_string" | "number" | "concatenation" | "simple_expansion" => {
                argv.push(walk_argument(child, src, inner, &mut local_scope)?);
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
    scope: &mut VarScope,
) -> Result<EnvAssignment, ParseForSecurityResult> {
    // variable_assignment has children: `variable_name` `=` then
    // a value node (word / string / raw_string / concatenation /
    // command_substitution / simple_expansion).
    //
    // Bash assignment RHS does NOT word-split or glob-expand, so
    // a value containing those characters is allowed at storage
    // time — the bare-arg unsafe check applies later, when the
    // variable is *dereferenced* as an unquoted argv token.
    let mut name: Option<String> = None;
    let mut value: Option<String> = None;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let kind = child.kind();
        match kind {
            "variable_name" => name = Some(node_text(child, src).to_string()),
            "=" => {}
            "word" | "string" | "raw_string" | "number" | "concatenation" => {
                value = Some(walk_argument(child, src, inner, scope)?);
            }
            "command_substitution" => {
                // RHS $() — recurse for inner commands, store the
                // placeholder as the value. apply_assignment_to_scope
                // collapses placeholder-bearing values to
                // VAR_PLACEHOLDER so a future bare $VAR rejects.
                if let Some(err) = collect_command_substitution(child, src, inner, scope) {
                    return Err(err);
                }
                value = Some(CMDSUB_PLACEHOLDER.to_string());
            }
            "simple_expansion" => {
                // RHS $OTHER — assignment context (no word-split).
                // resolve_simple_expansion(true) returns the literal
                // value if OTHER is a tracked literal, VAR_PLACEHOLDER
                // if OTHER is dynamic. Either way, valid storage.
                let v = resolve_simple_expansion(child, src, scope, true)?;
                value = Some(v);
            }
            "expansion" | "process_substitution" | "arithmetic_expansion" => {
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
    // SECURITY: tree-sitter-bash accepts invalid names (e.g.
    // `1VAR=value`) as variable_assignment. Bash itself only
    // recognises `[A-Za-z_][A-Za-z0-9_]*` — anything else is
    // executed as a COMMAND. `1VAR=value` → bash tries to run
    // `1VAR=value` from PATH. We must not treat it as an inert
    // assignment.
    if !is_valid_var_name(&name) {
        return Err(ParseForSecurityResult::too_complex_node(
            format!("Invalid variable name (bash treats as command): {name}"),
            "variable_assignment",
        ));
    }
    // SECURITY: Setting IFS changes word-splitting behaviour for
    // every subsequent unquoted $VAR expansion. Our bare-var
    // unsafe check only models default IFS (space/tab/newline) —
    // a custom IFS would let `IFS=: && VAR=a:b && rm $VAR` slip
    // through as ['rm','a:b'] while bash actually runs `rm a b`.
    if name == "IFS" {
        return Err(ParseForSecurityResult::too_complex_node(
            "IFS assignment changes word-splitting — cannot model statically",
            "variable_assignment",
        ));
    }
    // Empty value is legal (`VAR=`).
    let value = value.unwrap_or_default();
    Ok(EnvAssignment { name, value })
}

fn is_valid_var_name(s: &str) -> bool {
    let mut iter = s.chars();
    match iter.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    iter.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Apply `assign` to `scope`, collapsing the value to
/// [`VAR_PLACEHOLDER`] when it carries any placeholder embedded
/// (so a future `$VAR` deref correctly recognises the value as
/// runtime-non-literal).
///
/// This function does NOT model `+=`. tree-sitter exposes `+=`
/// as a distinct operator, but slice 4b deliberately rejects
/// `+=` upstream (in walk_variable_assignment, future hardening)
/// — modelling `+=` correctly across `||` / pipeline scope
/// resets is structurally fragile, and AliveCode's PS4 hardening
/// took five rounds of bypass patches before it stabilised.
/// For now any `+=` shows up as the literal token "+=" in the
/// match and falls through to too_complex.
fn apply_assignment_to_scope(scope: &mut VarScope, assign: &EnvAssignment) {
    let value = if contains_any_placeholder(&assign.value) {
        VAR_PLACEHOLDER.to_string()
    } else {
        assign.value.clone()
    };
    scope.insert(assign.name.clone(), value);
}

fn walk_argument<'a>(
    node: Node<'a>,
    src: &str,
    inner: &mut Vec<SimpleCommand>,
    scope: &mut VarScope,
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
        "string" => walk_double_string(node, src, inner, scope),
        "simple_expansion" => {
            // BARE $VAR at arg position. Tracked literal returns
            // the value (passing the bare-var unsafe filter);
            // tracked dynamic, SAFE_ENV_VARS, special vars all
            // reject — they'd hide a runtime path/flag from
            // permission matching.
            resolve_simple_expansion(node, src, scope, false)
        }
        "concatenation" => {
            // Concatenation: every part walked in the SAME
            // bare-arg context (the whole concat IS the argument).
            // `prefix$VAR` resolves $VAR with inside_string=false.
            let mut out = String::new();
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                out.push_str(&walk_argument(child, src, inner, scope)?);
            }
            Ok(out)
        }
        // BARE command_substitution at arg position is intentionally
        // rejected — the output IS the argument and could be a
        // path/flag (`rm $(echo /etc)`). Only `$()` *inside* a
        // double-quoted string is extracted (walk_double_string
        // gates on the saw_dynamic_placeholder + saw_literal_content
        // invariant).
        "command_substitution"
        | "expansion"
        | "process_substitution"
        | "arithmetic_expansion"
        | "subshell"
        | "brace_expression"
        | "ansi_c_string"
        | "translated_string" => Err(ParseForSecurityResult::too_complex_node(
            format!("Argument uses `{}`", node.kind()),
            node.kind(),
        )),
        other => Err(ParseForSecurityResult::too_complex_node(
            format!("Argument uses unsupported node `{other}`"),
            other,
        )),
    }
}

/// Resolve a `simple_expansion` node (`$VAR` or `$1`/`$?` etc.)
/// using `scope`. `inside_string` distinguishes bare-arg
/// (`rm $V`) from in-string (`echo "x: $V"`) since bash's
/// word-splitting and glob-expansion only apply to the bare-arg
/// form.
///
/// Resolution order:
///
///   1. If `scope` has a literal value, return it directly when
///      `inside_string` OR when the literal passes the bare-arg
///      unsafe filter; otherwise reject (bare `$VAR` with a
///      glob/IFS-bearing literal would change argv shape at
///      runtime).
///   2. If `scope` has a [`VAR_PLACEHOLDER`] entry, return the
///      placeholder when `inside_string`; reject as bare arg.
///   3. SAFE_ENV_VARS / SPECIAL_VAR_NAMES / numeric positional
///      vars: return [`VAR_PLACEHOLDER`] when `inside_string`;
///      reject as bare arg (we can't model the actual value).
///   4. Otherwise reject — untracked variable.
fn resolve_simple_expansion<'a>(
    node: Node<'a>,
    src: &str,
    scope: &VarScope,
    inside_string: bool,
) -> Result<String, ParseForSecurityResult> {
    let mut cursor = node.walk();
    let mut var_name: Option<&str> = None;
    let mut is_special = false;
    for child in node.children(&mut cursor) {
        match child.kind() {
            "variable_name" => {
                var_name = Some(node_text(child, src));
                break;
            }
            "special_variable_name" => {
                var_name = Some(node_text(child, src));
                is_special = true;
                break;
            }
            _ => {}
        }
    }
    let name = var_name.ok_or_else(|| {
        ParseForSecurityResult::too_complex_node(
            "simple_expansion with no variable name",
            "simple_expansion",
        )
    })?;
    if let Some(value) = scope.get(name) {
        if value == VAR_PLACEHOLDER || contains_any_placeholder(value) {
            if inside_string {
                return Ok(VAR_PLACEHOLDER.to_string());
            }
            return Err(ParseForSecurityResult::too_complex_node(
                format!("Bare ${name} resolves to a runtime-unknown value"),
                "simple_expansion",
            ));
        }
        // Literal value.
        if !inside_string {
            if value.is_empty() {
                return Err(ParseForSecurityResult::too_complex_node(
                    format!("Bare ${name} resolves to empty (would shift positional args)"),
                    "simple_expansion",
                ));
            }
            if bare_var_unsafe(value) {
                return Err(ParseForSecurityResult::too_complex_node(
                    format!("Bare ${name} resolves to value containing IFS or glob char"),
                    "simple_expansion",
                ));
            }
        }
        return Ok(value.clone());
    }
    if inside_string {
        if SAFE_ENV_VARS.iter().any(|&n| n == name) {
            return Ok(VAR_PLACEHOLDER.to_string());
        }
        if is_special
            && (SPECIAL_VAR_NAMES.iter().any(|&n| n == name)
                || name.chars().all(|c| c.is_ascii_digit()))
        {
            return Ok(VAR_PLACEHOLDER.to_string());
        }
    }
    Err(ParseForSecurityResult::too_complex_node(
        format!("Untracked variable ${name}"),
        "simple_expansion",
    ))
}

fn walk_double_string<'a>(
    node: Node<'a>,
    src: &str,
    inner: &mut Vec<SimpleCommand>,
    scope: &mut VarScope,
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
                let err = collect_command_substitution(child, src, inner, scope);
                if let Some(e) = err {
                    return Err(e);
                }
                out.push_str(CMDSUB_PLACEHOLDER);
                saw_dynamic_placeholder = true;
            }
            "simple_expansion" => {
                // $VAR inside "...". Tracked literal → real value;
                // tracked dynamic, SAFE_ENV_VARS, special vars →
                // VAR_PLACEHOLDER; untracked → reject.
                let v = resolve_simple_expansion(child, src, scope, true)?;
                if v == VAR_PLACEHOLDER {
                    saw_dynamic_placeholder = true;
                } else {
                    saw_literal_content = true;
                }
                out.push_str(&v);
            }
            "expansion" | "process_substitution" | "arithmetic_expansion" => {
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
    scope: &VarScope,
) -> Option<ParseForSecurityResult> {
    // Vars set BEFORE the $() are visible inside (bash subshell
    // semantics), but vars set INSIDE don't leak out. Walk the
    // inner statement(s) on a COPY of the outer scope so inner
    // assignments don't pollute the caller's chain.
    let mut inner_scope: VarScope = scope.clone();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let kind = child.kind();
        if matches!(kind, "$(" | "`" | ")") {
            continue;
        }
        if let Some(err) = collect_commands(child, src, inner, &mut inner_scope) {
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
    scope: &mut VarScope,
) -> Result<SimpleCommand, ParseForSecurityResult> {
    // Children: builtin name (`export`/`local`/...), then a
    // sequence of `word`s and `variable_assignment`s.
    //
    // declaration_command DOES mutate the chain scope — `export
    // FOO=bar` makes FOO visible to subsequent commands in the
    // same chain. Mirror walk_command's local_scope layering so
    // the side-effect propagates correctly.
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
                argv.push(walk_argument(child, src, inner, scope)?);
            }
            "variable_assignment" => {
                let assign = walk_variable_assignment(child, src, inner, scope)?;
                apply_assignment_to_scope(scope, &assign);
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

    // ---- brace-with-quote pre-check (slice 4c) -------------------------

    #[test]
    fn brace_with_single_quote_inside_rejects() {
        // Classic obfuscation: `{a'}',b}` — bash expands to
        // `a} b`. tree-sitter parses inconsistently; the pre-
        // check catches it.
        let (reason, _) = must_too_complex("echo {a'}',b}");
        assert!(reason.contains("brace with quote"));
    }

    #[test]
    fn brace_with_double_quote_inside_rejects() {
        let (reason, _) = must_too_complex(r#"echo {a"}",b}"#);
        assert!(reason.contains("brace with quote"));
    }

    #[test]
    fn json_payload_in_single_quotes_does_not_false_trigger() {
        // The brace `{` is inside a single-quoted span; the
        // masker hides it from the detector. The command still
        // hits Simple-or-TooComplex on other grounds (curl is
        // fine), but it must NOT be rejected for brace-with-quote.
        let r = parse_for_security(r#"curl -d '{"k":"v"}'"#);
        match r {
            ParseForSecurityResult::Simple { .. } => {}
            ParseForSecurityResult::TooComplex { reason, .. } => {
                assert!(
                    !reason.contains("brace with quote"),
                    "JSON in single quotes should not trip brace-with-quote: {reason}"
                );
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn json_payload_in_double_quotes_does_not_false_trigger() {
        let r = parse_for_security(r#"curl -d "{\"k\":\"v\"}""#);
        if let ParseForSecurityResult::TooComplex { reason, .. } = &r {
            assert!(
                !reason.contains("brace with quote"),
                "JSON in double quotes should not trip brace-with-quote: {reason}"
            );
        }
    }

    #[test]
    fn no_brace_no_check() {
        // Fast path: no `{` in the command at all.
        let cmds = must_simple("ls -la");
        assert_eq!(cmds[0].argv, vec!["ls", "-la"]);
    }

    #[test]
    fn legitimate_brace_expansion_passes_pre_check_but_walker_rejects() {
        // `{a,b,c}` has no quote chars — pre-check passes. The
        // tree-sitter walker rejects via brace_expression /
        // word-with-brace later. Either way, not a brace-with-
        // quote rejection.
        let r = parse_for_security("echo {a,b,c}");
        if let ParseForSecurityResult::TooComplex { reason, .. } = &r {
            assert!(!reason.contains("brace with quote"));
        }
    }

    // ---- variable scope tracking (slice 4b) ---------------------------

    #[test]
    fn tracked_var_resolves_to_literal_value_as_bare_arg() {
        // `VAR=/tmp && rm $VAR` — bare `VAR=/tmp` mutates scope
        // but emits no SimpleCommand (no actual program to run);
        // only `rm $VAR` reaches commands[]. Argv must carry the
        // real path so downstream path validation sees /tmp.
        let cmds = must_simple("VAR=/tmp && rm $VAR");
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].argv, vec!["rm", "/tmp"]);
    }

    #[test]
    fn tracked_var_resolves_in_string_argument() {
        let cmds = must_simple(r#"VAR=/tmp && echo "path: $VAR""#);
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].argv, vec!["echo", "path: /tmp"]);
    }

    #[test]
    fn tracked_var_with_glob_value_rejects_as_bare_arg() {
        // `VAR="/etc/*" && cat $VAR` — bash glob-expands at
        // runtime to every /etc file. Argv differential — reject.
        let (_, nt) = must_too_complex(r#"VAR="/etc/*" && cat $VAR"#);
        assert_eq!(nt.as_deref(), Some("simple_expansion"));
    }

    #[test]
    fn tracked_var_with_space_value_rejects_as_bare_arg() {
        // `VAR="-rf /" && rm $VAR` — bash word-splits to two args.
        let (_, nt) = must_too_complex(r#"VAR="-rf /" && rm $VAR"#);
        assert_eq!(nt.as_deref(), Some("simple_expansion"));
    }

    #[test]
    fn tracked_var_empty_value_rejects_as_bare_arg() {
        // `V="" && ls $V /etc` — bash drops the expansion (zero
        // fields). We'd carry a phantom "" in argv, shifting
        // positions. Reject.
        let (_, nt) = must_too_complex(r#"V="" && ls $V /etc"#);
        assert_eq!(nt.as_deref(), Some("simple_expansion"));
    }

    #[test]
    fn tracked_dynamic_var_rejects_as_bare_arg() {
        // `VAR=$(date) && rm $VAR` — VAR holds the cmdsub output,
        // unknown at static analysis. Bare arg → reject.
        let (_, nt) = must_too_complex("VAR=$(date) && rm $VAR");
        assert_eq!(nt.as_deref(), Some("simple_expansion"));
    }

    #[test]
    fn tracked_dynamic_var_resolves_to_placeholder_in_string() {
        // `VAR=$(date) && echo "now: $VAR"` — VAR_PLACEHOLDER
        // mixed with literal "now: " is a safe in-string use.
        // Inner $(date) and outer echo both extracted.
        let cmds = must_simple(r#"VAR=$(date) && echo "now: $VAR""#);
        assert!(cmds.iter().any(|c| c.argv == vec!["date"]));
        assert!(cmds.iter().any(|c| {
            c.argv.len() == 2 && c.argv[0] == "echo" && c.argv[1].starts_with("now: ")
        }));
    }

    #[test]
    fn untracked_var_rejects() {
        // No assignment in the same chain → reject.
        let (_, nt) = must_too_complex("rm $UNDEFINED");
        assert_eq!(nt.as_deref(), Some("simple_expansion"));
    }

    #[test]
    fn safe_env_var_in_string_resolves_to_placeholder() {
        // `$HOME` inside "..." is allowed (resolves to placeholder).
        // Outer string still has literal content so the solo-
        // placeholder gate doesn't reject.
        let cmds = must_simple(r#"echo "home: $HOME""#);
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].argv[0], "echo");
        assert!(cmds[0].argv[1].contains(VAR_PLACEHOLDER));
    }

    #[test]
    fn safe_env_var_as_bare_arg_rejects() {
        // `$HOME` as bare arg — could be any path. Reject.
        let (_, nt) = must_too_complex("rm $HOME");
        assert_eq!(nt.as_deref(), Some("simple_expansion"));
    }

    #[test]
    fn special_var_in_string_resolves_to_placeholder() {
        let cmds = must_simple(r#"echo "exit: $?""#);
        assert!(cmds[0].argv[1].contains(VAR_PLACEHOLDER));
    }

    #[test]
    fn ifs_assignment_rejects() {
        // IFS changes word-splitting — cannot model statically.
        let (_, nt) = must_too_complex("IFS=: && VAR=a:b && rm $VAR");
        assert_eq!(nt.as_deref(), Some("variable_assignment"));
    }

    #[test]
    fn invalid_var_name_rejects() {
        // tree-sitter accepts `1VAR=val` as variable_assignment;
        // bash actually treats it as a command. Reject.
        let (_, nt) = must_too_complex("1VAR=val");
        assert_eq!(nt.as_deref(), Some("variable_assignment"));
    }

    #[test]
    fn vars_set_before_or_do_not_leak_after_it() {
        // `FLAG=safe || rm $FLAG` — bash semantics: LHS is the
        // assignment (always succeeds, exit 0), RHS is skipped.
        // BUT if a future variant short-circuits the other way,
        // RHS may run with FLAG unset. Conservative: vars set
        // BEFORE `||` are NOT carried into the RHS segment.
        // Our reset-on-`||` makes the post-`||` scope a snapshot
        // of the entry scope (which has no FLAG), so `$FLAG`
        // is untracked → reject.
        let (_, nt) = must_too_complex("FLAG=safe || rm $FLAG");
        assert_eq!(nt.as_deref(), Some("simple_expansion"));
    }

    #[test]
    fn vars_do_not_leak_across_pipe_stages() {
        // Pipeline stages run in subshells — vars set in stage
        // 1 are NEVER visible in stage 2.
        let (_, nt) = must_too_complex("VAR=/tmp | rm $VAR");
        assert_eq!(nt.as_deref(), Some("simple_expansion"));
    }

    #[test]
    fn per_command_env_visible_in_same_command() {
        // `A=1 echo "$A"` — the leading assignment IS visible to
        // the same command's argv expansions.
        let cmds = must_simple(r#"A=1 echo "$A""#);
        assert_eq!(cmds[0].argv, vec!["echo", "1"]);
    }

    #[test]
    fn semicolon_chain_carries_scope() {
        let cmds = must_simple("VAR=/tmp; rm $VAR");
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].argv, vec!["rm", "/tmp"]);
    }

    #[test]
    fn ampersand_logical_separator_resets_scope() {
        // `A=1 & cmd $A` — `&` puts LHS in background subshell.
        let (_, nt) = must_too_complex("A=1 & cmd $A");
        assert_eq!(nt.as_deref(), Some("simple_expansion"));
    }

    #[test]
    fn cmdsub_value_rhs_via_simple_expansion_is_dynamic() {
        // `A=$(date); B=$A; rm $B` — A is dynamic (placeholder
        // sentinel), B inherits dynamic, bare $B rejects.
        let (_, nt) = must_too_complex("A=$(date); B=$A; rm $B");
        assert_eq!(nt.as_deref(), Some("simple_expansion"));
    }

    #[test]
    fn var_placeholder_constant_value() {
        assert_eq!(VAR_PLACEHOLDER, "__TRACKED_VAR__");
    }

    #[test]
    fn safe_env_vars_includes_common_ones() {
        for v in ["HOME", "PWD", "USER", "PATH"] {
            assert!(SAFE_ENV_VARS.contains(&v));
        }
    }

    #[test]
    fn bare_var_unsafe_detects_iffs_and_glob_chars() {
        assert!(bare_var_unsafe("foo bar"));
        assert!(bare_var_unsafe("/etc/*"));
        assert!(bare_var_unsafe("a?b"));
        assert!(bare_var_unsafe("[abc]"));
        assert!(bare_var_unsafe("line\nbreak"));
        assert!(!bare_var_unsafe("/safe/path"));
        assert!(!bare_var_unsafe("--flag=value"));
    }
}
