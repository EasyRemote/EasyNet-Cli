// EasyNet CLI — EAL Compile Pass
// ==============================
//
// File: src/eal/planner.rs
// Description: One-pass compile: AST → semantic-analyzed steps →
//              phase-partitioned Mission IR v2.
//
// Why the analyzer and planner are one module
// -------------------------------------------
//
// A classical compiler separates semantic analysis (symbol tables,
// cycle detection, type checks) from IR lowering. That split pays
// off when multiple back-ends consume the analyzed form, or when a
// large front-end needs to share the analyzer with tools like an
// LSP. Neither applies to EAL: the analyzer had exactly one
// consumer (this planner), the two passes lived at the same
// `pub(crate)` visibility, and the intermediate `AnalyzedStep`
// existed only so the planner could re-read fields the analyzer
// had just put there. The separation added a file without adding
// a boundary — every change had to update both files to stay in
// sync.
//
// This module collapses them. `compile` walks the AST once,
// producing the same IR shape the planner previously emitted, and
// every semantic check that used to live in `analyzer.rs` now
// fires during that walk. The public surface (`compile`) is
// unchanged; the previously `pub(crate)` `analyzer` module is
// gone.
//
// Algorithm
// ---------
//
//   1. First pass: walk statements, build an internal
//      `AnalyzedStep` vector with:
//        - unique step ids (`<binding>` or `__anon_N`)
//        - duplicate-binding rejection
//        - undefined-reference rejection
//        - retry-requires-retries check
//        - optional ∧ on_failure=abort conflict check
//      Dependencies are *inferred* from `VarRef` fields, not
//      declared.
//   2. Cycle detection on the dependency graph (DFS).
//   3. Phase assignment: each step is placed in
//      `max(phase(dep)) + 1` so all its producers are resolved
//      before it runs.
//   4. Topological lowering: steps sorted by phase index emit
//      `IrStep`s; `PhaseRange` boundaries track the contiguous
//      slices.
//
// Invariants (all guarded by this file's tests)
// ---------------------------------------------
// - Steps in the same `PhaseRange` have **no** mutual data
//   dependencies — so the interpreter may dispatch them in
//   parallel without racing.
// - `PhaseRange`s partition `steps[0..]` contiguously; no gaps,
//   no overlaps.
// - Every `input_refs` binding resolves to a step in a strictly
//   earlier phase.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use super::ast::*;
use super::ir::*;
use std::collections::{BTreeMap, HashMap, HashSet};

/// Compile an EAL program to Mission IR v2.
pub fn compile(program: &EalProgram) -> anyhow::Result<MissionIr> {
    let analyzed = analyze(program)?;
    let phase_of = assign_phases(&analyzed);
    let num_phases = phase_of.iter().copied().max().map(|p| p + 1).unwrap_or(0);

    let mut ordered: Vec<usize> = (0..analyzed.len()).collect();
    ordered.sort_by_key(|&i| phase_of[i]);

    let mut ir_steps = Vec::new();
    let mut phases = Vec::new();
    let mut cur_phase = 0usize;
    let mut phase_start = 0usize;

    for (out_idx, &orig_idx) in ordered.iter().enumerate() {
        let phase = phase_of[orig_idx];
        while cur_phase < phase {
            phases.push(PhaseRange {
                start: phase_start,
                end: out_idx,
            });
            phase_start = out_idx;
            cur_phase += 1;
        }
        ir_steps.push(lower(&analyzed[orig_idx])?);
    }
    if !ir_steps.is_empty() {
        phases.push(PhaseRange {
            start: phase_start,
            end: ir_steps.len(),
        });
    }
    while phases.len() < num_phases {
        let e = ir_steps.len();
        phases.push(PhaseRange { start: e, end: e });
    }

    Ok(MissionIr {
        name: program.mission.name.clone(),
        steps: ir_steps,
        phases,
        constraints: IrConstraints::default(),
    })
}

/// Private, per-compile-call record. Not a `pub(crate)` type — if a
/// future caller ever needs "analyzed but unplanned" steps, add a
/// dedicated accessor to this module rather than re-exposing the
/// old analyzer boundary.
///
/// `Debug` is derived so unit tests can `unwrap_err()` on
/// `Result<Vec<AnalyzedStep>, _>` without a `Debug` bound
/// failure; no other code prints analyzed steps.
#[derive(Debug)]
struct AnalyzedStep {
    step_id: String,
    binding: Option<String>,
    call: CallExpr,
    deps: HashSet<String>,
}

/// First pass: semantic analysis. Walks the AST once, builds the
/// symbol table + dependency graph, rejects every semantic error
/// the language guarantees to catch before IR lowering.
fn analyze(program: &EalProgram) -> anyhow::Result<Vec<AnalyzedStep>> {
    let mut symbols: HashMap<String, usize> = HashMap::new();
    let mut steps = Vec::new();
    let mut anon = 0u32;

    for stmt in &program.mission.statements {
        let (binding, call): (Option<String>, &CallExpr) = match stmt {
            Statement::LetCall { binding, call } => (Some(binding.clone()), call),
            Statement::Call(call) => (None, call),
        };

        if let Some(ref name) = binding {
            anyhow::ensure!(!symbols.contains_key(name), "duplicate binding '{name}'");
        }

        // Prefer the binding as the step id when present (one
        // allocation); allocate `__anon_N` only for unbound steps.
        let step_id = match &binding {
            Some(name) => name.clone(),
            None => {
                anon += 1;
                format!("__anon_{anon}")
            }
        };

        // Policy checks — keep in lock-step with the `FailurePolicy`
        // rustdoc in `ast.rs`. Failures here are user-authoring
        // errors, not language bugs, so `anyhow::ensure!` suffices.

        // Retry requires an explicit positive count.
        if matches!(call.options.on_failure, Some(FailurePolicy::Retry)) {
            let retries = call.options.max_retries.unwrap_or(0);
            anyhow::ensure!(
                retries > 0,
                "step '{step_id}': on_failure retry requires `retries N` with N > 0"
            );
        }

        // `optional` and `on_failure abort` are contradictory — a
        // step cannot be both best-effort and mission-critical.
        // Previously the interpreter silently let `optional` win;
        // rejecting here surfaces the conflict to the user.
        anyhow::ensure!(
            !(call.options.optional
                && matches!(call.options.on_failure, Some(FailurePolicy::Abort))),
            "step '{step_id}': `optional` and `on_failure abort` are contradictory; \
             pick one (use `optional` for best-effort, `on_failure abort` for mission-critical)"
        );

        // Inferred dependency collection. Every `VarRef` must name
        // an earlier binding.
        let mut deps = HashSet::new();
        for field in &call.arguments {
            if let FieldValue::VarRef { var_name } = &field.value {
                anyhow::ensure!(
                    symbols.contains_key(var_name),
                    "step '{step_id}': undefined variable '{var_name}'"
                );
                deps.insert(var_name.clone());
            }
        }

        if let Some(ref name) = binding {
            symbols.insert(name.clone(), steps.len());
        }

        steps.push(AnalyzedStep {
            step_id,
            binding,
            call: call.clone(),
            deps,
        });
    }

    detect_cycles(&steps)?;
    Ok(steps)
}

fn detect_cycles(steps: &[AnalyzedStep]) -> anyhow::Result<()> {
    let id_to_idx: HashMap<&str, usize> = steps
        .iter()
        .enumerate()
        .filter_map(|(i, s)| s.binding.as_deref().map(|b| (b, i)))
        .collect();
    let mut visited = vec![false; steps.len()];
    let mut in_stack = vec![false; steps.len()];
    for i in 0..steps.len() {
        if !visited[i] {
            dfs(i, steps, &id_to_idx, &mut visited, &mut in_stack)?;
        }
    }
    Ok(())
}

fn dfs(
    idx: usize,
    steps: &[AnalyzedStep],
    map: &HashMap<&str, usize>,
    visited: &mut [bool],
    in_stack: &mut [bool],
) -> anyhow::Result<()> {
    visited[idx] = true;
    in_stack[idx] = true;
    for dep in &steps[idx].deps {
        if let Some(&di) = map.get(dep.as_str()) {
            if !visited[di] {
                dfs(di, steps, map, visited, in_stack)?;
            } else if in_stack[di] {
                anyhow::bail!("cycle involving '{}' and '{dep}'", steps[idx].step_id);
            }
        }
    }
    in_stack[idx] = false;
    Ok(())
}

fn assign_phases(steps: &[AnalyzedStep]) -> Vec<usize> {
    let binding_to_idx: HashMap<&str, usize> = steps
        .iter()
        .enumerate()
        .filter_map(|(i, s)| s.binding.as_deref().map(|b| (b, i)))
        .collect();
    let mut phase = vec![0usize; steps.len()];
    for (i, step) in steps.iter().enumerate() {
        let mut max = 0usize;
        let mut has = false;
        for dep in &step.deps {
            if let Some(&di) = binding_to_idx.get(dep.as_str()) {
                has = true;
                max = max.max(phase[di]);
            }
        }
        phase[i] = if has { max + 1 } else { 0 };
    }
    phase
}

fn lower(step: &AnalyzedStep) -> anyhow::Result<IrStep> {
    use crate::core::agent_id::{AbilityName, AgentId};

    let mut static_args = serde_json::Map::new();
    let mut input_refs = BTreeMap::new();
    for f in &step.call.arguments {
        match &f.value {
            FieldValue::VarRef { var_name } => {
                input_refs.insert(f.key.clone(), var_name.clone());
            }
            FieldValue::String(s) => {
                static_args.insert(f.key.clone(), serde_json::json!(s));
            }
            FieldValue::Int(n) => {
                static_args.insert(f.key.clone(), serde_json::json!(n));
            }
            FieldValue::Float(v) => {
                static_args.insert(f.key.clone(), serde_json::json!(v));
            }
            FieldValue::Bool(b) => {
                static_args.insert(f.key.clone(), serde_json::json!(b));
            }
        }
    }

    // Resolve target by surface form. The parser sets `target_kind` to
    // record which production matched (member-call → Agent, traditional
    // `call ... on ...` → Device). The runtime dispatcher matches the
    // resolved `IrTarget` enum and never re-classifies by string lookup
    // — see `docs/AGENT_IDENTITY.md` invariant 2.
    let target = match step.call.target_kind {
        TargetKind::Agent => {
            let raw = step.call.target_node.as_deref().ok_or_else(|| {
                anyhow::anyhow!(
                    "step '{}': member-call form has no agent target (parser bug)",
                    step.step_id
                )
            })?;
            let agent_id = AgentId::parse(raw).map_err(|e| {
                anyhow::anyhow!(
                    "step '{}': agent target '{raw}' is not a valid agent id: {e}",
                    step.step_id
                )
            })?;
            IrTarget::Agent(agent_id)
        }
        TargetKind::Device => {
            // Traditional form may omit `on "..."` (legacy missions);
            // store as empty node id rather than failing the compile.
            // The dispatcher will surface a clearer error at runtime.
            let node_id = step.call.target_node.clone().unwrap_or_default();
            IrTarget::Device { node_id }
        }
    };

    let ability = AbilityName::parse(&step.call.function_name).map_err(|e| {
        anyhow::anyhow!(
            "step '{}': ability name '{}' is not valid: {e}",
            step.step_id,
            step.call.function_name
        )
    })?;

    Ok(IrStep {
        step_id: step.step_id.clone(),
        step_name: step.step_id.clone(),
        ability,
        target,
        static_arguments: serde_json::Value::Object(static_args),
        input_refs,
        output_binding: step.binding.clone(),
        timeout_seconds: step.call.options.timeout_seconds.unwrap_or(0),
        max_retries: step.call.options.max_retries.unwrap_or(0),
        // Exhaustive match: keeps the AST→IR lowering honest if a
        // new `FailurePolicy` variant is added later. The `None`
        // case folds into `Continue` because the IR default is
        // `Continue` and users who want that behaviour can omit
        // `on_failure` entirely.
        on_failure: match step.call.options.on_failure {
            Some(FailurePolicy::Abort) => IrFailurePolicy::Abort,
            Some(FailurePolicy::Skip) => IrFailurePolicy::Skip,
            Some(FailurePolicy::Retry) => IrFailurePolicy::Retry,
            Some(FailurePolicy::Continue) | None => IrFailurePolicy::Continue,
        },
        optional: step.call.options.optional,
        content_type: "application/json".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eal::parser;

    // ── Semantic-analysis behaviour (previously lived in analyzer.rs) ──

    #[test]
    fn deps_inferred() {
        let p = parser::parse(
            r#"mission "t" { let a = call "x" on "n" let b = call "y" on "n" with { i = a.output } }"#,
        )
        .unwrap();
        let steps = analyze(&p).unwrap();
        assert!(steps[1].deps.contains("a"));
    }

    #[test]
    fn undefined_ref_rejected() {
        let p =
            parser::parse(r#"mission "t" { let b = call "y" on "n" with { i = nope.output } }"#)
                .unwrap();
        assert!(analyze(&p).is_err());
    }

    #[test]
    fn duplicate_binding_rejected() {
        let p = parser::parse(
            r#"mission "t" { let a = call "x" on "n" let a = call "y" on "n" }"#,
        )
        .unwrap();
        let err = analyze(&p).unwrap_err();
        assert!(
            format!("{err}").contains("duplicate binding"),
            "error must name the duplicate: {err}"
        );
    }

    #[test]
    fn retry_without_positive_count_rejected() {
        // `retries` defaults to 0; `on_failure retry` without an
        // explicit positive count is a misconfiguration that the
        // interpreter's retry path cannot satisfy — catch it at
        // compile time.
        let p = parser::parse(
            r#"mission "t" { call "x" on "n" on_failure retry }"#,
        )
        .unwrap();
        let err = analyze(&p).unwrap_err();
        assert!(
            format!("{err}").contains("retry requires"),
            "error must explain the retry contract: {err}"
        );
    }

    #[test]
    fn optional_plus_on_failure_abort_is_rejected() {
        // Semantic conflict: `optional` marks the step best-effort
        // (cannot abort the mission); `on_failure abort` marks it
        // mission-critical. Previously the interpreter silently
        // let `optional` win, which made `on_failure abort` a dead
        // annotation in that combination.
        let p =
            parser::parse(r#"mission "t" { call "x" on "n" optional on_failure abort }"#)
                .unwrap();
        let err = analyze(&p).unwrap_err();
        assert!(
            format!("{err}").contains("contradictory"),
            "expected 'contradictory' in error, got: {err}"
        );
    }

    #[test]
    fn optional_plus_on_failure_skip_is_allowed() {
        let p =
            parser::parse(r#"mission "t" { call "x" on "n" optional on_failure skip }"#)
                .unwrap();
        assert!(analyze(&p).is_ok());
    }

    #[test]
    fn optional_alone_is_allowed() {
        let p = parser::parse(r#"mission "t" { call "x" on "n" optional }"#).unwrap();
        assert!(analyze(&p).is_ok());
    }

    #[test]
    fn forward_self_reference_rejected_as_undefined() {
        // `let a = call ... with { i = a.output }` references `a` in
        // its own argument list. Because bindings are registered
        // *after* their step's arguments are scanned (so a step
        // cannot read its own output), this lands in the
        // undefined-variable branch, not the cycle branch. That is
        // the correct, most-informative error for this shape — the
        // user has written "use the output of this call as input to
        // itself," which is an authoring mistake before it's a
        // graph-theoretic cycle.
        let p = parser::parse(
            r#"mission "t" { let a = call "x" on "n" with { i = a.output } }"#,
        )
        .unwrap();
        let err = analyze(&p).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("undefined variable 'a'"),
            "expected 'undefined variable' error, got: {msg}"
        );
    }

    // ── Phase partitioning (previously in planner.rs) ──

    #[test]
    fn independent_same_phase() {
        let ir = compile(
            &parser::parse(r#"mission "t" { let a = call "x" on "n1" let b = call "y" on "n2" }"#)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(ir.phases.len(), 1);
    }

    #[test]
    fn linear_chain() {
        let ir = compile(&parser::parse(r#"mission "t" { let a = call "x" on "n" let b = call "y" on "n" with { i = a.output } let c = call "z" on "n" with { i = b.output } }"#).unwrap()).unwrap();
        assert_eq!(ir.phases.len(), 3);
    }

    #[test]
    fn diamond() {
        let ir = compile(&parser::parse(r#"mission "d" { let a = call "a" on "n" let b = call "b" on "n" with { i = a.output } let c = call "c" on "n" with { i = a.output } let d = call "d" on "n" with { l = b.output, r = c.output } }"#).unwrap()).unwrap();
        assert_eq!(ir.phases.len(), 3);
        assert_eq!(ir.phases[0].end - ir.phases[0].start, 1); // a
        assert_eq!(ir.phases[1].end - ir.phases[1].start, 2); // b, c
        assert_eq!(ir.phases[2].end - ir.phases[2].start, 1); // d
    }

    #[test]
    fn example_files_compile() {
        let examples = &[
            include_str!("../../examples/hello.eal"),
            include_str!("../../examples/parallel.eal"),
            include_str!("../../examples/pipeline.eal"),
            include_str!("../../examples/diamond.eal"),
            include_str!("../../examples/daily-report.eal"),
        ];
        for (i, src) in examples.iter().enumerate() {
            let prog = parser::parse(src).unwrap_or_else(|e| panic!("example {i} parse: {e}"));
            let ir = compile(&prog).unwrap_or_else(|e| panic!("example {i} compile: {e}"));
            assert!(!ir.steps.is_empty(), "example {i} has no steps");
            // Verify IR is serializable
            serde_json::to_string(&ir).unwrap_or_else(|e| panic!("example {i} serialize: {e}"));
        }
    }

    /// Every dependency must land in an earlier phase than its consumer.
    /// This is the defining invariant of phase partitioning: within one
    /// phase, steps execute in parallel, so a same-phase dependency
    /// would race. A regression that merged a dependent step into its
    /// dependency's phase would silently corrupt data flow; this test
    /// locks the invariant down for every step in every example file.
    #[test]
    fn every_dependency_strictly_precedes_its_consumer() {
        let examples = &[
            include_str!("../../examples/hello.eal"),
            include_str!("../../examples/parallel.eal"),
            include_str!("../../examples/pipeline.eal"),
            include_str!("../../examples/diamond.eal"),
            include_str!("../../examples/daily-report.eal"),
        ];
        for (i, src) in examples.iter().enumerate() {
            let ir = compile(&parser::parse(src).unwrap()).unwrap();

            // Walk phases in order and build `binding → phase_index`
            // from what the planner actually emitted. A step that
            // references a binding must find it already registered.
            let mut binding_phase: HashMap<String, usize> = HashMap::new();
            for (phase_idx, range) in ir.phases.iter().enumerate() {
                // First, validate every input_refs in this phase
                // resolves to an earlier phase (strict <, not <=).
                for step in &ir.steps[range.start..range.end] {
                    for binding in step.input_refs.values() {
                        let dep_phase = binding_phase.get(binding).copied().unwrap_or_else(|| {
                            panic!(
                                "example {i}: step '{}' references unknown binding '{}'",
                                step.step_id, binding
                            )
                        });
                        assert!(
                            dep_phase < phase_idx,
                            "example {i}: step '{}' consumes '{}' from phase {}, \
                             but itself lives in phase {} — same-phase data flow \
                             would race under parallel dispatch",
                            step.step_id,
                            binding,
                            dep_phase,
                            phase_idx,
                        );
                    }
                }
                // Then, register this phase's output bindings so later
                // phases can resolve them. We delay registration until
                // after the input check so a step can't satisfy its
                // own data flow via a same-phase binding.
                for step in &ir.steps[range.start..range.end] {
                    if let Some(b) = &step.output_binding {
                        binding_phase.insert(b.clone(), phase_idx);
                    }
                }
            }
        }
    }

    /// Phase ranges must form a contiguous partition of `steps`. A gap
    /// or overlap would either skip execution of some steps or cause
    /// double-dispatch. Cheap invariant, expensive bug.
    #[test]
    fn phase_ranges_form_a_contiguous_partition() {
        let src = include_str!("../../examples/daily-report.eal");
        let ir = compile(&parser::parse(src).unwrap()).unwrap();
        let mut cursor = 0usize;
        for (i, range) in ir.phases.iter().enumerate() {
            assert_eq!(
                range.start, cursor,
                "phase {i} starts at {}, expected {cursor}",
                range.start
            );
            assert!(
                range.end >= range.start,
                "phase {i} has inverted range {range:?}"
            );
            cursor = range.end;
        }
        assert_eq!(
            cursor,
            ir.steps.len(),
            "phases must end at steps.len()={}",
            ir.steps.len()
        );
    }

    #[test]
    fn daily_report_phases() {
        let src = include_str!("../../examples/daily-report.eal");
        let ir = compile(&parser::parse(src).unwrap()).unwrap();
        assert_eq!(ir.name, "daily-report");
        // Phase 0: photo, config, metrics.ping, notify.send (4 independent — no data deps)
        // Phase 1: model.inference (depends on photo + config)
        // Phase 2: data.collect (depends on result)
        // Note: notify.send has no VarRef deps, so it lands in phase 0 even though
        // it appears late in the source. Dependencies are inferred, not positional.
        assert_eq!(ir.phases.len(), 3);
        assert_eq!(ir.steps.len(), 6);
        // Phase 0: 4 independent steps
        assert_eq!(ir.phases[0].end - ir.phases[0].start, 4);
        // Phase 1: inference
        assert_eq!(ir.phases[1].end - ir.phases[1].start, 1);
        // Phase 2: collect
        assert_eq!(ir.phases[2].end - ir.phases[2].start, 1);
    }
}
