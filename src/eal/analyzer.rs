// EasyNet CLI — EAL Analyzer
// ==========================
//
// File: src/eal/analyzer.rs
// Description: Semantic analysis — validates references, infers dependencies, detects cycles.
//
// Responsibilities:
// 1. Symbol table: registers all `let` bindings, rejects duplicates.
// 2. Variable resolution: every VarRef must reference an earlier binding.
// 3. Cycle detection: DFS-based cycle detection on the dependency graph.
// 4. Policy validation: `on_failure retry` requires explicit `retries N > 0`.
//
// Critical Invariant:
//   Dependencies are INFERRED from VarRef fields, not manually declared.
//   The dependency graph drives phase partitioning in the planner.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::{HashMap, HashSet};
use super::ast::*;

#[derive(Debug)]
pub struct AnalyzedStep {
    pub step_id: String,
    pub binding: Option<String>,
    pub call: CallExpr,
    pub deps: HashSet<String>,
}

#[derive(Debug)]
pub struct AnalyzedProgram {
    pub mission_name: String,
    pub steps: Vec<AnalyzedStep>,
}

pub fn analyze(program: &EalProgram) -> anyhow::Result<AnalyzedProgram> {
    let mut symbols: HashMap<String, usize> = HashMap::new();
    let mut steps = Vec::new();
    let mut anon = 0u32;

    for stmt in &program.mission.statements {
        let (binding, call) = match stmt {
            Statement::LetCall { binding, call } => (Some(binding.clone()), call),
            Statement::Call(call) => (None, call),
        };

        if let Some(ref name) = binding {
            anyhow::ensure!(!symbols.contains_key(name), "duplicate binding '{name}'");
        }

        let step_id = binding.clone().unwrap_or_else(|| { anon += 1; format!("__anon_{anon}") });

        // Match MissionControl semantics: Retry policy requires an explicit max_retries > 0.
        if matches!(call.options.on_failure, Some(FailurePolicy::Retry)) {
            let retries = call.options.max_retries.unwrap_or(0);
            anyhow::ensure!(
                retries > 0,
                "step '{step_id}': on_failure retry requires `retries N` with N > 0"
            );
        }

        let mut deps = HashSet::new();
        for field in &call.arguments {
            if let FieldValue::VarRef { var_name } = &field.value {
                anyhow::ensure!(symbols.contains_key(var_name), "step '{step_id}': undefined variable '{var_name}'");
                deps.insert(var_name.clone());
            }
        }

        if let Some(ref name) = binding {
            symbols.insert(name.clone(), steps.len());
        }

        steps.push(AnalyzedStep { step_id, binding, call: call.clone(), deps });
    }

    detect_cycles(&steps)?;
    Ok(AnalyzedProgram { mission_name: program.mission.name.clone(), steps })
}

fn detect_cycles(steps: &[AnalyzedStep]) -> anyhow::Result<()> {
    let id_to_idx: HashMap<&str, usize> = steps.iter().enumerate()
        .filter_map(|(i, s)| s.binding.as_deref().map(|b| (b, i))).collect();
    let mut visited = vec![false; steps.len()];
    let mut in_stack = vec![false; steps.len()];
    for i in 0..steps.len() {
        if !visited[i] { dfs(i, steps, &id_to_idx, &mut visited, &mut in_stack)?; }
    }
    Ok(())
}

fn dfs(idx: usize, steps: &[AnalyzedStep], map: &HashMap<&str, usize>, visited: &mut [bool], in_stack: &mut [bool]) -> anyhow::Result<()> {
    visited[idx] = true;
    in_stack[idx] = true;
    for dep in &steps[idx].deps {
        if let Some(&di) = map.get(dep.as_str()) {
            if !visited[di] { dfs(di, steps, map, visited, in_stack)?; }
            else if in_stack[di] { anyhow::bail!("cycle involving '{}' and '{dep}'", steps[idx].step_id); }
        }
    }
    in_stack[idx] = false;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eal::parser;

    #[test]
    fn deps_inferred() {
        let p = parser::parse(r#"mission "t" { let a = call "x" on "n" let b = call "y" on "n" with { i = a.output } }"#).unwrap();
        let a = analyze(&p).unwrap();
        assert!(a.steps[1].deps.contains("a"));
    }

    #[test]
    fn undefined_ref_rejected() {
        let p = parser::parse(r#"mission "t" { let b = call "y" on "n" with { i = nope.output } }"#).unwrap();
        assert!(analyze(&p).is_err());
    }
}
