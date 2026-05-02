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
    LetCall {
        binding: String,
        call: CallExpr,
    },
    Call(CallExpr),
    /// `loop "<name>?" max_iters: N { body { … } verify { … } }`.
    /// See RFC docs/rfc/eal-control-flow-v1.md §3.1. The parser
    /// populates this variant unchanged; the planner lowers it to
    /// `IrStep::Loop` (see `src/eal/ir.rs`). The anonymous form
    /// (no name) sets `name = None` and no binding is exported;
    /// the named form sets `name = Some(s)` and exports
    /// `<s>.result` at the enclosing scope.
    ///
    /// `loop` is the ONLY block form in v1. An earlier Draft of the
    /// RFC proposed `chat { }` and `handoff { }` block forms; both
    /// were removed in the approved revision (see RFC §10). `chat`
    /// at statement position — renamed to `discuss { }` pending
    /// consumer — is tracked in
    /// `docs/open-questions/discuss-eal-block.md`. `handoff` was
    /// deleted outright (expressible as two flat EAL statements).
    Loop(LoopBlock),
}

#[derive(Debug, Clone)]
pub struct LoopBlock {
    pub name: Option<String>,
    pub max_iters: u32,
    pub body: Vec<Statement>,
    pub verify: Vec<Statement>,
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
    VarRef {
        var_name: String,
    },
    /// Inline JSON-object literal. Lets a member-call carry nested
    /// args (e.g. `claude.invoke(args: { location: "Beijing" })`).
    /// Nesting is bounded by the parser's own recursion (no
    /// artificial cap; bad input bottoms out at the EAL stack
    /// limit). Lowered to `serde_json::Value::Object(...)` at IR
    /// time.
    Object(Vec<Field>),
}

#[derive(Debug, Clone, Default)]
pub struct StepOptions {
    pub timeout_seconds: Option<i32>,
    pub max_retries: Option<i32>,
    /// Failure disposition for this step. `None` is equivalent to
    /// `Some(FailurePolicy::Continue)` — the mission continues past
    /// the failure, the trace records the step as `Failed`, and any
    /// downstream consumers receive `ResolveError::UpstreamFailed`
    /// for the missing binding.
    pub on_failure: Option<FailurePolicy>,
    /// **Scheduling + best-effort marker.** An `optional` step:
    /// - runs *after* all required steps in the same phase, so
    ///   required work claims shared resources (API quotas, retry
    ///   budgets) first;
    /// - on failure, classifies as `Skipped` (not `Failed`) in the
    ///   trace, and downstream consumers see
    ///   `ResolveError::UpstreamSkipped` (a distinct category from
    ///   `UpstreamFailed`);
    /// - cannot abort the mission, regardless of `on_failure`.
    ///
    /// The interaction `optional = true, on_failure = Abort` is
    /// contradictory and rejected by the analyzer: a step cannot be
    /// simultaneously best-effort and mission-critical.
    pub optional: bool,
}

/// What to do when a step fails. Each variant has one distinct
/// runtime effect; see the interpreter's dispatch path for the
/// state-machine details.
#[derive(Debug, Clone, Copy)]
pub enum FailurePolicy {
    /// Abort the entire mission on failure. Overridden by
    /// `optional = true` (which forbids aborting); the combination
    /// is rejected at analysis time.
    Abort,
    /// Classify the outcome as `Skipped` (not `Failed`) and
    /// continue. Downstream consumers see `UpstreamSkipped`. This
    /// is the pure-failure-policy counterpart to `optional = true`;
    /// `optional = true` additionally defers scheduling.
    Skip,
    /// Re-dispatch up to `max_retries` times with exponential
    /// backoff. Requires `max_retries > 0` (analyzer-enforced).
    /// After retries are exhausted, the outcome is `Failed`.
    Retry,
    /// Default: mission continues past the failure; the step is
    /// classified `Failed`; downstream consumers see
    /// `UpstreamFailed`. This is the variant most missions want.
    Continue,
}
