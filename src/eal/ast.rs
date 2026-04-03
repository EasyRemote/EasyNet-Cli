// EasyNet CLI — EAL AST
// ======================
//
// File: src/eal/ast.rs
// Description: Abstract syntax tree for parsed EAL programs.
//
// Design Rationale:
// - In-memory only, NOT serializable. The serializable form is Mission IR v2 (ir.rs).
// - Represents the syntactic structure faithfully; semantic analysis happens in analyzer.rs.
// - FieldValue::VarRef captures variable references (e.g., `photo.output`) that the analyzer
//   uses to infer data dependencies — no manual `depends_on` in the language.
//
// Key Types:
//   EalProgram → MissionDecl → Vec<Statement> → CallExpr → Vec<Field> → FieldValue
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

/// Root of an EAL program.
#[derive(Debug, Clone)]
pub struct EalProgram {
    pub mission: MissionDecl,
}

/// `mission "name" { ... }`
#[derive(Debug, Clone)]
pub struct MissionDecl {
    pub name: String,
    pub statements: Vec<Statement>,
}

#[derive(Debug, Clone)]
pub enum Statement {
    LetCall { binding: String, call: CallExpr },
    Call(CallExpr),
}

/// `call "ability" on "node" with { ... } <options>`
#[derive(Debug, Clone)]
pub struct CallExpr {
    pub function_name: String,
    pub target_node: Option<String>,
    pub arguments: Vec<Field>,
    pub options: StepOptions,
}

#[derive(Debug, Clone)]
pub struct Field {
    pub key: String,
    pub value: FieldValue,
}

#[derive(Debug, Clone)]
pub enum FieldValue {
    String(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    VarRef { var_name: String },
}

#[derive(Debug, Clone, Default)]
pub struct StepOptions {
    pub timeout_seconds: Option<i32>,
    pub max_retries: Option<i32>,
    pub on_failure: Option<FailurePolicy>,
    pub optional: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum FailurePolicy {
    Abort,
    Skip,
    Retry,
    Continue,
}
