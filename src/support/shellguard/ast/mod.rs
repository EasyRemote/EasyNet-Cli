// EasyNet CLI — ShellGuard: AST stage of shell.run pipeline
// =========================================================
//
// File: src/support/shellguard/ast/mod.rs
// Description: Public surface of the AST sub-module. Exposes the
//              three result shapes a caller needs (`Redirect`,
//              `SimpleCommand`, `ParseForSecurityResult`) and the
//              one entry point `parse_for_security(cmd)` that
//              maps a bash command string to a structured verdict.
//
// Why a sub-module under `shellguard/`
// ------------------------------------
// shellguard's 8-stage pipeline routes EVERY shell.run command
// through the AST stage first. Subsequent stages (security
// pattern detectors in slice 4, permission rules in slice 5,
// readonly / pathconstraints / sed in slice 6) read the AST
// stage's `SimpleCommand` records and the `argv: Vec<String>`
// inside them — they don't touch the raw command string for
// argv-level decisions. Putting the parser + walker behind a
// stable type contract lets later stages compile against this
// module without re-deriving structure from text.
//
// The AST stage is intentionally narrow:
//
//   * Input  — a UTF-8 bash command string (`&str`).
//   * Output — `simple { commands: [...] }` if every node was
//              recognised AND every pre-check passed; otherwise
//              `too-complex { reason, node_type? }` naming what
//              caused the rejection. There is one third variant,
//              `parse-unavailable`, reserved for the case where
//              tree-sitter itself fails to instantiate (e.g. a
//              future runtime feature flag turns it off); it has
//              no production trigger today but the type carries
//              it so callers exhaustively match.
//
// Fail-closed invariant
// ---------------------
// The walker enumerates a small set of node types it understands
// and returns `too-complex` for everything else. New tree-sitter
// node types added in a future grammar bump fall into the
// fall-through arm and reject — this is correct: an unknown
// node type means the receiver cannot reason about whether the
// caller's intent maps to one or many simple commands, and that
// uncertainty must surface as a refusal, not a silent skip.
// Same property AliveCode's TypeScript port maintains.
//
// Out of scope for this slice
// ---------------------------
// AliveCode's full ast.ts adds two layers on top of the base
// walker that are deferred to slice 4:
//
//   1. Command-substitution recursion — `cmd "$(inner)"` extracts
//      the inner command, replaces the substitution with a
//      placeholder string in argv, and pushes the inner command
//      to `commands[]` as a sibling. This slice rejects all
//      `command_substitution` nodes as too-complex.
//   2. Variable scope tracking — `VAR=val && cmd $VAR`
//      substitutes the value when extracting argv. This slice
//      rejects `simple_expansion` ($VAR) as too-complex.
//
// Shipping (1) and (2) under the same slice as the foundational
// walker would explode the patch size; both layers reuse the
// `SimpleCommand` and `ParseForSecurityResult` types defined
// here, so the slice 4 follow-up is purely additive.
//
// Author: Silan.Hu
// Email: silan.hu@u.nus.edu
// Copyright (c) 2026-2027 easynet. All rights reserved.

mod parser;
mod walker;

pub use parser::{parse_root, ParseError};
pub use walker::parse_for_security;

/// A redirect operator and its target word, as extracted from a
/// `file_redirect` / `herestring_redirect` / `heredoc_redirect`
/// node. The downstream `pathconstraints` stage (slice 6) reads
/// these to decide whether a write redirect (`>`, `>>`, `>|`,
/// `&>`, `&>>`) escapes the caller's path allowlist.
///
/// `op` strings match the ones AliveCode's TypeScript port
/// uses (`>`, `>>`, `<`, `<<`, `>&`, `<&`, `>|`, `&>`, `&>>`,
/// `<<<`). Two implementations matching string-for-string keeps
/// audit events correlatable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redirect {
    /// Canonical operator token. Always one of the strings in
    /// the AliveCode-compatible set above. The set is small
    /// enough that callers can match on `op.as_str()` directly
    /// without an enum.
    pub op: String,
    /// Target word verbatim from the source. Quotes are
    /// preserved as-is — slice-6 path constraints unquote.
    pub target: String,
    /// Optional file descriptor when the operator carries one
    /// (`2>`, `2>>`, `2>&1`). `None` means stdout (`1`) or
    /// `op`-implied default.
    pub fd: Option<u32>,
}

/// A leaf bash command — argv (with quotes already stripped per
/// tree-sitter's tokenisation), the leading `VAR=val` env
/// assignments, the redirects on the wrapping `redirected_statement`
/// (if any), and the original source span for UI / receipt display.
///
/// `argv[0]` is always the command name (or builtin name for a
/// `declaration_command` like `export`). Permission rules in
/// slice 5 match against `argv[0]` and the rest of `argv` for
/// flag-allowlist decisions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimpleCommand {
    /// Command name + arguments. Empty argv means the parser saw
    /// a `command` node with no children — should not happen for
    /// well-formed input but the walker tolerates it (returns
    /// `too-complex` rather than panicking on `argv[0]` access).
    pub argv: Vec<String>,
    /// Leading `NAME=value` assignments (`MAKEFLAGS=-j4 make`).
    /// Order preserved to match the source.
    pub env_vars: Vec<EnvAssignment>,
    /// Redirects attached to this command. Always empty when the
    /// parent node is `command`; populated when the parent is
    /// `redirected_statement`.
    pub redirects: Vec<Redirect>,
    /// Original source span for this command, byte-aligned to
    /// the input string. Used by audit events and UI display.
    pub text: String,
}

/// One leading `NAME=value` assignment on a simple command.
/// Stored as separate `name` / `value` fields (rather than a
/// `(String, String)` tuple) so future stages can match on
/// `name` cheaply (e.g. an env-allowlist check).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvAssignment {
    /// Variable name. Always a bare identifier (tree-sitter
    /// rejects `1=val` etc. at parse time).
    pub name: String,
    /// Value verbatim from the source — quotes preserved.
    pub value: String,
}

/// Verdict from the AST stage. Three variants:
///
/// * `Simple { commands }` — every node was recognised and every
///   pre-check passed. Slice 4+ stages pick up `commands` and
///   continue the pipeline.
/// * `TooComplex { reason, node_type }` — at least one node /
///   pre-check rejected. `node_type` carries the offending
///   tree-sitter node type when the reject came from a fail-closed
///   walker arm; `None` when the reject came from a pre-check
///   string-test (those name themselves in `reason`).
/// * `ParseUnavailable` — reserved for future runtime feature flag
///   that disables tree-sitter. No production trigger in slice 3;
///   callers must still match exhaustively.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseForSecurityResult {
    Simple {
        commands: Vec<SimpleCommand>,
    },
    TooComplex {
        reason: String,
        node_type: Option<String>,
    },
    ParseUnavailable,
}

impl ParseForSecurityResult {
    /// Convenience: build a `TooComplex` with no node type
    /// attribution. Used for pre-check rejections.
    pub fn too_complex_pre(reason: impl Into<String>) -> Self {
        Self::TooComplex {
            reason: reason.into(),
            node_type: None,
        }
    }

    /// Convenience: build a `TooComplex` attributed to a tree-
    /// sitter node type. Used by the walker fall-through arms.
    pub fn too_complex_node(reason: impl Into<String>, node_type: impl Into<String>) -> Self {
        Self::TooComplex {
            reason: reason.into(),
            node_type: Some(node_type.into()),
        }
    }
}
