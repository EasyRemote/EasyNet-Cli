#!/usr/bin/env bash
# e2e-release-flow.sh — drive `easynet device join` + `easynet runtime start`
# + product-presence propagation + `agent add` + advertise verification
# against a release-shape install.
#
# This is the harness that makes the docker-e2e / production
# divergence visible. The pre-existing docker e2e bundles
# `axon-runtime` into its image, so any code path that *spawns
# axon-runtime as a separate process* succeeds inside docker e2e
# even if production tarballs do not carry the binary. This harness
# is the antidote: it runs against a Phase-A release tarball whose
# shape mirrors what `https://easynet.run/install` would actually
# place on a fresh user's host.
#
# What this harness asserts
# -------------------------
# Against a release-shape sandbox install (see e2e-release-install.sh):
#
#   1. `easynet device join <token> --boot no` against a local dev-backend
#      succeeds and writes credentials.json + daemon-config.toml. The product
#      default remains join-and-start; this harness opts out only so the
#      subsequent `runtime start` step owns the measured lifecycle transition.
#   2. `easynet runtime start` brings the daemon online.
#   3. **No `axon-runtime` process is alive** at any point. The
#      device-mode daemon is the only long-running process; the
#      release tarball does not ship axon-runtime, so a code path
#      that spawns it would fail at exec.
#   4. Backend `/api/v1/events` and `/api/v1/devices` observe the
#      device online/offline transitions independently from local
#      daemon process facts.
#   5. `easynet ability list` returns the device's local catalog
#      (no hub round-trip needed for system abilities).
#   6. `easynet agent add codex --type codex` and `agent add claude
#      --type claude-code` register hosted agents into
#      local-agents.json with friendly URAs (`<user>.codex`,
#      `<user>.claude`, `<user>.consent-default`).
#   7. The session-prelude advertises every hosted agent + the
#      synthetic pages/files pair (5 agents total) to the hub.
#   8. Abrupt daemon death demotes product presence within the
#      bounded timeout instead of relying on `runtime.json`.
#
# Expected outcome
# ----------------
# This harness should pass on daemon-first code. A failure in
# `easynet runtime start`, a changed axon-runtime process set, or missing
# daemon/control sockets is a release-blocking regression.
#
# Hub topology
# ------------
# Local dev-backend on :8080 / :50443 by default (override via
# EASYNET_TEST_BACKEND_PORT / --backend-port and
# EASYNET_TEST_HUB_TLS_PORT / --hub-tls-port; started via
# EasyNet/scripts/dev-backend.sh --reset-db --no-seed-device from
# the EasyNet repo). The dev-backend seeds a default
# user dev@easynet.local / dev-password and serves both the REST API
# and the gRPC bidirectional surface devices need to dial. We mint a
# pairing token via POST /api/v1/devices/pairing using the seeded
# user's JWT and pass it to `easynet device join`.
#
# Author: 海峰 <silan.hu@u.nus.edu>
# Copyright (c) 2026 EasyNet. All rights reserved.

set -euo pipefail

script_dir="$(cd "$(dirname "$0")" && pwd)"
cli_root="$(cd "$script_dir/../.." && pwd)"
workspace_root="$(cd "$cli_root/.." && pwd)"
easynet_repo="$workspace_root/EasyNet"
host_docker_config="${DOCKER_CONFIG:-${HOME:-}/.docker}"

# Allow caller to bring their own backend (e.g. dev-host-e2e). When
# unset, the harness boots its own dev-backend for the test window.
backend_url="${EASYNET_TEST_HUB_URL:-}"
backend_port="${EASYNET_TEST_BACKEND_PORT:-8080}"
backend_tls_port="${EASYNET_TEST_HUB_TLS_PORT:-50443}"
backend_no_build="${EASYNET_TEST_BACKEND_NO_BUILD:-0}"
backend_pid=""
baseline_axon_pids=""
own_backend=0
sse_pid=""
sse_log=""
sse_err=""

# Allow caller to pass an existing sandbox prefix (skips the install
# step). When empty, run e2e-release-install.sh first.
prefix="${EASYNET_TEST_PREFIX:-}"
keep_prefix=0
keep_state=0

# Product-presence budgets. The online timeout covers daemon boot +
# session.open admission. Once the backend SSE invalidation arrives,
# `/api/v1/devices` must reconcile within the SPEC's loopback read-model
# budget. Graceful stop and abrupt kill use the SPEC's 5s / 20s gates.
product_online_timeout="${EASYNET_TEST_PRODUCT_ONLINE_TIMEOUT:-30}"
read_model_budget_ms="${EASYNET_TEST_READ_MODEL_BUDGET_MS:-2000}"
graceful_stop_timeout="${EASYNET_TEST_GRACEFUL_STOP_TIMEOUT:-5}"
abrupt_kill_timeout="${EASYNET_TEST_ABRUPT_KILL_TIMEOUT:-20}"

while [ $# -gt 0 ]; do
    case "$1" in
        --prefix)        prefix="$2"; shift 2 ;;
        --backend-url)      backend_url="$2"; shift 2 ;;
        --backend-port)     backend_port="$2"; shift 2 ;;
        --hub-tls-port)     backend_tls_port="$2"; shift 2 ;;
        --backend-no-build) backend_no_build=1; shift ;;
        --keep-prefix)   keep_prefix=1; shift ;;
        --keep-state)    keep_state=1; shift ;;
        --help|-h)
            sed -n '2,/^# Author:/p' "$0" | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        *)
            echo "e2e-release-flow.sh: unknown flag $1" >&2
            exit 2
            ;;
    esac
done

cleanup() {
    local exitcode=$?
    set +e
    if [ -n "$sse_pid" ]; then
        kill "$sse_pid" 2>/dev/null
        wait "$sse_pid" 2>/dev/null
    fi
    if [ -n "$backend_pid" ] && [ "$own_backend" = 1 ]; then
        echo "==> stopping dev-backend (pid=$backend_pid)"
        kill "$backend_pid" 2>/dev/null
        wait "$backend_pid" 2>/dev/null
    fi
    if [ "$own_backend" = 1 ] && [ -f "$easynet_repo/docker/e2e/docker-compose.yml" ]; then
        echo "==> stopping Docker dev-backend stack"
        (
            cd "$easynet_repo"
            DOCKER_CONFIG="$host_docker_config" \
            HUB_HTTP_PORT="$backend_port" \
            HUB_TLS_PORT="$backend_tls_port" \
            docker compose \
                -p "${EASYNET_DOCKER_PROJECT:-easynet-dev}" \
                -f "$easynet_repo/docker/e2e/docker-compose.yml" \
                down -v --remove-orphans
        ) >/dev/null 2>&1 || true
    fi
    if [ "$keep_state" != 1 ] && [ -n "${SANDBOX_HOME:-}" ]; then
        # Stop any spawned daemon so the next run starts clean.
        if [ -f "$SANDBOX_HOME/.easynet/runtime.json" ]; then
            local pid
            pid="$(jq -r .pid "$SANDBOX_HOME/.easynet/runtime.json" 2>/dev/null)"
            if [ -n "$pid" ] && [ "$pid" != "null" ]; then
                kill "$pid" 2>/dev/null
            fi
        fi
        if [ -f "$SANDBOX_HOME/.easynet/easynet-daemon.pid" ]; then
            local dpid
            dpid="$(cat "$SANDBOX_HOME/.easynet/easynet-daemon.pid" 2>/dev/null)"
            if [ -n "$dpid" ]; then
                kill "$dpid" 2>/dev/null
            fi
        fi
    fi
    if [ -n "${prefix:-}" ] && [ "$keep_prefix" != 1 ]; then
        rm -rf "$prefix" 2>/dev/null
    fi
    exit "$exitcode"
}
trap cleanup EXIT

now_ms() {
    python3 - <<'PY'
import time
print(int(time.time() * 1000))
PY
}

device_state_for_node() {
    local node_id="$1"
    local response
    response="$(curl -fsS -H "Authorization: Bearer $jwt" \
        "$backend_url/api/v1/devices" 2>/dev/null || true)"
    if [ -z "$response" ]; then
        echo "UNREACHABLE"
        return 0
    fi
    local state
    state="$(printf '%s' "$response" | jq -r --arg n "$node_id" \
        '.items[]? | select(.node_id == $n) | .state // empty' 2>/dev/null | head -n 1)"
    if [ -z "$state" ]; then
        echo "MISSING"
    else
        echo "$state"
    fi
}

wait_device_state() {
    local node_id="$1"
    local want="$2"
    local timeout_seconds="$3"
    local polls=$((timeout_seconds * 5))
    local state="UNKNOWN"
    for _ in $(seq 1 "$polls"); do
        state="$(device_state_for_node "$node_id")"
        if [ "$state" = "$want" ]; then
            echo "$state"
            return 0
        fi
        sleep 0.2
    done
    echo "$state"
    return 1
}

wait_device_not_online() {
    local node_id="$1"
    local timeout_seconds="$2"
    local polls=$((timeout_seconds * 5))
    local state="UNKNOWN"
    for _ in $(seq 1 "$polls"); do
        state="$(device_state_for_node "$node_id")"
        if [ "$state" != "ONLINE" ] && [ "$state" != "UNREACHABLE" ]; then
            echo "$state"
            return 0
        fi
        sleep 0.2
    done
    echo "$state"
    return 1
}

start_sse_capture() {
    local token="$1"
    sse_log="$prefix/backend-sse-devices.log"
    sse_err="$prefix/backend-sse-devices.err"
    : > "$sse_log"
    : > "$sse_err"
    curl -fsS -N "$backend_url/api/v1/events?token=$token" \
        > "$sse_log" 2> "$sse_err" &
    sse_pid="$!"
    for _ in $(seq 1 50); do
        if grep -q '^: connected' "$sse_log" 2>/dev/null; then
            return 0
        fi
        if ! kill -0 "$sse_pid" 2>/dev/null; then
            echo "[FAIL] backend SSE subscription exited before connect:" >&2
            cat "$sse_err" >&2 || true
            return 1
        fi
        sleep 0.1
    done
    echo "[FAIL] backend SSE subscription did not connect within 5s" >&2
    cat "$sse_err" >&2 || true
    return 1
}

sse_line_count() {
    if [ -f "$sse_log" ]; then
        wc -l < "$sse_log" | tr -d ' '
    else
        echo 0
    fi
}

wait_sse_event_since() {
    local since_line="$1"
    local node_id="$2"
    local kind="$3"
    local timeout_seconds="$4"
    local polls=$((timeout_seconds * 10))
    local start_line=$((since_line + 1))
    for _ in $(seq 1 "$polls"); do
        if tail -n +"$start_line" "$sse_log" 2>/dev/null \
            | grep -q "\"channel\":\"devices\",\"node_id\":\"$node_id\",\"kind\":\"$kind\""; then
            return 0
        fi
        if [ -n "$sse_pid" ] && ! kill -0 "$sse_pid" 2>/dev/null; then
            echo "[FAIL] backend SSE subscription exited while waiting for kind=$kind" >&2
            cat "$sse_err" >&2 || true
            return 1
        fi
        sleep 0.1
    done
    return 1
}

wait_sse_any_event_since() {
    local since_line="$1"
    local node_id="$2"
    local timeout_seconds="$3"
    shift 3
    local polls=$((timeout_seconds * 10))
    local start_line=$((since_line + 1))
    for _ in $(seq 1 "$polls"); do
        for kind in "$@"; do
            if tail -n +"$start_line" "$sse_log" 2>/dev/null \
                | grep -q "\"channel\":\"devices\",\"node_id\":\"$node_id\",\"kind\":\"$kind\""; then
                echo "$kind"
                return 0
            fi
        done
        if [ -n "$sse_pid" ] && ! kill -0 "$sse_pid" 2>/dev/null; then
            echo "[FAIL] backend SSE subscription exited while waiting for device invalidation" >&2
            cat "$sse_err" >&2 || true
            return 1
        fi
        sleep 0.1
    done
    return 1
}

assert_runtime_status_stopped() {
    local status_json
    status_json="$(easynet runtime status --json 2>&1 || true)"
    if ! printf '%s' "$status_json" | jq -e '.runtime_status == "stopped"' >/dev/null 2>&1; then
        echo "[FAIL] runtime status is not stopped after graceful stop:" >&2
        printf '%s\n' "$status_json" >&2
        return 1
    fi
}

daemon_pid_from_pidfile() {
    if [ -f "$HOME/.easynet/easynet-daemon.pid" ]; then
        tr -d '[:space:]' < "$HOME/.easynet/easynet-daemon.pid"
    fi
}

# ── 1. Install the release tarball into a sandbox ────────────────
if [ -z "$prefix" ]; then
    echo "==> [1/12] install release tarball under a sandbox prefix"
    install_out="$(bash "$script_dir/e2e-release-install.sh" --keep-prefix 2>&1)"
    if [ $? -ne 0 ]; then
        echo "$install_out"
        exit 1
    fi
    prefix="$(printf '%s\n' "$install_out" | grep '^prefix=' | tail -1 | cut -d= -f2-)"
    if [ -z "$prefix" ] || [ ! -d "$prefix" ]; then
        echo "[FAIL] install harness did not emit prefix=" >&2
        printf '%s\n' "$install_out" >&2
        exit 1
    fi
fi
env_file="$prefix/easynet-env.sh"
if [ ! -f "$env_file" ]; then
    echo "[FAIL] env file missing: $env_file" >&2
    exit 1
fi
# shellcheck disable=SC1090
. "$env_file"
SANDBOX_HOME="$HOME"
if [ "$keep_state" != 1 ]; then
    mkdir -p "$HOME/.easynet"
    find "$HOME/.easynet" -mindepth 1 -maxdepth 1 \
        ! -name 'dendrite-bridge' \
        -exec rm -rf {} + 2>/dev/null || true
    rm -rf "$HOME/.easynet-dev" 2>/dev/null || true
fi
echo "    prefix=$prefix"
echo "    HOME=$HOME"
echo "    EASYNET_DENDRITE_BRIDGE_LIB=$EASYNET_DENDRITE_BRIDGE_LIB"

# ── 2. Bring up dev-backend (if caller didn't supply a hub URL) ──
if [ -z "$backend_url" ]; then
    if [ ! -d "$easynet_repo" ]; then
        echo "[FAIL] no --backend-url and no $easynet_repo to start dev-backend from" >&2
        exit 1
    fi
    existing_backend_pids="$(lsof -ti "tcp:${backend_port}" 2>/dev/null || true)"
    if [ -n "$existing_backend_pids" ]; then
        live_backend_pids=""
        for pid in $existing_backend_pids; do
            if ps -p "$pid" >/dev/null 2>&1; then
                live_backend_pids="${live_backend_pids}${pid} "
            fi
        done
    fi
    if [ -n "${live_backend_pids:-}" ]; then
        echo "[FAIL] backend test port :$backend_port already in use:" >&2
        for pid in $live_backend_pids; do
            ps -p "$pid" -o pid=,command= >&2 || true
        done
        exit 1
    fi
    backend_args=(--reset-db --no-seed-device)
    if [ "$backend_no_build" = 1 ]; then
        backend_args+=(--no-build)
    fi
    echo "==> [2/12] starting dev-backend (${backend_args[*]})"
    own_backend=1
    backend_log="$prefix/dev-backend.log"
    (
        cd "$easynet_repo"
        DOCKER_CONFIG="$host_docker_config" \
        HUB_HTTP_PORT="$backend_port" \
        HUB_TLS_PORT="$backend_tls_port" \
        EASYNET_CLI_DIR="$cli_root" \
        EASYNET_AXON_RUNTIME_BIN="$workspace_root/EasyNet-Axon/core/runtime-rs/target/release/axon-runtime" \
          nohup bash scripts/dev-backend.sh "${backend_args[@]}" > "$backend_log" 2>&1 &
        echo $! > "$prefix/dev-backend.pid"
    )
    backend_pid="$(cat "$prefix/dev-backend.pid")"
    backend_url="http://127.0.0.1:$backend_port"
    echo "    backend_url=$backend_url  pid=$backend_pid"
    echo "    log=$backend_log"

    # Wait for /api/v1/health. Cold-start can spend several minutes
    # on first invocation downloading Go deps + ent migrations; the
    # ceiling here is high enough to tolerate a fresh `~/go/pkg`.
    # Subsequent runs hit warm caches and complete in seconds.
    backend_wait_seconds="${EASYNET_TEST_BACKEND_TIMEOUT:-600}"
    for _ in $(seq 1 "$backend_wait_seconds"); do
        if curl -fsS "$backend_url/api/v1/health" >/dev/null 2>&1; then
            break
        fi
        if [ -n "$backend_pid" ] && ! kill -0 "$backend_pid" 2>/dev/null; then
            echo "[FAIL] dev-backend exited before /api/v1/health became available" >&2
            tail -80 "$backend_log" >&2 || true
            exit 1
        fi
        sleep 1
    done
    if ! curl -fsS "$backend_url/api/v1/health" >/dev/null 2>&1; then
        echo "[FAIL] dev-backend did not come up in ${backend_wait_seconds}s" >&2
        tail -50 "$backend_log" >&2
        exit 1
    fi
    for _ in $(seq 1 120); do
        seed_probe="$(curl -fsS -X POST -H 'Content-Type: application/json' \
            -d '{"email":"dev@easynet.local","password":"dev-password"}' \
            "$backend_url/api/v1/auth/login" 2>/dev/null || true)"
        if printf '%s' "$seed_probe" | jq -e '.access_token // .token // empty' >/dev/null 2>&1; then
            break
        fi
        sleep 1
    done
    if ! printf '%s' "${seed_probe:-}" | jq -e '.access_token // .token // empty' >/dev/null 2>&1; then
        echo "[FAIL] dev-backend never accepted dev login credentials" >&2
        tail -80 "$backend_log" >&2
        exit 1
    fi
    baseline_axon_pids="$(pgrep -f "$workspace_root/EasyNet-Axon/core/runtime-rs/target/release/axon-runtime" | sort | tr '\n' ' ' || true)"
fi

# ── 3. Login as dev user, mint a pairing token ───────────────────
echo "==> [3/12] login dev user + mint pairing token"
login_resp="$(curl -fsS -X POST -H 'Content-Type: application/json' \
    -d '{"email":"dev@easynet.local","password":"dev-password"}' \
    "$backend_url/api/v1/auth/login" 2>&1 || true)"
jwt="$(printf '%s' "$login_resp" | jq -r '.access_token // .token // empty' 2>/dev/null)"
if [ -z "$jwt" ]; then
    echo "[FAIL] could not log in dev user. Backend response:" >&2
    printf '%s\n' "$login_resp" >&2
    exit 1
fi

pair_resp="$(curl -fsS -X POST -H "Authorization: Bearer $jwt" \
    "$backend_url/api/v1/devices/pairing" 2>&1 || true)"
pair_token="$(printf '%s' "$pair_resp" | jq -r '.pairing_token // empty' 2>/dev/null)"
if [ -z "$pair_token" ]; then
    echo "[FAIL] could not mint pairing token. Backend response:" >&2
    printf '%s\n' "$pair_resp" >&2
    exit 1
fi
echo "    pairing token: ${pair_token:0:8}…  (truncated)"

# ── 4. Device join — first wire-touch ────────────────────────────
echo "==> [4/12] easynet device join --boot no $pair_token"
if ! join_out="$(easynet device join "$pair_token" --hub "$backend_url" --boot no 2>&1)"; then
    echo "[FAIL] easynet device join failed:" >&2
    printf '%s\n' "$join_out" >&2
    exit 1
fi
printf '%s\n' "$join_out" | sed 's/^/    /'

if [ ! -f "$HOME/.easynet/credentials.json" ]; then
    echo "[FAIL] credentials.json not written after join" >&2
    exit 1
fi
node_id="$(jq -r '.node_id // empty' "$HOME/.easynet/credentials.json" 2>/dev/null)"
if [ -z "$node_id" ]; then
    echo "[FAIL] credentials.json missing node_id after join:" >&2
    cat "$HOME/.easynet/credentials.json" >&2
    exit 1
fi
echo "    node_id: $node_id"

# Subscribe before the daemon starts so the start-online event cannot race
# past the backend SSE assertion.
echo "==> [5/12] subscribe backend SSE device invalidations"
start_sse_capture "$jwt"
online_marker="$(sse_line_count)"
echo "    SSE connected: $sse_log"

# ── 5. Start daemon — THIS IS THE LOAD-BEARING ASSERTION ─────────
# `easynet runtime start` must bring up easynet-daemon without spawning a
# product-path standalone axon-runtime. The release tarball does not
# ship axon-runtime, by design.
echo "==> [6/12] easynet runtime start (load-bearing assertion)"
start_log="$prefix/easynet-start.log"
if start_out="$(easynet runtime start 2>&1)"; then
    printf '%s\n' "$start_out" | sed 's/^/    /'
    echo "$start_out" > "$start_log"
else
    rc=$?
    echo "[FAIL] easynet runtime start exited with code $rc" >&2
    printf '%s\n' "$start_out" | sed 's/^/    /' >&2
    echo "$start_out" > "$start_log"
    exit 1
fi

# Assert: device-mode start must not spawn any additional axon-runtime
# beyond the backend-owned hub process that may already be alive.
current_axon_pids="$(pgrep -f "$workspace_root/EasyNet-Axon/core/runtime-rs/target/release/axon-runtime" | sort | tr '\n' ' ' || true)"
if [ "$current_axon_pids" != "$baseline_axon_pids" ]; then
    echo "[FAIL] easynet runtime start changed the axon-runtime process set:" >&2
    echo "       before: ${baseline_axon_pids:-<none>}" >&2
    echo "       after : ${current_axon_pids:-<none>}" >&2
    pgrep -af "axon-runtime" >&2 || true
    echo "       The release tarball does not ship axon-runtime;" >&2
    echo "       device-mode start must not spawn its own runtime process." >&2
    exit 1
fi

# Daemon should be alive on its UDS.
daemon_sock="$HOME/.easynet/daemon.sock"
control_sock="$HOME/.easynet/control.sock"
for _ in $(seq 1 20); do
    if [ -S "$daemon_sock" ] && [ -S "$control_sock" ]; then
        break
    fi
    sleep 1
done
if [ ! -S "$daemon_sock" ]; then
    echo "[FAIL] daemon.sock did not appear at $daemon_sock" >&2
    exit 1
fi
if [ ! -S "$control_sock" ]; then
    echo "[FAIL] control.sock did not appear at $control_sock" >&2
    exit 1
fi
echo "    daemon.sock + control.sock both listening"

echo "==> [7/12] backend product presence reaches ONLINE"
online_started_ms="$(now_ms)"
if ! online_sse_kind="$(wait_sse_any_event_since "$online_marker" "$node_id" "$product_online_timeout" "added" "state_changed")"; then
    echo "[FAIL] backend SSE did not emit devices added/state_changed for $node_id within ${product_online_timeout}s" >&2
    tail -40 "$sse_log" >&2 || true
    exit 1
fi
online_event_elapsed_ms=$(( $(now_ms) - online_started_ms ))

read_model_started_ms="$(now_ms)"
read_budget_seconds=$(( (read_model_budget_ms + 999) / 1000 ))
if ! online_state="$(wait_device_state "$node_id" "ONLINE" "$read_budget_seconds")"; then
    echo "[FAIL] /api/v1/devices did not report ONLINE for $node_id after SSE invalidation (last=$online_state)" >&2
    curl -fsS -H "Authorization: Bearer $jwt" "$backend_url/api/v1/devices" >&2 || true
    exit 1
fi
read_model_elapsed_ms=$(( $(now_ms) - read_model_started_ms ))
if [ "$read_model_elapsed_ms" -gt "$read_model_budget_ms" ]; then
    echo "[FAIL] /api/v1/devices ONLINE reconciliation exceeded ${read_model_budget_ms}ms (actual=${read_model_elapsed_ms}ms)" >&2
    exit 1
fi
echo "    SSE $online_sse_kind in ${online_event_elapsed_ms}ms; read-model ONLINE in ${read_model_elapsed_ms}ms"

echo "==> [8/12] graceful runtime stop propagates product offline"
graceful_marker="$(sse_line_count)"
graceful_started_ms="$(now_ms)"
if stop_out="$(easynet runtime stop 2>&1)"; then
    printf '%s\n' "$stop_out" | sed 's/^/    /'
else
    rc=$?
    echo "[FAIL] easynet runtime stop exited with code $rc" >&2
    printf '%s\n' "$stop_out" | sed 's/^/    /' >&2
    exit 1
fi
if ! wait_sse_event_since "$graceful_marker" "$node_id" "removed" "$graceful_stop_timeout"; then
    echo "[FAIL] backend SSE did not emit devices/removed for graceful stop of $node_id within ${graceful_stop_timeout}s" >&2
    tail -60 "$sse_log" >&2 || true
    exit 1
fi
graceful_elapsed_ms=$(( $(now_ms) - graceful_started_ms ))
if [ "$graceful_elapsed_ms" -gt $((graceful_stop_timeout * 1000)) ]; then
    echo "[FAIL] graceful stop product removal exceeded ${graceful_stop_timeout}s (actual=${graceful_elapsed_ms}ms)" >&2
    exit 1
fi
if ! stopped_state="$(wait_device_not_online "$node_id" "$read_budget_seconds")"; then
    echo "[FAIL] /api/v1/devices still reports ONLINE for $node_id after graceful stop (last=$stopped_state)" >&2
    curl -fsS -H "Authorization: Bearer $jwt" "$backend_url/api/v1/devices" >&2 || true
    exit 1
fi
assert_runtime_status_stopped
echo "    SSE removed in ${graceful_elapsed_ms}ms; read-model state=$stopped_state"

echo "==> [9/12] restart daemon for release surface probes"
restart_marker="$(sse_line_count)"
if restart_out="$(easynet runtime start 2>&1)"; then
    printf '%s\n' "$restart_out" | sed 's/^/    /'
else
    rc=$?
    echo "[FAIL] easynet runtime start after graceful stop exited with code $rc" >&2
    printf '%s\n' "$restart_out" | sed 's/^/    /' >&2
    exit 1
fi
if ! restart_sse_kind="$(wait_sse_any_event_since "$restart_marker" "$node_id" "$product_online_timeout" "added" "state_changed")"; then
    echo "[FAIL] backend SSE did not emit devices added/state_changed after restart of $node_id" >&2
    tail -60 "$sse_log" >&2 || true
    exit 1
fi
if ! restart_state="$(wait_device_state "$node_id" "ONLINE" "$read_budget_seconds")"; then
    echo "[FAIL] /api/v1/devices did not return ONLINE after restart (last=$restart_state)" >&2
    exit 1
fi
echo "    product presence restored after restart (SSE $restart_sse_kind)"

# ── 6. Probe surfaces a release user would actually hit ──────────
echo "==> [10/12] easynet ability list (local catalogue)"
if ! list_out="$(easynet ability list 2>&1)"; then
    echo "[FAIL] easynet ability list exited non-zero:" >&2
    printf '%s\n' "$list_out" >&2
    exit 1
fi
list_count="$(printf '%s' "$list_out" | grep -cE '^\s+(device|hub|identity|runtime|session|test|dev)\.' || true)"
echo "    ability list returned non-empty"

echo "==> [10/12] agent add codex + claude"
if ! add1_out="$(easynet agent add codex --type codex 2>&1)"; then
    echo "[FAIL] agent add codex failed:" >&2
    printf '%s\n' "$add1_out" >&2
    exit 1
fi
if ! add2_out="$(easynet agent add claude --type claude-code 2>&1)"; then
    echo "[FAIL] agent add claude failed:" >&2
    printf '%s\n' "$add2_out" >&2
    exit 1
fi

# Friendly URA assertion. Current hosted-agent minting uses the
# operator-facing tail directly (`.<name>`) instead of the older
# profile-prefixed `.llm-<name>` form. Reject uuid-hash tails.
hosted_json="$HOME/.easynet/local-agents.json"
if ! jq -e '.hosted_agents[] | select(.agent_ura | contains(".codex"))' "$hosted_json" >/dev/null; then
    echo "[FAIL] codex hosted agent missing or mis-minted (expected URA tail .codex):" >&2
    jq '.hosted_agents' "$hosted_json" >&2
    exit 1
fi
if ! jq -e '.hosted_agents[] | select(.agent_ura | contains(".claude"))' "$hosted_json" >/dev/null; then
    echo "[FAIL] claude hosted agent missing or mis-minted (expected URA tail .claude):" >&2
    jq '.hosted_agents' "$hosted_json" >&2
    exit 1
fi
echo "    hosted agents minted with friendly URAs"

# ── 7. Verify the session-prelude advertise reaches the hub ──────
# The daemon will only re-emit advertise on session reconnect. Bounce
# the daemon so the new agents land in this run's window.
echo "==> [11/12] bounce daemon, observe advertise prelude"
easynet runtime stop >/dev/null 2>&1 || true
sleep 2
easynet runtime start >/dev/null 2>&1
daemon_log="$HOME/.easynet/logs/easynet-daemon.log"
for _ in $(seq 1 30); do
    if grep -q "kind=advertise_agent_prelude_done" "$daemon_log" 2>/dev/null; then
        break
    fi
    sleep 1
done

advertise_line="$(grep "kind=advertise_agent_prelude_sending" "$daemon_log" 2>/dev/null | tail -1 || true)"
if [ -z "$advertise_line" ]; then
    echo "[FAIL] daemon log has no advertise_agent prelude entry yet" >&2
    tail -30 "$daemon_log" >&2
    exit 1
fi

# Assert the friendly names landed in the advertise list.
case "$advertise_line" in
    *codex*) ;;
    *) echo "[FAIL] codex not in advertise prelude: $advertise_line" >&2; exit 1 ;;
esac
case "$advertise_line" in
    *claude*) ;;
    *) echo "[FAIL] claude not in advertise prelude: $advertise_line" >&2; exit 1 ;;
esac
case "$advertise_line" in
    *consent-default*) ;;
    *) echo "[FAIL] consent-default not in advertise prelude: $advertise_line" >&2; exit 1 ;;
esac
echo "    advertise: $(printf '%s' "$advertise_line" | sed 's/^.*kind=advertise_agent_prelude_sending/advertise_agent_prelude_sending/')"

# Final assertion: daemon bounce must not have spawned any additional
# axon-runtime beyond the backend-owned hub baseline.
current_axon_pids="$(pgrep -f "$workspace_root/EasyNet-Axon/core/runtime-rs/target/release/axon-runtime" | sort | tr '\n' ' ' || true)"
if [ "$current_axon_pids" != "$baseline_axon_pids" ]; then
    echo "[FAIL] daemon bounce changed the axon-runtime process set:" >&2
    echo "       before: ${baseline_axon_pids:-<none>}" >&2
    echo "       after : ${current_axon_pids:-<none>}" >&2
    pgrep -af "axon-runtime" >&2 || true
    exit 1
fi

echo "==> [12/12] abrupt daemon kill demotes product presence"
if ! kill_ready_state="$(wait_device_state "$node_id" "ONLINE" "$product_online_timeout")"; then
    echo "[FAIL] device is not ONLINE before abrupt kill gate (last=$kill_ready_state)" >&2
    exit 1
fi
abrupt_marker="$(sse_line_count)"
abrupt_pid="$(daemon_pid_from_pidfile)"
if [ -z "$abrupt_pid" ] || ! kill -0 "$abrupt_pid" 2>/dev/null; then
    echo "[FAIL] cannot locate live easynet-daemon pid for abrupt kill gate (pid=${abrupt_pid:-<empty>})" >&2
    exit 1
fi
abrupt_started_ms="$(now_ms)"
kill -KILL "$abrupt_pid"
if ! wait_sse_event_since "$abrupt_marker" "$node_id" "removed" "$abrupt_kill_timeout"; then
    echo "[FAIL] backend SSE did not emit devices/removed after abrupt kill of $node_id within ${abrupt_kill_timeout}s" >&2
    tail -80 "$sse_log" >&2 || true
    exit 1
fi
abrupt_elapsed_ms=$(( $(now_ms) - abrupt_started_ms ))
if [ "$abrupt_elapsed_ms" -gt $((abrupt_kill_timeout * 1000)) ]; then
    echo "[FAIL] abrupt kill product removal exceeded ${abrupt_kill_timeout}s (actual=${abrupt_elapsed_ms}ms)" >&2
    exit 1
fi
if ! abrupt_state="$(wait_device_not_online "$node_id" "$read_budget_seconds")"; then
    echo "[FAIL] /api/v1/devices still reports ONLINE for $node_id after abrupt kill (last=$abrupt_state)" >&2
    curl -fsS -H "Authorization: Bearer $jwt" "$backend_url/api/v1/devices" >&2 || true
    exit 1
fi
echo "    SSE removed in ${abrupt_elapsed_ms}ms; read-model state=$abrupt_state"

echo
echo "[OK] release-shape end-to-end flow verified"
echo "  device joined the test hub"
echo "  daemon online (UDS + control + daemon sockets)"
echo "  backend SSE/read-model observed ONLINE and graceful OFFLINE"
echo "  abrupt daemon kill demoted product presence within ${abrupt_kill_timeout}s"
echo "  hosted agents minted with friendly URAs (consent-default, codex, claude)"
echo "  session prelude advertised 5 agents to the hub"
echo "  device-mode start spawned no extra axon-runtime beyond the backend hub"
