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
