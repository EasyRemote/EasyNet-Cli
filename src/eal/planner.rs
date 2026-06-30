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
    let analyzed_mission = analyze_mission(program)?;
    let analyzed = analyzed_mission.items;

    // Loops are sequential blocks with their own internal iteration
    // semantics (RFC §3.1); they are not participants in the outer
    // phase scheduler's parallel-when-independent logic. If any top-
    // level item is a Loop, collapse the outer schedule to one-step-
    // per-phase in source order. Pure-Call missions keep the parallel-
    // phase scheduling they had before PR-10 — the `Call`-only trace
    // fixtures consumed by `scripts/trace-parity.sh` are unaffected.
    let has_block = analyzed.iter().any(|a| matches!(a, AnalyzedItem::Loop(_)));

    let (ordered, phase_of, num_phases) = if has_block {
        let ordered: Vec<usize> = (0..analyzed.len()).collect();
        let phase_of: Vec<usize> = (0..analyzed.len()).collect();
        let num_phases = analyzed.len();
        (ordered, phase_of, num_phases)
    } else {
        let phase_of = assign_phases(&analyzed);
        let num_phases = phase_of.iter().copied().max().map(|p| p + 1).unwrap_or(0);
        let mut ordered: Vec<usize> = (0..analyzed.len()).collect();
        ordered.sort_by_key(|&i| phase_of[i]);
        (ordered, phase_of, num_phases)
    };

    let mut ir_steps = Vec::new();
    let mut phases = Vec::new();
    let mut cur_phase = 0usize;
    let mut phase_start = 0usize;

    // Consume `analyzed` in original index order into a Vec<Option> so
    // `lower_item` can take ownership. The schedule-order walk then
    // takes each item exactly once. (`ordered` may permute indices
    // away from source order when the Call-only phase scheduler
    // fires, so a simple `into_iter` over `analyzed` would lower the
    // wrong item at the wrong index.)
    let mut slots: Vec<Option<AnalyzedItem>> = analyzed.into_iter().map(Some).collect();

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
        let item = slots[orig_idx]
            .take()
            .expect("each AnalyzedItem index visited exactly once");
        ir_steps.push(lower_item(item)?);
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

    // Worst-case static call-count bound (RFC §4.1). Applies across
    // the entire IR tree, counting every leaf `IrCall` under every
    // `IrStep::Loop` as `max_iters * n_calls`.
    let total_bound: u64 = ir_steps.iter().map(|s| s.static_call_bound()).sum();
    let cap = IrConstraints::default_max_calls();
    anyhow::ensure!(
        total_bound <= cap,
        "mission '{}' worst-case static call count {total_bound} exceeds cap {cap} \
         (RFC §4.1); reduce `max_iters` or shrink the loop body",
        program.mission.name
    );

    Ok(MissionIr {
        name: program.mission.name.clone(),
        steps: ir_steps,
        phases,
        emits: analyzed_mission.emits,
        constraints: IrConstraints::default(),
    })
}

fn analyze_mission(program: &EalProgram) -> anyhow::Result<AnalyzedMission> {
    let mut anon_counter = 0u32;
    analyze_statements_inner(&program.mission.statements, &mut anon_counter, &[], true)
}

#[derive(Debug)]
struct AnalyzedMission {
    items: Vec<AnalyzedItem>,
    emits: Vec<IrEmit>,
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

/// Top-level mission item — either a flat call (`let x = a.b(...)`) or
/// a block form. `loop` is the only block form in v1; `chat` /
/// `handoff` were removed by the approved RFC (see RFC §10 and
/// `docs/open-questions/discuss-eal-block.md`).
#[derive(Debug)]
enum AnalyzedItem {
    Call(AnalyzedStep),
    Loop(AnalyzedLoop),
}

impl AnalyzedItem {
    /// Binding this item exports to the enclosing scope, if any.
    /// Calls export their `let` binding; Loops export `<name>.result`
    /// via `result_binding`.
    fn binding(&self) -> Option<&str> {
        match self {
            AnalyzedItem::Call(c) => c.binding.as_deref(),
            AnalyzedItem::Loop(l) => l.result_binding.as_deref(),
        }
    }

    /// Bindings this item references from the enclosing scope.
    /// Calls see their own `input_refs`; Loops are hermetic in v1
    /// (RFC §3.1: inner bindings do not leak out; v1 also takes the
    /// conservative reading that outer bindings are not visible
    /// inside). A future RFC may loosen the inbound side.
    fn outer_deps(&self) -> &HashSet<String> {
        static EMPTY: std::sync::OnceLock<HashSet<String>> = std::sync::OnceLock::new();
        match self {
            AnalyzedItem::Call(c) => &c.deps,
            AnalyzedItem::Loop(_) => EMPTY.get_or_init(HashSet::new),
        }
    }
}

/// Pre-lowered Loop block. Body and verify sub-statements are already
/// analysed + lowered to `Vec<IrStep>` in a fresh inner scope (no
/// outer bindings visible, inner bindings not leaked except via
/// `<name>.result`).
#[derive(Debug)]
struct AnalyzedLoop {
    name: Option<String>,
    max_iters: u32,
    body: Vec<IrStep>,
    verify: Vec<IrStep>,
    /// Binding exported to the enclosing scope. `Some("<name>.result")`
    /// for a named loop, `None` for an anonymous loop.
    result_binding: Option<String>,
}

/// First pass: semantic analysis. Walks the AST once, builds the
/// symbol table + dependency graph, rejects every semantic error
/// the language guarantees to catch before IR lowering.
#[cfg(test)]
fn analyze(program: &EalProgram) -> anyhow::Result<Vec<AnalyzedItem>> {
    Ok(analyze_mission(program)?.items)
}

/// Worker shared between the top-level mission and a loop's inner
/// `body` / `verify` blocks. `anon_counter` is threaded through so
/// `__anon_N` ids remain unique across the whole compile.
fn analyze_statements(
    stmts: &[Statement],
    anon_counter: &mut u32,
) -> anyhow::Result<Vec<AnalyzedItem>> {
    Ok(analyze_statements_inner(stmts, anon_counter, &[], false)?.items)
}

/// Same as `analyze_statements` but seeds the local symbol table
/// with the given binding names (mapped to sentinel indices outside
/// the returned items — they represent references to items that
/// live elsewhere, e.g. body bindings visible in verify).
///
/// The seed bindings participate in "is this VarRef defined?" checks
/// but do NOT land in the returned items' dependency graph, because
/// the returned items are a standalone block that the planner
/// lowers as a self-contained slice. The cross-block dependency
/// (verify on body) is expressed at execution time by the shared
/// `iter_captured` scope in `execute_loop` — not at the IR layer.
fn analyze_statements_with_seed(
    stmts: &[Statement],
    anon_counter: &mut u32,
    seed_bindings: &[String],
) -> anyhow::Result<Vec<AnalyzedItem>> {
    Ok(analyze_statements_inner(stmts, anon_counter, seed_bindings, false)?.items)
}

fn analyze_statements_inner(
    stmts: &[Statement],
    anon_counter: &mut u32,
    seed_bindings: &[String],
    allow_emit: bool,
) -> anyhow::Result<AnalyzedMission> {
    let mut symbols: HashMap<String, usize> = HashMap::new();
    // Sentinel indices `usize::MAX - i` for seed bindings so they
    // cannot collide with real item indices (0..items.len()). The
    // `item_idx` passed to `analyze_call` still uses the real
    // running count of items; only lookup for VarRef resolution
    // consults the sentinel entries.
    for (i, b) in seed_bindings.iter().enumerate() {
        symbols.insert(b.clone(), usize::MAX - i);
    }
    let mut items: Vec<AnalyzedItem> = Vec::new();
    let mut emits: Vec<IrEmit> = Vec::new();

    for stmt in stmts {
        match stmt {
            Statement::LetCall { binding, call } => {
                let step = analyze_call(
                    Some(binding.clone()),
                    call,
                    &mut symbols,
                    anon_counter,
                    items.len(),
                )?;
                items.push(AnalyzedItem::Call(step));
            }
            Statement::Call(call) => {
                let step = analyze_call(None, call, &mut symbols, anon_counter, items.len())?;
                items.push(AnalyzedItem::Call(step));
            }
            Statement::Loop(l) => {
                let analyzed = analyze_loop(l, anon_counter)?;
                if let Some(ref b) = analyzed.result_binding {
                    anyhow::ensure!(
                        !symbols.contains_key(b),
                        "duplicate binding '{b}' from loop result"
                    );
                    symbols.insert(b.clone(), items.len());
                }
                items.push(AnalyzedItem::Loop(analyzed));
            }
            Statement::Emit(e) => {
                anyhow::ensure!(
                    allow_emit,
                    "emit '{}' is only supported at top-level mission scope in EAL v1",
                    e.name
                );
                emits.push(analyze_emit(e, &symbols)?);
            }
        }
    }

    detect_cycles(&items)?;
    Ok(AnalyzedMission { items, emits })
}

/// Analyse one `Statement::Loop` → `AnalyzedLoop`.
///
/// Enforces RFC §3.1 / §4.2 / §4.4 / §5.1:
///   - `max_iters ∈ [1, 32]`.
///   - `body` and `verify` each non-empty.
///   - `verify` last statement is a call (so its output carries
///     `done: bool`).
///   - Nested loops rejected (v1 — RFC §4.2).
///   - Body and verify analysed in a fresh inner scope (hermetic).
fn analyze_loop(l: &LoopBlock, anon_counter: &mut u32) -> anyhow::Result<AnalyzedLoop> {
    let label = l.name.as_deref().unwrap_or("<anonymous>");

    anyhow::ensure!(
        l.max_iters >= 1,
        "loop '{label}': max_iters must be ≥ 1 (RFC §3.1)"
    );
    anyhow::ensure!(
        l.max_iters <= 32,
        "loop '{label}': max_iters must be ≤ 32 (RFC §3.1)"
    );
    anyhow::ensure!(
        !l.body.is_empty(),
        "loop '{label}': `body` sub-block must contain at least one statement"
    );
    anyhow::ensure!(
        !l.verify.is_empty(),
        "loop '{label}': `verify` sub-block must contain at least one statement (RFC §4.4)"
    );

    // v1: reject nested loops (RFC §4.2). A nested loop multiplies
    // the static call bound; the gallery has no mission needing it.
    for stmt in l.body.iter().chain(l.verify.iter()) {
        if let Statement::Loop(inner) = stmt {
            anyhow::bail!(
                "loop '{label}': nested loops are not supported in v1 \
                 (inner loop{} — RFC §4.2 static depth cap)",
                inner
                    .name
                    .as_ref()
                    .map(|n| format!(" '{n}'"))
                    .unwrap_or_default()
            );
        }
    }

    // Hermetic scope vs outer mission (RFC §3.1): outer bindings are
    // not visible inside `body` / `verify`. Within the loop, `verify`
    // does see `body`'s bindings — "not visible outside the loop"
    // does not mean "not visible to verify", since verify is inside
    // the loop. So we analyse body and verify in a **shared inner
    // scope** that starts empty (outer hidden) and accumulates body's
    // bindings before verify runs.
    let body_items = analyze_statements(&l.body, anon_counter)?;
    // Build a seed symbol table from body's exported bindings so
    // verify can reference them. `analyze_statements` builds its own
    // local symbol table from scratch, so we instead re-run the
    // verify analyse with a pre-populated symbol table.
    let body_bindings: Vec<String> = body_items
        .iter()
        .filter_map(|it| it.binding().map(|s| s.to_string()))
        .collect();
    let verify_items = analyze_statements_with_seed(&l.verify, anon_counter, &body_bindings)?;

    // RFC §4.4: the last statement in `verify` must be a call whose
    // output carries the termination predicate `done: bool`.
    let last_verify_is_call = verify_items
        .last()
        .is_some_and(|it| matches!(it, AnalyzedItem::Call(_)));
    anyhow::ensure!(
        last_verify_is_call,
        "loop '{label}': the last statement of `verify` must be an ability call \
         (RFC §4.4 — its output carries the `done: bool` termination predicate)"
    );

    let body_ir = body_items
        .into_iter()
        .map(lower_item)
        .collect::<anyhow::Result<Vec<_>>>()?;
    let verify_ir = verify_items
        .into_iter()
        .map(lower_item)
        .collect::<anyhow::Result<Vec<_>>>()?;

    let result_binding = l.name.as_ref().map(|n| format!("{n}.result"));

    Ok(AnalyzedLoop {
        name: l.name.clone(),
        max_iters: l.max_iters,
        body: body_ir,
        verify: verify_ir,
        result_binding,
    })
}

fn analyze_call(
    binding: Option<String>,
    call: &CallExpr,
    symbols: &mut HashMap<String, usize>,
    anon: &mut u32,
    item_idx: usize,
) -> anyhow::Result<AnalyzedStep> {
    if let Some(ref name) = binding {
        anyhow::ensure!(!symbols.contains_key(name), "duplicate binding '{name}'");
    }

    let step_id = match &binding {
        Some(name) => name.clone(),
        None => {
            *anon += 1;
            format!("__anon_{n}", n = *anon)
        }
    };

    if matches!(call.options.on_failure, Some(FailurePolicy::Retry)) {
        let retries = call.options.max_retries.unwrap_or(0);
        anyhow::ensure!(
            retries > 0,
            "step '{step_id}': on_failure retry requires `retries N` with N > 0"
        );
    }

    anyhow::ensure!(
        !(call.options.optional && matches!(call.options.on_failure, Some(FailurePolicy::Abort))),
        "step '{step_id}': `optional` and `on_failure abort` are contradictory; \
         pick one (use `optional` for best-effort, `on_failure abort` for mission-critical)"
    );

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
        symbols.insert(name.clone(), item_idx);
    }

    Ok(AnalyzedStep {
        step_id,
        binding,
        call: call.clone(),
        deps,
    })
}

fn analyze_emit(emit: &EmitStatement, symbols: &HashMap<String, usize>) -> anyhow::Result<IrEmit> {
    if let FieldValue::VarRef { var_name } = &emit.value {
        anyhow::ensure!(
            symbols.contains_key(var_name),
            "emit '{}': undefined variable '{var_name}'",
            emit.name
        );
    }
    Ok(IrEmit {
        name: emit.name.clone(),
        kind: emit.kind.clone(),
        value: emit_value_to_ir(&emit.value)?,
    })
}

fn emit_value_to_ir(value: &FieldValue) -> anyhow::Result<IrEmitValue> {
    match value {
        FieldValue::VarRef { var_name } => Ok(IrEmitValue::Binding {
            binding: var_name.clone(),
        }),
        other => Ok(IrEmitValue::Literal {
            value: field_value_to_json(other)?,
        }),
    }
}

fn detect_cycles(items: &[AnalyzedItem]) -> anyhow::Result<()> {
    let id_to_idx: HashMap<&str, usize> = items
        .iter()
        .enumerate()
        .filter_map(|(i, it)| it.binding().map(|b| (b, i)))
        .collect();
    let mut visited = vec![false; items.len()];
    let mut in_stack = vec![false; items.len()];
    for i in 0..items.len() {
        if !visited[i] {
            dfs(i, items, &id_to_idx, &mut visited, &mut in_stack)?;
        }
    }
    Ok(())
}

fn dfs(
    idx: usize,
    items: &[AnalyzedItem],
    map: &HashMap<&str, usize>,
    visited: &mut [bool],
    in_stack: &mut [bool],
) -> anyhow::Result<()> {
    visited[idx] = true;
    in_stack[idx] = true;
    for dep in items[idx].outer_deps() {
        if let Some(&di) = map.get(dep.as_str()) {
            if !visited[di] {
                dfs(di, items, map, visited, in_stack)?;
            } else if in_stack[di] {
                let here = match &items[idx] {
                    AnalyzedItem::Call(c) => c.step_id.as_str(),
                    AnalyzedItem::Loop(l) => l.name.as_deref().unwrap_or("<anonymous-loop>"),
                };
                anyhow::bail!("cycle involving '{here}' and '{dep}'");
            }
        }
    }
    in_stack[idx] = false;
    Ok(())
}

fn assign_phases(items: &[AnalyzedItem]) -> Vec<usize> {
    let binding_to_idx: HashMap<&str, usize> = items
        .iter()
        .enumerate()
        .filter_map(|(i, it)| it.binding().map(|b| (b, i)))
        .collect();
    let mut phase = vec![0usize; items.len()];
    for (i, it) in items.iter().enumerate() {
        let mut max = 0usize;
        let mut has = false;
        for dep in it.outer_deps() {
            if let Some(&di) = binding_to_idx.get(dep.as_str()) {
                has = true;
                max = max.max(phase[di]);
            }
        }
        phase[i] = if has { max + 1 } else { 0 };
    }
    phase
}

fn lower_item(item: AnalyzedItem) -> anyhow::Result<IrStep> {
    match item {
        AnalyzedItem::Call(s) => lower_call(&s),
        AnalyzedItem::Loop(l) => Ok(IrStep::Loop(IrLoop {
            kind: IrLoopTag,
            name: l.name,
            max_iters: l.max_iters,
            body: l.body,
            verify: l.verify,
            result_binding: l.result_binding,
        })),
    }
}

/// Convert a non-VarRef `FieldValue` to the JSON representation
/// `IrStep` carries in `static_args`. Pulled out as a helper so the
/// nested-object case can recurse — without it, an `Object` field
/// holding a nested `Object` would have to grow a copy of the
/// scalar match arms.
///
/// VarRef intentionally NOT supported here: the planner threads
/// VarRef through `input_refs` so the dispatcher resolves the
/// dependency at run time, not the planner. A VarRef inside a nested
/// object would lose its dependency edge — error out loud rather
/// than silently inline a placeholder.
fn field_value_to_json(v: &FieldValue) -> anyhow::Result<serde_json::Value> {
    match v {
        FieldValue::String(s) => Ok(serde_json::json!(s)),
        FieldValue::Int(n) => Ok(serde_json::json!(n)),
        FieldValue::Float(f) => Ok(serde_json::json!(f)),
        FieldValue::Bool(b) => Ok(serde_json::json!(b)),
        FieldValue::Object(fields) => {
            let mut map = serde_json::Map::with_capacity(fields.len());
            for f in fields {
                if matches!(f.value, FieldValue::VarRef { .. }) {
                    anyhow::bail!(
                        "field '{}' uses `<var>.output` inside a nested object, \
                         which is not supported (variable references must be \
                         top-level args so the planner can wire dependency edges)",
                        f.key
                    );
                }
                map.insert(f.key.clone(), field_value_to_json(&f.value)?);
            }
            Ok(serde_json::Value::Object(map))
        }
        FieldValue::VarRef { var_name } => {
            anyhow::bail!(
                "internal: `<var>.output` ({var_name}) reached field_value_to_json; \
                 should have been handled by the top-level VarRef arm in lower_call"
            );
        }
    }
}

fn lower_call(step: &AnalyzedStep) -> anyhow::Result<IrStep> {
    use crate::core::agent_id::{AbilityName, AgentId};

    let mut static_args = serde_json::Map::new();
    let mut input_refs = BTreeMap::new();
    for f in &step.call.arguments {
        match &f.value {
            FieldValue::VarRef { var_name } => {
                input_refs.insert(f.key.clone(), var_name.clone());
            }
            other => {
                static_args.insert(f.key.clone(), field_value_to_json(other)?);
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

    Ok(IrStep::Call(IrCall {
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
    }))
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
        match &steps[1] {
            AnalyzedItem::Call(c) => assert!(c.deps.contains("a")),
            AnalyzedItem::Loop(_) => panic!("expected Call at index 1"),
        }
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
        let p = parser::parse(r#"mission "t" { let a = call "x" on "n" let a = call "y" on "n" }"#)
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
        let p = parser::parse(r#"mission "t" { call "x" on "n" on_failure retry }"#).unwrap();
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
            parser::parse(r#"mission "t" { call "x" on "n" optional on_failure abort }"#).unwrap();
        let err = analyze(&p).unwrap_err();
        assert!(
            format!("{err}").contains("contradictory"),
            "expected 'contradictory' in error, got: {err}"
        );
    }

    #[test]
    fn optional_plus_on_failure_skip_is_allowed() {
        let p =
            parser::parse(r#"mission "t" { call "x" on "n" optional on_failure skip }"#).unwrap();
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
        let p = parser::parse(r#"mission "t" { let a = call "x" on "n" with { i = a.output } }"#)
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
    fn emit_lowers_to_ir_without_adding_a_step() {
        let ir = compile(
            &parser::parse(
                r#"mission "t" {
                    let rows = alice.map(prompt: "x")
                    emit "terminal_rows" kind answer value rows.output
                    emit "note" kind context value "static"
                }"#,
            )
            .unwrap(),
        )
        .unwrap();

        assert_eq!(ir.steps.len(), 1, "emit must not become an executable step");
        assert_eq!(ir.emits.len(), 2);
        assert_eq!(ir.emits[0].name, "terminal_rows");
        assert_eq!(ir.emits[0].kind, "answer");
        match &ir.emits[0].value {
            IrEmitValue::Binding { binding } => assert_eq!(binding, "rows"),
            other => panic!("expected binding emit, got {other:?}"),
        }
        match &ir.emits[1].value {
            IrEmitValue::Literal { value } => assert_eq!(value, "static"),
            other => panic!("expected literal emit, got {other:?}"),
        }
    }

    #[test]
    fn emit_forward_reference_is_rejected() {
        let p = parser::parse(
            r#"mission "t" {
                emit "terminal_rows" kind answer value rows.output
                let rows = alice.map(prompt: "x")
            }"#,
        )
        .unwrap();
        let err = compile(&p).unwrap_err().to_string();
        assert!(
            err.contains("undefined variable 'rows'"),
            "emit must not forward-reference future bindings; got: {err}"
        );
    }

    #[test]
    fn emit_inside_loop_is_rejected_for_v1() {
        let p = parser::parse(
            r#"mission "t" {
                loop max_iters: 2 {
                    body {
                        let rows = alice.map(prompt: "x")
                        emit "rows" kind answer value rows.output
                    }
                    verify { alice.check(prompt: "ok") }
                }
            }"#,
        )
        .unwrap();
        let err = compile(&p).unwrap_err().to_string();
        assert!(
            err.contains("only supported at top-level mission scope"),
            "loop-local emit must be rejected until scoped semantics exist; got: {err}"
        );
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
                // PR-10: this planner test only exercises Call-shape
                // missions — the phase/data-flow partitioning
                // invariant is defined for flat steps. Block
                // variants never appear in these fixtures; skipping
                // them with `as_call()` keeps the test tight.
                for step in &ir.steps[range.start..range.end] {
                    let Some(call) = step.as_call() else { continue };
                    for binding in call.input_refs.values() {
                        let dep_phase = binding_phase.get(binding).copied().unwrap_or_else(|| {
                            panic!(
                                "example {i}: step '{}' references unknown binding '{}'",
                                call.step_id, binding
                            )
                        });
                        assert!(
                            dep_phase < phase_idx,
                            "example {i}: step '{}' consumes '{}' from phase {}, \
                             but itself lives in phase {} — same-phase data flow \
                             would race under parallel dispatch",
                            call.step_id,
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
                    let Some(call) = step.as_call() else { continue };
                    if let Some(b) = &call.output_binding {
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

    // ── PR-10 Stage 3: loop planner audit hooks ────────────────────────

    /// RFC §3.1: `max_iters` is an explicit, bounded budget. The
    /// parser already enforces presence (`max_iters:` is required
    /// header syntax); the planner enforces the RFC's numeric
    /// bound `[1, 32]`. Both boundary values are rejected.
    #[test]
    fn loop_max_iters_out_of_range_rejected() {
        // max_iters: 0 is rejected.
        let p0 = parser::parse(
            r#"mission "t" { loop max_iters: 0 { body { a.x(p: "x") } verify { a.y(p: "y") } } }"#,
        )
        .unwrap();
        let err0 = compile(&p0).unwrap_err().to_string();
        assert!(err0.contains("max_iters must be ≥ 1"), "got: {err0}");

        // max_iters: 33 is rejected.
        let p33 = parser::parse(
            r#"mission "t" { loop max_iters: 33 { body { a.x(p: "x") } verify { a.y(p: "y") } } }"#,
        )
        .unwrap();
        let err33 = compile(&p33).unwrap_err().to_string();
        assert!(err33.contains("max_iters must be ≤ 32"), "got: {err33}");
    }

    /// RFC §4.2 v1: nested loops are compile-time errors. The
    /// motivating gallery mission does not need nesting; a future
    /// RFC may lift this cap, but v1 refuses.
    #[test]
    fn nested_loops_rejected_at_compile_time() {
        let src = r#"
            mission "t" {
                loop "outer" max_iters: 2 {
                    body {
                        loop "inner" max_iters: 2 {
                            body { a.x(p: "x") }
                            verify { a.y(p: "y") }
                        }
                    }
                    verify { a.z(p: "z") }
                }
            }"#;
        let p = parser::parse(src).unwrap();
        let err = compile(&p).unwrap_err().to_string();
        assert!(err.contains("nested loops are not supported"), "got: {err}");
    }

    /// Pins RFC §4.4's non-empty-verify invariant. An empty
    /// `verify { }` is rejected at either the parser (grammar) or
    /// planner (`analyze_loop` non-empty check); this test asserts
    /// only that compilation fails, not which layer catches it.
    ///
    /// The separate §5.1 "last statement of `verify` must be a
    /// call" guard in `analyze_loop` is defence-in-depth: today's
    /// grammar makes every `Statement` either a call (`LetCall` /
    /// `Call`) or a `Loop` block, and nested loops are rejected
    /// earlier. So the `last_verify_is_call` check has no
    /// dedicated test today — the grammar guarantees the invariant
    /// by construction. If a future RFC adds a non-call statement
    /// form (e.g. `if`, `assign`), add a test there.
    #[test]
    fn loop_verify_empty_rejected() {
        let src = r#"
            mission "t" {
                loop max_iters: 2 {
                    body { a.x(p: "x") }
                    verify { }
                }
            }"#;
        // Parser already bails on an empty verify block at the
        // `{}` boundary; the planner's own check is a defence in
        // depth. Either layer rejecting is fine — we just assert
        // it does not compile.
        let err = parser::parse(src)
            .and_then(|p| compile(&p))
            .unwrap_err()
            .to_string();
        assert!(!err.is_empty(), "empty verify must be rejected");
    }

    /// Hermetic scope (RFC §3.1 v1): outer mission `let` bindings
    /// are NOT visible inside loop body/verify. A step inside the
    /// body referencing an outer binding must be rejected as an
    /// undefined variable, same error category as a typo.
    #[test]
    fn hermetic_scope_outer_binding_not_visible_inside_body() {
        let src = r#"
            mission "t" {
                let outer = a.seed(p: "seed")
                loop max_iters: 2 {
                    body { let r = a.use(p: outer.output) }
                    verify { a.check(of: r.output) }
                }
            }"#;
        let p = parser::parse(src).unwrap();
        let err = compile(&p).unwrap_err().to_string();
        assert!(
            err.contains("undefined variable") && err.contains("outer"),
            "outer binding must be invisible to loop body; got: {err}"
        );
    }

    /// Planner emits `IrStep::Loop` with the supplied max_iters and
    /// `<name>.result` binding when the loop is named. Static call
    /// bound equals max_iters * (|body calls| + |verify calls|)
    /// per RFC §4.1.
    #[test]
    fn loop_lowers_to_ir_loop_with_result_binding_and_bound() {
        let src = r#"
            mission "t" {
                loop "review" max_iters: 3 {
                    body {
                        let a = rev.review(p: "x")
                        let b = res.fix(p: a.output)
                    }
                    verify { rev.ok(of: b.output) }
                }
            }"#;
        let p = parser::parse(src).unwrap();
        let ir = compile(&p).unwrap();
        assert_eq!(ir.steps.len(), 1);
        match &ir.steps[0] {
            IrStep::Loop(l) => {
                assert_eq!(l.max_iters, 3);
                assert_eq!(l.body.len(), 2);
                assert_eq!(l.verify.len(), 1);
                assert_eq!(l.result_binding.as_deref(), Some("review.result"));
            }
            IrStep::Call(_) => panic!("expected IrStep::Loop"),
        }
        // max_iters (3) * (body 2 + verify 1) = 9.
        assert_eq!(ir.steps[0].static_call_bound(), 9);
    }

    /// RFC §4.1: mission whose worst-case static call count exceeds
    /// the cap (`IrConstraints::default_max_calls() == 256`) is
    /// rejected at compile time. A 32-iter loop with 9 inner calls
    /// = 288 > 256.
    #[test]
    fn static_call_bound_over_cap_rejected() {
        let src = r#"
            mission "t" {
                loop max_iters: 32 {
                    body {
                        a.s1(p: "x") a.s2(p: "x") a.s3(p: "x")
                        a.s4(p: "x") a.s5(p: "x") a.s6(p: "x")
                        a.s7(p: "x") a.s8(p: "x")
                    }
                    verify { a.ok(p: "x") }
                }
            }"#;
        let p = parser::parse(src).unwrap();
        let err = compile(&p).unwrap_err().to_string();
        assert!(
            err.contains("worst-case static call count") && err.contains("exceeds cap"),
            "got: {err}"
        );
    }
}
