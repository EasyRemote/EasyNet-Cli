#!/usr/bin/env bash
# Hub API readiness preflight for product E2E harnesses.
#
# This script is intentionally not a daemon or RemoteApp implementation. It
# verifies that the product Hub API named by runtime connection state is
# reachable before higher-level E2E harnesses attempt daemon, frontend, media,
# or input evidence.

set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"
OUT_DIR="${EASYNET_HUB_API_READINESS_OUT_DIR:-$REPO_ROOT/target/e2e/hub-api-readiness/$(date -u +%Y%m%d-%H%M%S)-$$}"
MODE=run

usage() {
  cat <<'USAGE'
Usage:
  tools/scripts/hub-api-readiness-preflight.sh --run [--out-dir DIR]
  tools/scripts/hub-api-readiness-preflight.sh --self-test

Options:
  --run          Inspect runtime status and verify Hub API reachability.
  --self-test    Validate the harness source contract only.
  --out-dir DIR  Report directory.
  -h, --help     Show this help.

Environment:
  EASYNET_REMOTEAPP_EASYNET_BIN
                 Optional easynet binary override.
  EASYNET_PRODUCT_HUB_API_ENDPOINT
                 Optional Hub API endpoint override for diagnostics.

This preflight checks Hub API reachability only. It does not start Docker, does
not mutate credentials, and does not replace daemon credential verification.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --run) MODE=run; shift ;;
    --self-test) MODE=self-test; shift ;;
    --out-dir) OUT_DIR="${2:?missing value for --out-dir}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 64 ;;
  esac
done

write_report() {
  local status="$1"
  local reason="$2"
  python3 - "$OUT_DIR" "$status" "$reason" <<'PY'
import json
import pathlib
import sys

out_dir = pathlib.Path(sys.argv[1])
status = sys.argv[2]
reason = sys.argv[3]
out_dir.mkdir(parents=True, exist_ok=True)
details_path = out_dir / "details.json"
details = {}
if details_path.exists():
    details = json.loads(details_path.read_text(encoding="utf-8"))
report = {
    "script": "tools/scripts/hub-api-readiness-preflight.sh",
    "status": status,
    "reason": reason,
    "hub_api_endpoint": details.get("hub_api_endpoint"),
    "hub_endpoint": details.get("hub_endpoint"),
    "runtime_status": details.get("runtime_status"),
    "connection_state": details.get("connection_state"),
    "connection_failure": details.get("connection_failure"),
    "docker": details.get("docker"),
    "health": details.get("health"),
}
(out_dir / "report.json").write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
(out_dir / "report.md").write_text(
    "# Hub API Readiness Preflight\n\n"
    f"- Status: `{status}`\n"
    f"- Reason: `{reason}`\n"
    f"- Hub API endpoint: `{report.get('hub_api_endpoint') or ''}`\n"
    f"- Hub endpoint: `{report.get('hub_endpoint') or ''}`\n"
    f"- Runtime status: `{report.get('runtime_status') or ''}`\n"
    f"- Connection state: `{report.get('connection_state') or ''}`\n"
    f"- Connection failure: `{json.dumps(report.get('connection_failure'), sort_keys=True) if report.get('connection_failure') else ''}`\n",
    encoding="utf-8",
)
PY
}

run_with_timeout() {
  local timeout_sec="$1"
  shift
  python3 - "$timeout_sec" "$@" <<'PY'
import subprocess
import sys

timeout_sec = float(sys.argv[1])
cmd = sys.argv[2:]
try:
    completed = subprocess.run(cmd, timeout=timeout_sec)
except subprocess.TimeoutExpired:
    print(f"command timed out after {timeout_sec:g}s: {' '.join(cmd)}", file=sys.stderr)
    raise SystemExit(124)
raise SystemExit(completed.returncode)
PY
}

run_easynet() {
  local timeout_sec="${EASYNET_REMOTEAPP_EASYNET_COMMAND_TIMEOUT_SEC:-45}"
  if [[ -n "${EASYNET_REMOTEAPP_EASYNET_BIN:-}" ]]; then
    run_with_timeout "$timeout_sec" "$EASYNET_REMOTEAPP_EASYNET_BIN" "$@"
  elif [[ -x "$REPO_ROOT/target/debug/easynet" ]]; then
    run_with_timeout "$timeout_sec" "$REPO_ROOT/target/debug/easynet" "$@"
  else
    run_with_timeout "$timeout_sec" cargo run --quiet --bin easynet -- "$@"
  fi
}

if [[ "$MODE" == "self-test" ]]; then
  bash -n "$0"
  grep -q 'runtime status --json' "$0"
  grep -q 'hub_api_endpoint' "$0"
  grep -q 'connection_failure' "$0"
  grep -q '/api/v1/health' "$0"
  grep -q 'Hub API health is not reachable' "$0"
  grep -q 'Docker daemon is not reachable' "$0"
  grep -q 'does not start Docker' "$0"
  echo "hub-api-readiness-preflight self-test ok"
  exit 0
fi

mkdir -p "$OUT_DIR"
RUNTIME_STATUS_JSON="$OUT_DIR/runtime-status.json"
DETAILS_JSON="$OUT_DIR/details.json"

status_rc=0
run_easynet runtime status --json >"$RUNTIME_STATUS_JSON" 2>"$OUT_DIR/runtime-status.stderr" || status_rc=$?

if python3 - "$RUNTIME_STATUS_JSON" "$DETAILS_JSON" "${EASYNET_PRODUCT_HUB_API_ENDPOINT:-}" "$status_rc" <<'PY'
import json
import pathlib
import sys

status_path = pathlib.Path(sys.argv[1])
details_path = pathlib.Path(sys.argv[2])
override_endpoint = sys.argv[3].strip()
status_rc = int(sys.argv[4])
status = {}
if status_path.exists() and status_path.read_text(encoding="utf-8", errors="replace").strip():
    try:
        status = json.loads(status_path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        status = {"parse_error": str(exc)}
connection = status.get("connection") if isinstance(status, dict) else None
hub_api_endpoint = override_endpoint or (
    connection.get("hub_api_endpoint")
    if isinstance(connection, dict)
    else None
)
details = {
    "runtime_status_command_exit_code": status_rc,
    "runtime_status": status.get("runtime_status") if isinstance(status, dict) else None,
    "hub_api_endpoint": hub_api_endpoint,
    "hub_endpoint": connection.get("hub_endpoint") if isinstance(connection, dict) else None,
    "connection_state": connection.get("state") if isinstance(connection, dict) else None,
    "connection_failure": connection.get("failure") if isinstance(connection, dict) else None,
}
if status_rc != 0:
    details["preflight_error"] = "runtime status --json failed"
    details_path.write_text(json.dumps(details, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    raise SystemExit("runtime status --json failed")
if not hub_api_endpoint:
    details["preflight_error"] = "runtime status did not expose hub_api_endpoint; pair or pass EASYNET_PRODUCT_HUB_API_ENDPOINT"
    details_path.write_text(json.dumps(details, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    raise SystemExit("runtime status did not expose hub_api_endpoint; pair or pass EASYNET_PRODUCT_HUB_API_ENDPOINT")
details_path.write_text(json.dumps(details, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY
then
  :
else
  reason="$(python3 - "$DETAILS_JSON" <<'PY'
import json
import pathlib
import sys

details_path = pathlib.Path(sys.argv[1])
details = json.loads(details_path.read_text(encoding="utf-8")) if details_path.exists() else {}
print(details.get("preflight_error") or "runtime status preflight failed")
PY
)"
  write_report "failed" "$reason"
  python3 - "$DETAILS_JSON" <<'PY' >&2
import json
import pathlib
import sys

details = json.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))
print(f"runtime_status={details.get('runtime_status')}")
print(f"connection_state={details.get('connection_state')}")
print(f"hub_endpoint={details.get('hub_endpoint')}")
print(f"hub_api_endpoint={details.get('hub_api_endpoint')}")
failure = details.get("connection_failure")
if failure:
    print("connection_failure=" + json.dumps(failure, sort_keys=True))
PY
  exit 1
fi

docker_status="unknown"
docker_error=""
if command -v docker >/dev/null 2>&1; then
  if docker info >"$OUT_DIR/docker-info.stdout" 2>"$OUT_DIR/docker-info.stderr"; then
    docker_status="reachable"
  else
    docker_status="unreachable"
    docker_error="$(tr '\n' ' ' <"$OUT_DIR/docker-info.stderr" | cut -c1-1000)"
  fi
else
  docker_status="missing"
  docker_error="docker command not found"
fi

python3 - "$DETAILS_JSON" "$docker_status" "$docker_error" <<'PY'
import json
import pathlib
import sys

details_path = pathlib.Path(sys.argv[1])
details = json.loads(details_path.read_text(encoding="utf-8"))
details["docker"] = {
    "status": sys.argv[2],
    "error": sys.argv[3],
}
details_path.write_text(json.dumps(details, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY

if [[ "$docker_status" != "reachable" ]]; then
  write_report "failed" "Docker daemon is not reachable"
  echo "Docker daemon is not reachable: $docker_error" >&2
  exit 1
fi

if python3 - "$DETAILS_JSON" <<'PY'
import json
import pathlib
import sys
import urllib.error
import urllib.request

details_path = pathlib.Path(sys.argv[1])
details = json.loads(details_path.read_text(encoding="utf-8"))
base = (details.get("hub_api_endpoint") or "").rstrip("/")
url = f"{base}/api/v1/health"
health = {"url": url}
try:
    with urllib.request.urlopen(url, timeout=5) as response:
        body = response.read(4096).decode("utf-8", errors="replace")
        health.update({
            "status": "reachable",
            "http_status": response.status,
            "body_excerpt": body[:1000],
        })
except urllib.error.HTTPError as exc:
    health.update({
        "status": "http_error",
        "http_status": exc.code,
        "error": str(exc),
    })
except Exception as exc:
    health.update({
        "status": "unreachable",
        "error": str(exc),
    })
details["health"] = health
details_path.write_text(json.dumps(details, indent=2, sort_keys=True) + "\n", encoding="utf-8")
if health["status"] != "reachable":
    print(f"Hub API health is not reachable: {url}: {health.get('error')}", file=sys.stderr)
    raise SystemExit(1)
print(f"Hub API health reachable: {url}")
PY
then
  write_report "passed" "Hub API health reachable"
  echo "[hub-api-readiness-preflight] PASS: $OUT_DIR/report.md"
else
  write_report "failed" "Hub API health is not reachable"
  exit 1
fi
