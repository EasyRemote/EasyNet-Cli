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
/// or member-call form `<agent>.<ability>(args)`.
///
/// `target_kind` records which surface form the parser matched. The
/// planner uses it to lower into the correct `IrTarget` variant —
/// `Agent` for member-call forms, `Device` for the traditional
/// `call ... on ...` form. The two are intentionally distinct surfaces
/// for two different ontological roles (ontology §5: device is hosting
/// substrate; §6.4: agent is logical actor). The runtime dispatcher
/// branches on the resolved `IrTarget`, never on a string-based
/// `is_agent` check — see `docs/AGENT_IDENTITY.md` invariant 2.
#[derive(Debug, Clone)]
pub struct CallExpr {
    pub function_name: String,
    pub target_node: Option<String>,
    pub target_kind: TargetKind,
    pub arguments: Vec<Field>,
    pub options: StepOptions,
}

/// Which dispatch target kind the surface form addressed. Set by the
/// parser at production-match time, consumed by the planner during IR
/// lowering. There is no runtime classification — the surface form is
/// the only signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TargetKind {
    /// Member-call form `agent.ability(...)`. Lowers to
    /// `IrTarget::Agent(AgentId)`.
    Agent,
    /// Traditional form `call "ability" on "node"`. Lowers to
    /// `IrTarget::Device { node_id }`. This is the default because
    /// the traditional form historically addressed devices, and the
    /// EAL surface for "call an agent" is now the explicit member-call
    /// form (ontology §6.2 sugar example).
    #[default]
    Device,
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
