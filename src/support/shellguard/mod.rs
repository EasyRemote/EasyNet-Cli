// EasyNet CLI — ShellGuard: bash safety subsystem
// ===================================================
//
// File: src/support/shellguard/mod.rs
// Description: Self-contained subsystem hosting every check
//              that a shell-interpreted ability runs before
//              dispatch (AST, security patterns, permission
//              rules, read-only validation, path constraints,
//              sed-edit detection, sandbox decision, destructive
//              command list).
//
// Why this lives in `support/`, not `runtime/agents/`
// ---------------------------------------------------
// The bash safety pipeline is shared infrastructure. Three
// distinct ability handlers consume pieces of it:
//
//   * `process.exec`  — only needs `destructive` (refuses without
//                       the caller's `destructive_acknowledged`
//                       flag) plus the shared process-execution
//                       hardening (`runner`).
//   * `shell.run`     — needs every piece: AST parse, pattern
//                       detection, permission rules, read-only
//                       validation, path constraints, sed-edit
//                       detection, sandbox selection, destructive
//                       check.
//   * future `pty.attach` v2 — would consume `sandbox` selection
//                       to opt the spawned shell into platform
//                       sandbox.
//
// Hosting these helpers under `runtime/agents/shell_run_*.rs`
// would lock the security work into one ability's namespace.
// Lifting them to `support/shellguard/` keeps the security
// pipeline as one self-describing subsystem; ability handlers
// stay thin (one file, ~300 lines) and just call the
// subsystem's public API.
//
// AXIOM Tier 2.5 (`AXIOM.tex §"Tier 2.5 — Baseline Locomotion
// Profile"`) is the normative spec for what this subsystem
// must do. The 8-stage shell.run pipeline is mirrored
// stage-by-stage in `security/`, `permissions/`, `readonly`,
// `pathconstraints`, `sed`, `destructive`, `sandbox`. Each
// stage is independently testable; the integration `evaluate()`
// fn (added in a later slice) chains them in spec-order.
//
// Architectural Position
// ----------------------
// Leaf-level subsystem under `src/support/`. Has no dependency
// on `runtime/`, `persistence/`, `registry/`, `services/`. The
// only crate-internal types it needs are in `core/` (typed
// errors, IDs).
//
// Implementation reference
// ------------------------
// AliveCode's `BashTool` (vendor/AliveCode/src/tools/BashTool/
// in their tree) is the implementation reference for the
// 8-stage pipeline. The Rust translation here preserves the
// stage numbering, the pattern catalogue, and the rule
// classifier shape so audit events emitted by the two
// implementations correlate. Where AliveCode integrates with
// LLM tool-use specifics (React UI, prompt strings, growthbook
// feature flags), this Rust translation drops the LLM
// integration and keeps the pure security pipeline.
//
// Author: Silan.Hu
// Email: silan.hu@u.nus.edu
// Copyright (c) 2026-2027 easynet. All rights reserved.

/// AXIOM Tier 2.5 destructive-command list. Shared between
/// `process.exec` and `shell.run`. Receivers refuse calls that
/// invoke a destructive command without the caller's
/// `destructive_acknowledged: true` opt-in.
pub mod destructive;

/// Shared process-execution runner. Owns the tempfile-backed
/// stdout/stderr (`O_APPEND` + `O_NOFOLLOW`), process-group +
/// tree-kill, env override defaults (`GIT_EDITOR=true`,
/// `PAGER=cat`, `LESS=-FRX`), output cap detection, and
/// duration accounting. Used by both `process.exec` (with
/// argv-only invocation) and `shell.run` (with shell-and-arg
/// invocation, after the 8-stage security pipeline accepts the
/// command).
pub mod runner;

/// Stage-4 of the shell.run 8-stage pipeline: post-AST argv-level
/// dangerous-pattern detectors. Operates on the `SimpleCommand`
/// records the AST stage produces. Catches the things the AST
/// stage can't see — argv[0] being an eval-like / zsh-module
/// builtin, an interpreter `-c` flag carrying inline code,
/// content patterns like `/proc/self/environ` access or
/// `jq 'system(...)'`. Mirrors AliveCode's
/// `tools/BashTool/bashSecurity.ts` validators.
pub mod security;

/// Stage-5 permission rule matcher. After the security stage
/// has rejected the categorically-dangerous shapes, the
/// permission stage answers the policy question: "is this
/// caller allowed to run THIS command?". Caller-provided
/// allow / deny rule lists; deny wins; default-deny if no
/// allow rule matches. Rules match on argv[0] prefix plus an
/// optional flag allowlist.
pub mod permissions;

/// Stage-6 path-constraint matcher for write redirects.
/// Caller declares one or more "write-allowed roots"; every
/// redirect target whose canonical path is not under at least
/// one allowed root rejects the call. Stops a permitted
/// command from writing outside the caller's project tree
/// via `> /etc/passwd` or `>> ~/.ssh/authorized_keys`.
pub mod pathconstraints;

/// Stage-7 read-only classifier. Used when the caller passes
/// `read_only_only: true`: every command must come from a
/// known-read-only set AND carry no write redirects. Mirrors
/// AliveCode's read-only validation for the shell.run mode
/// where agents can inspect but not mutate.
pub mod readonly;

/// Tree-sitter-bash AST stage of the shell.run 8-stage pipeline.
/// Produces a `ParseForSecurityResult` with either a flat list of
/// `SimpleCommand` records (one per leaf `command` node in the
/// AST) or a `too-complex` verdict naming the offending node /
/// pre-check. Fail-closed: any node type not on the explicit
/// allowlist triggers `too-complex`. Mirrors AliveCode's
/// `src/utils/bash/ast.ts` so audit events from the two
/// implementations correlate.
pub mod ast;
