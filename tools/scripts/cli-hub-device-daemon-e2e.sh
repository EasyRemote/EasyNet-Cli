#!/usr/bin/env bash
# CLI-only Hub/Device daemon E2E.
#
# Scope:
# - Build/use only EasyNet-Cli binaries: easynet, easynet-daemon, easynet-keyring.
# - Start one CLI hub daemon and two CLI device daemons with isolated HOME dirs.
# - Use easynet CLI commands for registration, online status, CRUD, query, invoke,
#   run, and load entrypoints.
# - Do not build or call the Go backend HTTP API.

set -euo pipefail

SELF_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"
CLI_BIN="${EASYNET_CLI_BIN:-$REPO_ROOT/target/debug/easynet}"
DAEMON_BIN="${EASYNET_DAEMON_BIN:-$REPO_ROOT/target/debug/easynet-daemon}"
KEYRING_BIN="${EASYNET_KEYRING_BIN:-$REPO_ROOT/target/debug/easynet-keyring}"

REALM="${EASYNET_E2E_REALM:-localhost}"
HUB_URA="easynet:///r/${REALM}/hub"
ADMIN_URA="easynet:///r/${REALM}/user/admin"
USER_URA="easynet:///r/${REALM}/user/alice"
REQUESTS="${EASYNET_E2E_REQUESTS:-24}"
CONCURRENCY="${EASYNET_E2E_CONCURRENCY:-6}"
KEEP=0
BUILD=1
SELF_TEST=0
TIMESTAMP="$(date +%Y%m%d-%H%M%S)"
OUT_DIR="${EASYNET_E2E_OUT_DIR:-$REPO_ROOT/target/e2e/cli-hub-device-daemon/$TIMESTAMP}"

usage() {
  cat <<'USAGE'
Usage:
  tools/scripts/cli-hub-device-daemon-e2e.sh [options]

Options:
  --skip-build       Reuse existing target/debug/easynet* binaries.
  --keep             Keep temporary homes and daemons running after completion.
  --requests N       Requests per load-test target. Default: 24.
  --concurrency N    Concurrent workers per load-test target. Default: 6.
  --out-dir DIR      Report directory.
  --self-test        Syntax/prerequisite check only.
  -h, --help         Show this help.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-build) BUILD=0; shift ;;
    --keep) KEEP=1; shift ;;
    --requests) REQUESTS="${2:?missing value for --requests}"; shift 2 ;;
    --concurrency) CONCURRENCY="${2:?missing value for --concurrency}"; shift 2 ;;
    --out-dir) OUT_DIR="${2:?missing value for --out-dir}"; shift 2 ;;
    --self-test) SELF_TEST=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 64 ;;
  esac
done

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing command: $1" >&2
    exit 1
  }
}

need_cmd jq
need_cmd python3
need_cmd openssl

if [[ "$SELF_TEST" == "1" ]]; then
  bash -n "$0"
  grep -q "runtime start --as-hub" "$0"
  grep -q "device join" "$0"
  grep -q "agent add" "$0"
  grep -q "ability.unpublish" "$0"
  echo "cli-hub-device-daemon-e2e self-test ok"
  exit 0
fi

case "$REQUESTS:$CONCURRENCY" in
  *[!0-9:]*|'') echo "--requests and --concurrency must be positive integers" >&2; exit 2 ;;
esac
if [[ "$REQUESTS" -lt 1 || "$CONCURRENCY" -lt 1 ]]; then
  echo "--requests and --concurrency must be positive integers" >&2
  exit 2
fi

if [[ "$BUILD" == "1" || ! -x "$CLI_BIN" || ! -x "$DAEMON_BIN" || ! -x "$KEYRING_BIN" ]]; then
  echo "==> building CLI binaries only"
  "$REPO_ROOT/tools/scripts/build-daemon-process-set.sh" --bin easynet
fi

mkdir -p "$OUT_DIR"
# Keep the path short. macOS Unix-domain sockets fail with "path must be
# shorter than SUN_LEN" when HOME lives under the long per-user TMPDIR.
WORK_ROOT="$(mktemp -d "/tmp/easynet-chd.XXXXXX")"
HUB_HOME="$WORK_ROOT/hub"
DEV1_HOME="$WORK_ROOT/device-a"
DEV2_HOME="$WORK_ROOT/device-b"
mkdir -p "$HUB_HOME" "$DEV1_HOME" "$DEV2_HOME"
printf '%s\n' "$WORK_ROOT" > "$OUT_DIR/work-root.txt"

cli_home() {
  local home="$1"
  shift
  HOME="$home" \
  EASYNET_DAEMON_BIN="$DAEMON_BIN" \
  EASYNET_KEYRING_BIN="$KEYRING_BIN" \
  EASYNET_BOOTSTRAP_MEDIA_RESOURCES=0 \
  "$CLI_BIN" "$@"
}

dump_logs() {
  echo "==> daemon logs" >&2
  for home in "$HUB_HOME" "$DEV1_HOME" "$DEV2_HOME"; do
    echo "--- $home/.easynet/logs/easynet-daemon.log" >&2
    tail -200 "$home/.easynet/logs/easynet-daemon.log" >&2 2>/dev/null || true
    echo "--- $home/.easynet/logs/easynet-keyring.log" >&2
    tail -120 "$home/.easynet/logs/easynet-keyring.log" >&2 2>/dev/null || true
  done
}

cleanup() {
  local status="$1"
  if [[ "$status" -ne 0 ]]; then
    dump_logs
  fi
  if [[ "$KEEP" != "1" ]]; then
    cli_home "$DEV1_HOME" runtime stop >/dev/null 2>&1 || true
    cli_home "$DEV2_HOME" runtime stop >/dev/null 2>&1 || true
    cli_home "$HUB_HOME" runtime stop >/dev/null 2>&1 || true
    rm -rf "$WORK_ROOT"
  else
    echo "kept work root: $WORK_ROOT"
  fi
}
trap 'cleanup $?' EXIT

free_port() {
  python3 - <<'PY'
import socket
s = socket.socket()
s.bind(("127.0.0.1", 0))
print(s.getsockname()[1])
s.close()
PY
}

wait_tcp() {
  local port="$1"
  python3 - "$port" <<'PY'
import socket, sys, time
port = int(sys.argv[1])
deadline = time.time() + 30
while time.time() < deadline:
    try:
        with socket.create_connection(("127.0.0.1", port), timeout=0.25):
            sys.exit(0)
    except OSError:
        time.sleep(0.1)
sys.exit(1)
PY
}

wait_runtime() {
  local home="$1"
  local label="$2"
  for _ in $(seq 1 80); do
    if cli_home "$home" runtime status --json >"$OUT_DIR/status-$label.latest.json" 2>"$OUT_DIR/status-$label.latest.err"; then
      if jq -e '(.runtime_status // "") != "stopped"' "$OUT_DIR/status-$label.latest.json" >/dev/null; then
        return 0
      fi
    fi
    sleep 0.25
  done
  echo "runtime did not become ready: $label" >&2
  cat "$OUT_DIR/status-$label.latest.json" >&2 2>/dev/null || true
  cat "$OUT_DIR/status-$label.latest.err" >&2 2>/dev/null || true
  return 1
}

wait_hub_devices() {
  local node_a="$1"
  local node_b="$2"
  for _ in $(seq 1 80); do
    if cli_home "$HUB_HOME" device list --state all --format json >"$OUT_DIR/hub-device-list.json" 2>"$OUT_DIR/hub-device-list.err"; then
      if jq -e --arg a "$node_a" --arg b "$node_b" '
        (.nodes // []) as $nodes
        | ([$nodes[] | .node_id // ""] | index($a)) != null
        and ([$nodes[] | .node_id // ""] | index($b)) != null
      ' "$OUT_DIR/hub-device-list.json" >/dev/null; then
        return 0
      fi
    fi
    sleep 0.25
  done
  echo "hub did not observe both devices" >&2
  cat "$OUT_DIR/hub-device-list.json" >&2 2>/dev/null || true
  cat "$OUT_DIR/hub-device-list.err" >&2 2>/dev/null || true
  return 1
}

json_arg() {
  python3 - "$@" <<'PY'
import json, sys
kind = sys.argv[1]
if kind == "ability-publish":
    _, _, owner_ura, ability_name = sys.argv
    manifest = f'''schema_version = "1"
name = "{ability_name}"
description = "CLI daemon e2e custom ability"
[input_schema]
type = "object"
'''
    print(json.dumps({"owner_ura": owner_ura, "manifest_toml": manifest}, separators=(",", ":")))
elif kind == "ability-unpublish":
    _, _, ability_ura = sys.argv
    print(json.dumps({"ability_ura": ability_ura}, separators=(",", ":")))
elif kind == "skill-publish":
    _, _, owner_agent_id, skill_name = sys.argv
    body = f"# {skill_name}\n\nUse this skill during CLI daemon e2e for {owner_agent_id}.\n"
    print(json.dumps({
        "owner_agent_id": owner_agent_id,
        "skill_name": skill_name,
        "skill_md": body,
        "mission_run_id": "cli-hub-device-daemon-e2e",
    }, separators=(",", ":")))
elif kind == "skill-unpublish":
    _, _, owner_agent_id, skill_name = sys.argv
    print(json.dumps({"owner_agent_id": owner_agent_id, "skill_name": skill_name}, separators=(",", ":")))
elif kind == "chat":
    _, _, prompt = sys.argv
    print(json.dumps({"prompt": prompt}, separators=(",", ":")))
else:
    raise SystemExit(f"unknown json kind: {kind}")
PY
}

run_load() {
  local label="$1"
  local home="$2"
  shift 2
  local dir="$OUT_DIR/load-$label"
  mkdir -p "$dir"
  : >"$dir/samples.jsonl"
  echo "==> load: $label requests=$REQUESTS concurrency=$CONCURRENCY"
  local launched=0
  while [[ "$launched" -lt "$REQUESTS" ]]; do
    local pids=()
    local batch=0
    while [[ "$batch" -lt "$CONCURRENCY" && "$launched" -lt "$REQUESTS" ]]; do
      launched=$((launched + 1))
      batch=$((batch + 1))
      (
        start="$(python3 - <<'PY'
import time
print(time.perf_counter_ns())
PY
)"
        ok=true
        if ! cli_home "$home" "$@" >"$dir/$launched.out" 2>"$dir/$launched.err"; then
          ok=false
        fi
        end="$(python3 - <<'PY'
import time
print(time.perf_counter_ns())
PY
)"
        python3 - "$launched" "$ok" "$start" "$end" <<'PY'
import json, sys
print(json.dumps({
    "request": int(sys.argv[1]),
    "ok": sys.argv[2] == "true",
    "ms": (int(sys.argv[4]) - int(sys.argv[3])) / 1_000_000.0,
}, separators=(",", ":")))
PY
      ) >>"$dir/samples.jsonl" &
      pids+=("$!")
    done
    for pid in "${pids[@]}"; do
      wait "$pid" || true
    done
  done
  python3 - "$dir/samples.jsonl" >"$dir/summary.json" <<'PY'
import json, statistics, sys
rows = [json.loads(line) for line in open(sys.argv[1], encoding="utf-8") if line.strip()]
ok = [r["ms"] for r in rows if r["ok"]]
def pct(values, q):
    if not values:
        return None
    values = sorted(values)
    return values[min(len(values)-1, max(0, round((len(values)-1)*q)))]
print(json.dumps({
    "count": len(rows),
    "ok": sum(1 for r in rows if r["ok"]),
    "fail": sum(1 for r in rows if not r["ok"]),
    "avg_ms": round(statistics.mean(ok), 2) if ok else None,
    "p50_ms": round(pct(ok, 0.50), 2) if ok else None,
    "p95_ms": round(pct(ok, 0.95), 2) if ok else None,
    "p99_ms": round(pct(ok, 0.99), 2) if ok else None,
    "max_ms": round(max(ok), 2) if ok else None,
}, separators=(",", ":")))
PY
}

echo "==> generating local TLS material"
HUB_PORT="$(free_port)"
CERT_DIR="$WORK_ROOT/certs"
mkdir -p "$CERT_DIR"
cat >"$CERT_DIR/ca.cnf" <<'EOF'
[req]
distinguished_name = dn
x509_extensions = v3_ca
prompt = no
[dn]
CN = EasyNet CLI E2E CA
[v3_ca]
basicConstraints = critical, CA:true
keyUsage = critical, keyCertSign, cRLSign
EOF
cat >"$CERT_DIR/leaf.cnf" <<'EOF'
[req]
distinguished_name = dn
prompt = no
[dn]
CN = 127.0.0.1
[v3_leaf]
subjectAltName = @alt_names
basicConstraints = critical, CA:false
keyUsage = critical, digitalSignature, keyEncipherment
extendedKeyUsage = serverAuth
[alt_names]
IP.1 = 127.0.0.1
DNS.1 = localhost
EOF
openssl req -x509 -newkey rsa:2048 -nodes -days 3 \
  -keyout "$CERT_DIR/ca.key" \
  -out "$CERT_DIR/ca.crt" \
  -config "$CERT_DIR/ca.cnf" >/dev/null 2>&1
openssl req -newkey rsa:2048 -nodes \
  -keyout "$CERT_DIR/hub.key" \
  -out "$CERT_DIR/hub.csr" \
  -config "$CERT_DIR/leaf.cnf" >/dev/null 2>&1
openssl x509 -req -days 3 \
  -in "$CERT_DIR/hub.csr" \
  -CA "$CERT_DIR/ca.crt" \
  -CAkey "$CERT_DIR/ca.key" \
  -CAcreateserial \
  -out "$CERT_DIR/hub.crt" \
  -extfile "$CERT_DIR/leaf.cnf" \
  -extensions v3_leaf >/dev/null 2>&1

echo "==> starting CLI hub daemon"
cli_home "$HUB_HOME" runtime start --as-hub --tenant "$REALM" --bind "127.0.0.1:$HUB_PORT" \
  --cert "$CERT_DIR/hub.crt" --key "$CERT_DIR/hub.key" >"$OUT_DIR/hub-start.txt" 2>&1
wait_tcp "$HUB_PORT"
wait_runtime "$HUB_HOME" hub

echo "==> bootstrapping user principal through CLI hub"
cli_home "$HUB_HOME" principal bootstrap \
  --principal-ura "$ADMIN_URA" \
  --create-idempotency-key "admin-create-$TIMESTAMP" \
  --bind-idempotency-key "admin-bind-$TIMESTAMP" \
  --json >"$OUT_DIR/principal-admin.json"
ADMIN_BINDING="$(jq -r '.principal.bindings[0].binding_id' "$OUT_DIR/principal-admin.json")"

cli_home "$HUB_HOME" principal issue-enrollment \
  --issuer-ura "$ADMIN_URA" \
  --subject-principal-ura "$USER_URA" \
  --proof-ref "$ADMIN_BINDING" \
  --idempotency-key "alice-principal-$TIMESTAMP" \
  --json >"$OUT_DIR/enrollment-alice-principal.json"
ALICE_ENROLLMENT="$(jq -r '.principal.enrollments[-1].enrollment_id' "$OUT_DIR/enrollment-alice-principal.json")"

cli_home "$HUB_HOME" principal enroll \
  --principal-ura "$USER_URA" \
  --enrollment-id "$ALICE_ENROLLMENT" \
  --create-idempotency-key "alice-create-$TIMESTAMP" \
  --bind-idempotency-key "alice-bind-$TIMESTAMP" \
  --json >"$OUT_DIR/principal-alice.json"

issue_device_enrollment() {
  local name="$1"
  cli_home "$HUB_HOME" principal issue-enrollment \
    --issuer-ura "$ADMIN_URA" \
    --subject-principal-ura "$USER_URA" \
    --proof-ref "$ADMIN_BINDING" \
    --idempotency-key "$name-$TIMESTAMP" \
    --json >"$OUT_DIR/enrollment-$name.json"
  jq -r '.principal.enrollments[-1].enrollment_id' "$OUT_DIR/enrollment-$name.json"
}

DEV1_ENROLLMENT="$(issue_device_enrollment device-a)"
DEV2_ENROLLMENT="$(issue_device_enrollment device-b)"

echo "==> joining two CLI devices"
cli_home "$DEV1_HOME" device join "$HUB_URA" \
  --principal-ura "$USER_URA" \
  --principal-enrollment-id "$DEV1_ENROLLMENT" \
  --hub-ca "$CERT_DIR/ca.crt" \
  --hub-port "$HUB_PORT" \
  --boot no --yes >"$OUT_DIR/device-a-join.txt" 2>&1
cli_home "$DEV2_HOME" device join "$HUB_URA" \
  --principal-ura "$USER_URA" \
  --principal-enrollment-id "$DEV2_ENROLLMENT" \
  --hub-ca "$CERT_DIR/ca.crt" \
  --hub-port "$HUB_PORT" \
  --boot no --yes >"$OUT_DIR/device-b-join.txt" 2>&1

DEV1_NODE="$(jq -r '.node_id' "$DEV1_HOME/.easynet/credentials.json")"
DEV2_NODE="$(jq -r '.node_id' "$DEV2_HOME/.easynet/credentials.json")"
USERNAME="$(jq -r '.username' "$DEV1_HOME/.easynet/credentials.json")"

echo "==> starting CLI device daemons"
cli_home "$DEV1_HOME" runtime start >"$OUT_DIR/device-a-start.txt" 2>&1
cli_home "$DEV2_HOME" runtime start >"$OUT_DIR/device-b-start.txt" 2>&1
wait_runtime "$DEV1_HOME" device-a
wait_runtime "$DEV2_HOME" device-b

echo "==> querying device online state"
wait_hub_devices "$DEV1_NODE" "$DEV2_NODE"
cli_home "$HUB_HOME" device show "$DEV1_NODE" --format json >"$OUT_DIR/hub-device-a-show.json" 2>"$OUT_DIR/hub-device-a-show.err"
cli_home "$HUB_HOME" device show "$DEV2_NODE" --format json >"$OUT_DIR/hub-device-b-show.json" 2>"$OUT_DIR/hub-device-b-show.err"

FAKE_AGENT="$WORK_ROOT/fake-agent.sh"
cat >"$FAKE_AGENT" <<'EOF'
#!/usr/bin/env sh
agent="${1:-fake}"
prompt="$(cat)"
printf 'fake-agent[%s]: %s\n' "$agent" "$prompt"
EOF
chmod +x "$FAKE_AGENT"

AGENT_A="e2e_alpha"
AGENT_B="e2e_beta"
TMP_AGENT="e2e_delete_me"
ABILITY_A="custom_probe"
SKILL_A="custom-probe-skill"
OWNER_A="easynet:///r/${REALM}/agent/${USERNAME}.${AGENT_A}"
CUSTOM_ABILITY_URA="easynet:///r/${REALM}/ability/${USERNAME}.${AGENT_A}.${ABILITY_A}"
DEV1_ABILITY_PUBLISH="easynet:///r/${REALM}/ability/device.${DEV1_NODE}.ability.publish"
DEV1_ABILITY_UNPUBLISH="easynet:///r/${REALM}/ability/device.${DEV1_NODE}.ability.unpublish"
DEV1_SKILL_PUBLISH="easynet:///r/${REALM}/ability/device.${DEV1_NODE}.skill.publish"
DEV1_SKILL_UNPUBLISH="easynet:///r/${REALM}/ability/device.${DEV1_NODE}.skill.unpublish"

echo "==> agent CRUD and run"
cli_home "$DEV1_HOME" agent add "$AGENT_A" --type external --command "$FAKE_AGENT" --arg "$AGENT_A" --label "$AGENT_A" >"$OUT_DIR/agent-a-add.txt" 2>&1
cli_home "$DEV2_HOME" agent add "$AGENT_B" --type external --command "$FAKE_AGENT" --arg "$AGENT_B" --label "$AGENT_B" >"$OUT_DIR/agent-b-add.txt" 2>&1
cli_home "$DEV1_HOME" agent add "$TMP_AGENT" --type external --command "$FAKE_AGENT" --arg "$TMP_AGENT" --label "$TMP_AGENT" >"$OUT_DIR/agent-temp-add.txt" 2>&1
cli_home "$DEV1_HOME" agent set "$AGENT_A" --model "e2e-updated-model" >"$OUT_DIR/agent-a-set.txt" 2>&1
cli_home "$DEV1_HOME" agent list >"$OUT_DIR/agent-a-list.txt" 2>&1
cli_home "$DEV1_HOME" agent abilities "$AGENT_A" >"$OUT_DIR/agent-a-abilities.before.txt" 2>&1
cli_home "$DEV1_HOME" agent send "$AGENT_A" "hello from CLI daemon e2e" >"$OUT_DIR/agent-a-send.txt" 2>&1
cli_home "$DEV1_HOME" agent remove "$TMP_AGENT" --purge >"$OUT_DIR/agent-temp-remove.txt" 2>&1

echo "==> ability and skill publish/query/invoke/delete"
cli_home "$DEV1_HOME" ability invoke "$DEV1_ABILITY_PUBLISH" \
  --args "$(json_arg ability-publish "$OWNER_A" "$ABILITY_A")" --raw >"$OUT_DIR/ability-publish.json" 2>"$OUT_DIR/ability-publish.err"
cli_home "$DEV1_HOME" agent refresh --agent "$AGENT_A" >"$OUT_DIR/agent-a-refresh.txt" 2>&1 || true
cli_home "$DEV1_HOME" agent abilities "$AGENT_A" >"$OUT_DIR/agent-a-abilities.after-publish.txt" 2>&1
cli_home "$DEV1_HOME" ability list --agent "$AGENT_A" --format json >"$OUT_DIR/ability-list-agent-a.json" 2>"$OUT_DIR/ability-list-agent-a.err"

cli_home "$DEV1_HOME" ability invoke "$DEV1_SKILL_PUBLISH" \
  --args "$(json_arg skill-publish "$AGENT_A" "$SKILL_A")" --raw >"$OUT_DIR/skill-publish.json" 2>"$OUT_DIR/skill-publish.err"
cli_home "$DEV1_HOME" skill list --agent "$AGENT_A" --json >"$OUT_DIR/skill-list-agent-a.json" 2>"$OUT_DIR/skill-list-agent-a.err"

CHAT_URA="$(python3 - "$OUT_DIR/ability-list-agent-a.json" "$USERNAME" "$AGENT_A" <<'PY'
import json, sys
payload = json.load(open(sys.argv[1], encoding="utf-8"))
target_tail = f"/ability/{sys.argv[2]}.{sys.argv[3]}.chat"
def walk(v):
    if isinstance(v, dict):
        for key in ("ability_ura", "ura", "subject_ura"):
            value = v.get(key)
            if isinstance(value, str) and target_tail in value:
                return value
        for child in v.values():
            found = walk(child)
            if found:
                return found
    if isinstance(v, list):
        for child in v:
            found = walk(child)
            if found:
                return found
    return None
print(walk(payload) or f"easynet:///r/localhost/ability/{sys.argv[2]}.{sys.argv[3]}.chat")
PY
)"
cli_home "$DEV1_HOME" ability invoke "$CHAT_URA" \
  --args "$(json_arg chat "hello through ability invoke")" --raw >"$OUT_DIR/chat-ability-invoke.json" 2>"$OUT_DIR/chat-ability-invoke.err"

cli_home "$DEV1_HOME" ability invoke "$DEV1_ABILITY_UNPUBLISH" \
  --args "$(json_arg ability-unpublish "$CUSTOM_ABILITY_URA")" --raw >"$OUT_DIR/ability-unpublish.json" 2>"$OUT_DIR/ability-unpublish.err"
cli_home "$DEV1_HOME" ability invoke "$DEV1_SKILL_UNPUBLISH" \
  --args "$(json_arg skill-unpublish "$AGENT_A" "$SKILL_A")" --raw >"$OUT_DIR/skill-unpublish.json" 2>"$OUT_DIR/skill-unpublish.err"
cli_home "$DEV1_HOME" agent abilities "$AGENT_A" >"$OUT_DIR/agent-a-abilities.after-delete.txt" 2>&1
cli_home "$DEV1_HOME" skill list --agent "$AGENT_A" --json >"$OUT_DIR/skill-list-agent-a.after-delete.json" 2>"$OUT_DIR/skill-list-agent-a.after-delete.err"

echo "==> load testing CLI query entrypoints"
run_load "hub-device-list" "$HUB_HOME" device list --state all --format json
run_load "device-a-agent-list" "$DEV1_HOME" agent list
run_load "device-a-agent-abilities" "$DEV1_HOME" agent abilities "$AGENT_A"
run_load "device-a-ability-list" "$DEV1_HOME" ability list --agent "$AGENT_A" --format json
run_load "device-a-skill-list" "$DEV1_HOME" skill list --agent "$AGENT_A" --json

echo "==> removing second device through CLI hub"
DEVICE_B_URA="easynet:///r/${REALM}/device/${DEV2_NODE}"
cli_home "$HUB_HOME" device remove "$DEVICE_B_URA" --yes --reason "cli daemon e2e cleanup" >"$OUT_DIR/device-b-remove.txt" 2>"$OUT_DIR/device-b-remove.err"
cli_home "$HUB_HOME" device list --state all --format json >"$OUT_DIR/hub-device-list.after-remove.json" 2>"$OUT_DIR/hub-device-list.after-remove.err"

python3 - "$OUT_DIR" "$DEV1_NODE" "$DEV2_NODE" "$USERNAME" "$AGENT_A" "$AGENT_B" "$CHAT_URA" <<'PY' >"$OUT_DIR/report.json"
import json, sys
from pathlib import Path
out = Path(sys.argv[1])
def load_json(name):
    path = out / name
    if not path.exists() or not path.read_text(encoding="utf-8").strip():
        return None
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except Exception:
        return None
def contains(name, needle):
    path = out / name
    return path.exists() and needle in path.read_text(encoding="utf-8", errors="replace")
def node_ids(name):
    payload = load_json(name) or {}
    return [str(row.get("node_id", "")) for row in payload.get("nodes", []) if isinstance(row, dict)]
load = {}
for d in out.glob("load-*"):
    summary = load_json(f"{d.name}/summary.json")
    if summary is not None:
        load[d.name.removeprefix("load-")] = summary
print(json.dumps({
    "scope": "CLI-only hub/device daemon E2E; no Go backend HTTP API",
    "nodes": {"device_a": sys.argv[2], "device_b": sys.argv[3]},
    "username": sys.argv[4],
    "agents": {"device_a": sys.argv[5], "device_b": sys.argv[6]},
    "chat_ability_ura": sys.argv[7],
    "assertions": {
        "hub_saw_device_a": sys.argv[2] in node_ids("hub-device-list.json"),
        "hub_saw_device_b": sys.argv[3] in node_ids("hub-device-list.json"),
        "hub_removed_device_b": sys.argv[3] not in node_ids("hub-device-list.after-remove.json"),
        "agent_a_listed": contains("agent-a-list.txt", sys.argv[5]),
        "agent_a_send_ran_fake_agent": contains("agent-a-send.txt", "fake-agent[e2e_alpha]"),
        "custom_ability_visible_after_publish": contains("agent-a-abilities.after-publish.txt", "custom_probe"),
        "custom_ability_removed_after_unpublish": not contains("agent-a-abilities.after-delete.txt", "custom_probe"),
        "custom_skill_visible_after_publish": contains("skill-list-agent-a.json", "custom-probe-skill"),
        "custom_skill_removed_after_unpublish": not contains("skill-list-agent-a.after-delete.json", "custom-probe-skill"),
    },
    "load": load,
}, indent=2, sort_keys=True))
PY

jq -e '
  .assertions.hub_saw_device_a
  and .assertions.hub_saw_device_b
  and .assertions.hub_removed_device_b
  and .assertions.agent_a_listed
  and .assertions.agent_a_send_ran_fake_agent
  and .assertions.custom_ability_visible_after_publish
  and .assertions.custom_ability_removed_after_unpublish
  and .assertions.custom_skill_visible_after_publish
  and .assertions.custom_skill_removed_after_unpublish
  and ([.load[] | select(.fail != 0)] | length == 0)
' "$OUT_DIR/report.json" >/dev/null

python3 - "$OUT_DIR/report.json" >"$OUT_DIR/report.md" <<'PY'
import json, sys
r = json.load(open(sys.argv[1], encoding="utf-8"))
print("# CLI Hub/Device Daemon E2E")
print("")
print(f"- Scope: {r['scope']}")
print(f"- User: `{r['username']}`")
print(f"- Device A: `{r['nodes']['device_a']}`")
print(f"- Device B: `{r['nodes']['device_b']}`")
print(f"- Chat ability: `{r['chat_ability_ura']}`")
print("")
print("## Assertions")
for k, v in r["assertions"].items():
    print(f"- `{k}`: `{str(v).lower()}`")
print("")
print("## Load")
print("| Target | OK/Total | Avg ms | P50 | P95 | P99 | Max |")
print("|---|---:|---:|---:|---:|---:|---:|")
for name, s in sorted(r["load"].items()):
    print(f"| `{name}` | {s['ok']}/{s['count']} | {s['avg_ms']} | {s['p50_ms']} | {s['p95_ms']} | {s['p99_ms']} | {s['max_ms']} |")
PY

echo "PASS: $OUT_DIR/report.md"
