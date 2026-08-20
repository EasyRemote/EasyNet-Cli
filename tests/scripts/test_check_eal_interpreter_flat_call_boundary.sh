#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-eal-interpreter-flat-call-boundary.sh"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

SB="$(mktemp -d)"
trap 'rm -rf "$SB"' EXIT

mkdir -p "$SB/tools/scripts" "$SB/src/eal/interpreter" "$SB/src/eal/parser" "$SB/src/eal/runtime"
cp "$SCRIPT" "$SB/tools/scripts/check-eal-interpreter-flat-call-boundary.sh"

cat >"$SB/src/eal/interpreter/mod.rs" <<'RS'
use crate::eal::runtime::ir::{IrEmit, IrEmitValue, MissionIr};
RS

cat >"$SB/src/eal/interpreter/retry.rs" <<'RS'
use super::{millis_u64, RunContext, StepDispatchOutcome, StepDispatcher, StepExecResult};
use crate::eal::runtime::ir::{IrCall, IrFailurePolicy};

pub(super) fn execute_step_with_retry(step: &IrCall) {}
pub(super) fn resolve_arguments(step: &IrCall) {}
pub(super) fn process_step_result(step: &IrCall) {}
RS

cat >"$SB/src/eal/interpreter/phases.rs" <<'RS'
use super::*;
use crate::eal::runtime::ir::{IrCall, IrFailurePolicy, IrLoop, IrStep as RealIrStep};

struct BatchDispatchRequest<'a> {
    steps: &'a [IrCall],
}

fn dependency_receipts_from_captured(step: &IrCall) {}
fn process_batch(steps: &[IrCall]) {}
fn calls_from_partition(steps: &[RealIrStep]) -> Vec<IrCall> {
    Vec::new()
}
RS

cat >"$SB/src/eal/interpreter/tests.rs" <<'RS'
use crate::eal::runtime::ir::{IrCall, IrFailurePolicy};

fn fixture() {
    let step = IrCall {};
}
RS

cat >"$SB/src/eal/parser/ast.rs" <<'RS'
pub enum TargetKind { Device, Agent }
RS

cat >"$SB/src/eal/parser/mod.rs" <<'RS'
#[test]
fn traditional_call_requires_explicit_device_target() {}
RS

cat >"$SB/src/eal/runtime/planner.rs" <<'RS'
pub fn plan() {}
RS

(
  cd "$SB"
  bash tools/scripts/check-eal-interpreter-flat-call-boundary.sh
) >/dev/null || fail "happy path should pass"

cat >>"$SB/src/eal/interpreter/mod.rs" <<'RS'
type IrStep = IrCall;
RS

set +e
(
  cd "$SB"
  bash tools/scripts/check-eal-interpreter-flat-call-boundary.sh
) >/tmp/check-eal-interpreter-flat-call-boundary.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "private alias should exit 1 (got $rc)"
grep -Fq "private IrStep alias" /tmp/check-eal-interpreter-flat-call-boundary.out \
  || fail "alias failure should name private IrStep alias"

cat >"$SB/src/eal/interpreter/mod.rs" <<'RS'
use crate::eal::runtime::ir::{IrEmit, IrEmitValue, MissionIr};
RS
cat >"$SB/src/eal/interpreter/retry.rs" <<'RS'
use super::{millis_u64, IrStep, RunContext, StepDispatchOutcome, StepDispatcher, StepExecResult};
use crate::eal::runtime::ir::IrFailurePolicy;

pub(super) fn execute_step_with_retry(step: &IrStep) {}
RS

set +e
(
  cd "$SB"
  bash tools/scripts/check-eal-interpreter-flat-call-boundary.sh
) >/tmp/check-eal-interpreter-flat-call-boundary.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "super IrStep import should exit 1 (got $rc)"
grep -Fq "must not import an IrStep compatibility alias" \
  /tmp/check-eal-interpreter-flat-call-boundary.out \
  || fail "super import failure should name compatibility alias"

cat >"$SB/src/eal/interpreter/retry.rs" <<'RS'
use super::{millis_u64, RunContext, StepDispatchOutcome, StepDispatcher, StepExecResult};
use crate::eal::runtime::ir::{IrCall, IrFailurePolicy};

pub(super) fn execute_step_with_retry(step: &IrStep) {}
RS

set +e
(
  cd "$SB"
  bash tools/scripts/check-eal-interpreter-flat-call-boundary.sh
) >/tmp/check-eal-interpreter-flat-call-boundary.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "IrStep helper signature should exit 1 (got $rc)"
grep -Fq "must accept IrCall, not IrStep" /tmp/check-eal-interpreter-flat-call-boundary.out \
  || fail "signature failure should name IrCall requirement"

echo "test_check_eal_interpreter_flat_call_boundary.sh: all cases passed"
