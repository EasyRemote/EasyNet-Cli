#!/usr/bin/env bash
set -euo pipefail

DEFAULT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
ROOT="${CHECK_GO_PLUGINEXEC_PROJECTION_ROOT:-$DEFAULT_ROOT}"
cd "$ROOT"

fail() {
  printf 'check-go-pluginexec-projection-boundary: %s\n' "$1" >&2
  exit 1
}

SOURCE="sdk/go/provider/runtime/pluginexec/pluginexec.go"
TESTS="sdk/go/provider/runtime/pluginexec/pluginexec_test.go"

[[ -f "$SOURCE" ]] || fail "missing $SOURCE"
[[ -f "$TESTS" ]] || fail "missing $TESTS"

if ! rg -n 'type sidecarInvocationProjection struct' "$SOURCE" >/dev/null; then
  fail "Go pluginexec must use an explicit sidecarInvocationProjection object"
fi

for method in \
  'func \(p sidecarInvocationProjection\) project\(\) \(SidecarInvocation, error\)' \
  'func \(p sidecarInvocationProjection\) validateFrameType\(\) error' \
  'func \(p sidecarInvocationProjection\) validateTupleStrings\(\) error' \
  'func \(p sidecarInvocationProjection\) validateNonce\(\) error' \
  'func \(p sidecarInvocationProjection\) validateObjects\(\) error' \
  'func \(p sidecarInvocationProjection\) intoInvocation\(\) SidecarInvocation'
do
  if ! rg -n "$method" "$SOURCE" >/dev/null; then
    fail "sidecarInvocationProjection missing method: $method"
  fi
done

if ! python3 - "$SOURCE" <<'PY'
import re
import sys
from pathlib import Path

body = Path(sys.argv[1]).read_text()
project = re.search(
    r"func \(f requestFrame\) projectInvocation\(\) \(SidecarInvocation, error\) \{(?P<body>.*?)\n\}",
    body,
    re.S,
)
if not project:
    raise SystemExit("request_project_invocation_missing")
project_body = project.group("body")
if "sidecarInvocationProjection{" not in project_body or ".project()" not in project_body:
    raise SystemExit("request_projection_bypasses_sidecar_invocation_projection")
if re.search(r"func \(f sidecarInvocationFrame\) project\(", body):
    raise SystemExit("sidecar_invocation_frame_project_method_retired")
PY
then
  fail "Go pluginexec projection boundary is not centralized"
fi

for test in \
  TestSidecarInvocationProjectionCopiesNonceAndRejectsMutation \
  TestSidecarInvocationProjectionRejectsNonInvokeFrame
do
  if ! rg -n "func $test" "$TESTS" >/dev/null; then
    fail "missing Go projection regression test: $test"
  fi
done

echo "check-go-pluginexec-projection-boundary: ok"
