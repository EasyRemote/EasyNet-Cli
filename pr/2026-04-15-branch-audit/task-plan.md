# Task Plan Pack — EasyNet CLI vs main full-branch audit

Date: 2026-04-15
Scope: current workspace changes relative to `main` in `EasyNet-Cli`

## Objectives

1. Review and harden code logic, product logic, instruction-level engineering aesthetics, and architecture consistency.
2. Allow local/global refactors where release compatibility is not required.
3. Guarantee Rust checks pass with zero warnings (`clippy -D warnings`).
4. Joint-debug with `../EasyNet` and verify real flow:
   - create user,
   - connect device,
   - discover all agents and abilities under one user.

## Non-negotiable invariants

### Semantic invariants

1. Ability discovery must be tenant-scoped and complete by default (federation-wide when unpinned).
2. Ability invocation must end in deterministic routing semantics:
   - with `node_id`: pinned to node,
   - without `node_id`: runtime auto-route.
3. Node binding in MCP mode must not silently violate schema contracts.
4. A2A labels published at registration must be well-formed and parseable.

### Concurrency and boundedness invariants

1. Bound-node patching must be pure and deterministic for all input shapes.
2. Mission execution must continue to use pooled bridges; no per-call pool leak.

### Layering invariants

1. CLI UX orchestration stays in `cli/*`, contract semantics in `mcp/*`, shared parsing/helpers in `shared/*`.
2. Discovery contract text in MCP specs must match handler behavior and CLI behavior.

## Execution phases

1. Build change map and classify risks by layer.
2. Fix high-impact product/logic defects first.
3. Verify compile/test/lint with strict no-warning gate.
4. Run end-to-end integration checks with `../EasyNet` and capture evidence.
5. Summarize residual risks and next hardening steps.
