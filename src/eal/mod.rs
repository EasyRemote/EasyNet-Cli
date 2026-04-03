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
// Pipeline: source → lexer → parser → analyzer → planner → ir → interpreter
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

pub mod analyzer;
pub mod ast;
pub mod interpreter;
pub mod ir;
pub mod lexer;
pub mod parser;
pub mod planner;

