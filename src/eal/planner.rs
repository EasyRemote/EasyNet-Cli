// EasyNet CLI — EAL Planner
// =========================
//
// File: src/eal/planner.rs
// Description: Compiles EAL AST to Mission IR v2 via phase partitioning.
//
// Algorithm (topological layering):
//   1. Analyze AST → symbol table + dependency graph.
//   2. Assign each step to the earliest phase where all dependencies are resolved.
//      Phase 0 = steps with zero data-flow in-degree.
//      Phase N+1 = steps whose latest dependency is in phase N.
//   3. Lower each step: separate static_arguments from input_refs.
//   4. Emit MissionIr with PhaseRange boundaries.
//
// Key Property:
//   Steps within the same phase have NO mutual data dependencies →
//   the interpreter can dispatch them in parallel.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::HashMap;
use super::analyzer;
use super::ast::*;
use super::ir::*;

/// Compile an EAL program to Mission IR v2.
pub fn compile(program: &EalProgram) -> anyhow::Result<MissionIr> {
    let analyzed = analyzer::analyze(program)?;
    let phase_of = assign_phases(&analyzed.steps);
    let num_phases = phase_of.iter().copied().max().map(|p| p + 1).unwrap_or(0);

    let mut ordered: Vec<usize> = (0..analyzed.steps.len()).collect();
    ordered.sort_by_key(|&i| phase_of[i]);

    let mut ir_steps = Vec::new();
    let mut phases = Vec::new();
    let mut cur_phase = 0usize;
    let mut phase_start = 0usize;

    for (out_idx, &orig_idx) in ordered.iter().enumerate() {
        let phase = phase_of[orig_idx];
        while cur_phase < phase {
            phases.push(PhaseRange { start: phase_start, end: out_idx });
            phase_start = out_idx;
            cur_phase += 1;
        }
        ir_steps.push(lower(&analyzed.steps[orig_idx])?);
    }
    if !ir_steps.is_empty() {
        phases.push(PhaseRange { start: phase_start, end: ir_steps.len() });
    }
    while phases.len() < num_phases { let e = ir_steps.len(); phases.push(PhaseRange { start: e, end: e }); }

    Ok(MissionIr { name: analyzed.mission_name, steps: ir_steps, phases, constraints: IrConstraints::default() })
}

fn assign_phases(steps: &[analyzer::AnalyzedStep]) -> Vec<usize> {
    let binding_to_idx: HashMap<&str, usize> = steps.iter().enumerate()
        .filter_map(|(i, s)| s.binding.as_deref().map(|b| (b, i))).collect();
    let mut phase = vec![0usize; steps.len()];
    for (i, step) in steps.iter().enumerate() {
        let mut max = 0usize;
        let mut has = false;
        for dep in &step.deps {
            if let Some(&di) = binding_to_idx.get(dep.as_str()) { has = true; max = max.max(phase[di]); }
        }
        phase[i] = if has { max + 1 } else { 0 };
    }
    phase
}

fn lower(step: &analyzer::AnalyzedStep) -> anyhow::Result<IrStep> {
    use crate::shared::agent_id::{AbilityName, AgentId};

    let mut static_args = serde_json::Map::new();
    let mut input_refs = HashMap::new();
    for f in &step.call.arguments {
        match &f.value {
            FieldValue::VarRef { var_name } => { input_refs.insert(f.key.clone(), var_name.clone()); }
            FieldValue::String(s) => { static_args.insert(f.key.clone(), serde_json::json!(s)); }
            FieldValue::Int(n) => { static_args.insert(f.key.clone(), serde_json::json!(n)); }
            FieldValue::Float(v) => { static_args.insert(f.key.clone(), serde_json::json!(v)); }
            FieldValue::Bool(b) => { static_args.insert(f.key.clone(), serde_json::json!(b)); }
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
        on_failure: match step.call.options.on_failure {
            Some(FailurePolicy::Abort) => IrFailurePolicy::Abort,
            Some(FailurePolicy::Skip) => IrFailurePolicy::Skip,
            Some(FailurePolicy::Retry) => IrFailurePolicy::Retry,
            _ => IrFailurePolicy::Continue,
        },
        optional: step.call.options.optional,
        content_type: "application/json".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eal::parser;

    #[test]
    fn independent_same_phase() {
        let ir = compile(&parser::parse(r#"mission "t" { let a = call "x" on "n1" let b = call "y" on "n2" }"#).unwrap()).unwrap();
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
