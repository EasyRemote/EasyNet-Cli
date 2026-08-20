#!/usr/bin/env bash
set -euo pipefail

ROOT="${CHECK_EAL_INTERPRETER_FLAT_CALL_BOUNDARY_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

fail() {
  printf 'check-eal-interpreter-flat-call-boundary: %s\n' "$1" >&2
  exit 1
}

INTERPRETER_MOD="src/eal/interpreter/mod.rs"
RETRY="src/eal/interpreter/retry.rs"
PHASES="src/eal/interpreter/phases.rs"
TESTS="src/eal/interpreter/tests.rs"
PARSER_AST="src/eal/parser/ast.rs"
PARSER_MOD="src/eal/parser/mod.rs"
PLANNER="src/eal/runtime/planner.rs"

for path in "$INTERPRETER_MOD" "$RETRY" "$PHASES" "$TESTS" "$PARSER_AST" "$PARSER_MOD" "$PLANNER"; do
  [[ -f "$path" ]] || fail "missing $path"
done

if rg -n 'type\s+IrStep\s*=\s*IrCall' "$INTERPRETER_MOD"; then
  fail "interpreter must not keep a private IrStep alias over flat IrCall"
fi

if rg -n 'pre-PR-10 code without a churn-only rename|signature-compatible with the pre-PR-10 code' "$INTERPRETER_MOD"; then
  fail "interpreter comments must not justify compatibility-preserving flat-call aliases"
fi

if rg -n 'use\s+super::\{[^}]*\bIrStep\b' src/eal/interpreter/*.rs; then
  fail "interpreter modules must not import an IrStep compatibility alias from super"
fi

if ! rg -n 'use crate::eal::runtime::ir::\{IrCall, IrFailurePolicy\};' "$RETRY" >/dev/null; then
  fail "retry helpers must import explicit IrCall from the canonical runtime IR module"
fi

if ! rg -n 'use crate::eal::runtime::ir::\{IrCall, IrFailurePolicy, IrLoop, IrStep as RealIrStep\};' "$PHASES" >/dev/null; then
  fail "phase helpers must distinguish flat IrCall from runtime RealIrStep"
fi

if rg -n 'step:\s*&IrStep|steps:\s*&'\''a\s*\[IrStep\]|steps:\s*&\[IrStep\]' "$RETRY" "$PHASES"; then
  fail "per-call retry and batch helpers must accept IrCall, not IrStep"
fi

if ! rg -n 'fn calls_from_partition\(steps: &\[RealIrStep\]\) -> Vec<IrCall>' "$PHASES" >/dev/null; then
  fail "phase partitioning must keep an explicit RealIrStep-to-IrCall lowering seam"
fi

if rg -n 'let step = IrStep \{' "$TESTS"; then
  fail "interpreter unit tests must construct flat IrCall values directly"
fi

if rg -n 'call_expr\s+=\s*"call"\s+STRING\s+\("on"\s+STRING\)\?' "$PARSER_MOD"; then
  fail "traditional EAL call grammar must require an explicit device target"
fi

if rg -n 'let target_node = if \*self\.peek\(\) == Token::On|target_node\.clone\(\)\.unwrap_or_default\(\)|legacy missions|may omit `on' "$PARSER_MOD" "$PLANNER"; then
  fail "EAL traditional call target must fail closed before empty-device fallback"
fi

if rg -n 'enum TargetKind|derive\(Debug, Clone, Copy, PartialEq, Eq, Default\)|#\[default\]' "$PARSER_AST" \
  | grep -E 'Default|#\[default\]' >/dev/null; then
  fail "EAL TargetKind must not carry a default target kind"
fi

if ! rg -n 'traditional_call_requires_explicit_device_target' "$PARSER_MOD" >/dev/null; then
  fail "parser tests must pin explicit target requirement for traditional calls"
fi

echo "check-eal-interpreter-flat-call-boundary: ok"
