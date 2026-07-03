// EasyNet CLI — EAL (EasyNet Ability Language)
// =============================================
//
// File: src/eal/mod.rs
// Description: EAL compiler and execution pipeline namespace.
//
// Language Layering:
//   AAL (Agent Assembly Language) — future: agent behavior specification
//   EAL (this module)             — distributed ability orchestration DSL
//   Mission IR v2                 — serializable execution plan
//   Axon Invoke primitives        — gRPC execution backend
//
// Pipeline: source → lexer → parser → planner → ir → interpreter
//
// String-escape contract (F-024, normative): the lexer preserves
// escapes VERBATIM; consumers that machine-parse a string-literal
// payload (`*_json` args) peel the authoring escapes through
// `string_escape::unescape_string_literal` — never ad-hoc `replace`.
// Opaque payloads are forwarded untouched. See `string_escape.rs`.
//
//   `planner` collapses what used to be a separate analyzer pass +
//   planner pass into a single compile step (see `planner.rs`'s
//   module doc for why — the old two-file split added a boundary
//   without adding a user). The planner is the sole entry point
//   from AST to IR.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

pub(crate) mod diagnostics;
pub(crate) mod interpreter;
pub(crate) mod parser;
pub(crate) mod runtime;
