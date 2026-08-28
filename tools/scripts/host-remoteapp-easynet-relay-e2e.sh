#!/usr/bin/env bash
# Real Hub lease + Browser + daemon + coturn RemoteApp relay proof.

set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"
EASYNET_ROOT="${EASYNET_REMOTEAPP_EASYNET_RELAY_E2E_BACKEND_ROOT:-$REPO_ROOT/../EasyNet}"
FRONTEND_ROOT="${EASYNET_REMOTEAPP_EASYNET_RELAY_E2E_FRONTEND_ROOT:-$EASYNET_ROOT/Frontend}"
COMPOSE_FILE="$EASYNET_ROOT/docker/e2e/docker-compose.yml"
PROJECTOR="$SELF_DIR/project-remoteapp-network-scenario.py"
VERIFIER="$SELF_DIR/remoteapp-network-fallback-e2e.sh"
RELAY_REFRESH_VERIFIER="$SELF_DIR/verify-remoteapp-relay-refresh.py"
FIXTURE_DIR="$REPO_ROOT/tools/fixtures/remoteapp-turn-netem"
RUNTIME_CLI="${EASYNET_REMOTEAPP_EASYNET_RELAY_E2E_RUNTIME_CLI:-$REPO_ROOT/target/debug/easynet}"
RUNTIME_DAEMON="${EASYNET_REMOTEAPP_EASYNET_RELAY_E2E_RUNTIME_DAEMON:-$(dirname "$RUNTIME_CLI")/easynet-daemon}"
BROWSER_RUNNER="$FRONTEND_ROOT/scripts/remoteapp-browser-lifecycle.mjs"
CREDENTIALS_PATH="${EASYNET_REMOTEAPP_EASYNET_RELAY_E2E_CREDENTIALS_PATH:-$HOME/.easynet/credentials.json}"
RESUME_DISCONNECT_ACTION="${EASYNET_REMOTEAPP_EASYNET_RELAY_E2E_RESUME_DISCONNECT_ACTION:-}"
RESUME_RECONNECT_ACTION="${EASYNET_REMOTEAPP_EASYNET_RELAY_E2E_RESUME_RECONNECT_ACTION:-}"

MODE=skip
RELAY_REFRESH_RESUME=0
OUT_DIR="${EASYNET_REMOTEAPP_EASYNET_RELAY_E2E_OUT_DIR:-$REPO_ROOT/target/e2e/remoteapp-easynet-relay/$(date -u +%Y%m%d-%H%M%S)-$$}"
FRONTEND_URL="${EASYNET_REMOTEAPP_EASYNET_RELAY_E2E_FRONTEND_URL:-http://127.0.0.1:3000}"
DEVICE_ID="${EASYNET_REMOTEAPP_EASYNET_RELAY_E2E_DEVICE_ID:-}"
TARGET_KIND="${EASYNET_REMOTEAPP_EASYNET_RELAY_E2E_TARGET_KIND:-window}"
COMPOSE_PROJECT="${EASYNET_DOCKER_PROJECT:-easynet-dev}"
HUB_HTTP_PORT="${HUB_HTTP_PORT:-8080}"
HUB_TLS_PORT="${HUB_TLS_PORT:-50443}"
TURN_PORT="${EASYNET_REMOTEAPP_EASYNET_RELAY_E2E_PORT:-3480}"
TURN_MIN_PORT="${EASYNET_REMOTEAPP_EASYNET_RELAY_E2E_MIN_PORT:-49240}"
TURN_MAX_PORT="${EASYNET_REMOTEAPP_EASYNET_RELAY_E2E_MAX_PORT:-49260}"

usage() {
  cat <<'USAGE'
Usage:
  host-remoteapp-easynet-relay-e2e.sh --run --device-id UUID [--out-dir DIR]
  host-remoteapp-easynet-relay-e2e.sh --run --refresh-resume --device-id UUID [--out-dir DIR]
  host-remoteapp-easynet-relay-e2e.sh --self-test

Required run environment:
  EASYNET_REMOTEAPP_BROWSER_EMAIL
  EASYNET_REMOTEAPP_BROWSER_PASSWORD

The runner temporarily enables Hub-owned relay leases in the local easynet-dev
stack, starts coturn in TURN REST mode, restarts the paired host daemon without
static TURN environment credentials, drives a real Browser RemoteApp flow with
relay-only ICE, and restores both Hub and daemon ordinary configuration.

--refresh-resume accelerates the Hub lease TTL, waits for the daemon-owned
refresh state machine to rotate the lease, restarts the daemon, and requires
the same public session to bind a newer WebRTC transport using the refreshed
lease before terminal cleanup.

When the provider daemon does not run on this host, set both
EASYNET_REMOTEAPP_EASYNET_RELAY_E2E_RESUME_DISCONNECT_ACTION and
EASYNET_REMOTEAPP_EASYNET_RELAY_E2E_RESUME_RECONNECT_ACTION to commands that
stop and start the selected provider. The runner owns best-effort reconnect on
cleanup after the Browser lifecycle has been armed.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --run) MODE=run; shift ;;
    --self-test) MODE=self-test; shift ;;
    --refresh-resume) RELAY_REFRESH_RESUME=1; shift ;;
    --device-id) DEVICE_ID="${2:?missing value for --device-id}"; shift 2 ;;
    --out-dir) OUT_DIR="${2:?missing value for --out-dir}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 64 ;;
  esac
done

if [[ "$RELAY_REFRESH_RESUME" -eq 1 ]]; then
  RELAY_TTL_SECONDS="${EASYNET_REMOTEAPP_EASYNET_RELAY_E2E_LEASE_TTL_SECONDS:-30}"
  RELAY_REFRESH_LEAD_SECONDS="${EASYNET_REMOTEAPP_EASYNET_RELAY_E2E_REFRESH_LEAD_SECONDS:-20}"
else
  RELAY_TTL_SECONDS="${EASYNET_REMOTEAPP_EASYNET_RELAY_E2E_LEASE_TTL_SECONDS:-300}"
  RELAY_REFRESH_LEAD_SECONDS="${EASYNET_REMOTEAPP_EASYNET_RELAY_E2E_REFRESH_LEAD_SECONDS:-120}"
fi

mkdir -p "$OUT_DIR"
OUT_DIR="$(cd "$OUT_DIR" && pwd)"

write_status() {
  local status="$1"
  local reason="$2"
  python3 - "$OUT_DIR/report.json" "$status" "$reason" <<'PY'
import json
import pathlib
import sys

path, status, reason = sys.argv[1:]
pathlib.Path(path).write_text(json.dumps({
    "script": "tools/scripts/host-remoteapp-easynet-relay-e2e.sh",
    "status": status,
    "reason": reason,
    "coverage": {
        "easynet_relay": False,
        "relay_lease_refresh_resume": False,
    },
    "product_complete_claim": False,
}, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY
}

coturn_log_is_ready() {
  # Consume the complete log snapshot. A `grep -q` consumer exits at the first
  # match and can SIGPIPE `docker logs`; with `pipefail` that converts a real
  # ready marker into a failed pipeline.
  awk '/Relay ports initialization done/ { found = 1 } END { exit !found }'
}

if [[ "$MODE" == "skip" ]]; then
  write_status skipped "pass --run to execute the live EasyNet relay scenario"
  echo "[host-remoteapp-easynet-relay-e2e] skipped: $OUT_DIR/report.json"
  exit 0
fi

if [[ "$MODE" == "self-test" ]]; then
  bash -n "$0"
  {
    printf '%s\n' 'startup' 'Relay ports initialization done'
    for _ in {1..256}; do printf '%s\n' 'post-ready coturn log line'; done
  } | coturn_log_is_ready
  if printf '%s\n' 'startup without ready marker' | coturn_log_is_ready; then
    echo "coturn readiness parser accepted a missing marker" >&2
    exit 1
  fi
  python3 -m py_compile "$PROJECTOR" "$RELAY_REFRESH_VERIFIER"
  python3 "$RELAY_REFRESH_VERIFIER" --self-test >/dev/null
  "$VERIFIER" --self-test --out-dir "$OUT_DIR/verifier-self-test" >/dev/null
  write_status passed "script syntax, projector import, relay-refresh contract, and network evidence contract passed; no live coverage claimed"
  echo "host-remoteapp-easynet-relay-e2e self-test ok"
  exit 0
fi

for command in curl docker jq node openssl python3 shasum; do
  command -v "$command" >/dev/null 2>&1 || {
    write_status failed "required command is unavailable: $command"
    exit 1
  }
done
[[ -x "$RUNTIME_CLI" ]] || { write_status failed "runtime CLI missing: $RUNTIME_CLI"; exit 1; }
[[ -x "$RUNTIME_DAEMON" ]] || { write_status failed "runtime daemon missing: $RUNTIME_DAEMON"; exit 1; }
grep -a -q '/api/v1/devices/relay-leases/acquire' "$RUNTIME_DAEMON" || {
  write_status failed "runtime daemon was not built with the Hub relay lease adapter"
  exit 1
}
[[ -f "$BROWSER_RUNNER" ]] || { write_status failed "Browser runner missing: $BROWSER_RUNNER"; exit 1; }
[[ -n "$DEVICE_ID" ]] || { write_status failed "--device-id is required"; exit 64; }
[[ -f "$CREDENTIALS_PATH" ]] || { write_status failed "paired device credentials missing: $CREDENTIALS_PATH"; exit 1; }
credential_node_id="$(jq -er '.node_id' "$CREDENTIALS_PATH")" || {
  write_status failed "provider credentials do not contain node_id: $CREDENTIALS_PATH"
  exit 1
}
[[ "$credential_node_id" == "$DEVICE_ID" ]] || {
  write_status failed "relay terminal probe credentials belong to $credential_node_id, expected provider $DEVICE_ID"
  exit 1
}
[[ -f "$COMPOSE_FILE" ]] || { write_status failed "EasyNet dev compose missing: $COMPOSE_FILE"; exit 1; }
[[ -f "$PROJECTOR" && -f "$RELAY_REFRESH_VERIFIER" && -x "$VERIFIER" ]] || {
  write_status failed "network projector/verifier or relay-refresh verifier missing"
  exit 1
}
: "${EASYNET_REMOTEAPP_BROWSER_EMAIL:?EASYNET_REMOTEAPP_BROWSER_EMAIL is required}"
: "${EASYNET_REMOTEAPP_BROWSER_PASSWORD:?EASYNET_REMOTEAPP_BROWSER_PASSWORD is required}"
[[ "$TARGET_KIND" == "window" || "$TARGET_KIND" == "application" ]] || {
  write_status failed "target kind must be window or application"
  exit 64
}
for value in "$TURN_PORT" "$TURN_MIN_PORT" "$TURN_MAX_PORT"; do
  [[ "$value" =~ ^[0-9]+$ ]] && (( value > 0 && value <= 65535 )) || {
    write_status failed "TURN ports must be integers in 1..65535"
    exit 64
  }
done
(( TURN_MIN_PORT <= TURN_MAX_PORT )) || { write_status failed "TURN relay port range is invalid"; exit 64; }
for value in "$RELAY_TTL_SECONDS" "$RELAY_REFRESH_LEAD_SECONDS"; do
  [[ "$value" =~ ^[0-9]+$ ]] && (( value > 0 )) || {
    write_status failed "relay TTL and refresh lead must be positive integers"
    exit 64
  }
done
(( RELAY_TTL_SECONDS <= 900 )) || {
  write_status failed "relay TTL must not exceed the Hub's 15-minute lease bound"
  exit 64
}
(( RELAY_REFRESH_LEAD_SECONDS < RELAY_TTL_SECONDS )) || {
  write_status failed "relay refresh lead must be shorter than the lease TTL"
  exit 64
}
if [[ "$RELAY_REFRESH_RESUME" -eq 1 ]]; then
  (( RELAY_TTL_SECONDS - RELAY_REFRESH_LEAD_SECONDS <= 20 )) || {
    write_status failed "accelerated relay refresh threshold must fit the Browser lifecycle command deadline"
    exit 64
  }
  if [[ -z "$RESUME_DISCONNECT_ACTION" ]]; then
    printf -v RESUME_DISCONNECT_ACTION '%q runtime stop' "$RUNTIME_CLI"
  fi
  if [[ -z "$RESUME_RECONNECT_ACTION" ]]; then
    printf -v RESUME_RECONNECT_ACTION '%q runtime start' "$RUNTIME_CLI"
  fi
  for action in "$RESUME_DISCONNECT_ACTION" "$RESUME_RECONNECT_ACTION"; do
    [[ "$action" != *$'\n'* && "$action" != *$'\r'* ]] || {
      write_status failed "provider lifecycle actions must be single-line shell commands"
      exit 64
    }
  done
fi

compose() {
  HUB_HTTP_PORT="$HUB_HTTP_PORT" HUB_TLS_PORT="$HUB_TLS_PORT" \
    HUB_REALM=localhost HUB_PUBLIC_ENDPOINT="https://127.0.0.1:$HUB_TLS_PORT" \
    docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_FILE" "$@"
}

wait_for_hub() {
  for _ in {1..120}; do
    curl -fsS --max-time 2 "http://127.0.0.1:$HUB_HTTP_PORT/api/v1/health" >/dev/null 2>&1 && return 0
    sleep 1
  done
  return 1
}

TEMP_DIR="$(mktemp -d)"
CONTAINER="easynet-remoteapp-hub-relay-$$"
DAEMON_WAS_RUNNING=0
pgrep -f '/easynet-daemon$' >/dev/null 2>&1 && DAEMON_WAS_RUNNING=1
RESTORE_FAILED=0
RESUME_PROVIDER_LIFECYCLE_ARMED=0
RESUME_RECONNECT_COMPLETE_FILE=""

cleanup() {
  local exit_code=$?
  if [[ "$RESUME_PROVIDER_LIFECYCLE_ARMED" -eq 1 && ! -e "$RESUME_RECONNECT_COMPLETE_FILE" ]]; then
    /bin/sh -lc "$RESUME_RECONNECT_ACTION" >"$OUT_DIR/provider-restore.stdout.txt" \
      2>"$OUT_DIR/provider-restore.stderr.txt" || RESTORE_FAILED=1
  fi
  "$RUNTIME_CLI" runtime stop >/dev/null 2>&1 || true
  docker stop "$CONTAINER" >/dev/null 2>&1 || true
  if ! compose up -d --force-recreate hub >"$OUT_DIR/hub-restore.stdout.txt" 2>"$OUT_DIR/hub-restore.stderr.txt" || ! wait_for_hub; then
    RESTORE_FAILED=1
  fi
  if [[ "$DAEMON_WAS_RUNNING" -eq 1 ]]; then
    "$RUNTIME_CLI" runtime start >"$OUT_DIR/runtime-restore.stdout.txt" \
      2>"$OUT_DIR/runtime-restore.stderr.txt" || RESTORE_FAILED=1
  fi
  rm -rf "$TEMP_DIR"
  if [[ "$exit_code" -ne 0 && ! -f "$OUT_DIR/report.json" ]]; then
    write_status failed "live EasyNet relay scenario failed; inspect the preserved runner artifacts"
  fi
  if [[ "$RESTORE_FAILED" -ne 0 && "$exit_code" -eq 0 ]]; then
    write_status failed "EasyNet relay proof passed but ordinary Hub/daemon restore failed"
    exit 1
  fi
  exit "$exit_code"
}
trap cleanup EXIT INT TERM

fixture_hash="$(shasum -a 256 "$FIXTURE_DIR/Dockerfile" | awk '{print substr($1,1,16)}')"
fixture_image="easynet/remoteapp-turn-netem:e2e-$fixture_hash"
if ! docker image inspect "$fixture_image" >/dev/null 2>&1; then
  docker build --tag "$fixture_image" "$FIXTURE_DIR" \
    >"$OUT_DIR/fixture-build.stdout.txt" 2>"$OUT_DIR/fixture-build.stderr.txt"
fi

relay_secret="$(openssl rand -hex 32)"
docker run --rm -d --name "$CONTAINER" \
  -p "$TURN_PORT:$TURN_PORT/udp" -p "$TURN_PORT:$TURN_PORT/tcp" \
  -p "$TURN_MIN_PORT-$TURN_MAX_PORT:$TURN_MIN_PORT-$TURN_MAX_PORT/udp" \
  "$fixture_image" \
  --log-file=stdout --verbose --fingerprint --use-auth-secret \
  --static-auth-secret="$relay_secret" --realm=localhost \
  --listening-port="$TURN_PORT" --min-port="$TURN_MIN_PORT" --max-port="$TURN_MAX_PORT" \
  --allow-loopback-peers '--external-ip=$(detect-external-ip)' >/dev/null

ready=0
for _ in {1..100}; do
  if docker logs "$CONTAINER" 2>&1 | coturn_log_is_ready; then
    ready=1
    break
  fi
  sleep 0.1
done
[[ "$ready" -eq 1 ]] || { write_status failed "coturn fixture did not become ready"; exit 1; }

EASYNET_RELAY_ENABLED=true \
EASYNET_RELAY_URLS="turn:127.0.0.1:$TURN_PORT?transport=udp" \
EASYNET_RELAY_SHARED_SECRET="$relay_secret" \
EASYNET_RELAY_LEASE_TTL_SECONDS="$RELAY_TTL_SECONDS" \
EASYNET_RELAY_REFRESH_LEAD_SECONDS="$RELAY_REFRESH_LEAD_SECONDS" \
compose up -d --force-recreate hub >"$OUT_DIR/hub-relay-start.stdout.txt" \
  2>"$OUT_DIR/hub-relay-start.stderr.txt"
wait_for_hub || { write_status failed "Hub did not become healthy with relay leases enabled"; exit 1; }

constraints_applied_at_ms="$(python3 -c 'import time; print(int(time.time() * 1000))')"
"$RUNTIME_CLI" runtime stop >"$OUT_DIR/runtime-stop.stdout.txt" 2>"$OUT_DIR/runtime-stop.stderr.txt" || true
env -u EASYNET_REMOTE_DESKTOP_TURN_URLS \
  -u EASYNET_REMOTE_DESKTOP_TURN_USERNAME \
  -u EASYNET_REMOTE_DESKTOP_TURN_CREDENTIAL \
  "$RUNTIME_CLI" runtime start >"$OUT_DIR/runtime-start.stdout.txt" 2>"$OUT_DIR/runtime-start.stderr.txt"
sleep 2

browser_dir="$OUT_DIR/browser"
mkdir -p "$browser_dir"
relay_refresh_ready_file="$browser_dir/relay-refresh-ready.json"
resume_reconnect_complete_file="$browser_dir/resume-reconnect-complete"
if [[ "$RELAY_REFRESH_RESUME" -eq 1 && -e "$relay_refresh_ready_file" ]]; then
  write_status failed "refusing stale relay refresh coordination file: $relay_refresh_ready_file"
  exit 1
fi
if [[ "$RELAY_REFRESH_RESUME" -eq 1 && -e "$resume_reconnect_complete_file" ]]; then
  write_status failed "refusing stale provider reconnect marker: $resume_reconnect_complete_file"
  exit 1
fi
if [[ "$RELAY_REFRESH_RESUME" -eq 1 ]]; then
  RESUME_RECONNECT_COMPLETE_FILE="$resume_reconnect_complete_file"
  RESUME_PROVIDER_LIFECYCLE_ARMED=1
fi
(
  cd "$FRONTEND_ROOT"
  export EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_EVIDENCE_JSON="$browser_dir/evidence.json"
  export EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_FRONTEND_URL="$FRONTEND_URL"
  export EASYNET_REMOTEAPP_BROWSER_DEVICE_ID="$DEVICE_ID"
  export EASYNET_REMOTEAPP_BROWSER_TARGET_KIND="$TARGET_KIND"
  export EASYNET_REMOTEAPP_BROWSER_ICE_TRANSPORT_POLICY=relay
  if [[ "$RELAY_REFRESH_RESUME" -eq 1 ]]; then
    export EASYNET_REMOTEAPP_BROWSER_REQUIRE_RELAY_LEASE_REFRESH=1
    export EASYNET_REMOTEAPP_RELAY_REFRESH_READY_FILE="$relay_refresh_ready_file"
    export EASYNET_REMOTEAPP_RELAY_REFRESH_RECONNECT_COMPLETE_FILE="$resume_reconnect_complete_file"
    export EASYNET_REMOTEAPP_BROWSER_RELAY_REFRESH_READY_FILE="$relay_refresh_ready_file"
    export EASYNET_REMOTEAPP_RELAY_REFRESH_DISCONNECT_ACTION="$RESUME_DISCONNECT_ACTION"
    export EASYNET_REMOTEAPP_RELAY_REFRESH_RECONNECT_ACTION="$RESUME_RECONNECT_ACTION"
    export EASYNET_REMOTEAPP_BROWSER_RESUME_DISCONNECT_COMMAND='remaining=250; while [ ! -s "$EASYNET_REMOTEAPP_RELAY_REFRESH_READY_FILE" ]; do remaining=$((remaining - 1)); [ "$remaining" -gt 0 ] || exit 70; sleep 0.1; done; /bin/sh -lc "$EASYNET_REMOTEAPP_RELAY_REFRESH_DISCONNECT_ACTION"'
    export EASYNET_REMOTEAPP_BROWSER_RESUME_RECONNECT_COMMAND='/bin/sh -lc "$EASYNET_REMOTEAPP_RELAY_REFRESH_RECONNECT_ACTION" && : > "$EASYNET_REMOTEAPP_RELAY_REFRESH_RECONNECT_COMPLETE_FILE"'
  fi
  node "$BROWSER_RUNNER"
) >"$browser_dir/runner.stdout.txt" 2>"$browser_dir/runner.stderr.txt"

projector_args=(
  --browser-evidence "$browser_dir/evidence.json"
  --route-kind easynet_relay
  --constraints-applied-at-ms "$constraints_applied_at_ms"
)
if [[ "$RELAY_REFRESH_RESUME" -eq 1 ]]; then
  python3 "$RELAY_REFRESH_VERIFIER" \
    --browser-evidence "$browser_dir/evidence.json" \
    --output "$OUT_DIR/relay-refresh.json"
  projector_args+=(--relay-refresh "$OUT_DIR/relay-refresh.json")
fi

session_id="$(jq -er '.network_transport.session_id' "$browser_dir/evidence.json")"
resource_ura="$(jq -er '.network_transport.selected_resource_ura' "$browser_dir/evidence.json")"
release_probe_body="$TEMP_DIR/release-probe-response.json"
release_probe_status="$({
  jq -c --arg session_id "$session_id" --arg resource_ura "$resource_ura" '{
    node_id,
    credential_token,
    session_id: $session_id,
    resource_ura: $resource_ura
  }' "$CREDENTIALS_PATH" \
    | curl -sS -o "$release_probe_body" -w '%{http_code}' \
        -H 'Content-Type: application/json' --data-binary @- \
        "http://127.0.0.1:$HUB_HTTP_PORT/api/v1/devices/relay-leases/acquire"
})"
release_probe_code="$(jq -r '.code // .error.code // empty' "$release_probe_body" 2>/dev/null || true)"
release_probe_message="$(jq -r '.message // .msg // .error.message // empty' "$release_probe_body" 2>/dev/null || true)"
jq -n \
  --argjson status_code "$release_probe_status" \
  --arg provider_device_id "$credential_node_id" \
  --arg session_id "$session_id" \
  --arg resource_ura "$resource_ura" \
  --arg response_code "$release_probe_code" \
  --arg response_message "$release_probe_message" \
  --argjson observed_at_ms "$(python3 -c 'import time; print(int(time.time() * 1000))')" \
  '{
    status_code: $status_code,
    terminal_reacquire_rejected: ($status_code == 409),
    provider_device_id: $provider_device_id,
    session_id: $session_id,
    resource_ura: $resource_ura,
    response_code: $response_code,
    response_message: $response_message,
    observed_at_ms: $observed_at_ms
  }' >"$OUT_DIR/release-probe.json"
[[ "$release_probe_status" == "409" ]] || {
  write_status failed "Hub accepted or misclassified lease reacquire after terminal release"
  exit 1
}

docker logs "$CONTAINER" >"$TEMP_DIR/coturn.log" 2>&1
python3 "$PROJECTOR" \
  "${projector_args[@]}" \
  --allocation-log "$TEMP_DIR/coturn.log" \
  --release-probe "$OUT_DIR/release-probe.json" \
  --output "$OUT_DIR/evidence.json"

"$VERIFIER" --run --required-routes easynet_relay \
  --evidence-json "$OUT_DIR/evidence.json" --out-dir "$OUT_DIR/verified" >/dev/null
cp "$OUT_DIR/verified/report.json" "$OUT_DIR/report.json"
echo "[host-remoteapp-easynet-relay-e2e] PASS: $OUT_DIR/report.json"
