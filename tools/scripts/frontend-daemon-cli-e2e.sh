#!/usr/bin/env bash
# frontend-daemon-cli-e2e.sh - daemon-bound frontend replay through CLI commands
#
# This script intentionally exercises only the CLI/daemon boundary. It does not
# call EasyNet Backend HTTP routes, WebSocket endpoints, or Go/Rust test runners.

set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"
CLI_BIN="${EASYNET_CLI_BIN:-$REPO_ROOT/target/debug/easynet}"
DAEMON_BIN="${EASYNET_DAEMON_BIN:-$REPO_ROOT/target/debug/easynet-daemon}"
OUT_DIR="${EASYNET_E2E_OUT_DIR:-$REPO_ROOT/target/e2e/frontend-daemon-cli/$(date +%Y%m%d-%H%M%S)}"
ITERATIONS="${EASYNET_E2E_ITERATIONS:-5}"
CONCURRENCY="${EASYNET_E2E_CONCURRENCY:-16}"
REQUESTS="${EASYNET_E2E_REQUESTS:-64}"
MAX_AGENTS="${EASYNET_E2E_MAX_AGENTS:-5}"
START_RUNTIME=0
KEEP_RUNTIME=1
STARTED_RUNTIME=0
STARTED_DAEMON_PID=""

usage() {
  cat <<'EOF'
Usage:
  tools/scripts/frontend-daemon-cli-e2e.sh [options]

Options:
  --start-runtime       Start an isolated self-signed local device daemon if none is reachable.
  --keep-runtime        Leave a daemon started by this script running. Default.
  --stop-runtime        Stop a daemon started by this script before exit.
  --iterations N        Sequential samples per command. Default: EASYNET_E2E_ITERATIONS or 5.
  --concurrency N       Concurrent workers. Default: EASYNET_E2E_CONCURRENCY or 16.
  --requests N          Concurrent requests per command. Default: EASYNET_E2E_REQUESTS or 64.
  --max-agents N        Max discovered agents for per-agent probes. Default: 5.
  --out-dir DIR         Report directory. Default: target/e2e/frontend-daemon-cli/<timestamp>.
  --self-test           Syntax/configuration check only; does not require a running daemon.
  -h, --help            Show this help.

Environment:
  EASYNET_CLI_BIN       Path to the easynet CLI binary.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --start-runtime) START_RUNTIME=1; shift ;;
    --keep-runtime) KEEP_RUNTIME=1; shift ;;
    --stop-runtime) KEEP_RUNTIME=0; shift ;;
    --iterations) ITERATIONS="${2:?missing value for --iterations}"; shift 2 ;;
    --concurrency) CONCURRENCY="${2:?missing value for --concurrency}"; shift 2 ;;
    --requests) REQUESTS="${2:?missing value for --requests}"; shift 2 ;;
    --max-agents) MAX_AGENTS="${2:?missing value for --max-agents}"; shift 2 ;;
    --out-dir) OUT_DIR="${2:?missing value for --out-dir}"; shift 2 ;;
    --self-test)
      bash -n "$0"
      grep -q 'ability list --format json' "$0"
      grep -q 'skill list --json' "$0"
      grep -q 'agent abilities' "$0"
      echo "frontend-daemon-cli-e2e self-test ok"
      exit 0
      ;;
    -h|--help) usage; exit 0 ;;
    *) echo "[frontend-daemon-cli-e2e] unknown arg: $1" >&2; usage >&2; exit 2 ;;
  esac
done

case "$ITERATIONS:$CONCURRENCY:$REQUESTS:$MAX_AGENTS" in
  *[!0-9:]*|'') echo "[frontend-daemon-cli-e2e] numeric options must be positive integers" >&2; exit 2 ;;
esac
for value in "$ITERATIONS" "$CONCURRENCY" "$REQUESTS" "$MAX_AGENTS"; do
  if [[ "$value" -lt 1 ]]; then
    echo "[frontend-daemon-cli-e2e] numeric options must be positive integers" >&2
    exit 2
  fi
done

if [[ ! -x "$CLI_BIN" ]]; then
  echo "[frontend-daemon-cli-e2e] building easynet CLI..."
  (cd "$REPO_ROOT" && cargo build --bin easynet)
fi
if [[ "$START_RUNTIME" -eq 1 && ! -x "$DAEMON_BIN" ]]; then
  echo "[frontend-daemon-cli-e2e] building easynet-daemon..."
  (cd "$REPO_ROOT" && cargo build --bin easynet-daemon)
fi

cleanup() {
  if [[ "$STARTED_RUNTIME" -eq 1 && "$KEEP_RUNTIME" -eq 0 && -n "$STARTED_DAEMON_PID" ]]; then
    kill "$STARTED_DAEMON_PID" >/dev/null 2>&1 || true
    wait "$STARTED_DAEMON_PID" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

runtime_running() {
  local status_json
  if ! status_json="$("$CLI_BIN" runtime status --json 2>/dev/null)"; then
    return 1
  fi
  STATUS_JSON="$status_json" python3 - <<'PY'
import json
import os
import sys

try:
    payload = json.loads(os.environ["STATUS_JSON"])
except Exception:
    sys.exit(1)

status = payload.get("runtime_status")
daemon = payload.get("daemon")
runtime = payload.get("runtime")
if status and status != "stopped":
    sys.exit(0)
if daemon or runtime:
    sys.exit(0)
sys.exit(1)
PY
}

start_isolated_local_runtime() {
  local local_home
  local_home="$(mktemp -d /tmp/easynet-e2e.XXXXXX)"
  local state_dir="$local_home/.easynet"
  local node_id="local-e2e-device"
  mkdir -p "$state_dir"
  cat > "$state_dir/credentials.json" <<EOF
{
  "node_id": "$node_id",
  "credential_token": "local-e2e-token",
  "hub_endpoint": "http://127.0.0.1:9",
  "realm": "localhost",
  "deploy_signature": "local-e2e-self-signed",
  "hub_api_base": "http://127.0.0.1:9",
  "username": "local",
  "user_id": "local-user"
}
EOF
  chmod 600 "$state_dir/credentials.json" 2>/dev/null || true
  cat > "$state_dir/daemon-config.toml" <<'EOF'
[daemon]
mode = "device"
realm = "localhost"
hub_endpoint = "http://127.0.0.1:9"
EOF

  export HOME="$local_home"
  echo "[frontend-daemon-cli-e2e] starting isolated self-signed local device daemon..."
  EASYNET_NODE_ID="$node_id" "$DAEMON_BIN" >/dev/null 2>"$OUT_DIR/isolated-daemon.stderr" &
  STARTED_DAEMON_PID=$!
  STARTED_RUNTIME=1

  local sock="$state_dir/daemon.sock"
  for _ in $(seq 1 80); do
    if [[ -S "$sock" ]]; then
      echo "[frontend-daemon-cli-e2e] isolated HOME: $local_home"
      printf '%s\n' "$local_home" > "$OUT_DIR/isolated-home.txt"
      return 0
    fi
    if ! kill -0 "$STARTED_DAEMON_PID" >/dev/null 2>&1; then
      echo "[frontend-daemon-cli-e2e] isolated daemon exited before readiness" >&2
      cat "$OUT_DIR/isolated-daemon.stderr" >&2 || true
      return 1
    fi
    sleep 0.25
  done
  echo "[frontend-daemon-cli-e2e] isolated daemon did not expose $sock" >&2
  cat "$OUT_DIR/isolated-daemon.stderr" >&2 || true
  return 1
}

if ! runtime_running; then
  if [[ "$START_RUNTIME" -ne 1 ]]; then
    echo "[frontend-daemon-cli-e2e] local runtime is not reachable." >&2
    echo "[frontend-daemon-cli-e2e] Start it first or pass --start-runtime." >&2
    exit 1
  fi
  mkdir -p "$OUT_DIR"
  start_isolated_local_runtime
fi

mkdir -p "$OUT_DIR"

echo "[frontend-daemon-cli-e2e] running CLI daemon-bound replay..."
echo "[frontend-daemon-cli-e2e] report dir: $OUT_DIR"

python3 - "$CLI_BIN" "$OUT_DIR" "$ITERATIONS" "$CONCURRENCY" "$REQUESTS" "$MAX_AGENTS" <<'PY'
import concurrent.futures
import hashlib
import json
import os
import re
import shlex
import statistics
import subprocess
import sys
import time
from pathlib import Path

cli = sys.argv[1]
out_dir = Path(sys.argv[2])
iterations = int(sys.argv[3])
concurrency = int(sys.argv[4])
requests = int(sys.argv[5])
max_agents = int(sys.argv[6])

ansi = re.compile(r"\x1b\[[0-9;]*[A-Za-z]")

def strip_ansi(text: str) -> str:
    return ansi.sub("", text)

def run_cmd(args, timeout=120):
    start = time.perf_counter_ns()
    proc = subprocess.run(
        [cli, *args],
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=timeout,
    )
    end = time.perf_counter_ns()
    return {
        "args": args,
        "cmd": " ".join([shlex.quote(cli), *map(shlex.quote, args)]),
        "exit_code": proc.returncode,
        "latency_ms": (end - start) / 1_000_000.0,
        "stdout": proc.stdout,
        "stderr": proc.stderr,
    }

def canonical_hash(stdout, stderr, json_stdout=False):
    if json_stdout:
        try:
            payload = json.loads(stdout)
            data = json.dumps(payload, sort_keys=True, separators=(",", ":"))
        except Exception:
            data = stdout
    else:
        data = strip_ansi(stdout + "\n" + stderr)
    return hashlib.sha256(data.encode("utf-8")).hexdigest()

def percentile(values, q):
    if not values:
        return None
    ordered = sorted(values)
    index = max(0, min(len(ordered) - 1, int((len(ordered) * q + 99) // 100) - 1))
    return ordered[index]

def stats(samples):
    latencies = [s["latency_ms"] for s in samples if s["exit_code"] == 0]
    return {
        "samples": len(samples),
        "success": sum(1 for s in samples if s["exit_code"] == 0),
        "failed": sum(1 for s in samples if s["exit_code"] != 0),
        "min_ms": min(latencies) if latencies else None,
        "avg_ms": statistics.mean(latencies) if latencies else None,
        "p50_ms": percentile(latencies, 50),
        "p95_ms": percentile(latencies, 95),
        "p99_ms": percentile(latencies, 99),
        "max_ms": max(latencies) if latencies else None,
    }

def write_text(name, text):
    path = out_dir / name
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")
    return str(path)

def json_len(stdout):
    payload = json.loads(stdout)
    if isinstance(payload, list):
        return len(payload)
    if isinstance(payload, dict):
        for key in ("abilities", "items", "agents"):
            if isinstance(payload.get(key), list):
                return len(payload[key])
    return None

def parse_agent_names(text):
    clean = strip_ansi(text)
    names = []
    for raw in clean.splitlines():
        line = raw.strip()
        if not line or line.startswith("No agents"):
            continue
        if "NAME" in line and "TYPE" in line:
            continue
        if set(line) <= {"\u2500", "-", " "}:
            continue
        parts = line.split()
        if len(parts) >= 2 and parts[0] not in {"Run", "EasyNet"}:
            names.append(parts[0])
    return names

def parse_agent_ability_count(text):
    clean = strip_ansi(text)
    count = 0
    for raw in clean.splitlines():
        line = raw.strip()
        if not line:
            continue
        if line.startswith("agent ") or "No abilities declared" in line:
            continue
        if "ABILITY" in line and "TIMEOUT" in line:
            continue
        if set(line) <= {"\u2500", "-", " "}:
            continue
        parts = line.split()
        if parts and "." in parts[0]:
            count += 1
    return count

targets = []

baseline = {}

def capture_baseline(name, args, json_stdout=False, parser=None):
    result = run_cmd(args)
    baseline[name] = {
        "cmd": result["cmd"],
        "exit_code": result["exit_code"],
        "latency_ms": result["latency_ms"],
        "stdout_path": write_text(f"baseline/{name}.stdout", result["stdout"]),
        "stderr_path": write_text(f"baseline/{name}.stderr", result["stderr"]),
        "stdout_sha256": canonical_hash(result["stdout"], "", json_stdout),
        "combined_sha256": canonical_hash(result["stdout"], result["stderr"], json_stdout),
    }
    if result["exit_code"] != 0:
        raise RuntimeError(f"{name} failed: {result['cmd']}\n{result['stderr']}")
    if json_stdout:
        baseline[name]["count"] = json_len(result["stdout"])
    if parser:
        baseline[name].update(parser(result))
    targets.append({"name": name, "args": args, "json_stdout": json_stdout})
    return result

capture_baseline(
    "ability_list_all",
    ["ability", "list", "--format", "json"],
    json_stdout=True,
)
capture_baseline(
    "skill_list_all",
    ["skill", "list", "--json"],
    json_stdout=True,
)
agent_list_result = capture_baseline(
    "agent_list_all",
    ["agent", "list"],
    parser=lambda r: {"agent_names": parse_agent_names(r["stdout"] + r["stderr"])},
)
agent_names = baseline["agent_list_all"]["agent_names"][:max_agents]
baseline["agent_list_all"]["agent_count"] = len(baseline["agent_list_all"]["agent_names"])
baseline["agent_list_all"]["agent_probe_count"] = len(agent_names)

per_agent = {}
for agent in agent_names:
    safe = re.sub(r"[^A-Za-z0-9_.-]+", "_", agent)
    ability_name = f"agent_abilities__{safe}"
    ability_result = capture_baseline(
        ability_name,
        ["agent", "abilities", agent],
        parser=lambda r: {"declared_ability_count": parse_agent_ability_count(r["stdout"] + r["stderr"])},
    )
    ability_filter_name = f"ability_list_agent__{safe}"
    capture_baseline(
        ability_filter_name,
        ["ability", "list", "--agent", agent, "--format", "json"],
        json_stdout=True,
    )
    skill_filter_name = f"skill_list_agent__{safe}"
    capture_baseline(
        skill_filter_name,
        ["skill", "list", "--agent", agent, "--json"],
        json_stdout=True,
    )
    per_agent[agent] = {
        "agent_abilities_target": ability_name,
        "ability_list_agent_target": ability_filter_name,
        "skill_list_agent_target": skill_filter_name,
        "declared_ability_count": baseline[ability_name]["declared_ability_count"],
        "catalogue_ability_count": baseline[ability_filter_name].get("count"),
        "skill_count": baseline[skill_filter_name].get("count"),
        "agent_abilities_stdout": baseline[ability_name]["stdout_path"],
        "agent_abilities_stderr": baseline[ability_name]["stderr_path"],
    }

def run_samples(target, n):
    samples = []
    for _ in range(n):
        result = run_cmd(target["args"])
        result["combined_sha256"] = canonical_hash(
            result["stdout"],
            result["stderr"],
            target["json_stdout"],
        )
        samples.append(result)
    return samples

def concurrent_samples(target):
    def one(_):
        result = run_cmd(target["args"])
        result["combined_sha256"] = canonical_hash(
            result["stdout"],
            result["stderr"],
            target["json_stdout"],
        )
        return result
    with concurrent.futures.ThreadPoolExecutor(max_workers=concurrency) as pool:
        return list(pool.map(one, range(requests)))

bench = {}
for target in targets:
    sequential = run_samples(target, iterations)
    concurrent_run = concurrent_samples(target)
    for label, samples in (("sequential", sequential), ("concurrent", concurrent_run)):
        slim = [
            {
                "exit_code": s["exit_code"],
                "latency_ms": s["latency_ms"],
                "combined_sha256": s["combined_sha256"],
            }
            for s in samples
        ]
        write_text(
            f"samples/{target['name']}.{label}.json",
            json.dumps(slim, indent=2, sort_keys=True),
        )
    ok_hashes = sorted({s["combined_sha256"] for s in concurrent_run if s["exit_code"] == 0})
    bench[target["name"]] = {
        "cmd": " ".join(map(shlex.quote, [cli, *target["args"]])),
        "sequential": stats(sequential),
        "concurrent": stats(concurrent_run),
        "concurrency": concurrency,
        "requests": requests,
        "successful_output_hashes": ok_hashes,
        "stable_success_output": len(ok_hashes) <= 1,
    }

report = {
    "script": "tools/scripts/frontend-daemon-cli-e2e.sh",
    "scope": "daemon-bound CLI commands only",
    "cli": cli,
    "iterations": iterations,
    "concurrency": concurrency,
    "requests": requests,
    "max_agents": max_agents,
    "baseline": baseline,
    "per_agent": per_agent,
    "benchmarks": bench,
}
write_text("report.json", json.dumps(report, indent=2, sort_keys=True))

def fmt(v):
    if v is None:
        return "-"
    if isinstance(v, float):
        return f"{v:.2f}"
    return str(v)

lines = []
lines.append("# Frontend Daemon CLI E2E Report")
lines.append("")
lines.append(f"- Scope: daemon-bound CLI commands only")
lines.append(f"- CLI: `{cli}`")
lines.append(f"- Sequential iterations: `{iterations}`")
lines.append(f"- Concurrent requests/workers: `{requests}` / `{concurrency}`")
lines.append("")
lines.append("## Baseline Results")
lines.append("")
lines.append("| Target | Count | Latency ms | Command |")
lines.append("|---|---:|---:|---|")
for name, data in baseline.items():
    count = data.get("count", data.get("agent_count", data.get("declared_ability_count", "-")))
    lines.append(f"| `{name}` | {count} | {fmt(data['latency_ms'])} | `{data['cmd']}` |")
lines.append("")
lines.append("## Per-Agent Results")
lines.append("")
if per_agent:
    lines.append("| Agent | Declared abilities | Catalogue abilities | Skills |")
    lines.append("|---|---:|---:|---:|")
    for agent, data in per_agent.items():
        lines.append(
            f"| `{agent}` | {data['declared_ability_count']} | "
            f"{data['catalogue_ability_count']} | {data['skill_count']} |"
        )
else:
    lines.append("No registered agents were discovered by `easynet agent list`.")
lines.append("")
lines.append("## Latency")
lines.append("")
lines.append("| Target | Mode | OK/Total | Avg | P50 | P95 | P99 | Max | Stable result |")
lines.append("|---|---|---:|---:|---:|---:|---:|---:|---|")
for name, data in bench.items():
    stable = "yes" if data["stable_success_output"] else "NO"
    for mode in ("sequential", "concurrent"):
        s = data[mode]
        lines.append(
            f"| `{name}` | {mode} | {s['success']}/{s['samples']} | "
            f"{fmt(s['avg_ms'])} | {fmt(s['p50_ms'])} | {fmt(s['p95_ms'])} | "
            f"{fmt(s['p99_ms'])} | {fmt(s['max_ms'])} | {stable} |"
        )
lines.append("")
lines.append("Raw sample files are under `samples/`; command stdout/stderr baselines are under `baseline/`.")
write_text("report.md", "\n".join(lines) + "\n")

failed = [
    name
    for name, data in bench.items()
    if data["sequential"]["failed"] or data["concurrent"]["failed"] or not data["stable_success_output"]
]
if failed:
    print("[frontend-daemon-cli-e2e] FAIL: unstable or failing targets: " + ", ".join(failed), file=sys.stderr)
    print(f"[frontend-daemon-cli-e2e] report: {out_dir / 'report.md'}", file=sys.stderr)
    sys.exit(1)

print(f"[frontend-daemon-cli-e2e] PASS: {out_dir / 'report.md'}")
PY
