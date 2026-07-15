#!/usr/bin/env bash
set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SOURCE_ROOT="$(cd "$SELF_DIR/../.." && pwd)"
REPO_ROOT="$SOURCE_ROOT"
CARGO_BIN="${CARGO:-cargo}"
REPORT_BUILD_TIMEOUT_SECONDS="${SDK_CONFORMANCE_REPORT_BUILD_TIMEOUT_SECONDS:-300}"
REPORT_TIMEOUT_SECONDS="${SDK_CONFORMANCE_REPORT_TIMEOUT_SECONDS:-900}"
REPORT_TARGET_DIR="${SDK_CONFORMANCE_REPORT_TARGET_DIR:-$REPO_ROOT/target/sdk-conformance-reports}"
RESULT_DIR="${SDK_CONFORMANCE_RESULT_DIR:-$REPO_ROOT/target/sdk-conformance-live-results}"
TMP_DIR=""
SELF_TEST_TMP=""
SNAPSHOT_ROOT=""
SNAPSHOT_TREE_SHA256=""
RUNNER_BIN="$REPORT_TARGET_DIR/debug/sdk-conformance-runner"
RUNNER_BUILT=0
RUN_NONCE=""
REQUESTED_LANGUAGES="${SDK_CONFORMANCE_LANGUAGES:-}"

if [[ -n "$REQUESTED_LANGUAGES" ]]; then
  IFS=',' read -r -a requested_language_list <<<"$REQUESTED_LANGUAGES"
  normalized=""
  for requested in "${requested_language_list[@]}"; do
    case "$requested" in
      rust|c_abi|go|python|node|java|swift) ;;
      *)
        echo "check-sdk-conformance-reports: unknown language slice entry: $requested" >&2
        exit 1
        ;;
    esac
    if [[ ",$normalized," == *",$requested,"* ]]; then
      echo "check-sdk-conformance-reports: duplicate language slice entry: $requested" >&2
      exit 1
    fi
    normalized="${normalized:+$normalized,}$requested"
  done
  REQUESTED_LANGUAGES="$normalized"
fi

if ! command -v "$CARGO_BIN" >/dev/null 2>&1; then
  if [[ -x "$HOME/.cargo/bin/cargo" ]]; then
    CARGO_BIN="$HOME/.cargo/bin/cargo"
  fi
fi

cleanup() {
  if [[ -n "$TMP_DIR" ]]; then
    rm -rf "$TMP_DIR" 2>/dev/null || true
  fi
  if [[ -n "$SELF_TEST_TMP" ]]; then
    rm -rf "$SELF_TEST_TMP" 2>/dev/null || true
  fi
  if [[ -n "$SNAPSHOT_ROOT" ]]; then
    rm -rf "$SNAPSHOT_ROOT" 2>/dev/null || true
  fi
}

ensure_tmp_dir() {
  if [[ -z "$TMP_DIR" ]]; then
    mkdir -p "$REPO_ROOT/target"
    TMP_DIR="$(mktemp -d "$REPO_ROOT/target/sdk-conformance-report-output.XXXXXX")"
    trap cleanup EXIT
  fi
}

create_source_snapshot() {
  if [[ -n "$SNAPSHOT_ROOT" ]]; then
    return 0
  fi
  local parent
  parent="$(dirname "$SOURCE_ROOT")"
  SNAPSHOT_ROOT="$(mktemp -d "$parent/.easynet-cli-conformance-snapshot.XXXXXX")"
  trap cleanup EXIT
  echo "sdk-conformance-reports: capturing source snapshot $SNAPSHOT_ROOT" >&2
  SNAPSHOT_TREE_SHA256="$(python3 - "$SOURCE_ROOT" "$SNAPSHOT_ROOT" <<'PY'
from __future__ import annotations

import hashlib
import os
import shutil
import subprocess
import sys
from pathlib import Path

source = Path(sys.argv[1])
snapshot = Path(sys.argv[2])


def git_paths(root: Path) -> list[str]:
    output = subprocess.run(
        ["git", "ls-files", "-co", "--exclude-standard", "-z"],
        cwd=root,
        check=True,
        capture_output=True,
    ).stdout
    return sorted(raw.decode() for raw in output.split(b"\0") if raw)


def tree_digest(root: Path) -> str:
    material = bytearray()
    for path_text in git_paths(root):
        raw = path_text.encode()
        material.extend(raw)
        material.append(0)
        path = root / path_text
        material.extend(path.read_bytes() if path.is_file() else b"<deleted>")
        material.append(0)
    return hashlib.sha256(material).hexdigest()


paths = git_paths(source)
if not paths:
    raise SystemExit("source snapshot would be empty")

for path_text in paths:
    src = source / path_text
    dst = snapshot / path_text
    if src.is_symlink():
        dst.parent.mkdir(parents=True, exist_ok=True)
        os.symlink(os.readlink(src), dst)
    elif src.is_file():
        dst.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(src, dst)

subprocess.run(["git", "init", "-q"], cwd=snapshot, check=True)
source_git = source / ".git"
snapshot_git = snapshot / ".git"
if source_git.is_file():
    raise SystemExit("source snapshot requires a non-worktree .git directory")
index = source_git / "index"
if not index.is_file():
    raise SystemExit("source git index is required for snapshot inventory")
shutil.copy2(index, snapshot_git / "index")
source_exclude = source_git / "info" / "exclude"
if source_exclude.is_file():
    (snapshot_git / "info").mkdir(parents=True, exist_ok=True)
    shutil.copy2(source_exclude, snapshot_git / "info" / "exclude")

source_digest = tree_digest(source)
snapshot_digest = tree_digest(snapshot)
if source_digest != snapshot_digest:
    raise SystemExit(
        "source changed while snapshot was captured: "
        f"source={source_digest}:snapshot={snapshot_digest}"
    )
print(snapshot_digest)
PY
)"
  echo "$SNAPSHOT_TREE_SHA256" >&2
  REPO_ROOT="$SNAPSHOT_ROOT"
}

write_source_attestation_manifest() {
  if [[ -z "$SNAPSHOT_TREE_SHA256" ]]; then
    return 0
  fi
  mkdir -p "$RESULT_DIR"
  python3 - "$RESULT_DIR/source-attestation.json" "$SNAPSHOT_TREE_SHA256" <<'PY'
from __future__ import annotations

import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
tree_sha256 = sys.argv[2]
path.write_text(
    json.dumps(
        {
            "schema_version": 1,
            "source_state": "captured_source_snapshot",
            "tree_sha256": tree_sha256,
        },
        indent=2,
        sort_keys=True,
    )
    + "\n",
    encoding="utf-8",
)
PY
}

run_bounded() {
  local label="$1"
  local timeout_seconds="$2"
  local out="$3"
  shift 3
  python3 - "$label" "$timeout_seconds" "$out" "$@" <<'PY'
from __future__ import annotations

import os
import signal
import subprocess
import sys
import time
from pathlib import Path

label = sys.argv[1]
timeout_seconds = int(sys.argv[2])
out_path = Path(sys.argv[3])
command = sys.argv[4:]
proc = None


def terminate_process_group(signum=None) -> None:
    if proc is None or proc.poll() is not None:
        return
    try:
        os.killpg(proc.pid, signal.SIGTERM)
    except ProcessLookupError:
        return
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        try:
            os.killpg(proc.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        proc.wait()


def handle_signal(signum: int, _frame: object) -> None:
    terminate_process_group(signum)
    raise SystemExit(128 + signum)


signal.signal(signal.SIGINT, handle_signal)
signal.signal(signal.SIGTERM, handle_signal)

started = time.monotonic()
with out_path.open("wb") as out:
    proc = subprocess.Popen(
        command,
        stdout=out,
        stderr=subprocess.STDOUT,
        start_new_session=True,
    )

    while True:
        try:
            rc = proc.wait(timeout=10)
            break
        except subprocess.TimeoutExpired:
            elapsed = int(time.monotonic() - started)
            print(
                f"sdk-conformance-reports: {label} still running after {elapsed}s "
                f"(pid={proc.pid}, log={out_path})",
                file=sys.stderr,
            )
            if elapsed < timeout_seconds:
                continue
            print(
                f"sdk-conformance-reports: {label} timed out after {timeout_seconds}s",
                file=sys.stderr,
            )
            try:
                os.killpg(proc.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                try:
                    os.killpg(proc.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                proc.wait()
            rc = 124
            break

if rc != 0:
    print(f"sdk-conformance-reports: {label} failed with exit {rc}", file=sys.stderr)
    if out_path.exists():
        lines = out_path.read_text(encoding="utf-8", errors="replace").splitlines()
        for line in lines[-120:]:
            print(line, file=sys.stderr)
sys.exit(rc)
PY
}

ensure_runner() {
  if [[ "$RUNNER_BUILT" -eq 1 && -x "$RUNNER_BIN" ]]; then
    return 0
  fi
  ensure_tmp_dir
  echo "sdk-conformance-reports: building sdk-conformance-runner" >&2
  run_bounded \
    "build sdk-conformance-runner" \
    "$REPORT_BUILD_TIMEOUT_SECONDS" \
    "$TMP_DIR/build-runner.out" \
    env CARGO_TARGET_DIR="$REPORT_TARGET_DIR" "$CARGO_BIN" build --quiet --manifest-path "$REPO_ROOT/Cargo.toml" -p sdk-conformance-runner --bin sdk-conformance-runner
  RUNNER_BUILT=1
}

ensure_run_nonce() {
  ensure_runner
  if [[ -z "$RUN_NONCE" ]]; then
    RUN_NONCE="$("$RUNNER_BIN" --root "$REPO_ROOT" --issue-run-nonce)"
  fi
  [[ "$RUN_NONCE" =~ ^[0-9a-f]{64}$ ]] || {
    echo "sdk-conformance-reports: runner emitted an invalid run nonce" >&2
    exit 1
  }
}

run_report() {
  local language="$1"
  local report="$2"
  ensure_tmp_dir
  ensure_run_nonce
  local out="$TMP_DIR/$language.out"
  echo "sdk-conformance-reports: validating $language" >&2
  local bounded_rc=0
  run_bounded \
    "validate $language report" \
    "$REPORT_TIMEOUT_SECONDS" \
    "$out" \
    env \
      SDK_CONFORMANCE_RUN_NONCE="$RUN_NONCE" \
      SDK_CONFORMANCE_RESULT_DIR="$TMP_DIR/nested-live-results" \
      EASYNET_SDK_PARITY_RESULTS_DIR="$TMP_DIR/nested-live-results" \
      "$RUNNER_BIN" \
      --root "$REPO_ROOT" \
      --language "$language" \
      --adapter-report "$report" \
      --format json || bounded_rc=$?
  if [[ "$bounded_rc" -ne 0 ]]; then
    return "$bounded_rc"
  fi
  python3 - "$language" "$out" "$RESULT_DIR" <<'PY'
from __future__ import annotations

import json
import sys
from pathlib import Path

language = sys.argv[1]
records = json.loads(Path(sys.argv[2]).read_text(encoding="utf-8"))
result_dir = Path(sys.argv[3])
passed = [record for record in records if record["status"] == "passed"]
if not passed:
    raise SystemExit(f"{language}: runner emitted no passed required records")
for record in passed:
    if not record.get("case_sha256") or not record.get("selector") or not record.get("attestation_sha256"):
        raise SystemExit(f"{language}/{record['case_id']}: missing digest, selector, or attestation")
    if record.get("collected_tests") != [record["selector"]]:
        raise SystemExit(f"{language}/{record['case_id']}: selector was not collected exactly once")
    executions = record.get("executions") or []
    if not executions or not any(proof.get("phase") == "execution" for proof in executions):
        raise SystemExit(f"{language}/{record['case_id']}: missing execution command result")
    for field in ("run_nonce", "tree_sha256", "toolchain_sha256", "toolchain_version", "axon_revision"):
        if not record.get(field):
            raise SystemExit(f"{language}/{record['case_id']}: missing live attestation field {field}")
unsupported = [record for record in records if record["status"] == "unsupported"]
invalid = [record for record in records if record["status"] not in {"passed", "unsupported"}]
if invalid:
    raise SystemExit(f"{language}: runner emitted {len(invalid)} failed or unknown records")
if any(record.get("executions") for record in unsupported):
    raise SystemExit(f"{language}: unsupported records must not claim execution")
result_dir.mkdir(parents=True, exist_ok=True)
(result_dir / f"{language}.json").write_text(
    json.dumps(records, indent=2) + "\n", encoding="utf-8"
)
PY
}

is_terminal_report_status() {
  local rc="$1"
  [[ "$rc" -eq 124 || "$rc" -ge 128 ]]
}

run_selected_reports() {
  local status=0
  selected_count=0
  for language_report in "${language_reports[@]}"; do
    local language report rc
    IFS=: read -r language report <<<"$language_report"
    if [[ -n "$REQUESTED_LANGUAGES" && ",$REQUESTED_LANGUAGES," != *",$language,"* ]]; then
      continue
    fi
    selected_count=$((selected_count + 1))
    if run_report "$language" "$report" >/dev/null; then
      continue
    else
      rc=$?
    fi
    if is_terminal_report_status "$rc"; then
      return "$rc"
    fi
    status=1
  done

  if [[ "$selected_count" -eq 0 ]]; then
    echo "check-sdk-conformance-reports: SDK_CONFORMANCE_LANGUAGES selected no canonical language" >&2
    return 1
  fi

  return "$status"
}

language_reports=(
  "rust:sdk/conformance/runner/rust-action-adapter-report.json"
  "c_abi:sdk/conformance/runner/c-abi-action-adapter-report.json"
  "go:sdk/conformance/runner/go-action-adapter-report.json"
  "python:sdk/conformance/runner/python-action-adapter-report.json"
  "node:sdk/conformance/runner/node-action-adapter-report.json"
  "java:sdk/conformance/runner/java-action-adapter-report.json"
  "swift:sdk/conformance/runner/swift-action-adapter-report.json"
)

if [[ "${1:-}" == "--self-test" ]]; then
  tmp="$(mktemp -d "$REPO_ROOT/target/sdk-conformance-report-gate.XXXXXX")"
  SELF_TEST_TMP="$tmp"
  trap cleanup EXIT

  mkdir -p "$tmp/sdk/conformance/runner"
  cp "$REPO_ROOT/sdk/conformance/runner/go-action-adapter-report.json" \
    "$tmp/forged-status-report.json"
  python3 - "$tmp/forged-status-report.json" <<'PY'
from __future__ import annotations

import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["records"][0]["status"] = "passed"
path.write_text(json.dumps(report), encoding="utf-8")
PY

  if run_report go "$tmp/forged-status-report.json" >"$tmp/status.out" 2>&1; then
    echo "self-test expected committed status attestation to fail" >&2
    exit 1
  fi
  if ! grep -Fq 'unknown field `status`' "$TMP_DIR/go.out"; then
    echo "self-test expected committed status to be rejected by schema" >&2
    echo "wrapper output:" >&2
    cat "$tmp/status.out" >&2
    echo "raw runner output:" >&2
    cat "$TMP_DIR/go.out" >&2
    exit 1
  fi

  cp "$REPO_ROOT/sdk/conformance/runner/go-action-adapter-report.json" \
    "$tmp/forged-hash-report.json"
  python3 - "$tmp/forged-hash-report.json" <<'PY'
from __future__ import annotations

import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["records"][0]["evidence"][0]["sha256"] = "f" * 64
path.write_text(json.dumps(report), encoding="utf-8")
PY

  if run_report go "$tmp/forged-hash-report.json" >"$tmp/hash.out" 2>&1; then
    echo "self-test expected forged evidence hash to fail" >&2
    exit 1
  fi
  if ! grep -Fq "evidence hash mismatch" "$TMP_DIR/go.out"; then
    echo "self-test expected forged evidence hash to be rejected" >&2
    echo "wrapper output:" >&2
    cat "$tmp/hash.out" >&2
    echo "raw runner output:" >&2
    cat "$TMP_DIR/go.out" >&2
    exit 1
  fi

  minimal_root="$tmp/missing-binding-root"
  mkdir -p \
    "$minimal_root/sdk/conformance/cases" \
    "$minimal_root/sdk/conformance/fixtures" \
    "$minimal_root/sdk/conformance/runner" \
    "$minimal_root/sdk/go" \
    "$minimal_root/sdk/schemas"
  cat >"$minimal_root/sdk/conformance/cases/minimal.yaml" <<'EOF'
id: test/minimal
profile: runtime_core
required_for:
  - go
steps:
  - action: noop
expect:
  result: ok
EOF
  cat >"$minimal_root/sdk/schemas/minimal.schema.json" <<'EOF'
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "Minimal",
  "type": "object",
  "additionalProperties": false,
  "required": ["state"],
  "properties": {
    "state": {"const": "ok"}
  }
}
EOF
  printf '{"state":"ok"}\n' >"$minimal_root/sdk/conformance/fixtures/minimal.v1.json"
  cat >"$minimal_root/sdk/conformance/fixture-schema-bindings.json" <<'EOF'
{"schema_version":1,"bindings":[{"fixture":"minimal.v1.json","schema":"minimal.schema.json"}]}
EOF
  cat >"$minimal_root/sdk/go/minimal_test.go" <<'EOF'
package minimal

import "testing"

func TestMinimal(t *testing.T) {}
EOF
  printf '{"schema_version":1,"bindings":[]}\n' \
    >"$minimal_root/sdk/conformance/runner/execution-manifest.json"
  evidence_hash="$(python3 - "$minimal_root/sdk/go/minimal_test.go" <<'PY'
from __future__ import annotations

import hashlib
import sys
from pathlib import Path

print(hashlib.sha256(Path(sys.argv[1]).read_bytes()).hexdigest())
PY
)"
  cat >"$minimal_root/adapter.json" <<EOF
{
  "schema_version": 2,
  "language": "go",
  "adapter_kind": "unit_test",
  "records": [
    {
      "case_id": "test/minimal",
      "profile": "runtime_core",
      "evidence": [{"kind": "go_test", "ref_path": "sdk/go/minimal_test.go", "sha256": "$evidence_hash"}]
    }
  ]
}
EOF
  if env SDK_CONFORMANCE_RUN_NONCE="$RUN_NONCE" "$RUNNER_BIN" \
    --root "$minimal_root" \
    --language go \
    --adapter-report "$minimal_root/adapter.json" \
    --format json >"$tmp/missing-binding.out" 2>&1; then
    echo "self-test expected missing execution binding to fail" >&2
    exit 1
  fi
  if ! grep -Fq "has no runner-owned execution binding" "$tmp/missing-binding.out"; then
    echo "self-test expected missing execution binding to be rejected" >&2
    cat "$tmp/missing-binding.out" >&2
    exit 1
  fi

  (
    language_reports=("rust:rust-report" "go:go-report")
    REQUESTED_LANGUAGES=""
    selected_count=0
    cancel_trace="$tmp/cancel-sequence.out"
    run_report() {
      echo "$1" >>"$cancel_trace"
      if [[ "$1" == "rust" ]]; then
        return 130
      fi
      return 0
    }
    cancel_rc=0
    run_selected_reports >/dev/null 2>&1 || cancel_rc=$?
    if [[ "$cancel_rc" -eq 0 ]]; then
      echo "self-test expected terminal cancellation status" >&2
      exit 1
    fi
    if [[ "$cancel_rc" -ne 130 ]]; then
      echo "self-test expected cancellation status 130, got $cancel_rc" >&2
      exit 1
    fi
    if grep -Fq "go" "$cancel_trace"; then
      echo "self-test expected cancellation to stop before go" >&2
      cat "$cancel_trace" >&2
      exit 1
    fi
    grep -Fxq "rust" "$cancel_trace"
  )

  (
    language_reports=("rust:rust-report" "go:go-report")
    REQUESTED_LANGUAGES=""
    selected_count=0
    timeout_trace="$tmp/timeout-sequence.out"
    run_report() {
      echo "$1" >>"$timeout_trace"
      if [[ "$1" == "rust" ]]; then
        return 124
      fi
      return 0
    }
    timeout_rc=0
    run_selected_reports >/dev/null 2>&1 || timeout_rc=$?
    if [[ "$timeout_rc" -ne 124 ]]; then
      echo "self-test expected timeout status 124, got $timeout_rc" >&2
      exit 1
    fi
    if grep -Fq "go" "$timeout_trace"; then
      echo "self-test expected timeout to stop before go" >&2
      cat "$timeout_trace" >&2
      exit 1
    fi
    grep -Fxq "rust" "$timeout_trace"
  )

  echo "check-sdk-conformance-reports self-test ok"
  exit 0
fi

rm -rf "$RESULT_DIR"
create_source_snapshot
write_source_attestation_manifest
ensure_run_nonce
readonly RUN_NONCE

report_status=0
run_selected_reports || report_status=$?
if is_terminal_report_status "$report_status"; then
  echo "check-sdk-conformance-reports interrupted" >&2
  exit "$report_status"
fi
if [[ "$report_status" -ne 0 ]]; then
  echo "check-sdk-conformance-reports failed" >&2
  exit "$report_status"
fi

python3 - "$RESULT_DIR" <<'PY'
from __future__ import annotations

import json
import sys
from pathlib import Path

result_dir = Path(sys.argv[1])
nonce_by_language: dict[str, set[str]] = {}
tree_by_language: dict[str, set[str]] = {}
for result_path in sorted(result_dir.glob("*.json")):
    records = json.loads(result_path.read_text(encoding="utf-8"))
    if not isinstance(records, list):
        continue
    nonce_by_language[result_path.stem] = {
        str(record.get("run_nonce", "")) for record in records
    }
    tree_by_language[result_path.stem] = {
        str(record.get("tree_sha256", "")) for record in records
    }
all_nonces = {nonce for nonces in nonce_by_language.values() for nonce in nonces}
if len(all_nonces) != 1:
    detail = ", ".join(
        f"{language}={','.join(sorted(nonces))}"
        for language, nonces in sorted(nonce_by_language.items())
    )
    raise SystemExit(f"check-sdk-conformance-reports: mixed run nonce: {detail}")
all_trees = {tree for trees in tree_by_language.values() for tree in trees}
if len(all_trees) != 1:
    detail = ", ".join(
        f"{language}={','.join(sorted(trees))}"
        for language, trees in sorted(tree_by_language.items())
    )
    raise SystemExit(f"check-sdk-conformance-reports: mixed tree attestation: {detail}")
PY

echo "check-sdk-conformance-reports ok"
